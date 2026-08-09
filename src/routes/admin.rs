use crate::{
    application::{
        email_domain_block::CEmailDomainBlockService,
        geo_location::CGeoLocationService,
        warning::{
            CWarningService, EnCreateWarningOutcome, StCreateWarningCommand, StWarningPresentation,
        },
    },
    auth::CurrentUser,
    error::{AppError, Result},
    infra::postgres::{
        email_domain_block_repository::CEmailDomainBlockPgRepository,
        warning_repository::CWarningPgRepository,
    },
    request_timezone::stRequestTimezone,
    state::AppState,
};
use askama::Template;
use axum::{
    Form, Json, Router,
    extract::{Query, RawQuery, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

static SAME_IP_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\d+\.\d+\.\d+\.\d+$").expect("ip regex"));

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
        .route("/admin/email-domains", get(email_domains))
        .route("/admin/email-domains/add", post(email_domains_add))
        .route("/admin/email-domains/delete", post(email_domains_delete))
        .route(
            "/admin/search-reindex",
            get(search_reindex_form).post(search_reindex),
        )
        .route("/banip.jsp", post(ban_ip))
        .route("/delip.jsp", post(del_ip))
        .route("/sameip.jsp", get(same_ip))
        .route("/groupmod.jsp", get(groupmod_form).post(groupmod_save))
        // Java has POST-only parameter-conditioned mappings. Its configured
        // method-not-supported resolver deliberately turns GET/HEAD into 404.
        .route("/usermod.jsp", get(usermod_get).post(usermod))
        .route("/post-warning", get(post_warning_form).post(post_warning))
        .route("/clear-warning", post(clear_warning))
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

#[derive(Deserialize)]
pub struct ReindexForm {
    pub action: Option<String>,
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
    Form(form): Form<ReindexForm>,
) -> Result<Html<String>> {
    require_admin(&user)?;
    let sMessage = match enSearchReindexAction(form.action.as_deref())? {
        EnSearchReindexAction::All => {
            crate::search_index::vScheduleAllReindex(state)
                .await
                .map_err(|stError| AppError::Anyhow(anyhow::anyhow!(stError)))?;
            "Scheduled reindex"
        }
        EnSearchReindexAction::Current => {
            crate::search_index::vScheduleCurrentReindex(state);
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
    pub ban_days: Option<i64>,
    #[serde(default)]
    pub allow_posting: bool,
    #[serde(default)]
    pub captcha_required: bool,
}

/// BanIPController.banIP: standalone ban endpoint (distinct from
/// /delip.jsp's mass-delete-then-optionally-ban flow) - was missing
/// `time`/`allow_posting`/`captcha_required` entirely and always banned
/// unconditionally-and-permanently with no duration control.
async fn ban_ip(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<BanIpForm>,
) -> Result<Redirect> {
    let moderator = require_moderator(&user)?;
    let ip: std::net::IpAddr = form
        .ip
        .parse()
        .map_err(|_| AppError::BadRequest("Некорректный IP".into()))?;
    let ban_to: Option<chrono::DateTime<chrono::Utc>> = match form.time.as_str() {
        "hour" => Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        "day" => Some(chrono::Utc::now() + chrono::Duration::days(1)),
        "month" => Some(chrono::Utc::now() + chrono::Duration::days(30)),
        "3month" => Some(chrono::Utc::now() + chrono::Duration::days(90)),
        "6month" => Some(chrono::Utc::now() + chrono::Duration::days(180)),
        "custom" => {
            let days = form
                .ban_days
                .ok_or_else(|| AppError::BadRequest("Invalid days count".into()))?;
            if days <= 0 || days > 180 {
                return Err(AppError::BadRequest("Invalid days count".into()));
            }
            Some(chrono::Utc::now() + chrono::Duration::days(days))
        }
        "unlim" => None,
        "remove" => Some(chrono::Utc::now()),
        _ => return Err(AppError::BadRequest("Invalid count".into())),
    };
    sqlx::query(
        r#"INSERT INTO b_ips(ip,mod_id,date,reason,ban_date,allow_posting,captcha_required)
           VALUES($1::inet,$2,now(),$3,$4,$5,$6)
           ON CONFLICT(ip) DO UPDATE SET mod_id=EXCLUDED.mod_id, date=now(), reason=EXCLUDED.reason,
             ban_date=EXCLUDED.ban_date, allow_posting=EXCLUDED.allow_posting, captcha_required=EXCLUDED.captcha_required"#,
    )
        .bind(ip.to_string())
        .bind(moderator.id)
        .bind(&form.reason)
        .bind(ban_to)
        .bind(form.allow_posting)
        .bind(form.captcha_required)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to(&format!(
        "/sameip.jsp?ip={}",
        urlencoding::encode(&form.ip)
    )))
}

#[derive(Deserialize)]
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

/// Java's `/delip.jsp` (DelIPController.delIp) mass-deletes topics/comments
/// posted from an IP within a time window and optionally bans the IP - it is
/// NOT an unban endpoint. The previous Rust handler reused this exact
/// URL/method to delete a `b_ips` row (i.e. unban), which is the opposite
/// action: a moderator UI built against the real shape would silently unban
/// instead of cleaning up abuse.
async fn del_ip(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<DelIpForm>,
) -> Result<Html<String>> {
    let moderator = require_moderator(&user)?;
    let ip: std::net::IpAddr = form
        .ip
        .parse()
        .map_err(|_| AppError::BadRequest("Некорректный IP".into()))?;
    let ip = ip.to_string();

    let lookback = match form.time.as_str() {
        "hour" => chrono::Duration::hours(1),
        "day" => chrono::Duration::days(1),
        "3day" => chrono::Duration::days(3),
        "5day" => chrono::Duration::days(5),
        _ => return Err(AppError::BadRequest("Invalid count".into())),
    };
    let cutoff = chrono::Utc::now() - lookback;

    if let Some(ban_time) = form.ban_time.as_deref().filter(|s| !s.is_empty()) {
        let ban_to: Option<chrono::DateTime<chrono::Utc>> = match ban_time {
            "hour" => Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            "day" => Some(chrono::Utc::now() + chrono::Duration::days(1)),
            "month" => Some(chrono::Utc::now() + chrono::Duration::days(30)),
            "3month" => Some(chrono::Utc::now() + chrono::Duration::days(90)),
            "6month" => Some(chrono::Utc::now() + chrono::Duration::days(180)),
            "unlim" => None,
            "remove" => Some(chrono::Utc::now()),
            _ => return Err(AppError::BadRequest("Invalid count".into())),
        };
        let (allow_posting, captcha_required) = match form.ban_mode.as_deref() {
            Some("anonymous_and_captcha") => (true, true),
            Some("anonymous_only") => (true, false),
            _ => (false, false),
        };
        sqlx::query(
            r#"INSERT INTO b_ips(ip,mod_id,date,reason,ban_date,allow_posting,captcha_required)
               VALUES($1::inet,$2,now(),$3,$4,$5,$6)
               ON CONFLICT(ip) DO UPDATE SET
                 mod_id=EXCLUDED.mod_id, date=now(), reason=EXCLUDED.reason,
                 ban_date=EXCLUDED.ban_date, allow_posting=EXCLUDED.allow_posting, captcha_required=EXCLUDED.captcha_required"#,
        )
        .bind(&ip).bind(moderator.id).bind(&form.reason).bind(ban_to).bind(allow_posting).bind(captcha_required)
        .execute(&state.pool)
        .await?;
    }

    let topic_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM topics WHERE postip=$1::inet AND postdate>=$2 AND NOT deleted",
    )
    .bind(&ip)
    .bind(cutoff)
    .fetch_all(&state.pool)
    .await?;
    for id in &topic_ids {
        sqlx::query("UPDATE topics SET deleted=true WHERE id=$1")
            .bind(id)
            .execute(&state.pool)
            .await?;
        sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate) VALUES($1,$2,$3,now()) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now()")
            .bind(id).bind(moderator.id).bind(&form.reason).execute(&state.pool).await?;
    }

    let comment_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM comments WHERE postip=$1::inet AND postdate>=$2 AND NOT deleted",
    )
    .bind(&ip)
    .bind(cutoff)
    .fetch_all(&state.pool)
    .await?;
    for id in &comment_ids {
        sqlx::query("UPDATE comments SET deleted=true WHERE id=$1")
            .bind(id)
            .execute(&state.pool)
            .await?;
        sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate) VALUES($1,$2,$3,now()) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now()")
            .bind(id).bind(moderator.id).bind(&form.reason).execute(&state.pool).await?;
    }

    Ok(Html(format!(
        "<h1>Удаление по IP</h1><p>Удаляем темы и сообщения после {cutoff} с IP {ip_escaped}</p><p>Удалено тем: {topics}, комментариев: {comments}</p>",
        cutoff = cutoff,
        ip_escaped = html_escape::encode_text(&ip),
        topics = topic_ids.len(),
        comments = comment_ids.len(),
    )))
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

