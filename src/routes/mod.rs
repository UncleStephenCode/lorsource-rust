pub mod admin;
pub mod adv;
pub mod api;
pub mod auth;
pub mod boxlets;
pub mod canonical_host;
pub mod comments;
pub mod groups;
pub mod legacy;
pub mod legacy_redirects;
pub mod media;
pub mod realtime;
pub mod rss;
pub mod search;
pub mod static_cache;
pub mod tags;
pub mod topic_deletion;
pub mod topic_moderation;
pub mod topics;
pub mod users;

use crate::{error::AppError, state::AppState};
use askama::Template;
use axum::{
    Router,
    extract::{DefaultBodyLimit, State},
    handler::Handler,
    http::{HeaderName, Method, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{MethodRouter, any as axum_any, get, post},
};
use serde_json::json;

#[derive(Template)]
#[template(path = "legacy_content.html")]
struct StLegacyContentTemplate {
    sTitle: String,
    sContentHtml: String,
}

/// Spring selects a bare `@RequestMapping` for every method admitted by the
/// firewall, but two later servlet boundaries remain observable: OPTIONS is
/// synthesized without invoking the controller and a JSP forward accepts only
/// GET/HEAD/POST/OPTIONS. `@ResponseBody` and RedirectView responses bypass
/// that JSP method gate. Keep route declarations as `any(...)` for the source
/// extractor while adapting all three layers of that dispatch contract.
fn any<H, T>(stHandler: H) -> MethodRouter<AppState>
where
    H: Handler<T, AppState> + Clone + Send + Sync + 'static,
    T: 'static,
{
    axum_any(stHandler)
        .layer(axum::middleware::from_fn(
            crate::form::merge_servlet_post_form_into_query,
        ))
        .layer(axum::middleware::from_fn(spring_any_dispatch))
}

/// A single Axum path represents two differently constrained Spring mappings
/// for the forum `pageN`/calendar shapes. Its handler performs that dispatch
/// before rendering, so the generic bare-mapping adapter must not preempt its
/// method-specific OPTIONS/405 response.
fn controller_any<H, T>(stHandler: H) -> MethodRouter<AppState>
where
    H: Handler<T, AppState> + Clone + Send + Sync + 'static,
    T: 'static,
{
    axum_any(stHandler)
        .layer(axum::middleware::from_fn(
            crate::form::merge_servlet_post_form_into_query,
        ))
        .layer(axum::middleware::from_fn(spring_controller_auto_csrf))
}

const S_SPRING_UNRESTRICTED_ALLOW: &str = "GET,HEAD,POST,PUT,PATCH,DELETE,OPTIONS";
const S_SPRING_JSP_ALLOW: &str = "GET, HEAD, POST, OPTIONS";
const H_ACCEPT_PATCH: HeaderName = HeaderName::from_static("accept-patch");

pub(crate) fn stSpringUnrestrictedOptionsResponse() -> Response {
    (
        StatusCode::OK,
        [
            (header::ALLOW, S_SPRING_UNRESTRICTED_ALLOW),
            (H_ACCEPT_PATCH, ""),
            (header::CONTENT_LENGTH, "0"),
        ],
    )
        .into_response()
}

pub(crate) fn stSpringJspMethodNotAllowedResponse() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [
            (header::ALLOW, S_SPRING_JSP_ALLOW),
            (header::CONTENT_LENGTH, "0"),
        ],
    )
        .into_response()
}

fn stEmptyServletResponse(stStatus: StatusCode) -> Response {
    (stStatus, [(header::CONTENT_LENGTH, "0")]).into_response()
}

fn stSpringJspInternalErrorResponse() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [
            (header::ALLOW, S_SPRING_JSP_ALLOW),
            (header::CONTENT_LENGTH, "0"),
        ],
    )
        .into_response()
}

