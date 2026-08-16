use crate::{
    application::{
        admin::ip_mass_delete::CIpMassDeleteService,
        email_domain_block::CEmailDomainBlockService,
        geo_location::CGeoLocationService,
        warning::{
            CWarningService, EnCreateWarningOutcome, StCreateWarningCommand, StWarningPresentation,
        },
    },
    auth::CurrentUser,
    domain::{
        admin::ip_mass_delete::{StIpBanCommand, StIpMassDeleteActor, StIpMassDeleteCommand},
        comment::deletion::TrCommentReindexQueue,
        topic::options::TrTopicReindexQueue,
    },
    error::{AppError, Result},
    infra::{
        postgres::{
            email_domain_block_repository::CEmailDomainBlockPgRepository,
            ip_mass_delete_repository::CIpMassDeletePgRepository,
            warning_repository::CWarningPgRepository,
        },
        search_queue::CSearchQueueSender,
    },
    request_timezone::stRequestTimezone,
    state::AppState,
};
use askama::Template;
use axum::{
    Form, Json, Router,
    extract::{Query, RawQuery, Request, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Months, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;

use super::{any, auto};

static SAME_IP_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\d+\.\d+\.\d+\.\d+$").expect("ip regex"));

fn dtAddCalendarMonths(dtNow: DateTime<Utc>, iMonths: u32) -> Result<DateTime<Utc>> {
    dtNow
        .checked_add_months(Months::new(iMonths))
        .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("ban expiry is out of range")))
}

type TyIpBlockRow = (
    chrono::DateTime<chrono::Utc>,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<String>,
    bool,
    bool,
    i32,
);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/geoip", get(geoip))
        .route("/admin/email-domains", any(email_domains))
        .route("/admin/email-domains/add", auto(post(email_domains_add)))
        .route(
            "/admin/email-domains/delete",
            auto(post(email_domains_delete)),
        )
        .route(
            "/admin/search-reindex",
            get(search_reindex_form).post(search_reindex),
        )
        .route("/banip.jsp", auto(post(ban_ip)))
        .route("/delip.jsp", auto(post(del_ip)))
        .route("/sameip.jsp", any(same_ip))
        .route(
            "/groupmod.jsp",
            auto(get(groupmod_form).post(groupmod_save)),
        )
        // Java has POST-only parameter-conditioned mappings. Its configured
        // method-not-supported resolver deliberately turns GET/HEAD into 404.
        .route("/usermod.jsp", get(usermod_get).post(usermod))
        .route(
            "/post-warning",
            auto(get(post_warning_form).post(post_warning)),
        )
        .route("/clear-warning", auto(post(clear_warning)))
}

fn require_moderator(
    user: &Option<crate::models::UserSummary>,
) -> Result<&crate::models::UserSummary> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    if user.canmod {
        Ok(user)
    } else {
        Err(AppError::Forbidden)
    }
}

fn require_admin(user: &Option<crate::models::UserSummary>) -> Result<&crate::models::UserSummary> {
    // AdministratorOnly in Java gates on currentUser.administrator, which is
    // the `candel` flag (a strict superset of `canmod`/ModeratorOnly).
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    if user.candel {
        Ok(user)
    } else {
        Err(AppError::Forbidden)
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct StEmailDomainsQuery {
    #[serde(default)]
    pub offset: i32,
}

#[derive(Debug, Deserialize)]
pub struct StEmailDomainForm {
    pub domain: String,
}

#[derive(Debug)]
struct StPreparedEmailDomainBlock {
    domain: String,
    block_until: String,
    block_until_iso: String,
    moderator_nick: Option<String>,
    blocked_at: String,
    blocked_at_iso: String,
}

#[derive(Template)]
#[template(path = "email_domains.html")]
struct StEmailDomainsTemplate {
    blocks: Vec<StPreparedEmailDomainBlock>,
    csrf_token: String,
    has_previous: bool,
    previous_offset: i32,
    has_more: bool,
    next_offset: i32,
}

fn stPreparedEmailDomainBlock(
    stBlock: crate::domain::email_domain_block::model::StEmailDomainBlock,
    stTimezone: chrono_tz::Tz,
) -> StPreparedEmailDomainBlock {
    fn sIsoMoscow(dtValue: DateTime<Utc>) -> String {
        dtValue
            .with_timezone(&chrono_tz::Europe::Moscow)
            .to_rfc3339()
    }

    StPreparedEmailDomainBlock {
        domain: stBlock.sDomain,
        block_until: stBlock
            .dtBlockUntil
            .with_timezone(&stTimezone)
            .format("%d.%m.%y %H:%M:%S %Z")
            .to_string(),
        block_until_iso: sIsoMoscow(stBlock.dtBlockUntil),
        moderator_nick: stBlock.optModeratorNick,
        blocked_at: stBlock
            .dtBlockedAt
            .with_timezone(&stTimezone)
            .format("%d.%m.%y %H:%M:%S %Z")
            .to_string(),
        blocked_at_iso: sIsoMoscow(stBlock.dtBlockedAt),
    }
}

async fn email_domains(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Query(stQuery): Query<StEmailDomainsQuery>,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    stJar: CookieJar,
) -> Result<Html<String>> {
    let stModerator = require_moderator(&optUser)?;
    let cService =
        CEmailDomainBlockService::new(CEmailDomainBlockPgRepository::new(stState.pool.clone()));
    let stPage = cService
        .stListManual(stModerator.id, stQuery.offset)
        .await?;
    let stTimezone = stRequestTimezone(&stJar);
    let vecBlocks = stPage
        .vecBlocks
        .into_iter()
        .map(|stBlock| stPreparedEmailDomainBlock(stBlock, stTimezone))
        .collect();

    Ok(Html(
        StEmailDomainsTemplate {
            blocks: vecBlocks,
            csrf_token: sCsrfToken,
            has_previous: stPage.iOffset != 0,
            previous_offset: stPage.iOffset - stPage.iLimit,
            has_more: stPage.bHasMore,
            next_offset: stPage.iOffset + stPage.iLimit,
        }
        .render()?,
    ))
}

async fn email_domains_add(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Form(stForm): Form<StEmailDomainForm>,
) -> Result<Response> {
    let stModerator = require_moderator(&optUser)?;
    let cService =
        CEmailDomainBlockService::new(CEmailDomainBlockPgRepository::new(stState.pool.clone()));
    cService
        .vBlockManual(&stForm.domain, stModerator.id)
        .await?;
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, "/admin/email-domains")],
    )
        .into_response())
}

async fn email_domains_delete(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Form(stForm): Form<StEmailDomainForm>,
) -> Result<Response> {
    require_moderator(&optUser)?;
    let cService =
        CEmailDomainBlockService::new(CEmailDomainBlockPgRepository::new(stState.pool.clone()));
    cService.vUnblock(&stForm.domain).await?;
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, "/admin/email-domains")],
    )
        .into_response())
}

#[derive(Deserialize)]
pub struct GeoIpQuery {
    pub ip: String,
}

async fn geoip(
    State(stState): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<GeoIpQuery>,
) -> Result<Json<serde_json::Value>> {
    require_moderator(&user)?;
    let cService = CGeoLocationService::new(stState.http.clone());
    let stLocation = match cService.stGetLocation(&q.ip).await {
        Ok(stLocation) => stLocation,
        Err(sError) => {
            tracing::warn!(ip = %q.ip, error = %sError, "IP geolocation request failed");
            return Ok(Json(json!({"error": sError})));
        }
    };
    Ok(Json(json!({
        "country": stLocation.optCountry.unwrap_or_default(),
        "region": stLocation.optRegion.unwrap_or_default(),
        "city": stLocation.optCity.unwrap_or_default(),
        "org": stLocation.optOrganization.unwrap_or_default(),
    })))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnSearchReindexAction {
    All,
    Current,
}

fn enSearchReindexAction(optAction: Option<&str>) -> Result<EnSearchReindexAction> {
    match optAction {
        Some("all") => Ok(EnSearchReindexAction::All),
        Some("current") => Ok(EnSearchReindexAction::Current),
        // Spring has two parameter-conditioned POST mappings. A missing or
        // different action matches neither controller method and is a 404.
        _ => Err(AppError::NotFound),
    }
}

#[derive(Template)]
#[template(path = "search_reindex.html")]
struct StSearchReindexTemplate {
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "action_done.html")]
struct StActionDoneTemplate {
    message: String,
    big_message: Option<String>,
    link: Option<String>,
}

async fn search_reindex_form(
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    require_admin(&user)?;
    Ok(Html(StSearchReindexTemplate { csrf_token }.render()?))
}

async fn search_reindex(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    stRequest: Request,
) -> Result<Html<String>> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    // Spring mapping conditions run before CSRF and authorization. Missing
    // or unknown actions therefore remain a no-handler 404 even without a
    // token, rather than being converted into a middleware 403.
    let enAction = enSearchReindexAction(crate::form::get(&vecParameters, "action"))?;
    if !crate::csrf::bServletCsrfValid(&vecParameters, &sCsrfToken) {
        return Err(AppError::Forbidden);
    }
    require_admin(&user)?;
    let sMessage = match enAction {
        EnSearchReindexAction::All => {
            crate::search_index::vScheduleAllReindex(state)
                .await
                .map_err(|stError| AppError::Anyhow(anyhow::anyhow!(stError)))?;
            "Scheduled reindex"
        }
        EnSearchReindexAction::Current => {
            crate::search_index::vScheduleCurrentReindex(state)
                .await
                .map_err(|stError| AppError::Anyhow(anyhow::anyhow!(stError)))?;
            "Scheduled reindex last 3 month"
        }
    };

    Ok(Html(
        StActionDoneTemplate {
            message: sMessage.to_string(),
            big_message: None,
            link: None,
        }
        .render()?,
    ))
}

#[derive(Deserialize)]
pub struct BanIpForm {
    pub ip: String,
    pub reason: String,
    /// hour/day/month/3month/6month/custom/unlim/remove.
    pub time: String,
    pub ban_days: Option<String>,
    #[serde(default)]
    pub allow_posting: bool,
    #[serde(default)]
    pub captcha_required: bool,
}