async fn same_ip(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<SameIpQuery>,
) -> Result<Html<String>> {
    require_moderator(&user)?;

    let mask = q.mask.unwrap_or(32);
    if !(0..=32).contains(&mask) {
        return Err(AppError::BadRequest("bad mask".into()));
    }
    let ip_cidr: Option<String> = match &q.ip {
        None => None,
        Some(ip) => {
            let re = Lazy::force(&SAME_IP_RE);
            if !re.is_match(ip) {
                return Err(AppError::BadRequest("not ip".into()));
            }
            if mask == 0 {
                None
            } else if mask != 32 {
                Some(format!("{ip}/{mask}"))
            } else {
                Some(ip.clone())
            }
        }
    };

    let mut html = String::from("<h1>Сообщения и пользователи по IP / user-agent</h1>");
    html.push_str(&format!(
        r#"<form method="get" class="form">
<input name="ip" placeholder="IP" value="{}">
<select name="mask"><option value="32">Только IP</option><option value="24">Сеть /24</option><option value="23">Сеть /23</option><option value="22">Сеть /22</option><option value="21">Сеть /21</option><option value="16">Сеть /16</option><option value="0">Любой IP</option></select>
<select name="score"><option value="">Любой score</option><option value="-9999">anonymous</option><option value="46">score &lt;= 45</option><option value="50">score &lt; 50</option><option value="100">score &lt; 100</option></select>
<button type="submit">Искать</button>
</form>"#,
        html_escape::encode_double_quoted_attribute(q.ip.as_deref().unwrap_or("")),
    ));

    // Matched comments/topics, IP/UA filtered.
    let posts = sqlx::query_as::<
        _,
        (
            i32,
            String,
            String,
            chrono::DateTime<chrono::Utc>,
            Option<String>,
            Option<i32>,
        ),
    >(
        r#"SELECT c.id, u.nick, 'comment', c.postdate, host(c.postip), c.ua_id
           FROM comments c JOIN users u ON u.id=c.userid
           WHERE ($1::inet IS NULL OR c.postip <<= $1::inet) AND ($2::int IS NULL OR c.ua_id=$2)
           UNION ALL
           SELECT t.id, u.nick, 'topic', t.postdate, host(t.postip), t.ua_id
           FROM topics t JOIN users u ON u.id=t.userid
           WHERE ($1::inet IS NULL OR t.postip <<= $1::inet) AND ($2::int IS NULL OR t.ua_id=$2)
           ORDER BY postdate DESC LIMIT $3"#,
    )
    .bind(&ip_cidr)
    .bind(q.ua)
    .bind(SAME_IP_ROWS_LIMIT)
    .fetch_all(&state.pool)
    .await?;

    html.push_str(&format!("<h2>Сообщения ({})</h2><ul>", posts.len()));
    for (id, nick, kind, date, ip, ua) in &posts {
        let sDate = crate::request_timezone::sTimeTag("compact-interval", *date);
        html.push_str(&format!(
            "<li>#{id} <a href=\"/people/{nick}/profile\">{nick}</a> — {kind}, {date} · {} {}</li>",
            ip.as_deref().unwrap_or(""),
            ua.map(|u| format!("ua#{u}")).unwrap_or_default(),
            nick = html_escape::encode_double_quoted_attribute(nick),
            date = sDate,
        ));
    }
    html.push_str("</ul>");
    if posts.len() as i64 == SAME_IP_ROWS_LIMIT {
        html.push_str("<p class=\"muted\">Показаны не все результаты.</p>");
    }

    // Matched users, only meaningful when an ip/ua filter narrows things down
    // and we're not specifically asking for the anonymous-only bucket.
    if q.score != Some(SAME_IP_ANONYMOUS_SCORE_FILTER) && (ip_cidr.is_some() || q.ua.is_some()) {
        let users = sqlx::query_as::<_, (i32, String, i32)>(
            r#"SELECT DISTINCT u.id, u.nick, COALESCE(u.score,0) FROM users u
               WHERE u.id IN (
                 SELECT userid FROM comments WHERE ($1::inet IS NULL OR postip <<= $1::inet) AND ($2::int IS NULL OR ua_id=$2)
                 UNION
                 SELECT userid FROM topics WHERE ($1::inet IS NULL OR postip <<= $1::inet) AND ($2::int IS NULL OR ua_id=$2)
               )
               AND ($3::int IS NULL OR COALESCE(u.score,0) < $3)
               ORDER BY u.nick LIMIT $4"#,
        )
        .bind(&ip_cidr)
        .bind(q.ua)
        .bind(q.score)
        .bind(SAME_IP_ROWS_LIMIT)
        .fetch_all(&state.pool)
        .await?;
        html.push_str(&format!("<h2>Пользователи ({})</h2><ul>", users.len()));
        for (id, nick, score) in &users {
            html.push_str(&format!(
                "<li>#{id} <a href=\"/people/{nick}/profile\">{nick}</a> · score={score}</li>",
                nick = html_escape::encode_double_quoted_attribute(nick),
            ));
        }
        html.push_str("</ul>");
    }

    // Block info, exact-IP lookups only.
    if let (Some(ip), 32) = (&q.ip, mask) {
        let block: Option<TyIpBlockRow> = sqlx::query_as(
            "SELECT date, ban_date, reason, allow_posting, captcha_required, mod_id FROM b_ips WHERE ip=$1::inet",
        )
        .bind(ip)
        .fetch_optional(&state.pool)
        .await?;
        if let Some((date, ban_date, reason, allow_posting, captcha_required, mod_id)) = block {
            let moderator: Option<String> =
                sqlx::query_scalar("SELECT nick FROM users WHERE id=$1")
                    .bind(mod_id)
                    .fetch_optional(&state.pool)
                    .await?;
            html.push_str(&format!(
                "<h2>Информация о блокировке</h2><p>С {date}{} · причина: {} · модератор: {} · регистр. можно постить: {allow_posting} · капча: {captcha_required}</p>",
                ban_date.map(|d| format!(" до {d}")).unwrap_or_default(),
                html_escape::encode_text(reason.as_deref().unwrap_or("")),
                html_escape::encode_text(moderator.as_deref().unwrap_or("?")),
            ));
        }
    }

    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct GroupModQuery {
    pub group: Option<i32>,
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
    preview: bool,
    csrf_token: &str,
) -> String {
    let error_html = error
        .map(|e| format!("<p class=\"error\">{}</p>", html_escape::encode_text(e)))
        .unwrap_or_default();
    let preview_html = if preview {
        "<p class=\"muted\">Предпросмотр (не сохранено)</p>"
    } else {
        ""
    };
    // GroupModificationController: только администратор может менять
    // title/urlName - модератору эти поля показываются как read-only.
    let (title_field, url_field) = if is_admin {
        (
            format!(
                r#"<input name="title" value="{}">"#,
                html_escape::encode_double_quoted_attribute(title)
            ),
            format!(
                r#"<input name="urlName" value="{}">"#,
                html_escape::encode_double_quoted_attribute(urlname)
            ),
        )
    } else {
        (
            format!(
                r#"<input value="{}" disabled><input type="hidden" name="title" value="{}">"#,
                html_escape::encode_double_quoted_attribute(title),
                html_escape::encode_double_quoted_attribute(title)
            ),
            format!(
                r#"<input value="{}" disabled><input type="hidden" name="urlName" value="{}">"#,
                html_escape::encode_double_quoted_attribute(urlname),
                html_escape::encode_double_quoted_attribute(urlname)
            ),
        )
    };
    format!(
        r#"
{error_html}{preview_html}
<form method="post" action="/groupmod.jsp" class="form wide">
<input type="hidden" name="csrf" value="{csrf_token}">
<input type="hidden" name="group" value="{id}">
<label>Название {title_field}</label>
<label>URL {url_field}</label>
<label>Описание <textarea name="info">{info}</textarea></label>
<label>Подробно <textarea name="longinfo">{longinfo}</textarea></label>
<label><input type="checkbox" name="resolvable" value="true"{checked}> Тема может быть помечена как решённая</label>
<button type="submit" name="preview" value="1">Предпросмотр</button>
<button type="submit">Сохранить</button>
</form>
"#,
        info = html_escape::encode_text(info),
        longinfo = html_escape::encode_text(longinfo),
        checked = if resolvable { " checked" } else { "" },
    )
}

async fn groupmod_form(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<GroupModQuery>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let moderator = require_moderator(&user)?;
    let groups = sqlx::query_as::<_, (i32, String, String)>(
        "SELECT id,title,urlname FROM groups ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut html = String::from("<h1>Редактирование группы</h1><ul>");
    for (id, title, urlname) in groups {
        html.push_str(&format!(
            "<li><a href=\"/groupmod.jsp?group={id}\">#{id} {}</a> /{}</li>",
            html_escape::encode_text(&title),
            html_escape::encode_text(&urlname)
        ));
    }
    html.push_str("</ul>");
    if let Some(id) = q.group
        && let Some((title, urlname, info, longinfo, resolvable)) =
            sqlx::query_as::<_, (String, String, Option<String>, Option<String>, bool)>(
                "SELECT title,urlname,info,longinfo,resolvable FROM groups WHERE id=$1",
            )
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
    {
        html.push_str(&render_groupmod_form(
            id,
            &title,
            &urlname,
            info.as_deref().unwrap_or(""),
            longinfo.as_deref().unwrap_or(""),
            resolvable,
            moderator.candel,
            None,
            false,
            &csrf_token,
        ));
    }
    Ok(Html(html))
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

#[derive(Deserialize)]
pub struct GroupModForm {
    pub group: i32,
    pub title: String,
    pub info: String,
    #[serde(rename = "urlName")]
    pub url_name: String,
    pub longinfo: String,
    pub preview: Option<String>,
    pub resolvable: Option<String>,
}

async fn groupmod_save(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    Form(form): Form<GroupModForm>,
) -> Result<Html<String>> {
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
        return Ok(Html(render_groupmod_form(
            form.group,
            &effective_title,
            &effective_urlname,
            &info,
            &longinfo,
            resolvable,
            is_admin,
            None,
            true,
            &csrf_token,
        )));
    }

    if let Some(error) = validate_url_name(&effective_urlname) {
        return Ok(Html(render_groupmod_form(
            form.group,
            &effective_title,
            &effective_urlname,
            &info,
            &longinfo,
            resolvable,
            is_admin,
            Some(error),
            false,
            &csrf_token,
        )));
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

    Ok(Html("<h1>Параметры изменены</h1>".to_string()))
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

fn stUserModForm(
    optRawQuery: Option<&str>,
    mapForm: &HashMap<String, String>,
) -> Result<(crate::application::user::EnUserModAction, UserModForm)> {
    // Servlet request parameters include both query and form values. Tomcat
    // exposes a query value first when the same key is present in both, so do
    // the same here rather than silently making this body-only.
    let mapQuery: HashMap<String, String> = optRawQuery
        .map(serde_urlencoded::from_str)
        .transpose()
        .map_err(|_| AppError::NotFound)?
        .unwrap_or_default();
    let optParameter = |sName: &str| mapQuery.get(sName).or_else(|| mapForm.get(sName));

    // Spring selects the method from `action=...` before binding `id`.
    // Consequently unknown/missing actions are a 404 even if `id` is absent.
    let enAction = optParameter("action")
        .and_then(|sAction| crate::application::user::EnUserModAction::optFromForm(sAction))
        .ok_or(AppError::NotFound)?;
    let id = optParameter("id")
        .ok_or(AppError::NotFound)?
        .parse::<i32>()
        .map_err(|_| AppError::NotFound)?;
    if enAction == crate::application::user::EnUserModAction::Freeze
        && (optParameter("reason").is_none() || optParameter("shift").is_none())
    {
        return Err(AppError::NotFound);
    }

    Ok((
        enAction,
        UserModForm {
            id,
            reason: optParameter("reason").cloned(),
            shift: optParameter("shift").cloned(),
        },
    ))
}

async fn usermod_get() -> Result<Response> {
    Err(AppError::NotFound)
}

async fn usermod(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    RawQuery(optRawQuery): RawQuery,
    Form(mapForm): Form<HashMap<String, String>>,
) -> Result<Response> {
    use crate::application::user::{CUserModerationService, EnUserModOutcome, StUserModCommand};
    use crate::infra::postgres::user_moderation_repository::CUserModerationPgRepository;

    let (enAction, form) = stUserModForm(optRawQuery.as_deref(), &mapForm)?;
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
            for iTopicId in &stDelete.vecTopicIds {
                crate::search_index::index_topic(&state, *iTopicId, true).await;
            }
            for iCommentId in &stDelete.vecCommentIds {
                crate::search_index::index_comment(&state, *iCommentId).await;
            }
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
        EnSearchReindexAction, enSearchReindexAction, sJavaFormEncode, stProfileRedirect,
        stUserModForm, usermod_get,
    };
    use crate::{application::user::EnUserModAction, error::AppError};
    use axum::{
        http::{StatusCode, header},
        response::IntoResponse,
    };
    use std::collections::HashMap;

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
            Err(AppError::NotFound)
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
    Ok(Html(sWarningForm(
        &stPresentation,
        iTopicId,
        optCommentId,
        &csrf_token,
    )))
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
        EnCreateWarningOutcome::Validation(stPresentation) => Ok(Html(sWarningForm(
            &stPresentation,
            iTopicId,
            optCommentId,
            &csrf_token,
        ))
        .into_response()),
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