fn bSpringResponseBody(stResponse: &Response) -> bool {
    stResponse
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|stValue| stValue.to_str().ok())
        .and_then(|sValue| sValue.split(';').next())
        .is_some_and(|sMediaType| sMediaType.trim().eq_ignore_ascii_case("application/json"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnSpringServletBoundary {
    Pass,
    BadRequest,
    MethodNotAllowed,
    InternalError,
}

fn enSpringServletBoundary(stMethod: &Method, stResponse: &Response) -> EnSpringServletBoundary {
    if !matches!(stMethod, &Method::PUT | &Method::PATCH | &Method::DELETE)
        || stResponse.status().is_redirection()
        || bSpringResponseBody(stResponse)
        || stResponse.status() == StatusCode::FORBIDDEN
    {
        return EnSpringServletBoundary::Pass;
    }
    if stResponse.status() == StatusCode::BAD_REQUEST {
        // Spring's argument binder writes this response before selecting a
        // view, so it never reaches the JSP servlet.
        EnSpringServletBoundary::BadRequest
    } else if stResponse.status().is_success() || stResponse.status() == StatusCode::GONE {
        // A normal ModelAndView (including the retired-feed 410 view) reaches
        // the JSP servlet, whose method gate replaces it with an empty 405.
        EnSpringServletBoundary::MethodNotAllowed
    } else {
        // ExceptionResolver chooses an error JSP. Its secondary forward is
        // rejected for the unsafe method and Jetty exposes an empty 500.
        EnSpringServletBoundary::InternalError
    }
}

fn bSpringExplicitNotFoundViewGate(sPath: &str, stMethod: &Method, stResponse: &Response) -> bool {
    sPath == "/errors/404"
        && matches!(stMethod, &Method::PUT | &Method::PATCH | &Method::DELETE)
        && stResponse.status() == StatusCode::NOT_FOUND
}

async fn spring_any_dispatch(
    stRequest: axum::extract::Request,
    cNext: axum::middleware::Next,
) -> Response {
    if stRequest.method() == Method::OPTIONS {
        return stSpringUnrestrictedOptionsResponse();
    }
    if stRequest.method() == Method::POST {
        return crate::csrf::validate_auto_post(stRequest, cNext).await;
    }
    let stMethod = stRequest.method().clone();
    let sPath = stRequest.uri().path().to_owned();
    let mut stResponse = cNext.run(stRequest).await;
    let enBoundary = if bSpringExplicitNotFoundViewGate(&sPath, &stMethod, &stResponse) {
        EnSpringServletBoundary::MethodNotAllowed
    } else {
        enSpringServletBoundary(&stMethod, &stResponse)
    };
    match enBoundary {
        EnSpringServletBoundary::Pass => stResponse,
        EnSpringServletBoundary::BadRequest => stEmptyServletResponse(StatusCode::BAD_REQUEST),
        EnSpringServletBoundary::MethodNotAllowed => stSpringJspMethodNotAllowedResponse(),
        EnSpringServletBoundary::InternalError => {
            // ExceptionResolver reports the original controller failure before
            // its selected error JSP hits the secondary servlet method gate.
            // Preserve that diagnostic side effect while replacing only the
            // externally visible response.
            let optReport = stResponse
                .extensions_mut()
                .remove::<crate::error::StInternalErrorReport>();
            let mut stReplacement = stSpringJspInternalErrorResponse();
            if let Some(stReport) = optReport {
                stReplacement.extensions_mut().insert(stReport);
            }
            stReplacement
        }
    }
}

/// Attach Java's automatic CSRF interceptor only to methods that have a
/// selected Spring mapping. `route_layer` deliberately excludes MethodRouter's
/// 405 fallback, preserving handler-selection-before-interceptor ordering.
fn auto(stRoute: MethodRouter<AppState>) -> MethodRouter<AppState> {
    stRoute.route_layer(axum::middleware::from_fn(crate::csrf::validate_auto_post))
}

fn bForumGetOnlyPageMapping(sPath: &str) -> bool {
    sPath
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .is_some_and(|sPage| sPage.starts_with("page"))
}

async fn spring_controller_auto_csrf(
    stRequest: axum::extract::Request,
    cNext: axum::middleware::Next,
) -> Response {
    let bPageMapping = bForumGetOnlyPageMapping(stRequest.uri().path());
    if stRequest.method() == Method::POST && !bPageMapping {
        return crate::csrf::validate_auto_post(stRequest, cNext).await;
    }
    let bPageOptions = stRequest.method() == Method::OPTIONS && bPageMapping;
    let mut stResponse = cNext.run(stRequest).await;
    if bPageOptions {
        // DispatcherServlet adds this (empty) capability header to its
        // synthesized OPTIONS response even for a GET-only mapping.
        stResponse
            .headers_mut()
            .insert(H_ACCEPT_PATCH, "".parse().unwrap());
        stResponse
            .headers_mut()
            .insert(header::CONTENT_LENGTH, "0".parse().unwrap());
    }
    stResponse
}

/// The original Spring `RedirectView` is HTTP/1.0-compatible by default and
/// therefore sends 302. Axum's convenience redirect uses 303, so legacy
/// controller redirects must use this response helper instead.
pub(crate) fn stFoundRedirect(sLocation: impl Into<String>) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, sLocation.into())]).into_response()
}

