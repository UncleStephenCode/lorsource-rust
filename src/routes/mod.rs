pub mod admin;
pub mod api;
pub mod auth;
pub mod comments;
pub mod groups;
pub mod rss;
pub mod search;
pub mod tags;
pub mod topics;
pub mod users;

use crate::{error::AppError, state::AppState};
use askama::Template;
use axum::{http::StatusCode, response::{Html, IntoResponse}, routing::{get, post}, Router};
use serde_json::json;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(topics::index))
        .route("/index.jsp", get(topics::index))
        .route("/about", get(about))
        .route("/forum", get(groups::forum_index))
        .route("/forum/lenta", get(topics::lenta))
        .route("/forum/:group", get(groups::group_page))
        .route("/forum/:group/archive", get(groups::group_archive))
        .route("/forum/:group/:id", get(topics::topic_page))
        .route("/forum/:group/:id/page/:page", get(topics::topic_page_with_page))
        .route("/news", get(topics::section_topics))
        .route("/news/", get(topics::section_topics))
        .route("/news/:group", get(topics::section_group_topics))
        .route("/news/:group/:id", get(topics::topic_page))
        .route("/articles", get(topics::section_topics))
        .route("/articles/", get(topics::section_topics))
        .route("/articles/:group", get(topics::section_group_topics))
        .route("/articles/:group/:id", get(topics::topic_page))
        .route("/gallery", get(topics::section_topics))
        .route("/gallery/", get(topics::section_topics))
        .route("/gallery/:group", get(topics::section_group_topics))
        .route("/gallery/:group/:id", get(topics::topic_page))
        .route("/polls", get(topics::section_topics))
        .route("/polls/", get(topics::section_topics))
        .route("/polls/:group", get(topics::section_group_topics))
        .route("/polls/:group/:id", get(topics::topic_page))
        .route("/show-topics.jsp", get(topics::legacy_show_topics))
        .route("/view-message.jsp", get(topics::legacy_view_message))
        .route("/jump-message.jsp", get(comments::jump_message))
        .route("/add.jsp", get(topics::new_topic_form).post(topics::create_topic))
        .route("/add-section.jsp", get(topics::new_topic_form))
        .route("/edit.jsp", get(topics::edit_topic_form).post(topics::edit_topic))
        .route("/delete.jsp", post(topics::delete_topic))
        .route("/undelete", post(topics::undelete_topic))
        .route("/resolve.jsp", post(topics::resolve_topic))
        .route("/add_comment.jsp", post(comments::add_comment))
        .route("/add_comment_ajax", post(comments::add_comment_ajax))
        .route("/comment-message.jsp", get(comments::comment_message))
        .route("/edit_comment", get(comments::edit_comment_form).post(comments::edit_comment))
        .route("/delete_comment.jsp", post(comments::delete_comment))
        .route("/undelete_comment", post(comments::undelete_comment))
        .route("/search.jsp", get(search::search))
        .route("/tags", get(tags::all_tags))
        .route("/tags.jsp", get(tags::all_tags))
        .route("/tags/:first_letter", get(tags::tags_by_letter))
        .route("/tag/:tag", get(tags::tag_page))
        .route("/people/:nick", get(users::profile))
        .route("/people/:nick/profile", get(users::profile_full))
        .route("/people/:nick/reactions", get(users::reactions))
        .route("/people/:nick/remarks", get(users::remarks))
        .route("/whois.jsp", get(users::legacy_whois))
        .route("/login.jsp", get(auth::login_form))
        .route("/login_process", post(auth::login))
        .route("/logout", get(auth::logout).post(auth::logout))
        .route("/register.jsp", get(auth::register_form).post(auth::register))
        .route("/lostpwd.jsp", get(auth::lost_password_form).post(auth::lost_password))
        .route("/notifications", get(api::notifications))
        .route("/notifications-count", get(api::notifications_count))
        .route("/notifications-reset", post(api::notifications_reset))
        .route("/tracker", get(api::tracker))
        .route("/tracker.jsp", get(api::tracker))
        .route("/section-rss.jsp", get(rss::section_rss))
        .route("/rss", get(rss::main_rss))
        .route("/top10.boxlet", get(api::top10_boxlet))
        .route("/articles.boxlet", get(api::articles_boxlet))
        .route("/poll.boxlet", get(api::poll_boxlet))
        .merge(admin::router())
}

#[derive(Template)]
#[template(path = "about.html")]
struct AboutTemplate<'a> {
    version: &'a str,
}

async fn about() -> Result<Html<String>, AppError> {
    Ok(Html(AboutTemplate { version: env!("CARGO_PKG_VERSION") }.render()?))
}

pub async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, axum::Json(json!({"status":"ok"})))
}

pub async fn not_found() -> AppError {
    AppError::NotFound
}