fn optBanIpUntil(
    sTime: &str,
    optBanDays: Option<&str>,
    dtNow: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>> {
    match sTime {
        "hour" => Ok(Some(dtNow + chrono::Duration::hours(1))),
        "day" => Ok(Some(dtNow + chrono::Duration::days(1))),
        "month" => Ok(Some(dtAddCalendarMonths(dtNow, 1)?)),
        "3month" => Ok(Some(dtAddCalendarMonths(dtNow, 3)?)),
        "6month" => Ok(Some(dtAddCalendarMonths(dtNow, 6)?)),
        "custom" => {
            // ServletRequestUtils.getRequiredIntParameter throws an ordinary
            // ServletRequestBindingException for a missing/non-integer value;
            // unlike MissingServletRequestParameterException it is not mapped
            // to bad-parameter.jsp and therefore remains a reportable 500.
            let sDays = optBanDays.ok_or_else(|| {
                AppError::Anyhow(anyhow::anyhow!(
                    "Required int parameter 'ban_days' is missing"
                ))
            })?;
            let iDays: i64 = sDays.parse().map_err(|_| {
                AppError::Anyhow(anyhow::anyhow!(
                    "Required int parameter 'ban_days' is invalid"
                ))
            })?;
            if iDays <= 0 || iDays > 180 {
                return Err(AppError::stUserError("Invalid days count"));
            }
            Ok(Some(dtNow + chrono::Duration::days(iDays)))
        }
        "unlim" => Ok(None),
        "remove" => Ok(Some(dtNow)),
        _ => Err(AppError::stUserError("Invalid count")),
    }
}

/// BanIPController.banIP: standalone ban endpoint (distinct from
/// /delip.jsp's mass-delete-then-optionally-ban flow) - was missing
/// `time`/`allow_posting`/`captcha_required` entirely and always banned
/// unconditionally-and-permanently with no duration control.
async fn ban_ip(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<BanIpForm>,
) -> Result<Response> {
    let moderator = require_moderator(&user)?;
    // Java passes the raw value to PostgreSQL's `::inet` cast. This both
    // accepts PostgreSQL network notation and leaves invalid input as an
    // ordinary reportable database failure rather than a user error.
    let dtNow = chrono::Utc::now();
    let ban_to = optBanIpUntil(&form.time, form.ban_days.as_deref(), dtNow)?;
    sqlx::query(
        r#"INSERT INTO b_ips(ip,mod_id,date,reason,ban_date,allow_posting,captcha_required)
           VALUES($1::inet,$2,now(),$3,$4,$5,$6)
           ON CONFLICT(ip) DO UPDATE SET mod_id=EXCLUDED.mod_id, date=now(), reason=EXCLUDED.reason,
             ban_date=EXCLUDED.ban_date, allow_posting=EXCLUDED.allow_posting, captcha_required=EXCLUDED.captcha_required"#,
    )
        .bind(&form.ip)
        .bind(moderator.id)
        .bind(&form.reason)
        .bind(ban_to)
        .bind(form.allow_posting)
        .bind(form.captcha_required)
        .execute(&state.pool)
        .await?;
    Ok(crate::routes::stFoundRedirect(format!(
        "/sameip.jsp?ip={}",
        urlencoding::encode(&form.ip)
    )))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelIpForm {
    pub reason: String,
    pub ip: String,
    /// Deletion look-back window: hour/day/3day/5day.
    pub time: String,
    /// Optional ban duration: hour/day/month/3month/6month/unlim/remove.
    pub ban_time: Option<String>,
    /// Optional ban mode: anonymous_and_captcha/anonymous_only/(anything else = full block).
    pub ban_mode: Option<String>,
}

#[derive(Template)]
#[template(path = "delip.html")]
struct StDelIpTemplate {
    sCutoff: String,
    sIp: String,
    iTopics: usize,
    iComments: usize,
    vecSkipped: Vec<i32>,
}

#[derive(Template)]
#[template(path = "topic_edit_user_error.html")]
struct StDelIpUserErrorTemplate<'a> {
    exception_class: &'static str,
    message: &'a str,
}

fn stBindDelIpForm(vecParameters: &[(String, String)]) -> Result<DelIpForm> {
    let sRequired = |sName: &str| {
        crate::form::get(vecParameters, sName)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                AppError::BadParameter(format!(
                    "Required request parameter '{sName}' for method parameter type String is not present"
                ))
            })
    };
    Ok(DelIpForm {
        reason: sRequired("reason")?,
        ip: sRequired("ip")?,
        time: sRequired("time")?,
        ban_time: crate::form::get(vecParameters, "ban_time").map(ToOwned::to_owned),
        ban_mode: crate::form::get(vecParameters, "ban_mode").map(ToOwned::to_owned),
    })
}

fn stDelIpUserErrorResponse(sMessage: &str) -> Response {
    match (StDelIpUserErrorTemplate {
        exception_class: "ru.org.linux.user.UserErrorException",
        message: sMessage,
    })
    .render()
    {
        Ok(sBody) => (StatusCode::INTERNAL_SERVER_ERROR, Html(sBody)).into_response(),
        Err(stError) => AppError::Template(stError).into_response(),
    }
}

fn optDelIpLookback(sTime: &str) -> Option<chrono::Duration> {
    match sTime {
        "hour" => Some(chrono::Duration::hours(1)),
        "day" => Some(chrono::Duration::days(1)),
        "3day" => Some(chrono::Duration::days(3)),
        "5day" => Some(chrono::Duration::days(5)),
        _ => None,
    }
}

fn optDelIpBanUntil(sBanTime: &str, dtNow: DateTime<Utc>) -> Result<Option<Option<DateTime<Utc>>>> {
    Ok(match sBanTime {
        "hour" => Some(Some(dtNow + chrono::Duration::hours(1))),
        "day" => Some(Some(dtNow + chrono::Duration::days(1))),
        "month" => Some(Some(dtAddCalendarMonths(dtNow, 1)?)),
        "3month" => Some(Some(dtAddCalendarMonths(dtNow, 3)?)),
        "6month" => Some(Some(dtAddCalendarMonths(dtNow, 6)?)),
        "unlim" => Some(None),
        "remove" => Some(Some(dtNow)),
        _ => None,
    })
}

fn sJavaTimestamp(dtValue: DateTime<Utc>) -> String {
    // java.sql.Timestamp.toString uses the process-local timezone and trims
    // insignificant fractional-second zeroes, but always keeps `.0`.
    let dtLocal = dtValue.with_timezone(&chrono::Local);
    let sFraction = format!("{:03}", dtValue.timestamp_subsec_millis());
    let sFraction = sFraction.trim_end_matches('0');
    format!(
        "{}.{}",
        dtLocal.format("%Y-%m-%d %H:%M:%S"),
        if sFraction.is_empty() { "0" } else { sFraction }
    )
}

/// Java's `/delip.jsp` (DelIPController.delIp) mass-deletes topics/comments
/// posted from an IP within a time window and optionally bans the IP - it is
/// NOT an unban endpoint. The previous Rust handler reused this exact
/// URL/method to delete a `b_ips` row (i.e. unban), which is the opposite
/// action: a moderator UI built against the real shape would silently unban
/// instead of cleaning up abuse.
async fn del_ip(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    stRequest: Request,
) -> Result<Response> {
    // Spring resolves all required @RequestParam values before invoking the
    // ModeratorOnly body. Query parameters precede URL-encoded POST fields in
    // the Servlet first-value view.
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let form = stBindDelIpForm(&vecParameters)?;
    let moderator = require_moderator(&user)?;
    let Some(dtLookback) = optDelIpLookback(&form.time) else {
        return Ok(stDelIpUserErrorResponse("Invalid count"));
    };
    // java.sql.Timestamp is built from Instant.toEpochMilli, so sub-millisecond
    // precision is discarded before both SQL binding and result rendering.
    let dtNow = DateTime::<Utc>::from_timestamp_millis(Utc::now().timestamp_millis())
        .expect("current timestamp fits chrono");
    let dtCutoff = dtNow - dtLookback;

    let optBan = if let Some(ban_time) = form.ban_time.as_deref() {
        let dtNow = chrono::Utc::now();
        let optBanUntil = match optDelIpBanUntil(ban_time, dtNow)? {
            Some(optBanUntil) => optBanUntil,
            // Presence and value are separate in @RequestParam: an explicitly
            // empty ban_time is not the same as an absent parameter.
            None => return Ok(stDelIpUserErrorResponse("Invalid count")),
        };
        let (bAllowPosting, bCaptchaRequired) = match form.ban_mode.as_deref() {
            Some("anonymous_and_captcha") => (true, true),
            Some("anonymous_only") => (true, false),
            _ => (false, false),
        };
        Some(StIpBanCommand {
            sIp: form.ip.clone(),
            sReason: form.reason.clone(),
            optBanUntil,
            bAllowPosting,
            bCaptchaRequired,
        })
    } else {
        None
    };

    let cRepository = CIpMassDeletePgRepository::new(state.pool.clone());
    let cQueue = CSearchQueueSender::new(
        state.config.opensearch_url.as_deref(),
        &state.config.upload_dir,
    );
    let cService = CIpMassDeleteService::new(cRepository, cQueue);
    let stResult = cService
        .stExecute(
            StIpMassDeleteActor {
                iUserId: moderator.id,
                bModerator: moderator.canmod,
            },
            StIpMassDeleteCommand {
                sIp: form.ip.clone(),
                dtCutoff,
                sReason: form.reason,
                optBan,
            },
        )
        .await?;

    tracing::info!(
        ip = %form.ip,
        period = %form.time,
        moderator = %moderator.nick,
        topics = stResult.vecDeletedTopicIds.len(),
        comments = stResult.vecDeletedCommentIds.len(),
        "mass-deleted messages by IP"
    );

    Ok(Html(
        StDelIpTemplate {
            sCutoff: sJavaTimestamp(dtCutoff),
            sIp: form.ip,
            iTopics: stResult.vecDeletedTopicIds.len(),
            iComments: stResult.vecDeletedCommentIds.len(),
            vecSkipped: stResult.vecSkippedCommentIds,
        }
        .render()?,
    )
    .into_response())
}

