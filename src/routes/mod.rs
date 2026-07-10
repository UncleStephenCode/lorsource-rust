pub mod admin;
pub mod api;
pub mod auth;
pub mod comments;
pub mod groups;
pub mod legacy;
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
        .route("/delete.jsp", get(topics::delete_topic_form).post(topics::delete_topic))
        .route("/undelete", get(topics::undelete_topic_form).post(topics::undelete_topic))
        .route("/resolve.jsp", post(topics::resolve_topic))
        .route("/add_comment.jsp", get(comments::add_comment_form).post(comments::add_comment))
        .route("/add_comment_ajax", post(comments::add_comment_ajax))
        .route("/comment-message.jsp", get(comments::comment_message))
        .route("/edit_comment", get(comments::edit_comment_form).post(comments::edit_comment))
        .route("/delete_comment.jsp", get(comments::delete_comment_form).post(comments::delete_comment))
        .route("/undelete_comment", get(comments::undelete_comment_form).post(comments::undelete_comment))
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
        .route("/notifications", get(api::notifications).post(api::notifications_reset))
        .route("/notifications-count", get(api::notifications_count))
        .route("/notifications-reset", post(api::notifications_reset))
        .route("/tracker", get(api::tracker))
        .route("/tracker.jsp", get(api::tracker))
        .route("/section-rss.jsp", get(rss::section_rss))
        .route("/rss", get(rss::main_rss))
        .route("/top10.boxlet", get(api::top10_boxlet))
        .route("/articles.boxlet", get(api::articles_boxlet))
        .route("/poll.boxlet", get(api::poll_boxlet))

        // Legacy compatibility surface discovered from the original Spring controllers.
        // Many of these currently return 501 intentionally: the URL is reserved and tested,
        // but the service/controller logic still has to be ported from Scala.
        .route("/ExceptionResolver", get(legacy::exception_resolver))
        .route("/activate", get(legacy::not_implemented).post(legacy::not_implemented))
        .route("/activate.jsp", get(legacy::not_implemented).post(legacy::not_implemented))
        .route("/addphoto.jsp", get(legacy::not_implemented).post(legacy::not_implemented))
        .route("/articles/archive", get(legacy::archive_section))
        .route("/commit.jsp", get(topics::commit_topic_form).post(topics::commit_topic))
        .route("/delete_image", get(legacy::delete_image_form).post(legacy::delete_image))
        .route("/deregister.jsp", get(legacy::not_implemented).post(legacy::not_implemented))
        .route("/errors/403", get(legacy::error_403))
        .route("/errors/404", get(legacy::error_404))
        .route("/gallery/archive", get(legacy::archive_section))
        .route("/group-lastmod.jsp", get(legacy::group_lastmod_jsp))
        .route("/group.jsp", get(legacy::group_jsp))
        .route("/help/:page", get(legacy::help_page))
        .route("/logout_all_sessions", get(auth::logout).post(auth::logout))
        .route("/markup/preview", post(legacy::markup_preview))
        .route("/memories.jsp", post(legacy::memories))
        .route("/mt.jsp", get(topics::move_topic_form).post(topics::move_topic))
        .route("/mtn.jsp", get(topics::premoderated_move_form))
        .route("/news/archive", get(legacy::archive_section))
        .route("/notifications-click", post(legacy::notifications_click))
        .route("/notifications-click/ajax", post(legacy::notifications_click))
        .route("/polls/archive", get(legacy::archive_section))
        .route("/reactions", get(api::reactions_get).post(api::reactions_post))
        .route("/reactions/ajax", post(api::reactions_post))
        .route("/remove-userpic.jsp", post(legacy::remove_userpic))
        .route("/reset-password", get(legacy::reset_password_form).post(legacy::reset_password))
        .route("/setpostscore.jsp", get(legacy::set_post_score_form).post(legacy::set_post_score))
        .route("/show-comments.jsp", get(legacy::show_comments_jsp))
        .route("/show-replies.jsp", get(legacy::show_replies_jsp))
        .route("/tags/change", get(tags::change_form).post(tags::change_tag))
        .route("/tags/delete", get(tags::delete_form).post(tags::delete_tag))
        .route("/uncommit.jsp", get(topics::uncommit_form).post(topics::uncommit))
        .route("/user-filter", get(legacy::user_filter))
        .route("/user-filter/favorite-tag", post(legacy::favorite_tag))
        .route("/user-filter/ignore-tag", post(legacy::ignore_tag))
        .route("/user-filter/ignore-user", post(legacy::ignore_user))
        .route("/view-deleted", get(legacy::view_deleted))
        .route("/view-news.jsp", get(legacy::view_news_jsp))
        .route("/view-section.jsp", get(legacy::view_section_jsp))
        .route("/vote.jsp", post(api::vote))
        .route("/yandex-tableau", get(legacy::yandex_tableau))
        .route("/check-login", get(legacy::check_login))
        .route("/people/:nick/deleted-comments", get(comments::deleted_comments_by_user))
        .route("/people/:nick/deleted-topics", get(users::deleted_topics))
        .route("/people/:nick/drafts", get(users::drafts))
        .route("/people/:nick/edit", get(users::edit_profile_form).post(users::edit_profile))
        .route("/people/:nick/favs", get(users::favs))
        .route("/people/:nick/profile/wipe", get(users::profile_wipe))
        .route("/people/:nick/remark", get(users::remark_form).post(users::save_remark))
        .route("/people/:nick/settings", get(users::settings).post(users::save_settings))
        .route("/people/:nick/tracked", get(users::tracked))
        .route("/forum/:group/:id/history", get(legacy::topic_history))
        .route("/forum/:group/:id/:commentid/history", get(legacy::comment_history))
        .route("/news/:group/:id/history", get(legacy::topic_history))
        .route("/news/:group/:id/:commentid/history", get(legacy::comment_history))
        .route("/articles/:group/:id/history", get(legacy::topic_history))
        .route("/articles/:group/:id/:commentid/history", get(legacy::comment_history))
        .route("/gallery/:group/:id/history", get(legacy::topic_history))
        .route("/gallery/:group/:id/:commentid/history", get(legacy::comment_history))
        .route("/polls/:group/:id/history", get(legacy::topic_history))
        .route("/polls/:group/:id/:commentid/history", get(legacy::comment_history))
        .route("/forum/:group/:year/:month", get(legacy::forum_archive_month))
        .route("/news/archive/:year/:month", get(legacy::archive_section_month))
        .route("/articles/archive/:year/:month", get(legacy::archive_section_month))
        .route("/gallery/archive/:year/:month", get(legacy::archive_section_month))
        .route("/polls/archive/:year/:month", get(legacy::archive_section_month))
        .route("/forum/:group/:id/thread/:thread_root", get(legacy::topic_thread_redirect))
        .route("/news/:group/:id/thread/:thread_root", get(legacy::topic_thread_redirect))
        .route("/articles/:group/:id/thread/:thread_root", get(legacy::topic_thread_redirect))
        .route("/gallery/:group/:id/thread/:thread_root", get(legacy::topic_thread_redirect))
        .route("/polls/:group/:id/thread/:thread_root", get(legacy::topic_thread_redirect))
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
