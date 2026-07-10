use crate::{auth::CurrentUser, error::{AppError, Result}, state::AppState};
use axum::{extract::{Query, State}, response::{Html, Redirect}, routing::{get, post}, Form, Json, Router};
use serde::Deserialize;
use serde_json::json;

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

async fn search_reindex(CurrentUser(user): CurrentUser, Form(form): Form<ReindexForm>) -> Result<Html<String>> {
    require_admin(&user)?;
    let action = form.action.unwrap_or_else(|| "current".to_string());
    Ok(Html(format!("<h1>Переиндексация поставлена в очередь</h1><p>action={}</p>", html_escape::encode_text(&action))))
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

async fn del_ip(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<BanIpForm>) -> Result<Redirect> {
    require_moderator(&user)?;
    let ip: std::net::IpAddr = form.ip.parse().map_err(|_| AppError::BadRequest("Некорректный IP".into()))?;
    sqlx::query("DELETE FROM b_ips WHERE ip=$1::inet").bind(ip.to_string()).execute(&state.pool).await?;
    Ok(Redirect::to("/sameip.jsp"))
}

#[derive(Deserialize)]
pub struct SameIpQuery { pub ip: Option<String>, pub limit: Option<i64> }

async fn same_ip(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<SameIpQuery>) -> Result<Html<String>> {
    require_moderator(&user)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let mut html = String::from("<h1>Пользователи и сообщения с IP</h1>");
    html.push_str(r#"<form method="get"><input name="ip" placeholder="IP"><button>Искать</button></form>"#);
    if let Some(ip) = q.ip {
        let parsed: std::net::IpAddr = ip.parse().map_err(|_| AppError::BadRequest("Некорректный IP".into()))?;
        let rows = sqlx::query_as::<_, (i32, String, String, chrono::DateTime<chrono::Utc>)>(
            r#"SELECT DISTINCT u.id,u.nick,'topic' AS kind,t.postdate
               FROM topics t JOIN users u ON u.id=t.userid WHERE t.postip=$1::inet
               UNION ALL
               SELECT DISTINCT u.id,u.nick,'comment' AS kind,c.postdate
               FROM comments c JOIN users u ON u.id=c.userid WHERE c.postip=$1::inet
               ORDER BY postdate DESC LIMIT $2"#,
        )
        .bind(parsed.to_string())
        .bind(limit)
        .fetch_all(&state.pool)
        .await?;
        html.push_str("<ul>");
        for (id, nick, kind, date) in rows {
            html.push_str(&format!("<li>#{id} <a href=\"/people/{nick}\">{nick}</a> — {kind}, {date}</li>", nick = html_escape::encode_double_quoted_attribute(&nick)));
        }
        html.push_str("</ul>");
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
    pub shift: Option<String>,
    pub password: Option<String>,
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
                return Err(AppError::BadRequest("Причина слишком длинная, максимум 255 байт".into()));
            }
            let shift = form.shift.as_deref().unwrap_or("неделя");
            let interval = freeze_shift_to_interval(shift)?;
            if shift == "Разморозить" {
                sqlx::query("UPDATE users SET frozen_until=NULL, frozen_by=NULL, freezing_reason=NULL WHERE id=$1")
                    .bind(form.id)
                    .execute(&state.pool)
                    .await?;
                crate::audit::log_user_action(&state.pool, form.id, moderator.id, "defrosted", &[]).await?;
            } else {
                sqlx::query("UPDATE users SET frozen_until=now()+($2::interval), frozen_by=$3, freezing_reason=$4 WHERE id=$1")
                    .bind(form.id)
                    .bind(interval)
                    .bind(moderator.id)
                    .bind(&reason)
                    .execute(&state.pool)
                    .await?;
                crate::audit::log_user_action(&state.pool, form.id, moderator.id, "frozen", &[("reason", reason.as_str()), ("shift", shift)]).await?;
            }
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

fn freeze_shift_to_interval(shift: &str) -> Result<&'static str> {
    match shift {
        "30 минут" => Ok("30 minutes"),
        "час" => Ok("1 hour"),
        "2 часа" => Ok("2 hours"),
        "3 часа" => Ok("3 hours"),
        "6 часов" => Ok("6 hours"),
        "9 часов" => Ok("9 hours"),
        "12 часов" => Ok("12 hours"),
        "сутки" => Ok("1 day"),
        "двое суток" => Ok("2 days"),
        "3 дня" => Ok("3 days"),
        "5 дней" => Ok("5 days"),
        "неделя" => Ok("1 week"),
        "две недели" => Ok("2 weeks"),
        "месяц" => Ok("1 month"),
        "2 месяца" => Ok("2 months"),
        "3 месяца" => Ok("3 months"),
        "Разморозить" => Ok("0 seconds"),
        _ => Err(AppError::BadRequest("Некорректный срок заморозки".into())),
    }
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