#[cfg(test)]
mod delip_tests {
    use super::{
        StDelIpTemplate, optDelIpBanUntil, optDelIpLookback, sJavaTimestamp, stBindDelIpForm,
        stDelIpUserErrorResponse,
    };
    use crate::error::AppError;
    use askama::Template;
    use axum::{
        body::to_bytes,
        http::{StatusCode, header},
    };
    use chrono::{TimeZone, Utc};

    #[test]
    fn banip_uses_java_redirect_view_found_status() {
        let stResponse = crate::routes::stFoundRedirect("/sameip.jsp?ip=203.0.113.9");
        assert_eq!(stResponse.status(), StatusCode::FOUND);
        assert_eq!(
            stResponse
                .headers()
                .get(header::LOCATION)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("/sameip.jsp?ip=203.0.113.9")
        );

        let sHandler = include_str!("admin.rs")
            .split("async fn ban_ip(")
            .nth(1)
            .unwrap()
            .split("#[derive(Debug, Clone, PartialEq, Eq)]")
            .next()
            .unwrap();
        assert!(sHandler.contains("crate::routes::stFoundRedirect"));
        assert!(!sHandler.contains("Redirect::to"));
    }

    #[test]
    fn required_parameters_bind_before_handler_policy_and_use_query_precedence() {
        let vecParameters = vec![
            ("reason".to_owned(), "query reason".to_owned()),
            ("ip".to_owned(), "203.0.113.9".to_owned()),
            ("time".to_owned(), "hour".to_owned()),
            ("reason".to_owned(), "form reason".to_owned()),
        ];
        let stForm = stBindDelIpForm(&vecParameters).unwrap();
        assert_eq!(stForm.reason, "query reason");
        assert_eq!(stForm.ip, "203.0.113.9");

        let stMissing = stBindDelIpForm(&[("ip".to_owned(), String::new())]);
        assert!(matches!(stMissing, Err(AppError::BadParameter(_))));

        let sProduction = include_str!("admin.rs")
            .split("async fn del_ip(")
            .nth(1)
            .unwrap()
            .split("/// Matches SameIPController")
            .next()
            .unwrap();
        assert!(
            sProduction.find("stBindDelIpForm").unwrap()
                < sProduction.find("require_moderator").unwrap()
        );
    }

    #[test]
    fn time_enums_and_empty_optional_ban_match_user_error_boundaries() {
        assert_eq!(optDelIpLookback("hour"), Some(chrono::Duration::hours(1)));
        assert_eq!(optDelIpLookback("5day"), Some(chrono::Duration::days(5)));
        assert_eq!(optDelIpLookback("Hour"), None);

        let dtNow = Utc.with_ymd_and_hms(2026, 1, 31, 12, 0, 0).unwrap();
        assert_eq!(optDelIpBanUntil("", dtNow).unwrap(), None);
        assert_eq!(optDelIpBanUntil("unknown", dtNow).unwrap(), None);
        assert_eq!(optDelIpBanUntil("unlim", dtNow).unwrap(), Some(None));
        assert_eq!(
            optDelIpBanUntil("month", dtNow).unwrap().flatten(),
            Some(Utc.with_ymd_and_hms(2026, 2, 28, 12, 0, 0).unwrap())
        );
    }

    #[test]
    fn java_timestamp_shape_trims_millisecond_zeroes() {
        let dtValue = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap()
            + chrono::Duration::milliseconds(120);
        assert!(sJavaTimestamp(dtValue).ends_with("05.12"));
        assert!(
            sJavaTimestamp(Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap()).ends_with("05.0")
        );
    }

    #[test]
    fn result_template_preserves_java_dom_and_skipped_comment_links() {
        let sHtml = StDelIpTemplate {
            sCutoff: "2026-01-02 03:04:05.0".to_owned(),
            sIp: "203.0.113.9".to_owned(),
            iTopics: 2,
            iComments: 3,
            vecSkipped: vec![51, 49],
        }
        .render()
        .unwrap();
        assert!(sHtml.contains("<title>delip</title>"));
        assert!(sHtml.contains("rel=\"parent\" title=\"Linux.org.ru\" href=\"/\""));
        assert!(sHtml.contains("Удаление тем и сообщений"));
        assert!(sHtml.contains("Удалено тем: 2; удалено комментариев: 3"));
        assert!(sHtml.contains("delete_comment.jsp?msgid=51\">#51</a>"));
        assert!(sHtml.contains("delete_comment.jsp?msgid=49\">#49</a>"));
        assert!(sHtml.contains("data-lor-theme-stylesheet"));
    }

    #[tokio::test]
    async fn invalid_time_is_visible_user_error_with_java_500_semantics() {
        let stResponse = stDelIpUserErrorResponse("Invalid count");
        assert_eq!(stResponse.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            stResponse
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024).await.unwrap();
        let sBody = String::from_utf8(vecBody.to_vec()).unwrap();
        assert!(sBody.contains("Ошибка: ru.org.linux.user.UserErrorException"));
        assert!(sBody.contains("<h1>Invalid count</h1>"));
    }
}

/// Matches SameIPController: the previous handler only did an exact-IP
/// equality lookup with no mask/UA/score filtering and no matched-user
/// listing or block-info display.
#[derive(Deserialize)]
pub struct SameIpQuery {
    pub ip: Option<String>,
    pub mask: Option<i32>,
    pub ua: Option<i32>,
    pub score: Option<i32>,
}

const SAME_IP_ANONYMOUS_SCORE_FILTER: i32 = -9999;
const SAME_IP_ROWS_LIMIT: i64 = 50;

#[derive(Debug, sqlx::FromRow)]
struct StSameIpPostRow {
    iTopicId: i32,
    optCommentId: Option<i32>,
    sNick: String,
    sGroupTitle: String,
    sTitle: String,
    dtPostDate: DateTime<Utc>,
    bDeleted: bool,
    optDeleteReason: Option<String>,
    sMessage: String,
}

fn sRenderSameIpDeleteForm(sIp: &str, sCsrfToken: &str, bCurrentlyBlocked: bool) -> String {
    let sIp = html_escape::encode_double_quoted_attribute(sIp);
    let sCsrfToken = html_escape::encode_double_quoted_attribute(sCsrfToken);
    let sBanControls = if bCurrentlyBlocked {
        String::new()
    } else {
        r#"и <select name="ban_time" onchange="banTimeChange(this);">
<option value="remove">не блокировать</option><option value="hour">блокировать на 1 час</option><option value="day">блокировать на 1 день</option><option value="month">блокировать на 1 месяц</option><option value="3month">блокировать на 3 месяца</option><option value="6month">блокировать на 6 месяцев</option><option selected value="unlim">блокировать постоянно</option></select>
<label><input type="radio" name="ban_mode" value="anonymous_and_captcha">только anonymous, требовать captcha у зарегистрированных</label>
<label><input checked type="radio" name="ban_mode" value="anonymous_only">только anonymous</label><label><input type="radio" name="ban_mode" value="all">всех</label>"#.to_owned()
    };
    format!(
        r#"<fieldset><legend>Удалить темы и сообщения с IP</legend><form method="post" action="delip.jsp">
<input type="hidden" name="csrf" value="{sCsrfToken}"><input type="hidden" name="ip" value="{sIp}">
по причине: <br><input type="text" name="reason" maxlength="254" size="40" value=""><br>
за последний(ие) <select name="time"><option value="hour">1 час</option><option selected value="day">1 день</option><option value="3day">3 дня</option><option value="5day">5 дней</option></select>
{sBanControls}<p><button type="submit" name="del" class="btn btn-danger">del from ip</button>
<script>function banTimeChange(object){{if($(object).val()==="remove"){{$(object).parent().find("input[name=ban_mode]").parent().hide()}}else{{$(object).parent().find("input[name=ban_mode]").parent().show()}}}}</script>
</form></fieldset>"#
    )
}

fn sRenderSameIpBanForm(
    sIp: &str,
    sCsrfToken: &str,
    bAllowPosting: bool,
    bCaptchaRequired: bool,
) -> String {
    let sIp = html_escape::encode_double_quoted_attribute(sIp);
    let sCsrfToken = html_escape::encode_double_quoted_attribute(sCsrfToken);
    let sAllowChecked = if bAllowPosting { " checked" } else { "" };
    let sCaptchaChecked = if bCaptchaRequired { " checked" } else { "" };
    let sCaptchaDisabled = if bAllowPosting { "" } else { " disabled" };
    format!(
        r#"<h2>Управление</h2><fieldset><legend>забанить/разбанить IP</legend><form method="post" action="banip.jsp">
<input type="hidden" name="csrf" value="{sCsrfToken}"><input type="hidden" name="ip" value="{sIp}">
по причине: <br><input type="text" name="reason" maxlength="254" size="40" value=""><br>
<select name="time" onchange="checkCustomBan(this.selectedIndex);"><option value="hour">1 час</option><option value="day">1 день</option><option value="month">1 месяц</option><option value="3month">3 месяца</option><option value="6month">6 месяцев</option><option value="unlim">постоянно</option><option value="remove">не блокировать</option><option value="custom">указать (дней)</option></select>
<div id="custom_ban" style="display:none;"><br><input type="text" name="ban_days" value=""></div><br>
<label><input id="allowPosting" type="checkbox" name="allow_posting" value="true"{sAllowChecked} onchange="allowPostingOnChange(this);">разрешить постить ранее зарегистрированным</label>
<label><input id="captchaRequired" type="checkbox" name="captcha_required" value="true"{sCaptchaChecked}{sCaptchaDisabled}>требовать ввод каптчи</label>
<p><button type="submit" name="ban" class="btn btn-default">ban ip</button></form></fieldset>
<script>function allowPostingOnChange(object){{var captchaRequired=$('#captchaRequired');if($(object).is(':checked')){{captchaRequired.removeAttr('disabled')}}else{{captchaRequired.attr('disabled','disabled')}}}}function checkCustomBan(idx){{var custom=document.getElementById('custom_ban');if(custom){{custom.style.display=idx===7?'block':'none'}}}}</script>
<div><a href="/admin/email-domains">Блокировка почтовых доменов</a></div>"#
    )
}

