use crate::{
    application::email_domain_block::CEmailDomainBlockService,
    auth::CurrentUser,
    error::{AppError, Result},
    infra::postgres::email_domain_block_repository::CEmailDomainBlockPgRepository,
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

fn stRequestTimezone(stJar: &CookieJar) -> chrono_tz::Tz {
    stJar
        .get("tz")
        .map(|stCookie| stCookie.value())
        .filter(|sTimezone| !matches!(*sTimezone, "Factory" | "Etc/Unknown"))
        .and_then(|sTimezone| sTimezone.parse().ok())
        .or_else(|| {
            std::env::var("TZ")
                .ok()
                .and_then(|sTimezone| sTimezone.parse().ok())
        })
        .unwrap_or(chrono_tz::Europe::Moscow)
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
    CurrentUser(user): CurrentUser,
    Query(q): Query<GeoIpQuery>,
) -> Result<Json<serde_json::Value>> {
    require_moderator(&user)?;
    let parsed: std::net::IpAddr =
        q.ip.parse()
            .map_err(|_| AppError::BadRequest("Некорректный IP".into()))?;
    Ok(Json(
        json!({"ip": parsed.to_string(), "country": null, "city": null, "source": "not configured"}),
    ))
}

#[derive(Deserialize)]
pub struct ReindexForm {
    pub action: Option<String>,
}

async fn search_reindex_form(
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    require_admin(&user)?;
    Ok(Html(format!(
        r#"
<h1>Переиндексация поиска</h1>
<form method="post" action="/admin/search-reindex"><input type="hidden" name="csrf" value="{csrf_token}"><button name="action" value="current">Текущий месяц</button><button name="action" value="all">Всё</button></form>
"#
    )))
}

async fn search_reindex(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ReindexForm>,
) -> Result<Html<String>> {
    require_admin(&user)?;
    let action = form.action.unwrap_or_else(|| "current".to_string());
    match crate::search_index::reindex_all(&state).await {
        Ok((topics, comments)) => Ok(Html(format!(
            "<h1>Переиндексация завершена</h1><p>action={}</p><p>Тем: {topics}, комментариев: {comments}</p>",
            html_escape::encode_text(&action),
        ))),
        Err(e) => Ok(Html(format!(
            "<h1>Переиндексация не выполнена</h1><p class=\"error\">{}</p>",
            html_escape::encode_text(&e),
        ))),
    }
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
        html.push_str(&format!(
            "<li>#{id} <a href=\"/people/{nick}/profile\">{nick}</a> — {kind}, {date} · {} {}</li>",
            ip.as_deref().unwrap_or(""),
            ua.map(|u| format!("ua#{u}")).unwrap_or_default(),
            nick = html_escape::encode_double_quoted_attribute(nick),
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
    use crate::application::user::{
        CUserModerationService, EnUserModOutcome, StUserModCommand,
    };
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
    use super::{stJavaFormEncode, stProfileRedirect, stUserModForm, usermod_get};
    use crate::{application::user::EnUserModAction, error::AppError};
    use axum::{
        http::{StatusCode, header},
        response::IntoResponse,
    };
    use std::collections::HashMap;

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
        assert!(stUserModForm(
            Some("action=freeze&id=8&reason=&shift=%D1%87%D0%B0%D1%81"),
            &HashMap::new()
        )
        .is_ok());
    }

    #[test]
    fn profile_redirect_is_java_302_with_form_encoded_nick_and_nocache() {
        assert_eq!(stJavaFormEncode("a b+v"), "a+b%2Bv");
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
    pub user: Option<i32>,
}

async fn post_warning_form(
    CurrentUser(user): CurrentUser,
    Query(q): Query<WarningQuery>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    require_moderator(&user)?;
    Ok(Html(format!(
        r#"
<h1>Предупреждение</h1>
<form method="post" action="/post-warning" class="form">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="topic" value="{}">
  <input type="hidden" name="comment" value="{}">
  <input type="hidden" name="user" value="{}">
  <label>Причина <textarea name="reason" required></textarea></label>
  <button type="submit">Выдать предупреждение</button>
</form>
"#,
        q.topic.map(|v| v.to_string()).unwrap_or_default(),
        q.comment.map(|v| v.to_string()).unwrap_or_default(),
        q.user.map(|v| v.to_string()).unwrap_or_default()
    )))
}

#[derive(Deserialize)]
pub struct WarningForm {
    pub topic: Option<i32>,
    pub comment: Option<i32>,
    pub user: Option<i32>,
    pub reason: Option<String>,
    pub text: Option<String>,
    pub warning_type: Option<String>,
}

async fn post_warning(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<WarningForm>,
) -> Result<Redirect> {
    let moderator = require_moderator(&user)?;
    let target_user = if let Some(user_id) = form.user {
        user_id
    } else if let Some(comment_id) = form.comment {
        sqlx::query_scalar("SELECT userid FROM comments WHERE id=$1")
            .bind(comment_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?
    } else if let Some(topic_id) = form.topic {
        sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1")
            .bind(topic_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?
    } else {
        return Err(AppError::BadRequest("target is required".into()));
    };
    let message = form
        .text
        .or(form.reason)
        .unwrap_or_else(|| "warning".to_string());
    let warning_type = form.warning_type.unwrap_or_else(|| "rule".to_string());
    let warning_type = match warning_type.as_str() {
        "rule" | "tag" | "spelling" | "group" => warning_type,
        _ => "rule".to_string(),
    };
    let topic_id = if let Some(topic_id) = form.topic {
        topic_id
    } else if let Some(comment_id) = form.comment {
        sqlx::query_scalar("SELECT topic FROM comments WHERE id=$1")
            .bind(comment_id)
            .fetch_one(&state.pool)
            .await?
    } else {
        return Err(AppError::BadRequest("topic or comment is required".into()));
    };
    let warning_id: i32 = sqlx::query_scalar(
        "INSERT INTO message_warnings(topic,comment,author,message,warning_type) VALUES($1,$2,$3,$4,$5::warning_type) RETURNING id",
    )
        .bind(topic_id).bind(form.comment).bind(moderator.id).bind(&message).bind(&warning_type).fetch_one(&state.pool).await?;

    // WarningService.postWarning/UserEventService.addWarningEvent: notify
    // moderators always, plus correctors too for tag/spelling warnings
    // (those are the two types correctors are expected to police).
    let notify_correctors_too = matches!(warning_type.as_str(), "tag" | "spelling");
    let recipients: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM users WHERE canmod OR ($1 AND corrector)")
            .bind(notify_correctors_too)
            .fetch_all(&state.pool)
            .await?;
    let event_message = format!("[{warning_type}] {message}");
    for recipient in &recipients {
        sqlx::query(
            r#"INSERT INTO user_events(userid,type,private,message_id,comment_id,message,origin_user,warning_id)
               VALUES($1,'WARNING',true,$2,$3,$4,$5,$6)"#,
        )
        .bind(recipient)
        .bind(topic_id)
        .bind(form.comment)
        .bind(&event_message)
        .bind(moderator.id)
        .bind(warning_id)
        .execute(&state.pool)
        .await?;
    }
    if !recipients.is_empty() {
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=ANY($1)")
            .bind(&recipients)
            .execute(&state.pool)
            .await?;
    }

    if form.comment.is_none() {
        sqlx::query(
            r#"UPDATE topics SET open_warnings=(
                SELECT count(DISTINCT mw.author) FROM message_warnings mw
                WHERE mw.topic=topics.id AND mw.comment IS NULL AND mw.closed_by IS NULL AND mw.warning_type='rule'
            ) WHERE id=$1"#,
        )
        .bind(topic_id)
        .execute(&state.pool)
        .await?;
        Ok(Redirect::to(&format!("/jump-message.jsp?msgid={topic_id}")))
    } else {
        let nick: String = sqlx::query_scalar("SELECT nick FROM users WHERE id=$1")
            .bind(target_user)
            .fetch_one(&state.pool)
            .await?;
        Ok(Redirect::to(&format!(
            "/people/{}/profile",
            urlencoding::encode(&nick)
        )))
    }
}

#[derive(Deserialize)]
pub struct ClearWarningForm {
    pub id: i32,
}

async fn clear_warning(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ClearWarningForm>,
) -> Result<Redirect> {
    let moderator = require_moderator(&user)?;
    let topic_id: i32 = sqlx::query_scalar("SELECT topic FROM message_warnings WHERE id=$1")
        .bind(form.id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    sqlx::query("UPDATE message_warnings SET closed_by=$2, closed_when=now() WHERE id=$1 AND closed_by IS NULL")
        .bind(form.id).bind(moderator.id).execute(&state.pool).await?;
    sqlx::query(
        r#"UPDATE topics SET open_warnings=(
            SELECT count(DISTINCT mw.author) FROM message_warnings mw
            WHERE mw.topic=topics.id AND mw.comment IS NULL AND mw.closed_by IS NULL AND mw.warning_type='rule'
        ) WHERE id=$1"#,
    )
    .bind(topic_id)
    .execute(&state.pool)
    .await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={topic_id}")))
}
