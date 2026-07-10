use crate::{auth::CurrentUser, error::Result, state::AppState};
use axum::{extract::State, response::Html, Json};
use serde_json::json;

pub async fn notifications(CurrentUser(user): CurrentUser) -> Json<serde_json::Value> {
    Json(json!({"user": user.map(|u| u.nick), "events": []}))
}

pub async fn notifications_count(CurrentUser(user): CurrentUser) -> Json<serde_json::Value> {
    Json(json!({"user": user.map(|u| u.nick), "count": 0}))
}

pub async fn notifications_reset() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

pub async fn tracker(State(state): State<AppState>) -> Result<Json<serde_json::Value>> {
    let topics = crate::routes::topics::list_topics(&state, None, None, 0, 20).await?;
    Ok(Json(json!({"items": topics})))
}

pub async fn top10_boxlet(State(state): State<AppState>) -> Result<Html<String>> {
    let topics = crate::routes::topics::list_topics(&state, None, None, 0, 10).await?;
    let mut html = String::from("<ul class=\"boxlet\">");
    for t in topics { html.push_str(&format!("<li><a href=\"{}\">{}</a></li>", t.topic_url(), html_escape::encode_text(&t.title))); }
    html.push_str("</ul>");
    Ok(Html(html))
}

pub async fn articles_boxlet(State(state): State<AppState>) -> Result<Html<String>> {
    let topics = crate::routes::topics::list_topics(&state, Some("articles"), None, 0, 10).await?;
    let mut html = String::from("<ul class=\"boxlet\">");
    for t in topics { html.push_str(&format!("<li><a href=\"{}\">{}</a></li>", t.topic_url(), html_escape::encode_text(&t.title))); }
    html.push_str("</ul>");
    Ok(Html(html))
}

pub async fn poll_boxlet(State(state): State<AppState>) -> Result<Html<String>> {
    let topics = crate::routes::topics::list_topics(&state, Some("polls"), None, 0, 5).await?;
    let mut html = String::from("<ul class=\"boxlet\">");
    for t in topics { html.push_str(&format!("<li><a href=\"{}\">{}</a></li>", t.topic_url(), html_escape::encode_text(&t.title))); }
    html.push_str("</ul>");
    Ok(Html(html))
}

#[derive(serde::Deserialize)]
pub struct ReactionQuery { pub msgid: Option<i32>, pub topic: Option<i32>, pub comment: Option<i32> }

#[derive(serde::Deserialize)]
pub struct ReactionForm { pub msgid: Option<i32>, pub topic: Option<i32>, pub comment: Option<i32>, pub reaction: Option<String>, pub value: Option<bool> }

pub async fn reactions_get(State(state): State<AppState>, axum::extract::Query(q): axum::extract::Query<ReactionQuery>) -> Result<Json<serde_json::Value>> {
    let Some(msgid) = q.msgid.or(q.topic).or(q.comment) else {
        return Ok(Json(json!({"reactions": {}})));
    };
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT reaction, count(*) FROM reactions_log WHERE msgid=$1 AND set_value GROUP BY reaction ORDER BY reaction",
    )
    .bind(msgid)
    .fetch_all(&state.pool)
    .await?;
    let counts: serde_json::Map<String, serde_json::Value> = rows.into_iter().map(|(k, v)| (k, json!(v))).collect();
    Ok(Json(json!({"msgid": msgid, "reactions": counts})))
}

pub async fn reactions_post(State(state): State<AppState>, CurrentUser(user): CurrentUser, axum::Form(form): axum::Form<ReactionForm>) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(crate::error::AppError::Forbidden); };
    let msgid = form.msgid.or(form.topic).or(form.comment).ok_or_else(|| crate::error::AppError::BadRequest("missing msgid".into()))?;
    let reaction = form.reaction.unwrap_or_else(|| "+1".to_string());
    let value = form.value.unwrap_or(true);
    sqlx::query("INSERT INTO reactions_log(userid,msgid,reaction,set_value) VALUES($1,$2,$3,$4)")
        .bind(user.id).bind(msgid).bind(&reaction).bind(value).execute(&state.pool).await?;
    Ok(Json(json!({"ok": true, "msgid": msgid, "reaction": reaction, "value": value})))
}

#[derive(serde::Deserialize)]
pub struct VoteForm { pub vote: i32 }

pub async fn vote(State(state): State<AppState>, CurrentUser(user): CurrentUser, axum::Form(form): axum::Form<VoteForm>) -> Result<axum::response::Redirect> {
    let Some(user) = user else { return Err(crate::error::AppError::Forbidden); };
    sqlx::query("INSERT INTO vote_users(vote, userid) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(form.vote).bind(user.id).execute(&state.pool).await?;
    sqlx::query("UPDATE votes SET votes=votes+1 WHERE id=$1").bind(form.vote).execute(&state.pool).await?;
    Ok(axum::response::Redirect::to("/polls/"))
}