fn optSameIpCidr(optIp: Option<&str>, iMask: i32) -> Result<Option<String>> {
    let Some(sIp) = optIp else {
        // SameIPController performs both IP and mask validation inside
        // `Option(ip).flatMap`; a stray mask without an IP is ignored.
        return Ok(None);
    };
    if !Lazy::force(&SAME_IP_RE).is_match(sIp) {
        return Err(AppError::stBadInput("not ip"));
    }
    if !(0..=32).contains(&iMask) {
        return Err(AppError::stBadInput("bad mask"));
    }
    if iMask == 0 {
        Ok(None)
    } else if iMask != 32 {
        Ok(Some(format!("{sIp}/{iMask}")))
    } else {
        Ok(Some(sIp.to_owned()))
    }
}

async fn same_ip(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<SameIpQuery>,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    require_moderator(&user)?;

    let mask = q.mask.unwrap_or(32);
    let ip_cidr = optSameIpCidr(q.ip.as_deref(), mask)?;
    let bExactIp = q.ip.is_some() && mask == 32;
    let optBlock: Option<TyIpBlockRow> = if bExactIp {
        sqlx::query_as(
            "SELECT date, ban_date, reason, allow_posting, captcha_required, mod_id FROM b_ips WHERE ip=$1::inet",
        )
        .bind(q.ip.as_deref().expect("exact IP is present"))
        .fetch_optional(&state.pool)
        .await?
    } else {
        None
    };
    let bCurrentlyBlocked = optBlock
        .as_ref()
        .is_some_and(|(_, optBanDate, _, _, _, _)| {
            optBanDate.is_none_or(|dtBanDate| dtBanDate > Utc::now())
        });
    let optUserAgentName: Option<String> = if let Some(iUa) = q.ua {
        sqlx::query_scalar("SELECT name FROM user_agents WHERE id=$1")
            .bind(iUa)
            .fetch_optional(&state.pool)
            .await?
    } else {
        None
    };

    let mut html = String::from("<h1>Поиск сообщений по метаданным</h1>");
    if q.ua.is_some() {
        html.push_str(&format!(
            "Показаны сообщения с User-Agent:<br>{}",
            html_escape::encode_text(optUserAgentName.as_deref().unwrap_or(""))
        ));
    }
    let sUaHidden = q.ua.map_or_else(String::new, |iUa| {
        format!("<input type=\"hidden\" name=\"ua\" value=\"{iUa}\">")
    });
    let sMaskOptions = [
        (32, "Только IP"),
        (24, "Сеть /24"),
        (23, "Сеть /23"),
        (22, "Сеть /22"),
        (21, "Сеть /21"),
        (16, "Сеть /16"),
        (0, "Любой IP"),
    ]
    .into_iter()
    .map(|(iValue, sLabel)| {
        format!(
            "<option value=\"{iValue}\"{}>{sLabel}</option>",
            if iValue == mask { " selected" } else { "" }
        )
    })
    .collect::<String>();
    let sScoreOptions = [
        (None, "Любой score"),
        (Some(-9999), "anonymous"),
        (Some(46), "score &lt;= 45"),
        (Some(50), "score &lt; 50"),
        (Some(100), "score &lt; 100"),
    ]
    .into_iter()
    .map(|(optValue, sLabel)| {
        format!(
            "<option value=\"{}\"{}>{sLabel}</option>",
            optValue.map_or_else(String::new, |iValue| iValue.to_string()),
            if optValue == q.score { " selected" } else { "" }
        )
    })
    .collect::<String>();
    let sIpControls = q.ip.as_deref().map_or_else(String::new, |sIp| {
        format!(
            r#"<input class="input-lg" name="ip" type="search" size="17" maxlength="17" value="{}" id="ip-field" pattern="[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+">
<select name="mask" class="btn btn-default" onchange="this.form.submit()">{sMaskOptions}</select>"#,
            html_escape::encode_double_quoted_attribute(sIp),
        )
    });
    html.push_str(&format!(
        r#"<form action="sameip.jsp">
{sUaHidden}
{sIpControls}
<select name="score" class="btn btn-default" onchange="this.form.submit()">{sScoreOptions}</select>
 </form>"#,
    ));

    if let Some(sIp) = q.ip.as_deref().filter(|_| bExactIp) {
        html.push_str("<div><strong>Текущий статус: </strong>");
        if optBlock.is_some() {
            html.push_str("адрес заблокирован");
            if !bCurrentlyBlocked {
                html.push_str(" (блокировка истекла)");
            }
        } else {
            html.push_str("адрес не заблокирован");
        }
        html.push_str("</div>");
        html.push_str(&format!(
            r#"<div><strong>Местоположение {} (<a href="https://ipwhois.io" target="_blank">ipwhois.io</a>)</strong>: <span id="geolookup">...</span></div>
<script>$script.ready("jquery",function(){{$.ajax({{method:'GET',url:'/admin/geoip?ip={}',dataType:'json',success:function(json){{if(json.error){{$('#geolookup').text('rejected - '+json.error)}}else{{$('#geolookup').text(json.country+' / '+json.region+' / '+json.city+' ('+json.org+')')}}}}}})}});</script>"#,
            html_escape::encode_text(sIp),
            urlencoding::encode(sIp),
        ));
    }

    if q.score != Some(SAME_IP_ANONYMOUS_SCORE_FILTER) && (ip_cidr.is_some() || q.ua.is_some()) {
        let vecNewUsers =
            sqlx::query_as::<_, (String, bool, bool, DateTime<Utc>, Option<DateTime<Utc>>)>(
                r#"SELECT u.nick,u.blocked,u.activated,u.regdate,u.lastlogin
                 FROM users u JOIN user_log ul ON u.id=ul.userid
                WHERE u.regdate IS NOT NULL AND u.regdate>CURRENT_TIMESTAMP-'3 days'::interval
                  AND ul.action='register'::user_log_action
                  AND ($1::inet IS NULL OR (ul.info->'ip')::inet <<= $1::inet)
                  AND ($2::int IS NULL OR ul.info->'user_agent'=$2::text)
                ORDER BY u.regdate DESC"#,
            )
            .bind(&ip_cidr)
            .bind(q.ua)
            .fetch_all(&state.pool)
            .await?;
        if !vecNewUsers.is_empty() {
            html.push_str("<h2>Новые пользователи за 3 дня</h2><div class=\"forum\"><table width=\"100%\" class=\"message-table\"><thead><tr><th>Nick</th><th>Дата регистрации</th><th>Последнее посещение</th></tr></thead><tbody>");
            for (sNick, bBlocked, bActivated, dtRegDate, optLastLogin) in vecNewUsers {
                html.push_str(&format!(
                    "<tr><td>{}<a href=\"/people/{}/profile\">{}</a>{}</td><td>{}</td><td>{}</td></tr>",
                    if bBlocked { "<s>" } else { "" },
                    urlencoding::encode(&sNick),
                    html_escape::encode_text(&sNick),
                    if bBlocked { "</s>" } else { "" },
                    crate::request_timezone::sTimeTag("default", dtRegDate),
                    if bActivated {
                        optLastLogin.map_or_else(String::new, |dtLastLogin| {
                            crate::request_timezone::sTimeTag("interval", dtLastLogin)
                        })
                    } else {
                        "не активирован".to_owned()
                    },
                ));
            }
            html.push_str("</tbody></table></div>");
        }
    }

    // Matched comments/topics, IP/UA filtered.
    let posts = sqlx::query_as::<_, StSameIpPostRow>(
        r#"SELECT t.id AS "iTopicId",c.id AS "optCommentId",u.nick AS "sNick",
                  g.title AS "sGroupTitle",t.title AS "sTitle",c.postdate AS "dtPostDate",
                  c.deleted AS "bDeleted",di.reason AS "optDeleteReason",mb.message AS "sMessage"
             FROM groups g JOIN topics t ON g.id=t.groupid
             JOIN comments c ON c.topic=t.id JOIN users u ON u.id=c.userid
             JOIN msgbase mb ON mb.id=c.id LEFT JOIN del_info di ON di.msgid=c.id
            WHERE c.postdate>CURRENT_TIMESTAMP-'5 days'::interval
              AND ($1::inet IS NULL OR c.postip <<= $1::inet)
              AND ($2::int IS NULL OR c.ua_id=$2)
              AND ($3::int IS NULL OR c.userid IN (SELECT id FROM users WHERE score<$3 OR id=2))
           UNION ALL
           SELECT t.id AS "iTopicId",NULL::int AS "optCommentId",u.nick AS "sNick",
                  g.title AS "sGroupTitle",t.title AS "sTitle",t.postdate AS "dtPostDate",
                  t.deleted AS "bDeleted",di.reason AS "optDeleteReason",mb.message AS "sMessage"
             FROM groups g JOIN topics t ON g.id=t.groupid JOIN users u ON u.id=t.userid
             JOIN msgbase mb ON mb.id=t.id LEFT JOIN del_info di ON di.msgid=t.id
            WHERE t.postdate>CURRENT_TIMESTAMP-'5 days'::interval
              AND ($1::inet IS NULL OR t.postip <<= $1::inet)
              AND ($2::int IS NULL OR t.ua_id=$2)
              AND ($3::int IS NULL OR t.userid IN (SELECT id FROM users WHERE score<$3 OR id=2))
            ORDER BY "dtPostDate" DESC LIMIT $4"#,
    )
    .bind(&ip_cidr)
    .bind(q.ua)
    .bind(q.score)
    .bind(SAME_IP_ROWS_LIMIT)
    .fetch_all(&state.pool)
    .await?;

    html.push_str(&format!(
        "<h2>Сообщения за 5 дней{}</h2><div class=\"comments\">",
        if posts.len() as i64 == SAME_IP_ROWS_LIMIT {
            format!(" (показаны первые {SAME_IP_ROWS_LIMIT})")
        } else {
            String::new()
        }
    ));
    for stPost in &posts {
        let sDate = crate::request_timezone::sTimeTag("compact-interval", stPost.dtPostDate);
        let sUrl = stPost.optCommentId.map_or_else(
            || format!("jump-message.jsp?msgid={}", stPost.iTopicId),
            |iCommentId| {
                format!(
                    "jump-message.jsp?msgid={}&amp;cid={iCommentId}",
                    stPost.iTopicId
                )
            },
        );
        let sPlain = crate::markup::plain_text_for_index(&stPost.sMessage);
        let sPreview = if sPlain.chars().count() > 250 {
            format!("{}...", sPlain.chars().take(250).collect::<String>().trim())
        } else {
            sPlain
        };
        html.push_str(&format!(
            "<a href=\"{sUrl}\" class=\"comments-item\"><div class=\"comments-group\"><p><span class=\"group-label\">{}</span><br class=\"hideon-phone hideon-tablet\"><a href=\"/people/{}/profile\">{}</a></p></div><div class=\"comments-title\"><div class=\"text-preview-box\"><div class=\"text-preview\">{}{}</div></div></div><div class=\"comments-text\"><div class=\"text-preview-box\"><div class=\"text-preview\">{}{}</div></div>{}</div><div class=\"comments-date\"><p>{sDate}</p></div></a>",
            html_escape::encode_text(&stPost.sGroupTitle),
            urlencoding::encode(&stPost.sNick),
            html_escape::encode_text(&stPost.sNick),
            if stPost.optCommentId.is_some() { "<i class=\"icon-comment\"></i>" } else { "" },
            html_escape::encode_text(&crate::domain::title::sPlainForDisplay(&stPost.sTitle)),
            if stPost.bDeleted { "<s>" } else { "" },
            html_escape::encode_text(&sPreview),
            if stPost.bDeleted {
                format!(
                    "</s><br><img src=\"/img/del.png\" alt=\"[X]\" width=\"15\" height=\"15\"> Удалено по причине: {}",
                    html_escape::encode_text(stPost.optDeleteReason.as_deref().unwrap_or(""))
                )
            } else {
                String::new()
            },
        ));
    }
    html.push_str("</div>");

    if let Some(sIp) =
        q.ip.as_deref()
            .filter(|_| bExactIp && q.score.is_none() && !posts.is_empty())
    {
        html.push_str(&sRenderSameIpDeleteForm(
            sIp,
            &sCsrfToken,
            bCurrentlyBlocked,
        ));
    }

    // Matched users, only meaningful when an ip/ua filter narrows things down
    // and we're not specifically asking for the anonymous-only bucket.
    if q.score != Some(SAME_IP_ANONYMOUS_SCORE_FILTER) && (ip_cidr.is_some() || q.ua.is_some()) {
        let users = sqlx::query_as::<_, (DateTime<Utc>, String, Option<String>, bool)>(
            r#"SELECT MAX(c.postdate) AS lastdate,u.nick,ua.name AS user_agent,u.blocked
                 FROM (SELECT ua_id,userid,postdate,postip FROM comments
                       UNION ALL SELECT ua_id,userid,postdate,postip FROM topics) c
                 LEFT JOIN user_agents ua ON c.ua_id=ua.id JOIN users u ON c.userid=u.id
                WHERE c.postdate>CURRENT_TIMESTAMP-'1 year'::interval
                  AND ($1::inet IS NULL OR c.postip <<= $1::inet)
                  AND ($2::int IS NULL OR c.ua_id=$2)
                GROUP BY u.nick,u.blocked,c.ua_id,ua.name
                ORDER BY MAX(c.postdate) DESC,u.nick,ua.name LIMIT $3"#,
        )
        .bind(&ip_cidr)
        .bind(q.ua)
        .bind(SAME_IP_ROWS_LIMIT)
        .fetch_all(&state.pool)
        .await?;
        html.push_str(&format!(
            "<h2>Пользователи за год (по топикам и комментариям){}</h2><div class=\"forum\"><table width=\"100%\" class=\"message-table\"><thead><tr><th>Последний комментарий</th><th>Пользователь</th><th>User Agent</th></tr></thead><tbody>",
            if users.len() as i64 == SAME_IP_ROWS_LIMIT {
                format!(" (показаны первые {SAME_IP_ROWS_LIMIT})")
            } else {
                String::new()
            }
        ));
        for (dtLastDate, sNick, optUserAgent, bBlocked) in &users {
            html.push_str(&format!(
                "<tr><td>{}</td><td>{}<a href=\"/people/{}/profile\">{}</a>{}</td><td>{}</td></tr>",
                crate::request_timezone::sTimeTag("default", *dtLastDate),
                if *bBlocked { "<s>" } else { "" },
                urlencoding::encode(sNick),
                html_escape::encode_text(sNick),
                if *bBlocked { "</s>" } else { "" },
                html_escape::encode_text(optUserAgent.as_deref().unwrap_or("")),
            ));
        }
        html.push_str("</tbody></table></div>");
    }

    // Block info and management, exact-IP lookups only.
    if let Some(ip) = q.ip.as_deref().filter(|_| bExactIp) {
        if let Some((date, ban_date, reason, allow_posting, captcha_required, mod_id)) = &optBlock {
            let moderator: Option<String> =
                sqlx::query_scalar("SELECT nick FROM users WHERE id=$1")
                    .bind(*mod_id)
                    .fetch_optional(&state.pool)
                    .await?;
            html.push_str(&format!(
                "<h2>Информация о блокировке</h2><p>С {date}{} · причина: {} · модератор: {} · регистр. можно постить: {allow_posting} · капча: {captcha_required}</p>",
                ban_date.map(|d| format!(" до {d}")).unwrap_or_default(),
                html_escape::encode_text(reason.as_deref().unwrap_or("")),
                html_escape::encode_text(moderator.as_deref().unwrap_or("?")),
            ));
        }
        html.push_str(&sRenderSameIpBanForm(
            ip,
            &sCsrfToken,
            optBlock
                .as_ref()
                .is_some_and(|(_, _, _, bAllow, _, _)| *bAllow),
            optBlock
                .as_ref()
                .is_none_or(|(_, _, _, _, bCaptcha, _)| *bCaptcha),
        ));
    }

    Ok(Html(crate::routes::sRenderLegacyContent(
        "Поиск сообщений по метаданным",
        html,
    )?))
}