/// Wraps legacy controller content in the same base document used by normal
/// Askama pages. The theme middleware intentionally only replaces hooks in
/// that document, so returning a bare fragment here would bypass all themes.
pub(crate) fn sRenderLegacyContent(sTitle: &str, sContentHtml: String) -> Result<String, AppError> {
    Ok(StLegacyContentTemplate {
        sTitle: sTitle.to_owned(),
        sContentHtml,
    }
    .render()?)
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", any(topics::index))
        .route("/index.jsp", any(topics::index))
        .route("/about", any(about))
        .route("/ws", get(realtime::websocket))
        .route("/forum", any(groups::forum_index))
        .route("/forum/", any(groups::forum_index))
        .route("/forum/lenta", any(topics::lenta))
        .route("/forum/{group}", any(groups::group_page))
        .route("/forum/{group}/archive", any(groups::group_archive))
        .route("/forum/{group}/archive/", any(groups::group_archive))
        .route("/forum/{group}/{id}", any(topics::topic_page_any))
        .route(
            "/forum/{group}/{id_or_year}/{page_or_month}",
            controller_any(legacy::forum_page_or_archive),
        )
        .route(
            "/forum/{group}/{id_or_year}/{page_or_month}/",
            controller_any(legacy::forum_page_or_archive),
        )
        .route("/news/", any(topics::section_topics))
        .route("/news/{group}", any(topics::section_group_topics))
        .route("/news/{group}/{id}", any(topics::topic_page_any))
        .route(
            "/news/{group}/{id}/{page_marker}",
            get(topics::topic_page_with_page),
        )
        .route("/articles/", any(topics::section_topics))
        .route("/articles/{group}", any(topics::section_group_topics))
        .route("/articles/{group}/{id}", any(topics::topic_page_any))
        .route(
            "/articles/{group}/{id}/{page_marker}",
            get(topics::topic_page_with_page),
        )
        .route("/gallery/", any(topics::section_topics))
        .route("/gallery/preview/{file}", get(media::gallery_preview))
        .route(
            "/gallery-uploads/preview/{file}",
            get(media::gallery_preview),
        )
        .route("/gallery/{group}", any(topics::section_group_topics))
        .route("/gallery/{group}/{id}", any(topics::topic_page_any))
        .route(
            "/gallery/{group}/{id}/{page_marker}",
            get(topics::topic_page_with_page),
        )
        .route("/polls/", any(topics::section_topics))
        .route("/polls/{group}", any(topics::section_group_topics))
        .route("/polls/{group}/{id}", any(topics::topic_page_any))
        .route(
            "/polls/{group}/{id}/{page_marker}",
            get(topics::topic_page_with_page),
        )
        .route("/images/{id}/{file}", get(media::finalized_image))
        .route("/photos/{file}", get(media::userpic))
        .route("/show-topics.jsp", get(topics::legacy_show_topics))
        .route("/view-all.jsp", get(topics::view_all))
        .route("/view-message.jsp", any(legacy::legacy_view_message))
        .route("/jump-message.jsp", get(comments::jump_message))
        .route(
            "/add.jsp",
            get(topics::new_topic_form)
                .post(topics::create_topic)
                .layer(DefaultBodyLimit::max(34 * 1024 * 1024)),
        )
        .route("/add-section.jsp", any(topics::choose_topic_section))
        .route("/edit.jsp", auto(topics::stEditTopicRoute()))
        .route("/delete.jsp", auto(topic_deletion::stDeleteRoute()))
        .route("/undelete", auto(topic_deletion::stUndeleteRoute()))
        .route("/resolve.jsp", auto(topic_moderation::stResolveRoute()))
        .route(
            "/add_comment.jsp",
            get(comments::add_comment_form).post(comments::add_comment),
        )
        .route("/add_comment_ajax", post(comments::add_comment_ajax))
        .route("/comment-message.jsp", comments::stCommentMessageRoute())
        .route(
            "/edit_comment",
            get(comments::edit_comment_form).post(comments::edit_comment),
        )
        .route(
            "/delete_comment.jsp",
            auto(comments::stDeleteCommentRoute()),
        )
        .route(
            "/undelete_comment",
            auto(comments::stUndeleteCommentRoute()),
        )
        .route("/search.jsp", get(search::search))
        .route("/tags", any(tags::all_tags))
        .route("/tags.jsp", any(tags::old_tags_redirect))
        .route("/tags/{first_letter}", any(tags::tags_by_letter))
        .route("/tag/{tag}", get(tags::tag_page))
        // Put exact people sub-pages before the short /people/{nick} route.
        // This keeps the surface compatible with Spring MVC and avoids accidental
        // 404s on /people/<nick>/profile and /people/<nick>/settings in Axum.
        .route("/people/{nick}/profile", get(users::profile_full))
        .route("/people/{nick}/profile/", get(users::profile_full))
        .route("/people/{nick}/reactions", any(users::reactions))
        .route(
            "/people/{nick}/reactions/{mode}",
            any(users::reactions_mode),
        )
        .route("/people/{nick}/remarks", any(users::remarks))
        .route("/whois.jsp", any(users::legacy_whois))
        .route("/login.jsp", get(auth::login_form))
        .route("/login_process", auto(post(auth::login)))
        // Java's LoginController only maps POST for the actual clearing
        // action (a bare `<a href>` GET would be a CSRF-able logout) - the
        // base.html top-nav link now submits a POST form to match.
        .route("/logout", auto(get(auth::logout_link).post(auth::logout)))
        .route(
            "/register.jsp",
            auto(get(auth::register_form).post(auth::register)),
        )
        .route(
            "/lostpwd.jsp",
            auto(get(auth::lost_password_form).post(auth::lost_password)),
        )
        .route(
            "/notifications",
            auto(get(api::notifications).post(api::notifications_mark_read)),
        )
        .route("/notifications-count", get(api::notifications_count))
        .route("/notifications-reset", auto(post(api::notifications_reset)))
        .route("/tracker", any(api::tracker))
        .route("/tracker/", any(api::tracker))
        .route("/tracker.jsp", any(api::tracker_old_redirect))
        .route("/section-rss.jsp", any(rss::section_rss))
        .route("/top10.boxlet", any(api::top10_boxlet))
        .route("/articles.boxlet", any(api::articles_boxlet))
        .route("/poll.boxlet", any(api::poll_boxlet))
        .route("/gallery.boxlet", any(boxlets::gallery))
        .route("/tagcloud.boxlet", any(boxlets::tagCloud))
        // Legacy compatibility surface discovered from the original Spring controllers.
        // The URL shapes are declared here; deep business-rule parity is tracked in docs.
        .route("/ExceptionResolver", any(legacy::exception_resolver))
        .route(
            "/activate",
            auto(get(legacy::activate_form).post(legacy::activate_post)),
        )
        .route(
            "/activate.jsp",
            auto(get(legacy::activate_form).post(legacy::activate_post)),
        )
        .route(
            "/addphoto.jsp",
            auto(get(legacy::addphoto_form).post(legacy::upload_userpic)),
        )
        .route("/articles/archive", any(legacy::archive_section))
        .route("/articles/archive/", any(legacy::archive_section))
        .route("/commit.jsp", topics::stCommitTopicRoute())
        .route(
            "/delete_image",
            auto(get(legacy::delete_image_form).post(legacy::delete_image)),
        )
        .route(
            "/deregister.jsp",
            auto(get(legacy::deregister_form).post(legacy::deregister_post)),
        )
        .route("/errors/403", any(legacy::error_403))
        .route("/errors/404", any(legacy::error_404))
        .route("/gallery/archive", any(legacy::archive_section))
        .route("/gallery/archive/", any(legacy::archive_section))
        .route("/group-lastmod.jsp", any(legacy::group_lastmod_jsp))
        .route("/group.jsp", any(legacy::group_jsp))
        .route("/help/{page}", any(legacy::help_page))
        .route(
            "/logout_all_sessions",
            auto(get(auth::logout_link).post(auth::logout_all_sessions)),
        )
        .route("/markup/preview", auto(post(legacy::markup_preview)))
        .route("/memories.jsp", post(legacy::memories))
        .route("/mt.jsp", auto(topic_moderation::stMoveRoute()))
        .route("/mtn.jsp", topic_moderation::stPremoderatedMoveRoute())
        .route("/news/archive", any(legacy::archive_section))
        .route("/news/archive/", any(legacy::archive_section))
        .route(
            "/notifications-click",
            auto(post(legacy::notifications_click)),
        )
        .route(
            "/notifications-click/ajax",
            auto(post(legacy::notifications_click_ajax)),
        )
        .route("/polls/archive", any(legacy::archive_section))
        .route("/polls/archive/", any(legacy::archive_section))
        .route(
            "/reactions",
            auto(get(api::reactions_get).post(api::reactions_post)),
        )
        .route("/reactions/ajax", auto(post(api::reactions_post_ajax)))
        .route("/remove-userpic.jsp", auto(post(legacy::remove_userpic)))
        .route(
            "/reset-password",
            auto(get(legacy::reset_password_form).post(auth::reset_password_with_code)),
        )
        .route(
            "/setpostscore.jsp",
            auto(get(legacy::set_post_score_form).post(legacy::set_post_score)),
        )
        .route("/show-comments.jsp", any(legacy::show_comments_jsp))
        .route("/show-replies.jsp", get(legacy::show_replies_jsp))
        .route(
            "/tags/change",
            auto(get(tags::change_form).post(tags::change_tag)),
        )
        .route(
            "/tags/delete",
            auto(get(tags::delete_form).post(tags::delete_tag)),
        )
        .route("/uncommit.jsp", auto(topic_moderation::stUncommitRoute()))
        .route("/user-filter", get(legacy::user_filter))
        .route("/user-filter/favorite-tag", post(legacy::favorite_tag))
        .route("/user-filter/ignore-tag", post(legacy::ignore_tag))
        .route("/user-filter/ignore-user", post(legacy::ignore_user))
        .route("/view-deleted", any(legacy::view_deleted))
        .route("/view-news.jsp", get(legacy::view_news_jsp))
        .route("/view-section.jsp", any(legacy::view_section_jsp))
        .route("/vote.jsp", auto(post(api::vote)))
        .route("/yandex-tableau", get(legacy::yandex_tableau))
        .route("/check-login", any(legacy::check_login))
        .route(
            "/people/{nick}/deleted-comments",
            any(comments::deleted_comments_by_user),
        )
        .route("/people/{nick}/deleted-topics", get(users::deleted_topics))
        .route("/people/{nick}/drafts", any(users::drafts))
        .route(
            "/people/{nick}/edit",
            auto(get(users::edit_profile_form).post(users::edit_profile)),
        )
        .route("/people/{nick}/favs", any(users::favs))
        .route("/people/{nick}/profile/wipe", get(users::profile_wipe))
        .route(
            "/people/{nick}/remark",
            auto(get(users::remark_form).post(users::save_remark)),
        )
        .route(
            "/people/{nick}/remark/",
            auto(get(users::remark_form).post(users::save_remark)),
        )
        .route(
            "/people/{nick}/settings",
            auto(get(users::settings).post(users::save_settings)),
        )
        .route(
            "/people/{nick}/settings/",
            auto(get(users::settings).post(users::save_settings)),
        )
        .route("/people/{nick}/tracked", any(users::tracked))
        .route("/people/{nick}", any(users::topic_feed))
        .route("/people/{nick}/", any(users::topic_feed))
        .route("/forum/{group}/{id}/history", any(legacy::topic_history))
        .route(
            "/forum/{group}/{id}/{commentid}/history",
            any(legacy::comment_history),
        )
        .route("/news/{group}/{id}/history", any(legacy::topic_history))
        .route(
            "/news/{group}/{id}/{commentid}/history",
            any(legacy::comment_history),
        )
        .route("/articles/{group}/{id}/history", any(legacy::topic_history))
        .route(
            "/articles/{group}/{id}/{commentid}/history",
            any(legacy::comment_history),
        )
        .route("/gallery/{group}/{id}/history", any(legacy::topic_history))
        .route(
            "/gallery/{group}/{id}/{commentid}/history",
            any(legacy::comment_history),
        )
        .route("/polls/{group}/{id}/history", any(legacy::topic_history))
        .route(
            "/polls/{group}/{id}/{commentid}/history",
            any(legacy::comment_history),
        )
        .route(
            "/news/archive/{year}/{month}",
            any(legacy::archive_section_month),
        )
        .route(
            "/news/archive/{year}/{month}/",
            any(legacy::archive_section_month),
        )
        .route(
            "/articles/archive/{year}/{month}",
            any(legacy::archive_section_month),
        )
        .route(
            "/articles/archive/{year}/{month}/",
            any(legacy::archive_section_month),
        )
        .route(
            "/gallery/archive/{year}/{month}",
            any(legacy::archive_section_month),
        )
        .route(
            "/gallery/archive/{year}/{month}/",
            any(legacy::archive_section_month),
        )
        .route(
            "/polls/archive/{year}/{month}",
            any(legacy::archive_section_month),
        )
        .route(
            "/polls/archive/{year}/{month}/",
            any(legacy::archive_section_month),
        )
        .route(
            "/forum/{group}/{id}/thread/{thread_root}",
            get(topics::topic_thread),
        )
        .route(
            "/news/{group}/{id}/thread/{thread_root}",
            get(topics::topic_thread),
        )
        .route(
            "/articles/{group}/{id}/thread/{thread_root}",
            get(topics::topic_thread),
        )
        .route(
            "/gallery/{group}/{id}/thread/{thread_root}",
            get(topics::topic_thread),
        )
        .route(
            "/polls/{group}/{id}/thread/{thread_root}",
            get(topics::topic_thread),
        )
        .merge(admin::router())
}

