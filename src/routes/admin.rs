use crate::{auth::CurrentUser, error::{AppError, Result}, state::AppState};
use axum::{extract::{Query, State}, response::{Html, Redirect}, routing::{get, post}, Form, Json, Router};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;

static SAME_IP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d+\.\d+\.\d+\.\d+$").expect("ip regex"));

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/geoip", get(geoip))
        .route("/admin/search-reindex", get(search_reindex_form).post(search_reindex))
        .route("/banip.jsp", post(ban_ip))
        .route("/delip.jsp", post(del_ip))
        .route("/sameip.jsp", get(same_ip))
        .route("/groupmod.jsp", get(groupmod_form).post(groupmod_save))
        .route("/usermod.jsp", post(usermod))
        .route("/post-warning", get(post_warning_form).post(post_warning))
        .route("/clear-warning", post(clear_warning))
}

fn require_moderator(user: &Option<crate::models::UserSummary>) -> Result<&crate::models::UserSummary> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if user.canmod { Ok(user) } else { Err(AppError::Forbidden) }
}

fn require_admin(user: &Option<crate::models::UserSummary>) -> Result<&crate::models::UserSummary> {
    // In the original code AdministratorOnly is stricter than ModeratorOnly.
    // The Rust compatibility schema does not expose every Spring role, so use
    // `canmod && max_score >= 100` as the dev-port approximation and keep the
    // gate explicit for later tightening.
    let user = require_moderator(user)?;
    if user.max_score.unwrap_or(0) >= 100 { Ok(user) } else { Err(AppError::Forbidden) }
}

#[derive(Deserialize)]
pub struct GeoIpQuery { pub ip: String }

async fn geoip(CurrentUser(user): CurrentUser, Query(q): Query<GeoIpQuery>) -> Result<Json<serde_json::Value>> {
    require_moderator(&user)?;
    let parsed: std::net::IpAddr = q.ip.parse().map_err(|_| AppError::BadRequest("Некорректный IP".into()))?;
    Ok(Json(json!({"ip": parsed.to_string(), "country": null, "city": null, "source": "not configured"})))
}

#[derive(Deserialize)]
pub struct ReindexForm { pub action: Option<String> }