fn iBindRequiredGroup(optRawQuery: Option<&str>) -> Result<i32> {
    let vecParameters = optRawQuery
        .map(serde_urlencoded::from_str::<Vec<(String, String)>>)
        .transpose()
        .map_err(|_| AppError::BadParameter("Некорректные параметры запроса".to_owned()))?
        .unwrap_or_default();
    let sGroup = vecParameters
        .iter()
        .find_map(|(sName, sValue)| (sName == "group").then_some(sValue.as_str()))
        .ok_or_else(|| AppError::BadParameter("Не задан параметр group".to_owned()))?;
    sGroup
        .trim()
        .parse::<i32>()
        .map_err(|_| AppError::BadParameter("Некорректное значение параметра group".to_owned()))
}

fn render_groupmod_form(
    id: i32,
    title: &str,
    urlname: &str,
    info: &str,
    longinfo: &str,
    resolvable: bool,
    is_admin: bool,
    error: Option<&str>,
    csrf_token: &str,
) -> String {
    let error_html = error
        .map(|e| format!("<div class=\"error\">{}</div>", html_escape::encode_text(e)))
        .unwrap_or_default();
    let mut sGroupInfo = String::from("<div class=\"infoblock\">");
    if !info.is_empty() {
        sGroupInfo.push_str(&format!(
            "<p style=\"margin-top: 0\"><em>{}</em></p>",
            html_escape::encode_text(info)
        ));
    }
    if !longinfo.is_empty() {
        sGroupInfo.push_str(&format!(
            "<div class=\"infoblock-small\">{}</div>",
            crate::markup::render_markdown(longinfo)
        ));
    }
    sGroupInfo.push_str(&format!(
        "<p>[<a href=\"groupmod.jsp?group={id}\">править</a>]</p></div>"
    ));
    // GroupModificationController: только администратор может менять
    // title/urlName - модератору эти поля показываются как read-only.
    let sReadonly = if is_admin { "" } else { " readonly" };
    let title_field = format!(
        r#"<input type="text" name="title" size="70" value="{}"{sReadonly}>"#,
        html_escape::encode_double_quoted_attribute(title)
    );
    let url_field = format!(
        r#"<input type="text" name="urlName" size="70" value="{}"{sReadonly}>"#,
        html_escape::encode_double_quoted_attribute(urlname)
    );
    format!(
        r#"
{sGroupInfo}{error_html}
<form id="groupModForm" action="groupmod.jsp" method="POST">
<input type="hidden" name="csrf" value="{csrf_token}">
<input type="hidden" name="group" value="{id}">
<label>Заголовок: {title_field}</label><br>
<label>Строка описания: <input type="text" name="info" size="70" value="{info_attr}"></label><br>
<label>Имя для URL: {url_field}</label><br>
<label>Можно помечать темы как решенные: <input type="checkbox" name="resolvable"{checked}></label><br>
<label>Подробное описание:</label><br>
<div class="control-group" data-format-mode="markdown"><div class="markup-tabs"><ul class="markup-tabs__nav"><li class="markup-tabs__tab active" data-tab="editor">Markdown</li></ul><div class="markup-tabs__content"><div class="markup-tabs__panel active" data-panel="editor"><textarea rows="20" cols="70" name="longinfo" id="form_longinfo">{longinfo}</textarea></div></div></div><div class="help-block"><a href="/help/markdown.md">Помощь по Markdown</a></div></div>
<div class="form-actions"><input type="submit" value="Изменить"> <button type="submit" name="preview" class="btn btn-default">Предпросмотр</button></div>
</form>
"#,
        info_attr = html_escape::encode_double_quoted_attribute(info),
        longinfo = html_escape::encode_text(longinfo),
        checked = if resolvable { " checked" } else { "" },
    )
}