#[derive(Template)]
#[template(path = "about.html")]
struct AboutTemplate<'a> {
    version: &'a str,
    moderators: Vec<String>,
    correctors: Vec<String>,
}

/// ServerInfoController populates moderators/correctors from UserService -
/// the previous AboutTemplate only had a version string.
async fn about(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Html<String>, AppError> {
    let moderators: Vec<String> = sqlx::query_scalar(
        "SELECT nick FROM users WHERE canmod AND NOT COALESCE(blocked,false) ORDER BY nick",
    )
    .fetch_all(&state.pool)
    .await?;
    let correctors: Vec<String> = sqlx::query_scalar(
        "SELECT nick FROM users WHERE corrector AND NOT COALESCE(blocked,false) ORDER BY nick",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Html(
        AboutTemplate {
            version: env!("CARGO_PKG_VERSION"),
            moderators,
            correctors,
        }
        .render()?,
    ))
}

pub async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, axum::Json(json!({"status":"ok"})))
}

pub async fn readyz(State(stState): State<AppState>) -> impl IntoResponse {
    let futDatabase = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&stState.pool),
    );
    let futOpenSearch = async {
        let sBaseUrl = stState.config.opensearch_url.as_deref()?;
        Some(
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                stState
                    .http
                    .head(format!("{sBaseUrl}/{}", crate::search_index::INDEX))
                    .send(),
            )
            .await
            .is_ok_and(|stResult| {
                stResult.is_ok_and(|stResponse| stResponse.status().is_success())
            }),
        )
    };
    let (stDatabaseResult, optOpenSearchReady) = tokio::join!(futDatabase, futOpenSearch);
    let bDatabaseReady = stDatabaseResult.is_ok_and(|stResult| stResult.is_ok());
    let bOpenSearchReady = optOpenSearchReady.unwrap_or(true);
    let bReady = bDatabaseReady && bOpenSearchReady;
    let stStatus = if bReady {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        stStatus,
        axum::Json(json!({
            "status": if bReady { "ready" } else { "not_ready" },
            "database": if bDatabaseReady { "ok" } else { "unavailable" },
            "opensearch": match optOpenSearchReady {
                None => "disabled",
                Some(true) => "ok",
                Some(false) => "unavailable",
            }
        })),
    )
}

