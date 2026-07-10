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