async fn groupmod_form(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    RawQuery(optRawQuery): RawQuery,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    // Spring resolves the required `@RequestParam("group")` before invoking
    // `ModeratorOnly`; missing and malformed values use bad-parameter.jsp.
    let id = iBindRequiredGroup(optRawQuery.as_deref())?;
    let moderator = require_moderator(&user)?;
    let (title, urlname, info, longinfo, resolvable) =
        sqlx::query_as::<_, (String, String, Option<String>, Option<String>, bool)>(
            "SELECT title,urlname,info,longinfo,resolvable FROM groups WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let html = format!(
        "<h1>Правка группы {}</h1>{}",
        html_escape::encode_text(&title),
        render_groupmod_form(
            id,
            &title,
            &urlname,
            info.as_deref().unwrap_or(""),
            longinfo.as_deref().unwrap_or(""),
            resolvable,
            moderator.candel,
            None,
            &csrf_token,
        )
    );
    Ok(Html(crate::routes::sRenderLegacyContent(
        "Правка группы",
        html,
    )?))
}

/// GroupModificationController.validateUrlName.
fn validate_url_name(url_name: &str) -> Option<&'static str> {
    if url_name.is_empty() {
        return Some("Имя для URL не может быть пустым");
    }
    if url_name.contains('/') {
        return Some("Имя для URL не может содержать символ '/'");
    }
    if !url_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Some(
            "Имя для URL может содержать только латинские буквы, цифры, дефис и подчёркивание",
        );
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
pub struct GroupModForm {
    pub group: i32,
    pub title: String,
    pub info: String,
    pub url_name: String,
    pub longinfo: String,
    pub preview: Option<String>,
    pub resolvable: Option<String>,
}

fn stBindGroupModForm(vecParameters: &[(String, String)]) -> Result<GroupModForm> {
    let sRequired = |sName: &str| {
        crate::form::get(vecParameters, sName)
            .map(str::to_owned)
            .ok_or_else(|| AppError::BadParameter(format!("Не задан параметр {sName}")))
    };
    let sGroup = sRequired("group")?;
    Ok(GroupModForm {
        group: sGroup.trim().parse::<i32>().map_err(|_| {
            AppError::BadParameter("Некорректное значение параметра group".to_owned())
        })?,
        title: sRequired("title")?,
        info: sRequired("info")?,
        url_name: sRequired("urlName")?,
        longinfo: sRequired("longinfo")?,
        preview: crate::form::get(vecParameters, "preview").map(str::to_owned),
        resolvable: crate::form::get(vecParameters, "resolvable").map(str::to_owned),
    })
}

async fn groupmod_save(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    stRequest: Request,
) -> Result<Html<String>> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let form = stBindGroupModForm(&vecParameters)?;
    let moderator = require_moderator(&user)?;
    let (existing_title, existing_urlname): (String, String) =
        sqlx::query_as("SELECT title,urlname FROM groups WHERE id=$1")
            .bind(form.group)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?;

    let is_admin = moderator.candel;
    let effective_title = if is_admin {
        form.title.clone()
    } else {
        existing_title
    };
    let effective_urlname = if is_admin {
        form.url_name.clone()
    } else {
        existing_urlname
    };
    let info = form.info.clone();
    let longinfo = form.longinfo.clone();
    let resolvable = form.resolvable.is_some();

    if form.preview.is_some() {
        let sContentHtml = format!(
            "<h1>Правка группы {} - Предпросмотр</h1>{}",
            html_escape::encode_text(&effective_title),
            render_groupmod_form(
                form.group,
                &effective_title,
                &effective_urlname,
                &info,
                &longinfo,
                resolvable,
                is_admin,
                None,
                &csrf_token,
            )
        );
        return Ok(Html(crate::routes::sRenderLegacyContent(
            "Правка группы",
            sContentHtml,
        )?));
    }

    if let Some(error) = validate_url_name(&effective_urlname) {
        let sContentHtml = format!(
            "<h1>Правка группы {}</h1>{}",
            html_escape::encode_text(&effective_title),
            render_groupmod_form(
                form.group,
                &effective_title,
                &effective_urlname,
                &info,
                &longinfo,
                resolvable,
                is_admin,
                Some(error),
                &csrf_token,
            )
        );
        return Ok(Html(crate::routes::sRenderLegacyContent(
            "Правка группы",
            sContentHtml,
        )?));
    }

    sqlx::query(
        "UPDATE groups SET title=$2, urlname=$3, info=$4, longinfo=$5, resolvable=$6 WHERE id=$1",
    )
    .bind(form.group)
    .bind(&effective_title)
    .bind(&effective_urlname)
    .bind(&info)
    .bind(&longinfo)
    .bind(resolvable)
    .execute(&state.pool)
    .await?;

    Ok(Html(crate::routes::sRenderLegacyContent(
        "Параметры изменены",
        "<h1>Параметры изменены</h1>".to_owned(),
    )?))
}

#[derive(Debug, Deserialize)]
pub struct UserModForm {
    pub id: i32,
    pub reason: Option<String>,
    pub shift: Option<String>,
}

#[derive(Template)]
#[template(path = "usermod_mass_delete.html")]
struct StUserModMassDeleteTemplate {
    iTopics: usize,
    iComments: usize,
    vecSkipped: Vec<i32>,
}

#[derive(Template)]
#[template(path = "usermod_password_reset.html")]
struct StUserModPasswordResetTemplate {
    sProfileLink: String,
}

#[derive(Template)]
#[template(path = "usermod_user_error.html")]
struct StUserModErrorTemplate {
    sMessage: String,
}

fn sJavaFormEncode(sValue: &str) -> String {
    serde_urlencoded::to_string([("value", sValue)])
        .expect("encoding a string cannot fail")
        .trim_start_matches("value=")
        .to_owned()
}

pub(crate) fn stProfileRedirect(sNick: &str) -> Response {
    let sLocation = format!(
        "/people/{}/profile?nocache={}",
        sJavaFormEncode(sNick),
        rand::random::<i32>()
    );
    (StatusCode::FOUND, [(header::LOCATION, sLocation)]).into_response()
}

pub(crate) fn stUserModErrorResponse(sMessage: String) -> Response {
    let sBody = StUserModErrorTemplate { sMessage }
        .render()
        .unwrap_or_else(|_| "Внутренняя ошибка сервера".to_owned());
    (StatusCode::INTERNAL_SERVER_ERROR, Html(sBody)).into_response()
}

#[cfg(test)]
fn stUserModForm(
    optRawQuery: Option<&str>,
    mapForm: &std::collections::HashMap<String, String>,
) -> Result<(crate::application::user::EnUserModAction, UserModForm)> {
    // Servlet request parameters include both query and form values. Tomcat
    // exposes a query value first when the same key is present in both, so do
    // the same here rather than silently making this body-only.
    let mut vecParameters: Vec<(String, String)> = optRawQuery
        .map(serde_urlencoded::from_str)
        .transpose()
        .map_err(|_| AppError::NotFound)?
        .unwrap_or_default();
    vecParameters.extend(
        mapForm
            .iter()
            .map(|(sName, sValue)| (sName.clone(), sValue.clone())),
    );
    let enAction = enUserModMapping(&vecParameters)?;
    let stForm = stBindUserModForm(&vecParameters, enAction)?;
    Ok((enAction, stForm))
}

fn enUserModMapping(
    vecParameters: &[(String, String)],
) -> Result<crate::application::user::EnUserModAction> {
    // Spring selects the exact `params = "action=..."` method before CSRF,
    // binding and authentication. Unknown/missing values match no handler.
    crate::form::get(vecParameters, "action")
        .and_then(crate::application::user::EnUserModAction::optFromForm)
        .ok_or(AppError::NotFound)
}

fn stBindUserModForm(
    vecParameters: &[(String, String)],
    enAction: crate::application::user::EnUserModAction,
) -> Result<UserModForm> {
    let optParameter = |sName: &str| crate::form::get(vecParameters, sName);
    let id = optParameter("id")
        .ok_or_else(|| AppError::BadRequest("Required request parameter 'id'".to_owned()))?
        .parse::<i32>()
        .map_err(|_| AppError::BadRequest("Failed to convert request parameter 'id'".to_owned()))?;
    if enAction == crate::application::user::EnUserModAction::Freeze
        && (optParameter("reason").is_none() || optParameter("shift").is_none())
    {
        return Err(AppError::BadRequest(
            "Required freeze request parameter is missing".to_owned(),
        ));
    }
    Ok(UserModForm {
        id,
        reason: optParameter("reason").map(ToOwned::to_owned),
        shift: optParameter("shift").map(ToOwned::to_owned),
    })
}

async fn usermod_get() -> Result<Response> {
    Err(AppError::NotFound)
}

async fn usermod(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    stRequest: Request,
) -> Result<Response> {
    use crate::application::user::{CUserModerationService, EnUserModOutcome, StUserModCommand};
    use crate::infra::postgres::user_moderation_repository::CUserModerationPgRepository;

    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let enAction = enUserModMapping(&vecParameters)?;
    if !crate::csrf::bServletCsrfValid(&vecParameters, &sCsrfToken) {
        return Err(AppError::Forbidden);
    }
    let form = stBindUserModForm(&vecParameters, enAction)?;
    let stModerator = require_moderator(&user)?;
    let cService =
        CUserModerationService::new(CUserModerationPgRepository::new(state.pool.clone()));
    let enOutcome = match cService
        .enExecute(
            stModerator,
            StUserModCommand {
                iTargetUserId: form.id,
                enAction,
                optReason: form.reason,
                optShift: form.shift,
            },
        )
        .await
    {
        Ok(enOutcome) => enOutcome,
        // The original controller throws UserErrorException for these
        // validation failures. Its common exception page deliberately keeps
        // HTTP 500 and displays the message.
        Err(AppError::BadRequest(sMessage)) => return Ok(stUserModErrorResponse(sMessage)),
        Err(stError) => return Err(stError),
    };

    Ok(match enOutcome {
        EnUserModOutcome::ProfileRedirect { sNick } => stProfileRedirect(&sNick),
        EnUserModOutcome::PasswordReset { sNick } => {
            let sProfileLink = format!(
                "/people/{}/profile?nocache={}",
                sJavaFormEncode(&sNick),
                rand::random::<i32>()
            );
            Html(StUserModPasswordResetTemplate { sProfileLink }.render()?).into_response()
        }
        EnUserModOutcome::MassDelete(stDelete) => {
            let cQueue = CSearchQueueSender::new(
                state.config.opensearch_url.as_deref(),
                &state.config.upload_dir,
            );
            for iTopicId in &stDelete.vecTopicIds {
                cQueue.vUpdateMessage(*iTopicId, true).await?;
            }
            cQueue.vUpdateComments(&stDelete.vecCommentIds).await?;
            Html(
                StUserModMassDeleteTemplate {
                    iTopics: stDelete.vecTopicIds.len(),
                    iComments: stDelete.vecCommentIds.len(),
                    vecSkipped: stDelete.vecSkippedCommentIds,
                }
                .render()?,
            )
            .into_response()
        }
    })
}