pub async fn not_found(method: Method, uri: axum::http::Uri) -> Response {
    let path = uri.path();

    // Spring MVC / the historic LOR instance are tolerant to a trailing slash
    // on many legacy URLs. Axum is exact: /forum and /forum/ are different
    // routes. Normalize unknown trailing-slash paths to their canonical form
    // for safe requests only. Redirecting POST/PUT/PATCH/DELETE from a
    // fallback could replay a body against an unintended legacy handler.
    if matches!(method, Method::GET | Method::HEAD) && path.len() > 1 && path.ends_with('/') {
        let mut target = path.trim_end_matches('/').to_string();
        if let Some(query) = uri.query() {
            target.push('?');
            target.push_str(query);
        }
        return stFoundRedirect(target);
    }

    AppError::NotFound.into_response()
}

#[cfg(test)]
mod theme_shell_tests {
    use std::{net::SocketAddr, str::FromStr};

    use axum::body::{Body, to_bytes};
    use axum::extract::ConnectInfo;
    use axum::http::{Method, Request, StatusCode, Uri, header};
    use axum::response::IntoResponse;
    use axum::{Router, routing::any as axum_any};
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::{
        EnSpringServletBoundary, S_SPRING_JSP_ALLOW, S_SPRING_UNRESTRICTED_ALLOW,
        bSpringExplicitNotFoundViewGate, enSpringServletBoundary, not_found, router,
        sRenderLegacyContent, stFoundRedirect, stSpringJspMethodNotAllowedResponse,
        stSpringUnrestrictedOptionsResponse,
    };