async fn search_reindex_form(CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    require_admin(&user)?;
    Ok(Html(r#"
<h1>Переиндексация поиска</h1>
<form method="post" action="/admin/search-reindex"><button name="action" value="current">Текущий месяц</button><button name="action" value="all">Всё</button></form>
"#.to_string()))
}

async fn search_reindex(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<ReindexForm>) -> Result<Html<String>> {
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
pub struct BanIpForm { pub ip: String, pub reason: Option<String> }

async fn ban_ip(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<BanIpForm>) -> Result<Redirect> {
    let moderator = require_moderator(&user)?;
    let ip: std::net::IpAddr = form.ip.parse().map_err(|_| AppError::BadRequest("Некорректный IP".into()))?;
    sqlx::query("INSERT INTO b_ips(ip,mod_id,reason,date) VALUES($1::inet,$2,$3,now()) ON CONFLICT(ip) DO UPDATE SET mod_id=EXCLUDED.mod_id, reason=EXCLUDED.reason, date=now()")
        .bind(ip.to_string())
        .bind(moderator.id)
        .bind(form.reason.unwrap_or_default())
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to("/sameip.jsp"))
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
async fn del_ip(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<DelIpForm>) -> Result<Html<String>> {
    let moderator = require_moderator(&user)?;
    let ip: std::net::IpAddr = form.ip.parse().map_err(|_| AppError::BadRequest("Некорректный IP".into()))?;
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
    .bind(&ip).bind(cutoff).fetch_all(&state.pool).await?;
    for id in &topic_ids {
        sqlx::query("UPDATE topics SET deleted=true WHERE id=$1").bind(id).execute(&state.pool).await?;
        sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate) VALUES($1,$2,$3,now()) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now()")
            .bind(id).bind(moderator.id).bind(&form.reason).execute(&state.pool).await?;
    }

    let comment_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM comments WHERE postip=$1::inet AND postdate>=$2 AND NOT deleted",
    )
    .bind(&ip).bind(cutoff).fetch_all(&state.pool).await?;
    for id in &comment_ids {
        sqlx::query("UPDATE comments SET deleted=true WHERE id=$1").bind(id).execute(&state.pool).await?;
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

async fn same_ip(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<SameIpQuery>) -> Result<Html<String>> {
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
            if mask == 0 { None } else if mask != 32 { Some(format!("{ip}/{mask}")) } else { Some(ip.clone()) }
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
    let posts = sqlx::query_as::<_, (i32, String, String, chrono::DateTime<chrono::Utc>, Option<String>, Option<i32>)>(
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
        let block: Option<(chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>, Option<String>, bool, bool, i32)> = sqlx::query_as(
            "SELECT date, ban_date, reason, allow_posting, captcha_required, mod_id FROM b_ips WHERE ip=$1::inet",
        )
        .bind(ip)
        .fetch_optional(&state.pool)
        .await?;
        if let Some((date, ban_date, reason, allow_posting, captcha_required, mod_id)) = block {
            let moderator: Option<String> = sqlx::query_scalar("SELECT nick FROM users WHERE id=$1").bind(mod_id).fetch_optional(&state.pool).await?;
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
pub struct GroupModQuery { pub group: Option<i32> }

async fn groupmod_form(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<GroupModQuery>) -> Result<Html<String>> {
    require_moderator(&user)?;
    let groups = sqlx::query_as::<_, (i32, String, String)>("SELECT id,title,urlname FROM groups ORDER BY id")
        .fetch_all(&state.pool)
        .await?;
    let mut html = String::from("<h1>Редактирование группы</h1><ul>");
    for (id, title, urlname) in groups {
        html.push_str(&format!("<li><a href=\"/groupmod.jsp?group={id}\">#{id} {}</a> /{}</li>", html_escape::encode_text(&title), html_escape::encode_text(&urlname)));
    }
    html.push_str("</ul>");
    if let Some(id) = q.group {
        if let Some((title, info, longinfo)) = sqlx::query_as::<_, (String, Option<String>, Option<String>)>("SELECT title,info,longinfo FROM groups WHERE id=$1")
            .bind(id).fetch_optional(&state.pool).await? {
            html.push_str(&format!(r#"
<form method="post" action="/groupmod.jsp" class="form wide">
<input type="hidden" name="id" value="{id}">
<label>Название <input name="title" value="{title}"></label>
<label>Описание <textarea name="info">{info}</textarea></label>
<label>Подробно <textarea name="longinfo">{longinfo}</textarea></label>
<button type="submit">Сохранить</button>
</form>
"#, title=html_escape::encode_double_quoted_attribute(&title), info=html_escape::encode_text(info.as_deref().unwrap_or("")), longinfo=html_escape::encode_text(longinfo.as_deref().unwrap_or(""))));
        }
    }
    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct GroupModForm { pub id: i32, pub title: Option<String>, pub info: Option<String>, pub longinfo: Option<String> }

async fn groupmod_save(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<GroupModForm>) -> Result<Redirect> {
    require_moderator(&user)?;
    sqlx::query("UPDATE groups SET title=COALESCE($2,title), info=$3, longinfo=$4 WHERE id=$1")
        .bind(form.id).bind(form.title).bind(form.info).bind(form.longinfo).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/groupmod.jsp?group={}", form.id)))
}

#[derive(Deserialize)]
pub struct UserModForm {
    pub id: i32,
    pub action: String,
    pub reason: Option<String>,
    pub delta: Option<i32>,
    pub password: Option<String>,
    pub shift: Option<String>,
}

/// UserModificationController's FreezeDurations/LongFreezeDurations/Unfreeze
/// maps - the previous "freeze" action hardcoded a fixed 7-day freeze and
/// had no way to unfreeze a user at all through this endpoint.
fn freeze_duration(shift: &str) -> Option<chrono::Duration> {
    use chrono::Duration;
    Some(match shift {
        "Разморозить" => Duration::zero(),
        "30 минут" => Duration::minutes(30),
        "час" => Duration::hours(1),
        "2 часа" => Duration::hours(2),
        "3 часа" => Duration::hours(3),
        "6 часов" => Duration::hours(6),
        "9 часов" => Duration::hours(9),
        "12 часов" => Duration::hours(12),
        "сутки" => Duration::days(1),
        "двое суток" => Duration::days(2),
        "3 дня" => Duration::days(3),
        "5 дней" => Duration::days(5),
        "неделя" => Duration::weeks(1),
        "две недели" => Duration::weeks(2),
        "месяц" => Duration::days(30),
        "2 месяца" => Duration::days(60),
        "3 месяца" => Duration::days(90),
        _ => return None,
    })
}

async fn usermod(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<UserModForm>) -> Result<Redirect> {
    let moderator = require_moderator(&user)?;
    match form.action.as_str() {
        "block" => {
            let reason = form.reason.clone().unwrap_or_else(|| "blocked by moderator".to_string());
            sqlx::query("UPDATE users SET blocked=true WHERE id=$1").bind(form.id).execute(&state.pool).await?;
            sqlx::query("INSERT INTO ban_info(userid,ban_by,reason,bandate) VALUES($1,$2,$3,now()) ON CONFLICT(userid) DO UPDATE SET ban_by=EXCLUDED.ban_by, reason=EXCLUDED.reason, bandate=now()")
                .bind(form.id).bind(moderator.id).bind(&reason).execute(&state.pool).await?;
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, "block_user", &[("reason", reason.as_str())]).await?;
        }
        "unblock" => {
            sqlx::query("UPDATE users SET blocked=false WHERE id=$1").bind(form.id).execute(&state.pool).await?;
            sqlx::query("DELETE FROM ban_info WHERE userid=$1").bind(form.id).execute(&state.pool).await?;
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, "unblock_user", &[]).await?;
        }
        "score50" => {
            sqlx::query("UPDATE users SET score=GREATEST(score,50), max_score=GREATEST(max_score,50) WHERE id=$1").bind(form.id).execute(&state.pool).await?;
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, "score50", &[]).await?;
        }
        "toggle_corrector" => {
            let was_corrector: bool = sqlx::query_scalar("SELECT corrector FROM users WHERE id=$1").bind(form.id).fetch_one(&state.pool).await?;
            sqlx::query("UPDATE users SET corrector=NOT corrector WHERE id=$1").bind(form.id).execute(&state.pool).await?;
            let action = if was_corrector { "unset_corrector" } else { "set_corrector" };
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, action, &[]).await?;
        }
        "reset-password" => {
            let password = form.password.unwrap_or_else(|| "change-me".to_string());
            let hash = crate::security::password::hash(&password).map_err(|e| AppError::Anyhow(e.into()))?;
            sqlx::query("UPDATE users SET passwd=$2 WHERE id=$1").bind(form.id).bind(hash).execute(&state.pool).await?;
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, "reset_password", &[]).await?;
        }
        "remove_userinfo" => {
            sqlx::query("UPDATE users SET userinfo='' WHERE id=$1").bind(form.id).execute(&state.pool).await?;
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, "reset_info", &[]).await?;
        }
        "remove_town" => {
            sqlx::query("UPDATE users SET town='' WHERE id=$1").bind(form.id).execute(&state.pool).await?;
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, "reset_town", &[]).await?;
        }
        "remove_url" => {
            sqlx::query("UPDATE users SET url='' WHERE id=$1").bind(form.id).execute(&state.pool).await?;
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, "reset_url", &[]).await?;
        }
        "freeze" => {
            let reason = form.reason.clone().unwrap_or_default();
            if reason.len() > 255 {
                return Err(AppError::BadRequest("Причина слишком длиная, максимум 255 байт".into()));
            }
            let shift = form.shift.as_deref().unwrap_or("");
            let Some(duration) = freeze_duration(shift) else {
                return Err(AppError::BadRequest("некорректный срок заморозки".into()));
            };
            let (target_canmod, target_blocked): (bool, bool) = sqlx::query_as("SELECT COALESCE(canmod,false), COALESCE(blocked,false) FROM users WHERE id=$1")
                .bind(form.id).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
            // isFreezable: moderator can (un)freeze anyone except another moderator.
            if target_canmod {
                return Err(AppError::Forbidden);
            }
            let is_unfreeze = duration == chrono::Duration::zero();
            if !is_unfreeze && target_blocked {
                return Err(AppError::BadRequest("Пользователь блокирован, его нельзя заморозить".into()));
            }
            let until = chrono::Utc::now() + duration;
            sqlx::query("UPDATE users SET frozen_until=$2 WHERE id=$1").bind(form.id).bind(until).execute(&state.pool).await?;
            let action = if is_unfreeze { "defrosted" } else { "frozen" };
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, action, &[("reason", reason.as_str())]).await?;
        }
        "block-n-delete-comments" => {
            sqlx::query("UPDATE users SET blocked=true WHERE id=$1").bind(form.id).execute(&state.pool).await?;
            sqlx::query("UPDATE comments SET deleted=true WHERE userid=$1").bind(form.id).execute(&state.pool).await?;
            crate::audit::log_user_action(&state.pool, form.id, moderator.id, "block_user", &[("reason", "block-n-delete-comments")]).await?;
        }
        other => return Err(AppError::BadRequest(format!("unknown usermod action: {other}"))),
    }
    let nick: String = sqlx::query_scalar("SELECT nick FROM users WHERE id=$1").bind(form.id).fetch_one(&state.pool).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&nick))))
}