#[cfg(test)]
mod usermod_tests {
    use super::{
        EnSearchReindexAction, dtAddCalendarMonths, enSearchReindexAction, enUserModMapping,
        iBindRequiredGroup, optBanIpUntil, optSameIpCidr, render_groupmod_form, sJavaFormEncode,
        sRenderSameIpBanForm, sRenderSameIpDeleteForm, stBindGroupModForm, stBindUserModForm,
        stProfileRedirect, stUserModForm, usermod_get,
    };
    use crate::{application::user::EnUserModAction, error::AppError};
    use axum::{
        http::{StatusCode, header},
        response::IntoResponse,
    };
    use std::collections::HashMap;

    #[test]
    fn groupmod_get_uses_spring_required_parameter_binding() {
        assert_eq!(iBindRequiredGroup(Some("group=42")).unwrap(), 42);
        assert_eq!(iBindRequiredGroup(Some("group=+42+")).unwrap(), 42);
        assert!(matches!(
            iBindRequiredGroup(None),
            Err(AppError::BadParameter(_))
        ));
        assert!(matches!(
            iBindRequiredGroup(Some("group=nope")),
            Err(AppError::BadParameter(_))
        ));
    }

    #[test]
    fn groupmod_form_preserves_original_dom_and_readonly_submission() {
        let sHtml = render_groupmod_form(
            7,
            "Forum",
            "forum",
            "line",
            "**details**",
            true,
            false,
            None,
            "token",
        );
        assert!(sHtml.contains("id=\"groupModForm\""));
        assert!(sHtml.contains("action=\"groupmod.jsp\" method=\"POST\""));
        assert!(sHtml.contains("name=\"title\" size=\"70\" value=\"Forum\" readonly"));
        assert!(sHtml.contains("name=\"urlName\" size=\"70\" value=\"forum\" readonly"));
        assert!(!sHtml.contains(" disabled"));
        assert!(sHtml.contains("name=\"longinfo\" id=\"form_longinfo\""));
        assert!(sHtml.contains("name=\"preview\" class=\"btn btn-default\""));
        assert!(sHtml.contains("<strong>details</strong>"));
    }

    #[test]
    fn groupmod_post_binding_uses_bad_parameter_for_missing_or_invalid_required_values() {
        let vecValid = vec![
            ("group".to_owned(), "7".to_owned()),
            ("title".to_owned(), "Forum".to_owned()),
            ("info".to_owned(), String::new()),
            ("urlName".to_owned(), "forum".to_owned()),
            ("longinfo".to_owned(), String::new()),
            ("preview".to_owned(), String::new()),
        ];
        let stForm = stBindGroupModForm(&vecValid).unwrap();
        assert_eq!(stForm.group, 7);
        assert_eq!(stForm.preview.as_deref(), Some(""));

        let mut vecMissing = vecValid.clone();
        vecMissing.retain(|(sName, _)| sName != "info");
        assert!(matches!(
            stBindGroupModForm(&vecMissing),
            Err(AppError::BadParameter(_))
        ));
        let mut vecInvalid = vecValid;
        vecInvalid[0].1 = "seven".to_owned();
        assert!(matches!(
            stBindGroupModForm(&vecInvalid),
            Err(AppError::BadParameter(_))
        ));
    }

    #[test]
    fn sameip_exact_ip_controls_preserve_original_actions_and_fields() {
        let sDelete = sRenderSameIpDeleteForm("203.0.113.7", "token", false);
        for sNeedle in [
            "action=\"delip.jsp\"",
            "name=\"csrf\" value=\"token\"",
            "name=\"ip\" value=\"203.0.113.7\"",
            "name=\"reason\"",
            "name=\"time\"",
            "name=\"ban_time\"",
            "name=\"ban_mode\"",
            "name=\"del\"",
            "function banTimeChange(object)",
            "input[name=ban_mode]",
        ] {
            assert!(sDelete.contains(sNeedle), "missing {sNeedle}");
        }
        let sAlreadyBlocked = sRenderSameIpDeleteForm("203.0.113.7", "token", true);
        assert!(!sAlreadyBlocked.contains("name=\"ban_time\""));
        assert!(!sAlreadyBlocked.contains("name=\"ban_mode\""));

        let sBan = sRenderSameIpBanForm("203.0.113.7", "token", true, true);
        for sNeedle in [
            "action=\"banip.jsp\"",
            "name=\"time\"",
            "name=\"ban_days\"",
            "name=\"allow_posting\"",
            "name=\"captcha_required\"",
            "name=\"ban\"",
            "id=\"custom_ban\"",
        ] {
            assert!(sBan.contains(sNeedle), "missing {sNeedle}");
        }

        let sProduction = include_str!("admin.rs")
            .split("async fn same_ip(")
            .nth(1)
            .unwrap()
            .split("fn iBindRequiredGroup")
            .next()
            .unwrap();
        assert!(sProduction.contains("CURRENT_TIMESTAMP-'5 days'::interval"));
        assert!(sProduction.contains("score<$3 OR id=2"));
        assert!(sProduction.contains("t.title AS \"sTitle\",c.postdate"));
        assert!(!sProduction.contains("c.title AS \"sTitle\""));
        assert!(sProduction.contains("q.ip.as_deref().map_or_else(String::new"));
        assert!(sProduction.contains("jump-message.jsp?msgid="));
        assert!(sProduction.contains("href=\\\"/people/{}/profile\\\""));
        assert!(sProduction.contains("/admin/geoip?ip="));
        assert!(sProduction.contains("ul.action='register'::user_log_action"));
    }

    #[test]
    fn sameip_validates_mask_only_when_ip_is_present() {
        assert_eq!(optSameIpCidr(None, 33).unwrap(), None);
        assert_eq!(
            optSameIpCidr(Some("203.0.113.7"), 24).unwrap().as_deref(),
            Some("203.0.113.7/24")
        );
        for stError in [
            optSameIpCidr(Some("not-an-ip"), 32).unwrap_err(),
            optSameIpCidr(Some("203.0.113.7"), 33).unwrap_err(),
        ] {
            assert!(matches!(stError, AppError::UserError { .. }));
        }
    }

    #[test]
    fn ban_months_use_calendar_clamping_like_offset_date_time_plus_months() {
        use chrono::{TimeZone, Utc};

        let dtCommonYear = Utc.with_ymd_and_hms(2025, 1, 31, 12, 30, 0).unwrap();
        assert_eq!(
            dtAddCalendarMonths(dtCommonYear, 1).unwrap(),
            Utc.with_ymd_and_hms(2025, 2, 28, 12, 30, 0).unwrap()
        );
        let dtLeapYear = Utc.with_ymd_and_hms(2024, 1, 31, 12, 30, 0).unwrap();
        assert_eq!(
            dtAddCalendarMonths(dtLeapYear, 1).unwrap(),
            Utc.with_ymd_and_hms(2024, 2, 29, 12, 30, 0).unwrap()
        );
        assert_eq!(
            dtAddCalendarMonths(dtCommonYear, 3).unwrap(),
            Utc.with_ymd_and_hms(2025, 4, 30, 12, 30, 0).unwrap()
        );
    }

    #[test]
    fn banip_preserves_java_internal_error_classes() {
        use chrono::{TimeZone, Utc};

        let dtNow = Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap();
        assert_eq!(
            optBanIpUntil("custom", Some("7"), dtNow).unwrap(),
            Some(dtNow + chrono::Duration::days(7))
        );
        for stError in [
            optBanIpUntil("custom", Some("0"), dtNow).unwrap_err(),
            optBanIpUntil("invalid", None, dtNow).unwrap_err(),
        ] {
            assert!(matches!(stError, AppError::UserError { .. }));
        }
        for stError in [
            optBanIpUntil("custom", None, dtNow).unwrap_err(),
            optBanIpUntil("custom", Some("seven"), dtNow).unwrap_err(),
        ] {
            assert!(matches!(stError, AppError::Anyhow(_)));
        }

        let sHandler = include_str!("admin.rs")
            .split(concat!("async fn ", "ban_ip("))
            .nth(1)
            .unwrap()
            .split(concat!("#[derive(Debug, Clone", ", PartialEq, Eq)]"))
            .next()
            .unwrap();
        assert!(sHandler.contains("VALUES($1::inet"));
        assert!(sHandler.contains(".bind(&form.ip)"));
        assert!(!sHandler.contains(".parse()"));
        assert!(!sHandler.contains("AppError::BadRequest"));
    }

    #[test]
    fn search_reindex_actions_match_spring_parameter_conditionals() {
        assert_eq!(
            enSearchReindexAction(Some("all")).expect("all action"),
            EnSearchReindexAction::All
        );
        assert_eq!(
            enSearchReindexAction(Some("current")).expect("current action"),
            EnSearchReindexAction::Current
        );
        assert!(matches!(
            enSearchReindexAction(None),
            Err(AppError::NotFound)
        ));
        assert!(matches!(
            enSearchReindexAction(Some("ALL")),
            Err(AppError::NotFound)
        ));
    }

    #[test]
    fn request_parameters_accept_query_and_form_with_query_precedence() {
        let mapForm = HashMap::from([
            ("action".to_owned(), "block".to_owned()),
            ("id".to_owned(), "20".to_owned()),
            ("reason".to_owned(), "body".to_owned()),
        ]);
        let (enAction, stForm) = stUserModForm(
            Some("action=freeze&id=10&reason=query&shift=30+%D0%BC%D0%B8%D0%BD%D1%83%D1%82"),
            &mapForm,
        )
        .expect("combined servlet parameters");

        assert_eq!(enAction, EnUserModAction::Freeze);
        assert_eq!(stForm.id, 10);
        assert_eq!(stForm.reason.as_deref(), Some("query"));
        assert_eq!(stForm.shift.as_deref(), Some("30 минут"));
    }