    fn stCsrfRoutingApp() -> Router {
        let oPool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("lazy test pool");
        let stState = crate::state::AppState::new(
            crate::config::Config {
                host: "127.0.0.1".to_owned(),
                port: 0,
                database_url: "postgres://unused:unused@127.0.0.1:1/unused".to_owned(),
                public_url: "http://127.0.0.1".to_owned(),
                ws_url: "ws://127.0.0.1/".to_owned(),
                static_dir: "static".to_owned(),
                upload_dir: "uploads".to_owned(),
                site_secret: "test-site-secret-test-site-secret".to_owned(),
                opensearch_url: None,
                captcha_public_key: None,
                captcha_private_key: None,
                captcha_verify_url: "https://hcaptcha.com/siteverify".to_owned(),
                admin_email: None,
                smtp_host: "localhost".to_owned(),
                smtp_port: 25,
                smtp_helo_name: "localhost".to_owned(),
                telegram_token: None,
                fallback_proxy_url: None,
                enable_background_jobs: false,
                clean_old_userpics: false,
                trusted_proxy_cidrs: Vec::new(),
                page_size: 30,
                enable_hsts: false,
                enable_dev_bypasses: false,
            },
            oPool,
        );
        Router::new()
            .merge(router())
            .fallback_service(
                axum_any(not_found)
                    .layer(axum::middleware::from_fn(crate::csrf::validate_auto_post)),
            )
            .layer(axum::middleware::from_fn_with_state(
                stState.clone(),
                crate::csrf::apply,
            ))
            .with_state(stState)
    }