#[derive(Deserialize)]
pub struct WarningQuery { pub topic: Option<i32>, pub comment: Option<i32>, pub user: Option<i32> }

async fn post_warning_form(CurrentUser(user): CurrentUser, Query(q): Query<WarningQuery>) -> Result<Html<String>> {
    require_moderator(&user)?;
    Ok(Html(format!(r#"
<h1>Предупреждение</h1>
<form method="post" action="/post-warning" class="form">
  <input type="hidden" name="topic" value="{}">
  <input type="hidden" name="comment" value="{}">
  <input type="hidden" name="user" value="{}">
  <label>Причина <textarea name="reason" required></textarea></label>
  <button type="submit">Выдать предупреждение</button>
</form>
"#, q.topic.map(|v| v.to_string()).unwrap_or_default(), q.comment.map(|v| v.to_string()).unwrap_or_default(), q.user.map(|v| v.to_string()).unwrap_or_default())))
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

async fn post_warning(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<WarningForm>) -> Result<Redirect> {
    let moderator = require_moderator(&user)?;
    let target_user = if let Some(user_id) = form.user {
        user_id
    } else if let Some(comment_id) = form.comment {
        sqlx::query_scalar("SELECT userid FROM comments WHERE id=$1").bind(comment_id).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?
    } else if let Some(topic_id) = form.topic {
        sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1").bind(topic_id).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?
    } else {
        return Err(AppError::BadRequest("target is required".into()));
    };
    let message = form.text.or(form.reason).unwrap_or_else(|| "warning".to_string());
    let warning_type = form.warning_type.unwrap_or_else(|| "rule".to_string());
    let warning_type = match warning_type.as_str() {
        "rule" | "tag" | "spelling" | "group" => warning_type,
        _ => "rule".to_string(),
    };
    let topic_id = if let Some(topic_id) = form.topic {
        topic_id
    } else if let Some(comment_id) = form.comment {
        sqlx::query_scalar("SELECT topic FROM comments WHERE id=$1").bind(comment_id).fetch_one(&state.pool).await?
    } else {
        return Err(AppError::BadRequest("topic or comment is required".into()));
    };
    let warning_id: i32 = sqlx::query_scalar(
        "INSERT INTO message_warnings(topic,comment,author,message,warning_type) VALUES($1,$2,$3,$4,$5::warning_type) RETURNING id",
    )
        .bind(topic_id).bind(form.comment).bind(moderator.id).bind(message).bind(warning_type).fetch_one(&state.pool).await?;
    let _ = warning_id;
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
        Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&nick))))
    }
}

#[derive(Deserialize)]
pub struct ClearWarningForm { pub id: i32 }

async fn clear_warning(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<ClearWarningForm>) -> Result<Redirect> {
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