    #[test]
    fn unknown_action_is_404_before_required_id_binding() {
        assert!(matches!(
            stUserModForm(Some("action=BLOCK"), &HashMap::new()),
            Err(AppError::NotFound)
        ));
    }

    #[test]
    fn freeze_requires_reason_and_shift_like_spring_binding() {
        assert!(matches!(
            stUserModForm(Some("action=freeze&id=8&reason=x"), &HashMap::new()),
            Err(AppError::BadRequest(_))
        ));
        assert!(
            stUserModForm(
                Some("action=freeze&id=8&reason=&shift=%D1%87%D0%B0%D1%81"),
                &HashMap::new()
            )
            .is_ok()
        );
    }

    #[test]
    fn usermod_mapping_is_selected_before_csrf_and_argument_binding() {
        let vecUnknown = vec![("action".to_owned(), "BLOCK".to_owned())];
        assert!(matches!(
            enUserModMapping(&vecUnknown),
            Err(AppError::NotFound)
        ));

        let vecSelected = vec![("action".to_owned(), "block".to_owned())];
        let enAction = enUserModMapping(&vecSelected).expect("selected mapping");
        assert!(matches!(
            stBindUserModForm(&vecSelected, enAction),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn profile_redirect_is_java_302_with_form_encoded_nick_and_nocache() {
        assert_eq!(sJavaFormEncode("a b+v"), "a+b%2Bv");
        let stResponse = stProfileRedirect("a b+v");
        assert_eq!(stResponse.status(), StatusCode::FOUND);
        let sLocation = stResponse
            .headers()
            .get(header::LOCATION)
            .and_then(|stValue| stValue.to_str().ok())
            .expect("redirect location");
        assert!(sLocation.starts_with("/people/a+b%2Bv/profile?nocache="));
    }

    #[tokio::test]
    async fn legacy_get_is_not_found() {
        assert!(matches!(usermod_get().await, Err(AppError::NotFound)));
        assert_eq!(
            AppError::NotFound.into_response().status(),
            StatusCode::NOT_FOUND
        );
    }
}
#[derive(Deserialize)]
pub struct WarningQuery {
    pub topic: Option<i32>,
    pub comment: Option<i32>,
}

const VEC_WARNING_RULE_TYPES: &[&str] = &[
    "3.1 Дубль",
    "3.2 Неверная кодировка",
    "3.3 Некорректное форматирование",
    "3.4 Пустое сообщение",
    "4.1 Офтопик",
    "4.2 Вызывающе неверная информация",
    "4.3 Провокация flame",
    "4.4 Обсуждение действий модераторов",
    "4.5 Тестовые сообщения",
    "4.6 Спам",
    "4.7 Флуд",
    "4.8 Дискуссия не на русском языке",
    "4.9 Офтопик-лист, п. ",
    "5.1 Нецензурные выражения",
    "5.2 Оскорбление участников дискуссии",
    "5.3 Национальные/политические/религиозные споры",
    "5.4 Личная переписка",
    "5.5 Преднамеренное нарушение правил русского языка",
    "6 Нарушение copyright",
    "6.2 Warez",
    "7.1 Ответ на некорректное сообщение",
    "7.2 Чрезмерно исправленное сообщение",
];

fn sWarningForm(
    stPresentation: &StWarningPresentation,
    iTopicId: i32,
    optCommentId: Option<i32>,
    sCsrfToken: &str,
) -> String {
    let vecTypes = &stPresentation.vecTypes;
    let sTypeField = if vecTypes.len() == 1 {
        "<input type=\"hidden\" name=\"warningType\" value=\"rule\">".to_owned()
    } else {
        let mut sOptions = String::new();
        for enType in vecTypes {
            let sType = enType.sId();
            let sName = enType.sName();
            sOptions.push_str(&format!("<option value=\"{sType}\">{sName}</option>"));
        }
        format!(
            "<label>Проблема <select id=\"warning-select\" name=\"warningType\">{sOptions}</select></label>"
        )
    };
    let mut sRuleOptions = "<option value=\"\"></option>".to_owned();
    for sRule in VEC_WARNING_RULE_TYPES {
        let sEscaped = html_escape::encode_double_quoted_attribute(sRule);
        sRuleOptions.push_str(&format!("<option value=\"{sEscaped}\">{sEscaped}</option>"));
    }
    let sError = stPresentation
        .optError
        .map(|sValue| {
            format!(
                "<div class=\"error\">{}</div>",
                html_escape::encode_text(sValue)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<h1>Уведомить модераторов</h1>
<form method="post" action="/post-warning" class="form-horizontal">
<input type="hidden" name="csrf" value="{sCsrfToken}">
<input type="hidden" name="topic" value="{iTopicId}">
<input type="hidden" name="comment" value="{sCommentId}">
{sTypeField}
<label>Пункт правил <select id="rule-select" name="ruleType">{sRuleOptions}</select></label>
<label>Комментарий <textarea id="reason-input" name="text" maxlength="256"></textarea></label>
{sError}
<button type="submit">Уведомить</button>
</form>
<p><a href="{sTopicUrl}">Вернуться к сообщению</a></p>"#,
        sCommentId = optCommentId
            .map(|iValue| iValue.to_string())
            .unwrap_or_default(),
        sTopicUrl = stPresentation.stContext.sTopicUrl,
    )
}

async fn post_warning_form(
    State(stState): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<WarningQuery>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let stUser = user.as_ref().ok_or(AppError::Forbidden)?;
    let iTopicId = q.topic.ok_or(AppError::NotFound)?;
    let optCommentId = q.comment.filter(|iValue| *iValue != 0);
    let cService = CWarningService::new(CWarningPgRepository::new(stState.pool.clone()));
    let stPresentation = cService.stPrepare(stUser, iTopicId, optCommentId).await?;
    let sContentHtml = sWarningForm(&stPresentation, iTopicId, optCommentId, &csrf_token);
    Ok(Html(crate::routes::sRenderLegacyContent(
        "Уведомить модераторов",
        sContentHtml,
    )?))
}

#[derive(Deserialize)]
pub struct WarningForm {
    pub topic: Option<i32>,
    pub comment: Option<i32>,
    pub reason: Option<String>,
    pub text: Option<String>,
    #[serde(alias = "warningType")]
    pub warning_type: Option<String>,
    #[serde(alias = "ruleType")]
    pub rule_type: Option<String>,
}

async fn post_warning(
    State(stState): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    Form(form): Form<WarningForm>,
) -> Result<Response> {
    let stUser = user.as_ref().ok_or(AppError::Forbidden)?;
    let iTopicId = form.topic.ok_or(AppError::NotFound)?;
    let optCommentId = form.comment.filter(|iValue| *iValue != 0);
    let cService = CWarningService::new(CWarningPgRepository::new(stState.pool.clone()));
    match cService
        .enCreate(
            stUser,
            StCreateWarningCommand {
                iTopicId,
                optCommentId,
                optReason: form.reason,
                optText: form.text,
                optWarningType: form.warning_type,
                optRuleType: form.rule_type,
            },
        )
        .await?
    {
        EnCreateWarningOutcome::Validation(stPresentation) => {
            let sContentHtml = sWarningForm(&stPresentation, iTopicId, optCommentId, &csrf_token);
            Ok(Html(crate::routes::sRenderLegacyContent(
                "Уведомить модераторов",
                sContentHtml,
            )?)
            .into_response())
        }
        EnCreateWarningOutcome::Created { sLink } => Ok(Html(
            StActionDoneTemplate {
                message: "Уведомление отправлено".to_owned(),
                big_message: None,
                link: Some(sLink),
            }
            .render()?,
        )
        .into_response()),
    }
}

#[derive(Deserialize)]
pub struct ClearWarningForm {
    pub id: i32,
}

async fn clear_warning(
    State(stState): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ClearWarningForm>,
) -> Result<Response> {
    let stActor = user.as_ref().ok_or(AppError::Forbidden)?;
    let cService = CWarningService::new(CWarningPgRepository::new(stState.pool.clone()));
    let sLink = cService.sClear(stActor, form.id).await?;
    Ok((StatusCode::FOUND, [(header::LOCATION, sLink)]).into_response())
}

#[cfg(test)]
mod warning_tests {
    use super::sWarningForm;
    use crate::{
        application::warning::{StWarningContext, StWarningPresentation, vecWarningTypes},
        domain::warning::model::EnWarningType,
    };

    fn stPresentation(bPremoderated: bool) -> StWarningPresentation {
        StWarningPresentation {
            stContext: StWarningContext {
                bPremoderated,
                sTopicUrl: "/forum/general/42".to_owned(),
                optEligibilityError: None,
            },
            vecTypes: vecWarningTypes(bPremoderated, None),
            optError: None,
        }
    }

    #[test]
    fn warning_types_follow_comment_and_section_rules() {
        assert_eq!(vecWarningTypes(false, Some(7)), [EnWarningType::Rule]);
        assert_eq!(
            vecWarningTypes(false, None),
            [
                EnWarningType::Rule,
                EnWarningType::Tag,
                EnWarningType::Group
            ]
        );
        assert_eq!(
            vecWarningTypes(true, None),
            [
                EnWarningType::Rule,
                EnWarningType::Spelling,
                EnWarningType::Tag,
                EnWarningType::Group
            ]
        );
    }

    #[test]
    fn warning_form_uses_original_java_bean_names_and_zero_comment_shape() {
        let mut stPresentation = stPresentation(false);
        stPresentation.optError = Some("ошибка");
        let sHtml = sWarningForm(&stPresentation, 42, None, "csrf-value");
        assert!(sHtml.contains("name=\"warningType\""));
        assert!(sHtml.contains("name=\"ruleType\""));
        assert!(sHtml.contains("name=\"text\""));
        assert!(sHtml.contains("name=\"comment\" value=\"\""));
        assert!(sHtml.contains("maxlength=\"256\""));
        assert!(sHtml.contains("ошибка"));
    }
}