    #[test]
    fn route_table_constructs_without_overlapping_paths() {
        let _stRouter = router();
    }

    #[tokio::test]
    async fn full_router_selects_mapping_before_auto_csrf() {
        let stApp = stCsrfRoutingApp();
        for (sPath, stExpected, optAllow) in [
            ("/definitely-missing", StatusCode::FORBIDDEN, None),
            ("/mtn.jsp", StatusCode::METHOD_NOT_ALLOWED, Some("GET")),
            (
                "/forum/games/9101003/pagebad",
                StatusCode::METHOD_NOT_ALLOWED,
                Some("GET"),
            ),
            ("/forum/games/2026/8", StatusCode::FORBIDDEN, None),
            ("/about", StatusCode::FORBIDDEN, None),
            ("/notifications-reset", StatusCode::FORBIDDEN, None),
        ] {
            let mut stRequest = Request::builder()
                .method(Method::POST)
                .uri(sPath)
                .body(Body::empty())
                .unwrap();
            stRequest.extensions_mut().insert(ConnectInfo(
                SocketAddr::from_str("127.0.0.1:12345").unwrap(),
            ));
            let stResponse = stApp.clone().oneshot(stRequest).await.unwrap();
            assert_eq!(stResponse.status(), stExpected, "{sPath}");
            assert_eq!(
                stResponse
                    .headers()
                    .get(header::ALLOW)
                    .and_then(|stValue| stValue.to_str().ok()),
                optAllow,
                "{sPath}"
            );
        }

        let stValidFallback = Request::builder()
            .method(Method::POST)
            .uri("/definitely-missing")
            .header(header::COOKIE, "CSRF_TOKEN=csrf")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("csrf=csrf"))
            .unwrap();
        assert_eq!(
            stApp
                .clone()
                .oneshot(stValidFallback)
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );

        let mut stPageOptions = Request::builder()
            .method(Method::OPTIONS)
            .uri("/forum/games/9101003/page2")
            .body(Body::empty())
            .unwrap();
        stPageOptions.extensions_mut().insert(ConnectInfo(
            SocketAddr::from_str("127.0.0.1:12345").unwrap(),
        ));
        let stPageOptions = stApp.oneshot(stPageOptions).await.unwrap();
        assert_eq!(stPageOptions.status(), StatusCode::OK);
        assert_eq!(stPageOptions.headers().get("accept-patch").unwrap(), "");
        assert_eq!(
            stPageOptions.headers().get(header::CONTENT_LENGTH).unwrap(),
            "0"
        );
    }

    #[test]
    fn spring_redirect_view_helper_uses_302_and_preserves_location() {
        let stResponse = stFoundRedirect("/forum/general?offset=30");
        assert_eq!(stResponse.status(), StatusCode::FOUND);
        assert_eq!(
            stResponse.headers().get(header::LOCATION).unwrap(),
            "/forum/general?offset=30"
        );
    }

    #[tokio::test]
    async fn trailing_slash_fallback_redirects_only_safe_methods() {
        let stGet = not_found(Method::GET, Uri::from_static("/missing/?x=1")).await;
        assert_eq!(stGet.status(), StatusCode::FOUND);
        assert_eq!(
            stGet.headers().get(header::LOCATION).unwrap(),
            "/missing?x=1"
        );

        let stPost = not_found(Method::POST, Uri::from_static("/missing/")).await;
        assert_eq!(stPost.status(), StatusCode::NOT_FOUND);
        assert!(stPost.headers().get(header::LOCATION).is_none());
    }

    #[tokio::test]
    async fn unrestricted_spring_options_is_empty_and_advertises_all_methods() {
        let stResponse = stSpringUnrestrictedOptionsResponse();
        assert_eq!(stResponse.status(), StatusCode::OK);
        assert_eq!(
            stResponse.headers().get(header::ALLOW).unwrap(),
            S_SPRING_UNRESTRICTED_ALLOW
        );
        assert_eq!(
            stResponse.headers().get(header::CONTENT_LENGTH).unwrap(),
            "0"
        );
        assert_eq!(stResponse.headers().get("accept-patch").unwrap(), "");
        assert!(stResponse.headers().get(header::CONTENT_TYPE).is_none());
        assert!(
            to_bytes(stResponse.into_body(), 1)
                .await
                .expect("empty OPTIONS body")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn jsp_servlet_gate_has_the_pinned_allow_contract() {
        let stResponse = stSpringJspMethodNotAllowedResponse();
        assert_eq!(stResponse.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            stResponse.headers().get(header::ALLOW).unwrap(),
            S_SPRING_JSP_ALLOW
        );
        assert_eq!(
            stResponse.headers().get(header::CONTENT_LENGTH).unwrap(),
            "0"
        );
        assert!(stResponse.headers().get(header::CONTENT_TYPE).is_none());
        assert!(
            to_bytes(stResponse.into_body(), 1)
                .await
                .expect("empty method-gate body")
                .is_empty()
        );
    }

    #[test]
    fn unsafe_any_dispatch_separates_response_body_redirect_binding_and_view_outcomes() {
        let stHtml = (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "page",
        )
            .into_response();
        let stRss = (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/rss+xml")],
            "feed",
        )
            .into_response();
        let stJson = (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json;charset=utf-8")],
            "{}",
        )
            .into_response();
        let stRedirect = StatusCode::FOUND.into_response();
        let stBadBinding = StatusCode::BAD_REQUEST.into_response();
        let stSecurityDenied = StatusCode::FORBIDDEN.into_response();
        let stRetiredView = StatusCode::GONE.into_response();
        let stErrorView = StatusCode::NOT_FOUND.into_response();

        assert_eq!(
            enSpringServletBoundary(&Method::GET, &stHtml),
            EnSpringServletBoundary::Pass
        );
        assert_eq!(
            enSpringServletBoundary(&Method::PUT, &stJson),
            EnSpringServletBoundary::Pass
        );
        assert_eq!(
            enSpringServletBoundary(&Method::PUT, &stRedirect),
            EnSpringServletBoundary::Pass
        );
        assert_eq!(
            enSpringServletBoundary(&Method::PATCH, &stSecurityDenied),
            EnSpringServletBoundary::Pass
        );
        for stResponse in [&stHtml, &stRss, &stRetiredView] {
            assert_eq!(
                enSpringServletBoundary(&Method::PUT, stResponse),
                EnSpringServletBoundary::MethodNotAllowed
            );
        }
        assert_eq!(
            enSpringServletBoundary(&Method::PUT, &stBadBinding),
            EnSpringServletBoundary::BadRequest
        );
        assert_eq!(
            enSpringServletBoundary(&Method::DELETE, &stErrorView),
            EnSpringServletBoundary::InternalError
        );
        assert!(bSpringExplicitNotFoundViewGate(
            "/errors/404",
            &Method::DELETE,
            &stErrorView
        ));
        assert!(!bSpringExplicitNotFoundViewGate(
            "/tags/a",
            &Method::DELETE,
            &stErrorView
        ));
    }

    #[test]
    fn legacy_browser_content_is_rendered_inside_the_theme_shell() {
        let sHtml = sRenderLegacyContent(
            "Проверка <title>",
            "<h1 id=\"legacy-content\">Проверка</h1>".to_owned(),
        )
        .expect("legacy content template");

        assert!(sHtml.starts_with("<!doctype html>"));
        assert!(sHtml.contains("<!-- LOR_THEME_HEADER -->"));
        assert!(sHtml.contains("<!-- LOR_THEME_FOOTER -->"));
        assert!(sHtml.contains("<main id=\"bd\">"));
        assert!(sHtml.contains("<h1 id=\"legacy-content\">Проверка</h1>"));
        // Askama 0.16's HTML escaper uses numeric entities for angle
        // brackets.  Assert the exact safe output so a raw nested `<title>`
        // can never slip into the themed legacy shell.
        assert!(sHtml.contains("<title>Проверка &#60;title&#62;</title>"));
    }
}
