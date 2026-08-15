use crate::{
    application::{
        edit_history::{CEditHistoryService, StPreparedEditHistory},
        user::{account::CUserAccountService, userpic::CUserpicService},
    },
    auth::CurrentUser,
    error::{AppError, Result},
    infra::postgres::{
        edit_history_repository::CEditHistoryPgRepository,
        user_account_repository::CUserAccountPgRepository,
        userpic_repository::CUserpicPgRepository,
    },
    markup,
    models::{CommentItem, PagerQuery, TopicSummary},
    pagination::Pager,
    state::AppState,
};
use askama::Template;
use axum::{
    Form, Json,
    extract::{ConnectInfo, Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, Uri, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;

pub async fn error_403() -> AppError {
    AppError::Forbidden
}
pub async fn error_404() -> AppError {
    AppError::NotFound
}

pub async fn exception_resolver() -> Response {
    // ExceptionController.defaultExceptionHandler is reached by the servlet
    // container with RequestDispatcher.ERROR_EXCEPTION set.  A direct client
    // request has no such server-side attribute and Java redirects it home;
    // clients cannot manufacture an exception dispatch in Axum either.
    stLegacyFoundRedirect("/".to_owned())
}

#[cfg(test)]
mod legacy_error_tests {
    use axum::{Router, http::header, routing::any};

    use super::{error_403, error_404, exception_resolver};

    async fn stStartServer() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let cApp = Router::new()
            .route("/ExceptionResolver", any(exception_resolver))
            .route("/errors/403", any(error_403))
            .route("/errors/404", any(error_404));
        let stListener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener");
        let stAddress = stListener.local_addr().expect("listener address");
        let hServer = tokio::spawn(async move {
            axum::serve(stListener, cApp)
                .await
                .expect("legacy error test server");
        });
        (stAddress, hServer)
    }

    #[tokio::test]
    async fn exception_resolver_direct_requests_match_java_redirect_for_all_mapped_methods() {
        let (stAddress, hServer) = stStartServer().await;
        let cClient = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("test client");

        for eMethod in [
            reqwest::Method::GET,
            reqwest::Method::HEAD,
            reqwest::Method::POST,
            reqwest::Method::PUT,
        ] {
            let stResponse = cClient
                .request(eMethod, format!("http://{stAddress}/ExceptionResolver"))
                .send()
                .await
                .expect("ExceptionResolver request");
            assert_eq!(stResponse.status(), reqwest::StatusCode::FOUND);
            assert_eq!(
                stResponse
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|stValue| stValue.to_str().ok()),
                Some("/")
            );
        }

        hServer.abort();
    }

    #[tokio::test]
    async fn legacy_code_pages_keep_status_content_type_and_public_html() {
        let (stAddress, hServer) = stStartServer().await;
        let cClient = reqwest::Client::new();

        for (sPath, stExpected, sMarker) in [
            (
                "/errors/403",
                reqwest::StatusCode::FORBIDDEN,
                "403 Forbidden",
            ),
            ("/errors/404", reqwest::StatusCode::NOT_FOUND, "Error 404"),
        ] {
            let stResponse = cClient
                .get(format!("http://{stAddress}{sPath}"))
                .send()
                .await
                .expect("legacy code page request");
            assert_eq!(stResponse.status(), stExpected);
            assert_eq!(
                stResponse
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|stValue| stValue.to_str().ok()),
                Some("text/html; charset=utf-8")
            );
            let sBody = stResponse.text().await.expect("legacy code page body");
            assert!(sBody.contains("id=\"warning-body\""));
            assert!(sBody.contains(sMarker));
            assert!(!sBody.contains("Exception resolver compatibility endpoint"));
        }

        hServer.abort();
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct LegacyIndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    news: Vec<crate::routes::topics::NewsTopicView>,
    main_page: bool,
    tracker_layout: bool,
    navigation: Option<crate::routes::topics::TopicListNavigation>,
    prev_link: Option<String>,
    next_link: Option<String>,
}

#[derive(Deserialize)]
pub struct LegacyGroupQuery {
    pub group: Option<String>,
    pub offset: Option<String>,
}

fn iRequiredLegacyParameter(optValue: Option<&str>, sName: &str) -> Result<i32> {
    let sValue = optValue.ok_or_else(|| {
        AppError::BadParameter(format!("Не задан обязательный параметр `{sName}`"))
    })?;
    sValue
        .parse()
        .map_err(|_| AppError::BadParameter(format!("Некорректное значение параметра `{sName}`")))
}

fn sRequiredLegacyParameter(optValue: Option<String>, sName: &str) -> Result<String> {
    optValue
        .ok_or_else(|| AppError::BadParameter(format!("Не задан обязательный параметр `{sName}`")))
}

fn optLegacyI64Parameter(optValue: Option<&str>, sName: &str) -> Result<Option<i64>> {
    optValue
        .map(|sValue| {
            sValue.parse().map_err(|_| {
                AppError::BadParameter(format!("Некорректное значение параметра `{sName}`"))
            })
        })
        .transpose()
}

pub async fn group_jsp(
    State(state): State<AppState>,
    Query(q): Query<LegacyGroupQuery>,
) -> Result<Redirect> {
    group_redirect(state, q, false).await
}

pub async fn group_lastmod_jsp(
    State(state): State<AppState>,
    Query(q): Query<LegacyGroupQuery>,
) -> Result<Redirect> {
    group_redirect(state, q, true).await
}

async fn group_redirect(state: AppState, q: LegacyGroupQuery, lastmod: bool) -> Result<Redirect> {
    let iGroupId = iRequiredLegacyParameter(q.group.as_deref(), "group")?;
    let optOffset = optLegacyI64Parameter(q.offset.as_deref(), "offset")?;
    let (section, group): (String, String) = sqlx::query_as(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END,
                  g.urlname
           FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1"#,
    )
    .bind(iGroupId)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let mut url = format!("/{section}/{group}");
    let mut params = Vec::new();
    if let Some(offset) = optOffset {
        params.push(format!("offset={offset}"));
    }
    if lastmod {
        params.push("lastmod=true".to_string());
    }
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }
    Ok(Redirect::to(&url))
}

#[derive(Deserialize)]
pub struct LegacySectionQuery {
    pub section: Option<String>,
}

pub async fn view_section_jsp(
    State(state): State<AppState>,
    Query(q): Query<LegacySectionQuery>,
) -> Result<Redirect> {
    let iSectionId = iRequiredLegacyParameter(q.section.as_deref(), "section")?;
    let section: String = sqlx::query_scalar(
        r#"SELECT CASE id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(name) END
           FROM sections WHERE id=$1"#,
    )
    .bind(iSectionId)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let target = if section == "forum" {
        "/forum".to_string()
    } else {
        format!("/{section}/")
    };
    Ok(Redirect::to(&target))
}

#[derive(Deserialize)]
pub struct ViewNewsQuery {
    pub tag: Option<String>,
}

fn stLegacyFoundRedirect(sLocation: String) -> Response {
    // The legacy Spring controllers use RedirectView's default 302. Axum's
    // Redirect::to is a 303 and is therefore not protocol-compatible here.
    (StatusCode::FOUND, [(header::LOCATION, sLocation)]).into_response()
}

fn sEncodeSpringUriPath(sValue: &str) -> String {
    // Spring's UriTemplate expands this value as a URI path, not as a form or
    // path-segment value. RFC 3986 pchar and '/' remain literal; all other
    // UTF-8 bytes are percent encoded with upper-case hex digits.
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut sEncoded = String::with_capacity(sValue.len());

    for iByte in sValue.bytes() {
        if iByte.is_ascii_alphanumeric()
            || matches!(
                iByte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b':'
                    | b'@'
                    | b'/'
            )
        {
            sEncoded.push(char::from(iByte));
        } else {
            sEncoded.push('%');
            sEncoded.push(char::from(HEX[usize::from(iByte >> 4)]));
            sEncoded.push(char::from(HEX[usize::from(iByte & 0x0f)]));
        }
    }

    sEncoded
}

fn stViewNewsRedirect(stQuery: ViewNewsQuery) -> Result<Response> {
    // TagTopicListController.tagFeedOld is selected only by the Spring
    // `params = "tag"` mapping condition. A request without that required
    // parameter is rejected by Spring with HTTP 400 before rendering.
    let sTag = stQuery
        .tag
        .ok_or_else(|| AppError::BadRequest("Required parameter 'tag' is missing".to_owned()))?;
    Ok(stLegacyFoundRedirect(format!(
        "/tag/{}",
        sEncodeSpringUriPath(&sTag)
    )))
}

pub async fn view_news_jsp(Query(stQuery): Query<ViewNewsQuery>) -> Result<Response> {
    stViewNewsRedirect(stQuery)
}

#[derive(Deserialize)]
pub struct PreviewForm {
    pub text: Option<String>,
    pub msg: Option<String>,
    pub message: Option<String>,
    pub markup: Option<String>,
}

/// MarkupPreviewController.preview: validates the markup id against
/// UserPermissionService.allowedFormats before rendering, and caps input at
/// MaxTextLength - the previous handler accepted any `markup` string
/// (including e.g. "html", which the site no longer allows anyone to pick,
/// see profile.rs's FORMAT_MODES) with no permission check at all.
pub async fn markup_preview(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<PreviewForm>,
) -> Result<Json<serde_json::Value>> {
    let text = form.text.or(form.msg).or(form.message).unwrap_or_default();

    let markup_id = form
        .markup
        .as_deref()
        .unwrap_or(crate::profile::DEFAULT_FORMAT_MODE);
    if !crate::profile::is_format_mode(markup_id) {
        return Ok(Json(json!({"error": "Недопустимый режим разметки"})));
    }
    let _ = &user; // allowed_formats is identical for anon/registered in this port (see profile::FORMAT_MODES)

    if text.is_empty() {
        return Ok(Json(json!({"html": ""})));
    }
    if text.chars().count() > 65_536 {
        return Ok(Json(json!({"error": "Слишком длинный текст"})));
    }
    let stored_markup = match markup_id {
        "markdown" => "MARKDOWN",
        "ntobr" => "BBCODE_ULB",
        "lorcode" => "BBCODE_TEX",
        _ => "PLAIN",
    };
    let stMarkupUsers = state
        .markup
        .stResolveBatch([(&*text, stored_markup)])
        .await?;
    let html = markup::render_message_with_markup_policy_and_users(
        &text,
        Some(stored_markup),
        None,
        true,
        Some(&state.config.public_url),
        Some(&stMarkupUsers),
    );
    Ok(Json(json!({"html": html})))
}

#[derive(Deserialize)]
pub struct CheckLoginQuery {
    pub nick: Option<String>,
}

pub async fn check_login(
    State(state): State<AppState>,
    Query(q): Query<CheckLoginQuery>,
) -> Result<Json<serde_json::Value>> {
    let nick = q.nick.unwrap_or_default();
    let result = if nick.is_empty() {
        "Не задан nick.".to_string()
    } else if !valid_login_name_for_java(&nick) {
        "Некорректное имя пользователя.".to_string()
    } else if nick.len() > 19 {
        "Слишком длинное имя пользователя.".to_string()
    } else if user_exists_or_similar(&state, &nick).await? {
        "Это имя пользователя уже используется. Пожалуйста выберите другое имя.".to_string()
    } else {
        "true".to_string()
    };
    Ok(Json(json!(result)))
}

/// Matches UserEventApiController.getYandexWidget: `{}` for anonymous,
/// `{"notifications": N}` once authenticated - the previous implementation
/// returned an unrelated widget-manifest shape that no real Yandex.Tableau
/// integration understands.
pub async fn yandex_tableau(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else {
        return Ok(Json(json!({})));
    };
    let count: i32 = sqlx::query_scalar("SELECT unread_events FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!({"notifications": count})))
}

/// Matches HelpController.HelpPages exactly - only these 3 real pages
/// exist; anything else 404s (the previous handler rendered a placeholder
/// for any string, which never 404'd).
fn help_page_title(page: &str) -> Option<&'static str> {
    match page {
        "lorcode.md" => Some("Разметка сообщений (LORCODE)"),
        "markdown.md" => Some("Разметка сообщений (Markdown)"),
        "rules.md" => Some("Правила форума"),
        _ => None,
    }
}

#[derive(Template)]
#[template(path = "help.html")]
struct HelpTemplate {
    title: &'static str,
    html: String,
}

pub async fn help_page(
    State(state): State<AppState>,
    Path(page): Path<String>,
) -> Result<Html<String>> {
    let Some(title) = help_page_title(&page) else {
        return Err(AppError::NotFound);
    };
    let path = format!("{}/help/{page}", state.config.static_dir);
    let source = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| AppError::NotFound)?;
    let html = markup::render_message(&source, Some(false));
    Ok(Html(HelpTemplate { title, html }.render()?))
}

const MONTH_NAMES: [&str; 12] = [
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];

pub(crate) fn month_name(month: i32) -> &'static str {
    MONTH_NAMES
        .get((month - 1) as usize)
        .copied()
        .unwrap_or("?")
}

#[derive(Template)]
#[template(path = "archive_index.html")]
pub(crate) struct ArchiveIndexTemplate {
    pub(crate) title: String,
    pub(crate) heading: String,
    pub(crate) back_url: String,
    pub(crate) back_label: String,
    pub(crate) active_url: Option<String>,
    pub(crate) archive_url: String,
    pub(crate) section_id: i32,
    pub(crate) section_urlname: String,
    pub(crate) group_urlname: Option<String>,
    pub(crate) uncommitted_count: i64,
    pub(crate) add_url: Option<String>,
    pub(crate) add_reason: String,
    pub(crate) months: Vec<ArchiveMonthLink>,
}

pub(crate) struct ArchiveMonthLink {
    pub(crate) year: i32,
    pub(crate) month_name: &'static str,
    pub(crate) count: i64,
    pub(crate) url: String,
}

/// ArchiveDao.getArchiveStats is backed by `monthly_stats` in Java.  Compute
/// the same projection live so a newly committed topic is visible without
/// waiting for the ten-minute maintenance job, while retaining the exact
/// original visibility predicate used by `update_monthly_stats()`.
pub(crate) async fn list_archive_year_months(
    state: &AppState,
    section: Option<&str>,
    group: Option<&str>,
) -> Result<Vec<(i32, i32, i64)>> {
    Ok(sqlx::query_as::<_, (i32, i32, i64)>(
        r#"SELECT EXTRACT(YEAR FROM t.postdate)::int AS y, EXTRACT(MONTH FROM t.postdate)::int AS m, count(*) AS c
           FROM topics t
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE ($1::text IS NULL OR CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END = $1)
             AND ($2::text IS NULL OR g.urlname=$2)
             AND (t.moderate OR NOT s.moderate)
             AND NOT t.deleted
           GROUP BY y, m
           ORDER BY y, m"#,
    )
    .bind(section)
    .bind(group)
    .fetch_all(&state.pool)
    .await?)
}

pub async fn archive_section(
    State(state): State<AppState>,
    uri: Uri,
    CurrentUser(current_user): CurrentUser,
) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let section_name = match section {
        "news" => "Новости",
        "forum" => "Форум",
        "gallery" => "Галерея",
        "articles" => "Статьи",
        "polls" => "Опросы",
        _ => "Темы",
    };
    let rows = list_archive_year_months(&state, Some(section), None).await?;
    let navigation =
        crate::routes::topics::build_topic_list_navigation(&state, section, None, &current_user)
            .await?;
    let months = rows
        .into_iter()
        .map(|(y, m, c)| ArchiveMonthLink {
            year: y,
            month_name: month_name(m),
            count: c,
            url: format!("/{section}/archive/{y}/{m}/"),
        })
        .collect();
    Ok(Html(
        ArchiveIndexTemplate {
            title: format!("{section_name} - Архив"),
            heading: section_name.to_string(),
            back_url: format!("/{section}/"),
            back_label: "Лента".to_string(),
            active_url: None,
            archive_url: format!("/{section}/archive/"),
            section_id: navigation.section_id,
            section_urlname: section.to_string(),
            group_urlname: None,
            uncommitted_count: navigation.uncommitted_count,
            add_url: navigation.add_url,
            add_reason: navigation.add_reason,
            months,
        }
        .render()?,
    ))
}

pub async fn archive_section_month(
    State(state): State<AppState>,
    uri: Uri,
    Path((year, month)): Path<(i32, i32)>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
) -> Result<Html<String>> {
    validate_year_month(year, month)?;
    let section = section_from_uri(&uri).unwrap_or("news");
    render_archive(
        state,
        Some(section),
        None,
        Some(year),
        Some(month),
        q,
        current_user,
    )
    .await
}

pub async fn forum_archive_month(
    State(state): State<AppState>,
    Path((group, year, month)): Path<(String, i32, i32)>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
) -> Result<Html<String>> {
    validate_year_month(year, month)?;
    render_archive(
        state,
        Some("forum"),
        Some(group),
        Some(year),
        Some(month),
        q,
        current_user,
    )
    .await
}

async fn render_archive(
    state: AppState,
    section: Option<&str>,
    group: Option<String>,
    year: Option<i32>,
    month: Option<i32>,
    q: PagerQuery,
    _current_user: Option<crate::models::UserSummary>,
) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_archive_topics(
        &state,
        section,
        group.as_deref(),
        year,
        month,
        pager.offset,
        pager.limit,
    )
    .await?;
    let news =
        crate::routes::topics::prepare_news_topics(&state, topics.clone(), group.is_none()).await?;
    let prev_link = pager.prev_offset.map(|offset| format!("?offset={offset}"));
    let next_link = Some(format!("?offset={}", pager.next_offset));
    let title = match (section, group.as_deref(), year, month) {
        (Some(sec), Some(group), Some(y), Some(m)) => {
            format!("Архив: {sec}/{group}, {y:04}-{m:02}")
        }
        (Some(sec), _, Some(y), Some(m)) => format!("Архив: {sec}, {y:04}-{m:02}"),
        (Some(sec), _, _, _) => format!("Архив: {sec}"),
        _ => "Архив".to_string(),
    };
    Ok(Html(
        LegacyIndexTemplate {
            title,
            topics,
            news,
            main_page: false,
            tracker_layout: false,
            navigation: None,
            prev_link,
            next_link,
        }
        .render()?,
    ))
}

async fn list_archive_topics(
    state: &AppState,
    section: Option<&str>,
    group: Option<&str>,
    year: Option<i32>,
    month: Option<i32>,
    offset: i64,
    limit: i64,
) -> Result<Vec<TopicSummary>> {
    Ok(sqlx::query_as::<_, TopicSummary>(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                  (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                     FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                    WHERE tg.msgid=t.id) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE ($1::text IS NULL OR CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END = $1)
             AND ($2::text IS NULL OR g.urlname=$2)
             AND ($3::int IS NULL OR EXTRACT(YEAR FROM t.postdate)::int=$3)
             AND ($4::int IS NULL OR EXTRACT(MONTH FROM t.postdate)::int=$4)
             AND NOT t.deleted
             AND NOT t.draft
             AND (t.moderate OR NOT s.moderate)
           ORDER BY t.postdate DESC
           OFFSET $5 LIMIT $6"#,
    )
    .bind(section)
    .bind(group)
    .bind(year)
    .bind(month)
    .bind(offset)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?)
}

#[derive(Template)]
#[template(path = "history.html")]
struct StHistoryTemplate {
    topic_id: i32,
    histories: Vec<StPreparedEditHistory>,
    can_restore: bool,
}

pub async fn topic_history(
    State(state): State<AppState>,
    Path((_group, id)): Path<(String, i32)>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>> {
    let Some(stUser) = user else {
        return Err(AppError::Forbidden);
    };
    let stTopic = crate::routes::topics::get_topic(&state, id).await?;
    crate::routes::topics::check_topic_viewable(&state, id, &Some(stUser.clone())).await?;
    let bExpired = crate::routes::comments::is_topic_expired(&state, id).await?;
    if !stUser.canmod && stUser.id != stTopic.author_id && bExpired {
        return Err(AppError::Forbidden);
    }
    let stRules = crate::routes::topics::load_topic_edit_rules(&state, id).await?;
    let bCanRestore = crate::routes::topics::b_topic_content_editable(&stTopic, &stRules, &stUser);
    let cService = CEditHistoryService::new(CEditHistoryPgRepository::new(state.pool.clone()));
    let vecHistories = cService
        .vecTopicHistory(id, &state.markup, &state.config.public_url)
        .await?;
    Ok(Html(
        StHistoryTemplate {
            topic_id: id,
            histories: vecHistories,
            can_restore: bCanRestore,
        }
        .render()?,
    ))
}

pub async fn comment_history(
    State(state): State<AppState>,
    Path((_group, id, commentid)): Path<(String, i32, i32)>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>> {
    let stTopic = crate::routes::topics::get_topic(&state, id).await?;
    crate::routes::topics::check_topic_viewable(&state, id, &user).await?;
    let cService = CEditHistoryService::new(CEditHistoryPgRepository::new(state.pool.clone()));
    let vecHistories = cService
        .vecCommentHistory(
            stTopic.id,
            commentid,
            &state.markup,
            &state.config.public_url,
        )
        .await?;
    Ok(Html(
        StHistoryTemplate {
            topic_id: id,
            histories: vecHistories,
            can_restore: false,
        }
        .render()?,
    ))
}

#[derive(Deserialize)]
pub struct ShowCommentsQuery {
    pub nick: Option<String>,
}

fn sShowCommentsLocation(sCanonicalNick: &str) -> String {
    // ShowCommentsController constructs a relative RedirectView target, but
    // the servlet container exposes the normalized context-root path in the
    // Location header.
    format!(
        "/search.jsp?range=COMMENTS&user={}&sort=DATE",
        urlencoding::encode(sCanonicalNick)
    )
}

pub async fn show_comments_jsp(
    State(stState): State<AppState>,
    Query(stQuery): Query<ShowCommentsQuery>,
) -> Result<Response> {
    let sRequestedNick = sRequiredLegacyParameter(stQuery.nick, "nick")?;
    // Java resolves the user before redirecting. Besides rejecting an unknown
    // nick, this puts the canonical database spelling in Location.
    let stUser = crate::routes::users::get_user_exact(&stState, &sRequestedNick).await?;
    Ok(stLegacyFoundRedirect(sShowCommentsLocation(&stUser.nick)))
}

#[cfg(test)]
mod legacy_list_redirect_tests {
    use axum::{
        http::{StatusCode, header},
        response::IntoResponse,
    };

    use super::{
        ViewNewsQuery, iRequiredLegacyParameter, optLegacyI64Parameter, sEncodeSpringUriPath,
        sRequiredLegacyParameter, sShowCommentsLocation, stLegacyFoundRedirect, stViewNewsRedirect,
    };
    use crate::error::AppError;

    #[test]
    fn view_news_requires_the_original_tag_mapping_condition() {
        let stError = stViewNewsRedirect(ViewNewsQuery { tag: None })
            .expect_err("the tag mapping condition is required");

        assert!(matches!(stError, AppError::BadRequest(_)));
        assert_eq!(stError.into_response().status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn view_news_encodes_the_tag_and_uses_java_302() {
        let stResponse = stViewNewsRedirect(ViewNewsQuery {
            tag: Some("c++ / rust".to_owned()),
        })
        .expect("legacy tag redirect");

        assert_eq!(stResponse.status(), StatusCode::FOUND);
        assert_eq!(
            stResponse
                .headers()
                .get(header::LOCATION)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("/tag/c++%20/%20rust")
        );
    }

    #[test]
    fn spring_uri_template_path_encoding_preserves_only_path_characters() {
        assert_eq!(
            sEncodeSpringUriPath("a:b@c;d,e=f&g!h$i'j(k)l*m+n/o?p#q[r]s%t u"),
            "a:b@c;d,e=f&g!h$i'j(k)l*m+n/o%3Fp%23q%5Br%5Ds%25t%20u"
        );
        assert_eq!(sEncodeSpringUriPath("тег"), "%D1%82%D0%B5%D0%B3");
    }

    #[test]
    fn show_comments_uses_the_servlet_normalized_canonical_redirect_target() {
        let stResponse = stLegacyFoundRedirect(sShowCommentsLocation("maxcom"));

        assert_eq!(stResponse.status(), StatusCode::FOUND);
        assert_eq!(
            stResponse
                .headers()
                .get(header::LOCATION)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("/search.jsp?range=COMMENTS&user=maxcom&sort=DATE")
        );
    }

    #[test]
    fn legacy_spring_binding_failures_use_bad_parameter_404() {
        for stError in [
            iRequiredLegacyParameter(None, "group").expect_err("missing group"),
            iRequiredLegacyParameter(Some("not-an-id"), "section").expect_err("invalid section"),
            optLegacyI64Parameter(Some("not-an-offset"), "offset").expect_err("invalid offset"),
            sRequiredLegacyParameter(None, "nick").expect_err("missing nick"),
        ] {
            assert!(matches!(stError, AppError::BadParameter(_)));
            assert_eq!(stError.into_response().status(), StatusCode::NOT_FOUND);
        }

        assert_eq!(iRequiredLegacyParameter(Some("42"), "group").unwrap(), 42);
        assert_eq!(
            optLegacyI64Parameter(Some("300"), "offset").unwrap(),
            Some(300)
        );
    }
}

#[derive(Deserialize)]
pub struct ShowRepliesQuery {
    pub nick: Option<String>,
    pub output: Option<String>,
    pub filter: Option<String>,
    pub offset: Option<i64>,
}

/// UserEventController's three `/show-replies.jsp` branches (Spring
/// disambiguates them via param presence - `!output`+`!nick`,
/// `!output`+`nick`, `output`):
/// 1. bare -> redirect to /notifications (must be logged in)
/// 2. `?nick=X` (no output) -> moderator-only read view of X's notifications
/// 3. `?output=rss|atom&nick=X` -> real XML feed of X's events
pub async fn show_replies_jsp(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<ShowRepliesQuery>,
) -> Result<Response> {
    if let Some(output) = q.output.as_deref() {
        let nick = q.nick.clone().unwrap_or_default();
        if !valid_login_name_for_java(&nick) {
            return Err(AppError::BadRequest("некорректное имя пользователя".into()));
        }
        let target: Option<(i32, String)> =
            sqlx::query_as("SELECT id, nick FROM users WHERE lower(nick)=lower($1)")
                .bind(&nick)
                .fetch_optional(&state.pool)
                .await?;
        let Some((target_id, target_nick)) = target else {
            return Err(AppError::NotFound);
        };
        let view_by_owner = user
            .as_ref()
            .map(|u| u.nick.eq_ignore_ascii_case(&target_nick))
            .unwrap_or(false);
        let db_type = q
            .filter
            .as_deref()
            .and_then(crate::routes::api::filter_db_type);
        let events =
            crate::routes::api::fetch_events(&state, target_id, db_type, view_by_owner, 200, 0)
                .await?;

        let is_atom = output == "atom";
        let stMarkupUsers = state
            .markup
            .stResolveBatch(
                events
                    .iter()
                    .map(|stEvent| (&*stEvent.message_text, &*stEvent.message_markup)),
            )
            .await?;
        let body = render_replies_feed(&state, &target_nick, &events, is_atom, &stMarkupUsers);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            (if is_atom {
                "application/atom+xml; charset=utf-8"
            } else {
                "application/rss+xml; charset=utf-8"
            })
            .parse()
            .unwrap(),
        );
        // Java sets `Expires: now + 90s` on this feed endpoint.
        let expires = (chrono::Utc::now() + chrono::Duration::seconds(90)).to_rfc2822();
        headers.insert(axum::http::header::EXPIRES, expires.parse().unwrap());
        return Ok((headers, body).into_response());
    }

    let Some(nick) = q.nick.clone() else {
        if user.is_none() {
            return Err(AppError::Forbidden);
        }
        return Ok(Redirect::to("/notifications").into_response());
    };
    if !valid_login_name_for_java(&nick) {
        return Err(AppError::BadRequest("некорректное имя пользователя".into()));
    }
    let Some(current) = user else {
        return Err(AppError::Forbidden);
    };
    if current.nick.eq_ignore_ascii_case(&nick) {
        return Ok(Redirect::to("/notifications").into_response());
    }
    if !current.canmod {
        return Err(AppError::Forbidden);
    }

    let target_id: Option<i32> =
        sqlx::query_scalar("SELECT id FROM users WHERE lower(nick)=lower($1)")
            .bind(&nick)
            .fetch_optional(&state.pool)
            .await?;
    let Some(target_id) = target_id else {
        return Err(AppError::NotFound);
    };
    let db_type = q
        .filter
        .as_deref()
        .and_then(crate::routes::api::filter_db_type);
    let offset = q.offset.unwrap_or(0).max(0);
    let events =
        crate::routes::api::fetch_events(&state, target_id, db_type, true, 20, offset).await?;

    let sTitle = format!("Уведомления пользователя {nick}");
    let mut html = format!(
        "<h1>{}</h1><p class=\"muted\">Просмотр от имени модератора {}.</p><ul class=\"notifications-list\">",
        html_escape::encode_text(&sTitle),
        html_escape::encode_text(&current.nick)
    );
    for e in &events {
        let sDate = crate::request_timezone::sTimeTag("interval", e.event_date);
        let sSubjectPlain = e.sSubjectPlain();
        html.push_str(&format!(
            "<li{}><a href=\"{}\">{}</a> <small>{} · {}</small></li>",
            if e.unread { " class=\"unread\"" } else { "" },
            e.link(),
            html_escape::encode_text(&sSubjectPlain),
            sDate,
            html_escape::encode_text(&e.event_type),
        ));
    }
    if events.is_empty() {
        html.push_str("<li class=\"muted\">Нет уведомлений</li>");
    }
    html.push_str("</ul>");
    Ok(Html(crate::routes::sRenderLegacyContent(&sTitle, html)?).into_response())
}

fn render_replies_feed(
    state: &AppState,
    nick: &str,
    events: &[crate::routes::api::NotificationEvent],
    atom: bool,
    stMarkupUsers: &crate::domain::markup::model::StMarkupUserDirectory,
) -> String {
    let title = format!("Уведомления пользователя {nick}");
    if atom {
        let mut body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>{}</title><link href="{}/show-replies.jsp?nick={}&amp;output=atom" rel="self"/><id>{}/show-replies.jsp?nick={}</id>"#,
            html_escape::encode_text(&title),
            state.config.public_url,
            urlencoding::encode(nick),
            state.config.public_url,
            urlencoding::encode(nick),
        );
        for e in events {
            let link = format!("{}{}", state.config.public_url, e.link());
            let sDescription = sNotificationFeedDescription(e, state, stMarkupUsers);
            let sAuthor = e
                .cid
                .map(|_| {
                    format!(
                        "<author><name>{}</name></author>",
                        html_escape::encode_text(&e.author_nick)
                    )
                })
                .unwrap_or_default();
            body.push_str(&format!(
                "<entry><title>{}</title><link href=\"{}\"/><id>{}</id><updated>{}</updated>{author}{description}</entry>",
                html_escape::encode_text(&html_escape::decode_html_entities(&e.subj)),
                html_escape::encode_double_quoted_attribute(&link),
                e.id,
                e.event_date.to_rfc3339(),
                author = sAuthor,
                description = sDescription
                    .map(|sValue| format!("<summary type=\"html\">{}</summary>", html_escape::encode_text(&sValue)))
                    .unwrap_or_default(),
            ));
        }
        body.push_str("</feed>");
        body
    } else {
        let mut body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>{}</title><link>{}/show-replies.jsp?nick={}</link><description>{}</description>"#,
            html_escape::encode_text(&title),
            state.config.public_url,
            urlencoding::encode(nick),
            html_escape::encode_text(&title),
        );
        for e in events {
            let link = format!("{}{}", state.config.public_url, e.link());
            let sDescription = sNotificationFeedDescription(e, state, stMarkupUsers)
                .map(|sValue| {
                    format!(
                        "<description>{}</description>",
                        html_escape::encode_text(&sValue)
                    )
                })
                .unwrap_or_default();
            let sAuthor = e
                .cid
                .map(|_| {
                    format!(
                        "<author>{}</author>",
                        html_escape::encode_text(&e.author_nick)
                    )
                })
                .unwrap_or_default();
            body.push_str(&format!(
                "<item><title>{}</title><link>{}</link><guid isPermaLink=\"false\">{}</guid><pubDate>{}</pubDate>{author}{description}</item>",
                html_escape::encode_text(&html_escape::decode_html_entities(&e.subj)),
                html_escape::encode_text(&link),
                e.id,
                e.event_date.to_rfc2822(),
                author = sAuthor,
                description = sDescription,
            ));
        }
        body.push_str("</channel></rss>");
        body
    }
}

fn sNotificationFeedDescription(
    stEvent: &crate::routes::api::NotificationEvent,
    stState: &AppState,
    stMarkupUsers: &crate::domain::markup::model::StMarkupUserDirectory,
) -> Option<String> {
    let sRendered = markup::render_message_with_markup_policy_and_users(
        &stEvent.message_text,
        Some(&stEvent.message_markup),
        None,
        false,
        Some(&stState.config.public_url),
        Some(stMarkupUsers),
    );
    let sRendered = sRemoveInvalidXmlChars(&sRendered);
    if stEvent.event_type == "REACTION" {
        Some(format!(
            "@{} поставил {}<br>{sRendered}",
            stEvent.author_nick,
            stEvent.reaction.as_deref().unwrap_or("X")
        ))
    } else if sRendered.is_empty() {
        None
    } else {
        Some(sRendered)
    }
}

fn sRemoveInvalidXmlChars(sValue: &str) -> String {
    sValue
        .chars()
        .filter(|cValue| {
            matches!(*cValue, '\u{9}' | '\u{A}' | '\u{D}')
                || ('\u{20}'..='\u{D7FF}').contains(cValue)
                || ('\u{E000}'..='\u{FFFD}').contains(cValue)
                || ('\u{10000}'..='\u{10FFFF}').contains(cValue)
        })
        .collect()
}

#[derive(Deserialize)]
pub struct StViewDeletedQuery {
    pub id: i32,
}

struct StPreparedDeletedComment {
    stComment: CommentItem,
    optDeleteInfo: Option<(String, String)>,
}

async fn optLoadComment(stState: &AppState, iCommentId: i32) -> Result<Option<CommentItem>> {
    Ok(sqlx::query_as::<_, CommentItem>(
        r#"SELECT c.id, c.topic, c.replyto, c.title, m.message, m.markup::text AS markup,
                  c.postdate, u.id AS author_id, u.nick AS author,
                  COALESCE(u.score,0) AS author_score,
                  COALESCE(u.blocked,false) AS author_blocked,
                  COALESCE(u.passwd,'')='' AS author_anonymous,
                  COALESCE(u.frozen_until > CURRENT_TIMESTAMP,false) AS author_frozen,
                  c.deleted
           FROM comments c JOIN msgbase m ON m.id=c.id JOIN users u ON u.id=c.userid
           WHERE c.id=$1"#,
    )
    .bind(iCommentId)
    .fetch_optional(&stState.pool)
    .await?)
}

async fn optLoadDeleteInfo(
    stState: &AppState,
    iCommentId: i32,
) -> Result<Option<(String, String)>> {
    Ok(sqlx::query_as(
        r#"SELECT u.nick,di.reason FROM del_info di JOIN users u ON u.id=di.delby
           WHERE di.msgid=$1"#,
    )
    .bind(iCommentId)
    .fetch_optional(&stState.pool)
    .await?)
}

fn sRenderDeletedComment(
    stPrepared: &StPreparedDeletedComment,
    sSiteOrigin: &str,
    stMarkupUsers: &crate::domain::markup::model::StMarkupUserDirectory,
) -> String {
    let stComment = &stPrepared.stComment;
    let sDeleteInfo = if stComment.deleted {
        stPrepared
            .optDeleteInfo
            .as_ref()
            .map(|(sNick, sReason)| {
                format!(
                    " {} по причине: {}",
                    html_escape::encode_text(sNick),
                    html_escape::encode_text(sReason)
                )
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    let sDeletedTitle = if stComment.deleted {
        format!("<div class=\"title\"><strong>Сообщение удалено{sDeleteInfo}</strong></div>")
    } else {
        String::new()
    };
    let sTitle = stComment
        .optTitlePlain()
        .map(|sTitlePlain| format!("<h1>{}</h1>", html_escape::encode_text(&sTitlePlain)))
        .unwrap_or_default();
    format!(
        "<article class=\"msg\" id=\"comment-{id}\">{deleted_title}<div class=\"msg-container\"><div class=\"msg_body\"><div class=\"msg-text\">{title}{body}</div><div class=\"sign\"><a href=\"/people/{author_url}/profile\">{author}</a>, {date}</div></div></div></article>",
        id = stComment.id,
        deleted_title = sDeletedTitle,
        title = sTitle,
        body = markup::render_message_with_markup_policy_and_users(
            &stComment.message,
            Some(&stComment.markup),
            None,
            stComment.bNofollowAuthorLinks(),
            Some(sSiteOrigin),
            Some(stMarkupUsers),
        ),
        author_url = urlencoding::encode(&stComment.author),
        author = html_escape::encode_text(&stComment.author),
        date = crate::request_timezone::sTimeTag("default", stComment.postdate),
    )
}

fn bCanViewDeletedComment(
    bCanViewAll: bool,
    iViewerId: i32,
    iAuthorId: i32,
    bViewerFrozen: bool,
    dtDeleted: chrono::DateTime<chrono::Utc>,
    dtNow: chrono::DateTime<chrono::Utc>,
) -> bool {
    bCanViewAll
        || (iViewerId == iAuthorId
            && !bViewerFrozen
            && dtDeleted > dtNow - chrono::Duration::days(14))
}

pub async fn view_deleted(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Query(stQuery): Query<StViewDeletedQuery>,
) -> Result<Html<String>> {
    let stUser = optUser.as_ref().ok_or(AppError::Forbidden)?;
    let stComment = optLoadComment(&stState, stQuery.id)
        .await?
        .filter(|stComment| stComment.deleted)
        .ok_or(AppError::NotFound)?;
    let optDeleteRow: Option<(chrono::DateTime<chrono::Utc>, String, String)> = sqlx::query_as(
        r#"SELECT di.deldate,u.nick,di.reason FROM del_info di JOIN users u ON u.id=di.delby
           WHERE di.msgid=$1"#,
    )
    .bind(stComment.id)
    .fetch_optional(&stState.pool)
    .await?;
    let Some((dtDeleted, sDeletedBy, sDeleteReason)) = optDeleteRow else {
        return Err(AppError::NotFound);
    };

    let bCanViewAll =
        crate::routes::topics::allow_view_all_deleted_comments(&stState, stComment.topic, &optUser)
            .await?;
    let bFrozen: bool = sqlx::query_scalar(
        "SELECT COALESCE(frozen_until>CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
    )
    .bind(stUser.id)
    .fetch_one(&stState.pool)
    .await?;
    if !bCanViewDeletedComment(
        bCanViewAll,
        stUser.id,
        stComment.author_id,
        bFrozen,
        dtDeleted,
        chrono::Utc::now(),
    ) {
        return Err(AppError::Forbidden);
    }

    let (sTopicUrl, iPostScore): (String, i32) = sqlx::query_as(
        r#"SELECT '/'||(CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery'
                    WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END)
                  ||'/'||g.urlname||'/'||t.id, COALESCE(t.postscore,-9999)
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(stComment.topic)
    .fetch_optional(&stState.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let mut vecChain = Vec::new();
    if iPostScore != 10002 {
        let mut optParentId = stComment.replyto.filter(|iValue| *iValue != 0);
        while let Some(iParentId) = optParentId {
            let stParent = optLoadComment(&stState, iParentId)
                .await?
                .ok_or(AppError::NotFound)?;
            let optDeleteInfo = if stParent.deleted {
                optLoadDeleteInfo(&stState, stParent.id).await?
            } else {
                None
            };
            let bContinue = stParent.deleted
                && optDeleteInfo
                    .as_ref()
                    .is_some_and(|(_, sReason)| sReason.starts_with("7.1 "));
            optParentId = bContinue
                .then_some(stParent.replyto)
                .flatten()
                .filter(|iValue| *iValue != 0);
            vecChain.push(StPreparedDeletedComment {
                stComment: stParent,
                optDeleteInfo,
            });
            if !bContinue {
                break;
            }
        }
        vecChain.reverse();
    }

    let sBackLink = if stUser.canmod {
        format!("{sTopicUrl}?cid={}", stComment.id)
    } else {
        sTopicUrl
    };
    let stMarkupUsers = stState
        .markup
        .stResolveBatch(
            vecChain
                .iter()
                .map(|stPrepared| {
                    (
                        stPrepared.stComment.message.as_str(),
                        stPrepared.stComment.markup.as_str(),
                    )
                })
                .chain(std::iter::once((
                    stComment.message.as_str(),
                    stComment.markup.as_str(),
                ))),
        )
        .await?;
    let mut sHtml = format!(
        "<h1>Просмотр удаленного комментария</h1><nav><a class=\"btn btn-default\" href=\"{}\">Перейти в топик</a></nav><div class=\"messages\">",
        html_escape::encode_double_quoted_attribute(&sBackLink)
    );
    for stParent in &vecChain {
        sHtml.push_str("<h2>Ответ на:</h2>");
        sHtml.push_str(&sRenderDeletedComment(
            stParent,
            &stState.config.public_url,
            &stMarkupUsers,
        ));
    }
    if !vecChain.is_empty() {
        sHtml.push_str("<h2>Удаленный комментарий:</h2>");
    }
    sHtml.push_str(&sRenderDeletedComment(
        &StPreparedDeletedComment {
            stComment,
            optDeleteInfo: Some((sDeletedBy, sDeleteReason)),
        },
        &stState.config.public_url,
        &stMarkupUsers,
    ));
    sHtml.push_str("</div>");
    Ok(Html(crate::routes::sRenderLegacyContent(
        "Просмотр удаленного комментария",
        sHtml,
    )?))
}

#[derive(Deserialize)]
pub struct NotificationsClickForm {
    #[serde(rename = "firstId")]
    pub first_id: i32,
    #[serde(rename = "lastId")]
    pub last_id: i32,
}

async fn topic_link(
    state: &AppState,
    topic_id: i32,
    comment_id: Option<i32>,
    event_type: &str,
) -> Result<String> {
    if event_type == "DEL"
        && let Some(iCommentId) = comment_id
    {
        return Ok(format!(
            "/view-deleted?id={iCommentId}#comment-{iCommentId}"
        ));
    }
    let prefix: Option<(String, String)> = sqlx::query_as(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END,
                  g.urlname
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((section, group)) = prefix else {
        return Ok("/notifications".to_string());
    };
    let anchor = comment_id
        .map(|id| format!("?cid={id}"))
        .unwrap_or_default();
    Ok(format!("/{section}/{group}/{topic_id}{anchor}"))
}

#[derive(Debug)]
struct StNotificationClickEvent {
    user_id: i32,
    unread: bool,
    event_type: String,
    topic_id: Option<i32>,
    comment_id: Option<i32>,
}

type TyNotificationClickRow = (i32, bool, String, Option<i32>, Option<i32>);

fn bValidNotificationClickRange(
    first_id: i32,
    first: &StNotificationClickEvent,
    last_id: i32,
    last: &StNotificationClickEvent,
) -> bool {
    if first_id > last_id || first.unread != last.unread {
        return false;
    }
    match last.event_type.as_str() {
        "WATCH" => first.event_type == "WATCH" && first.topic_id == last.topic_id,
        "REACTION" => {
            first.event_type == "REACTION"
                && first.topic_id == last.topic_id
                && first.comment_id == last.comment_id
        }
        _ => first_id == last_id && first.event_type == last.event_type,
    }
}

#[cfg(test)]
mod notification_click_tests {
    use super::*;

    fn stEvent(sType: &str, iTopicId: i32, optCommentId: Option<i32>) -> StNotificationClickEvent {
        StNotificationClickEvent {
            user_id: 1,
            unread: true,
            event_type: sType.into(),
            topic_id: Some(iTopicId),
            comment_id: optCommentId,
        }
    }

    #[test]
    fn watch_range_requires_same_topic_and_order() {
        let stFirst = stEvent("WATCH", 10, Some(1));
        let stLast = stEvent("WATCH", 10, Some(9));
        assert!(bValidNotificationClickRange(2, &stFirst, 5, &stLast));
        assert!(!bValidNotificationClickRange(5, &stFirst, 2, &stLast));
        assert!(!bValidNotificationClickRange(
            2,
            &stFirst,
            5,
            &stEvent("WATCH", 11, None)
        ));
    }

    #[test]
    fn reaction_range_requires_same_topic_and_comment() {
        let stFirst = stEvent("REACTION", 10, Some(7));
        assert!(bValidNotificationClickRange(
            2,
            &stFirst,
            5,
            &stEvent("REACTION", 10, Some(7))
        ));
        assert!(!bValidNotificationClickRange(
            2,
            &stFirst,
            5,
            &stEvent("REACTION", 10, Some(8))
        ));
    }

    #[test]
    fn ordinary_event_must_be_a_single_matching_event() {
        let stEvent = stEvent("REF", 10, None);
        assert!(bValidNotificationClickRange(2, &stEvent, 2, &stEvent));
        assert!(!bValidNotificationClickRange(2, &stEvent, 3, &stEvent));
    }

    #[test]
    fn deleted_comment_visibility_matches_java_owner_window_and_global_gate() {
        let dtNow = chrono::Utc::now();
        assert!(bCanViewDeletedComment(
            true,
            9,
            10,
            true,
            dtNow - chrono::Duration::days(30),
            dtNow,
        ));
        assert!(bCanViewDeletedComment(
            false,
            9,
            9,
            false,
            dtNow - chrono::Duration::days(13),
            dtNow,
        ));
        assert!(!bCanViewDeletedComment(
            false,
            9,
            9,
            true,
            dtNow - chrono::Duration::days(1),
            dtNow,
        ));
        assert!(!bCanViewDeletedComment(
            false,
            9,
            9,
            false,
            dtNow - chrono::Duration::days(14),
            dtNow,
        ));
        assert!(!bCanViewDeletedComment(
            false,
            9,
            10,
            false,
            dtNow - chrono::Duration::days(1),
            dtNow,
        ));
    }

    #[test]
    fn notification_feed_removes_invalid_xml_characters() {
        assert_eq!(sRemoveInvalidXmlChars("ok\u{0} text\n"), "ok text\n");
    }
}

async fn process_notifications_click(
    state: &AppState,
    user_id: i32,
    form: &NotificationsClickForm,
) -> Result<String> {
    let optFirst: Option<TyNotificationClickRow> = sqlx::query_as(
        "SELECT userid,unread,type::text,message_id,comment_id FROM user_events WHERE id=$1",
    )
    .bind(form.first_id)
    .fetch_optional(&state.pool)
    .await?;
    let optLast: Option<TyNotificationClickRow> = sqlx::query_as(
        "SELECT userid,unread,type::text,message_id,comment_id FROM user_events WHERE id=$1",
    )
    .bind(form.last_id)
    .fetch_optional(&state.pool)
    .await?;

    let (Some(stFirstRow), Some(stLastRow)) = (optFirst, optLast) else {
        return Ok("/notifications".to_string());
    };
    let stFirst = StNotificationClickEvent {
        user_id: stFirstRow.0,
        unread: stFirstRow.1,
        event_type: stFirstRow.2,
        topic_id: stFirstRow.3,
        comment_id: stFirstRow.4,
    };
    let stLast = StNotificationClickEvent {
        user_id: stLastRow.0,
        unread: stLastRow.1,
        event_type: stLastRow.2,
        topic_id: stLastRow.3,
        comment_id: stLastRow.4,
    };
    if user_id != stFirst.user_id || user_id != stLast.user_id {
        return Err(AppError::Forbidden);
    }

    if stLast.unread {
        if !bValidNotificationClickRange(form.first_id, &stFirst, form.last_id, &stLast) {
            return Err(AppError::BadRequest(
                "invalid notification click range".into(),
            ));
        }
        let mut tx = state.pool.begin().await?;
        match stLast.event_type.as_str() {
            "WATCH" => {
                sqlx::query("UPDATE user_events SET unread=false WHERE userid=$1 AND unread AND type='WATCH'::event_type AND message_id=$2")
                    .bind(user_id).bind(stLast.topic_id).execute(&mut *tx).await?;
            }
            "REACTION" => {
                sqlx::query("UPDATE user_events SET unread=false WHERE userid=$1 AND unread AND type='REACTION'::event_type AND id BETWEEN $2 AND $3 AND message_id IS NOT DISTINCT FROM $4 AND comment_id IS NOT DISTINCT FROM $5")
                    .bind(user_id).bind(form.first_id).bind(form.last_id).bind(stLast.topic_id).bind(stLast.comment_id).execute(&mut *tx).await?;
            }
            _ => {
                sqlx::query(
                    "UPDATE user_events SET unread=false WHERE userid=$1 AND unread AND id=$2",
                )
                .bind(user_id)
                .bind(form.last_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=$1")
            .bind(user_id).execute(&mut *tx).await?;
        tx.commit().await?;
        state.realtime.vNotifyEvents([user_id]);
    }

    match stFirst.topic_id {
        Some(iTopicId) => {
            topic_link(state, iTopicId, stFirst.comment_id, &stFirst.event_type).await
        }
        None => Ok("/notifications".to_string()),
    }
}

pub async fn notifications_click(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<NotificationsClickForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let url = process_notifications_click(&state, user.id, &form).await?;
    Ok((StatusCode::FOUND, [(axum::http::header::LOCATION, url)]).into_response())
}

pub async fn notifications_click_ajax(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<NotificationsClickForm>,
) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let url = process_notifications_click(&state, user.id, &form).await?;
    Ok(Json(json!({"url": url})))
}

#[derive(Deserialize)]
pub struct ActivationQuery {
    pub nick: Option<String>,
    pub activation: Option<String>,
    pub error: Option<String>,
}

pub async fn activate_form(
    Query(q): Query<ActivationQuery>,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let sNick = q
        .nick
        .as_deref()
        .filter(|sValue| valid_login_name_for_java(sValue))
        .unwrap_or("");
    let sActivation = q
        .activation
        .as_deref()
        .filter(|sValue| sValue.chars().all(char::is_alphanumeric))
        .unwrap_or("");
    render_activation_form(
        sNick,
        sActivation,
        q.error.as_deref(),
        &csrf_token,
        optUser.is_some(),
    )
}

#[derive(Deserialize)]
pub struct ActivationForm {
    pub nick: Option<String>,
    pub activation: String,
    pub passwd: Option<String>,
    pub action: Option<String>,
}

pub async fn activate_post(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    jar: CookieJar,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    Form(form): Form<ActivationForm>,
) -> Result<impl IntoResponse> {
    if form.action.is_some() {
        let nick = form.nick.as_deref().unwrap_or("").trim();
        let password = form.passwd.as_deref().unwrap_or("");
        let Some((id, db_nick, email, regdate, activated)) = sqlx::query_as::<
            _,
            (
                i32,
                String,
                Option<String>,
                Option<chrono::DateTime<chrono::Utc>>,
                bool,
            ),
        >(
            "SELECT id,nick,email,regdate,activated FROM users WHERE lower(nick)=lower($1)",
        )
        .bind(nick)
        .fetch_optional(&state.pool)
        .await?
        else {
            return Ok(render_activation_form(
                nick,
                &form.activation,
                Some("Пользователь не найден"),
                &csrf_token,
                false,
            )?
            .into_response());
        };

        if activated {
            return Ok(Redirect::to("/").into_response());
        }

        match crate::auth::verify_login(&state.pool, nick, password).await? {
            crate::auth::LoginOutcome::NotActivated => {}
            crate::auth::LoginOutcome::Failed => {
                return Ok(render_activation_form(
                    nick,
                    &form.activation,
                    Some("Неправильный логин или пароль"),
                    &csrf_token,
                    false,
                )?
                .into_response());
            }
            crate::auth::LoginOutcome::Blocked => {
                // Java lets the uncaught LockedException reach its global 500
                // exception resolver on this activation branch.
                return Err(AppError::Anyhow(anyhow::anyhow!(
                    "blocked user cannot be activated"
                )));
            }
            crate::auth::LoginOutcome::Success(_) => return Ok(Redirect::to("/").into_response()),
        }

        if !verify_activation_code(
            &state,
            &db_nick,
            email.as_deref().unwrap_or(""),
            regdate,
            &form.activation,
        ) {
            return Ok(render_activation_form(
                nick,
                &form.activation,
                Some("Неправильный код активации"),
                &csrf_token,
                false,
            )?
            .into_response());
        }

        sqlx::query("UPDATE users SET activated=true,lastlogin=now() WHERE id=$1")
            .bind(id)
            .execute(&state.pool)
            .await?;
        crate::audit::log_user_action(&state.pool, id, id, "register", &[]).await?;
        let Some(stIdentity) = crate::auth::optLoadLoginIdentity(&state.pool, id).await? else {
            return Err(AppError::Anyhow(anyhow::anyhow!(
                "activated user cannot be loaded for remember-me cookie"
            )));
        };
        let cookie = Cookie::build((
            crate::security::remember_me::COOKIE_NAME,
            crate::auth::sMakeRememberMeCookieValue(&stIdentity, &state.config.site_secret),
        ))
        .path("/")
        .max_age(time::Duration::seconds(
            crate::security::remember_me::VALIDITY_SECONDS,
        ))
        .http_only(true)
        .secure(crate::security::is_secure_request(
            &headers,
            Some(stPeerAddress.ip()),
            &state.config.trusted_proxy_cidrs,
        ))
        .build();
        return Ok((jar.add(cookie), Redirect::to("/")).into_response());
    }

    let Some(user) = current_user else {
        return Err(AppError::Forbidden);
    };
    let Some((email, regdate)) = sqlx::query_as::<
        _,
        (Option<String>, Option<chrono::DateTime<chrono::Utc>>),
    >("SELECT new_email,regdate FROM users WHERE id=$1")
    .bind(user.id)
    .fetch_optional(&state.pool)
    .await?
    else {
        return Err(AppError::NotFound);
    };
    let Some(new_email) = email else {
        return Err(AppError::BadRequest("new_email == null".into()));
    };

    if !verify_activation_code(&state, &user.nick, &new_email, regdate, &form.activation) {
        return Ok(render_activation_form(
            &user.nick,
            &form.activation,
            Some("Неправильный код активации"),
            &csrf_token,
            true,
        )?
        .into_response());
    }
    sqlx::query("UPDATE users SET email=new_email,new_email=NULL WHERE id=$1")
        .bind(user.id)
        .execute(&state.pool)
        .await?;
    crate::audit::log_user_action(&state.pool, user.id, user.id, "accept_new_email", &[]).await?;
    Ok(Redirect::to(&format!(
        "/people/{}/profile",
        urlencoding::encode(&user.nick)
    ))
    .into_response())
}

fn render_activation_form(
    nick: &str,
    activation: &str,
    error: Option<&str>,
    csrf_token: &str,
    b_authenticated: bool,
) -> Result<Html<String>> {
    #[derive(Template)]
    #[template(path = "activate.html")]
    struct StActivateTemplate<'a> {
        sNick: &'a str,
        sActivation: &'a str,
        optError: Option<&'a str>,
        sCsrfToken: &'a str,
        bAuthenticated: bool,
    }

    Ok(Html(
        StActivateTemplate {
            sNick: nick,
            sActivation: activation,
            optError: error,
            sCsrfToken: csrf_token,
            bAuthenticated: b_authenticated,
        }
        .render()?,
    ))
}

#[cfg(test)]
mod activation_template_tests {
    use super::render_activation_form;
    use axum::response::Html;

    #[test]
    fn anonymous_activation_matches_java_form_and_uses_theme_shell() {
        let Html(sHtml) =
            render_activation_form("alice", "ABC123", Some("Ошибка"), "csrf-value", false)
                .expect("activation template");

        assert!(sHtml.contains("<!-- LOR_THEME_HEADER -->"));
        assert!(sHtml.contains("action=\"/activate.jsp\""));
        assert!(sHtml.contains("id=\"activateForm\" class=\"form-horizontal\""));
        assert!(sHtml.contains("name=\"action\" value=\"new\""));
        assert!(sHtml.contains("id=\"field_nick\" value=\"alice\""));
        assert!(sHtml.contains("id=\"field_password\""));
        assert!(sHtml.contains("id=\"field_code\" value=\"ABC123\""));
        assert!(sHtml.contains("<div class=\"error\">Ошибка</div>"));
    }

    #[test]
    fn authenticated_activation_only_asks_for_the_code() {
        let Html(sHtml) = render_activation_form("alice", "ABC123", None, "csrf-value", true)
            .expect("activation template");

        assert!(sHtml.contains("name=\"activation\" required autofocus id=\"field_code\""));
        assert!(!sHtml.contains("name=\"nick\""));
        assert!(!sHtml.contains("name=\"passwd\""));
        assert!(!sHtml.contains("name=\"action\""));
    }
}

fn verify_activation_code(
    state: &AppState,
    nick: &str,
    email: &str,
    regdate: Option<chrono::DateTime<chrono::Utc>>,
    supplied: &str,
) -> bool {
    if state.config.enable_dev_bypasses && supplied == "dev-activate" {
        return true;
    }
    let Some(regdate) = regdate else {
        return false;
    };
    crate::security::secret_tokens::verify_activation_code(
        &state.config.site_secret,
        nick,
        email,
        regdate.timestamp_millis(),
        supplied,
    )
}

pub async fn addphoto_form(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    vCheckLoadUserpic(&state, &user).await?;
    stRenderAddphoto(user.nick, csrf_token, None, StatusCode::OK)
}

pub async fn upload_userpic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    mut multipart: Multipart,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let cService = CUserpicService::new(
        CUserpicPgRepository::new(state.pool.clone()),
        state.config.upload_dir.clone(),
    );
    cService.vCheckUpload(user.id).await?;
    let mut optUpload: Option<bytes::Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|stError| AppError::BadRequest(format!("ошибка multipart: {stError}")))?
    {
        let bFile = field.name() == Some("file");
        let arrData = field
            .bytes()
            .await
            .map_err(|stError| AppError::BadRequest(format!("ошибка чтения файла: {stError}")))?;
        if bFile {
            optUpload = Some(arrData);
            break;
        }
    }
    let Some(arrData) = optUpload else {
        return stRenderAddphoto(
            user.nick,
            csrf_token,
            Some("изображение не задано".to_owned()),
            StatusCode::BAD_REQUEST,
        );
    };
    if arrData.is_empty() {
        // `MultipartFile.isEmpty` is handled before Java's try/catch and
        // therefore keeps the default 200 status while redisplaying the form.
        return stRenderAddphoto(
            user.nick,
            csrf_token,
            Some("изображение не задано".to_owned()),
            StatusCode::OK,
        );
    }

    if let Err(stError) = cService.sInstall(user.id, &arrData).await {
        return match stError {
            AppError::BadRequest(sMessage) => stRenderAddphoto(
                user.nick,
                csrf_token,
                Some(sMessage),
                StatusCode::BAD_REQUEST,
            ),
            stError => Err(stError),
        };
    }

    Ok(crate::routes::admin::stProfileRedirect(&user.nick))
}

#[derive(Template)]
#[template(path = "addphoto.html")]
struct StAddphotoTemplate {
    sNick: String,
    sCsrfToken: String,
    optError: Option<String>,
}

fn stRenderAddphoto(
    sNick: String,
    sCsrfToken: String,
    optError: Option<String>,
    stStatus: StatusCode,
) -> Result<Response> {
    let sBody = StAddphotoTemplate {
        sNick,
        sCsrfToken,
        optError,
    }
    .render()?;
    Ok((stStatus, Html(sBody)).into_response())
}

#[cfg(test)]
mod userpic_http_contract_tests {
    use axum::{body::to_bytes, http::StatusCode};

    use super::stRenderAddphoto;

    async fn sBody(stResponse: axum::response::Response) -> String {
        String::from_utf8(
            to_bytes(stResponse.into_body(), 128 * 1024)
                .await
                .expect("addphoto response body")
                .to_vec(),
        )
        .expect("UTF-8 addphoto response")
    }

    #[tokio::test]
    async fn empty_multipart_file_redisplays_the_themed_form_with_java_200() {
        let stResponse = stRenderAddphoto(
            "JB".to_owned(),
            "csrf".to_owned(),
            Some("изображение не задано".to_owned()),
            StatusCode::OK,
        )
        .expect("render empty upload");
        assert_eq!(stResponse.status(), StatusCode::OK);
        let sHtml = sBody(stResponse).await;
        assert!(sHtml.contains("<!-- LOR_THEME_HEADER -->"));
        assert!(sHtml.contains("Ошибка! изображение не задано"));
        assert!(sHtml.contains("action=\"addphoto.jsp\""));
        assert!(sHtml.contains("name=\"file\""));
    }

    #[tokio::test]
    async fn rejected_image_redisplays_the_same_form_with_java_400() {
        let stResponse = stRenderAddphoto(
            "JB".to_owned(),
            "csrf".to_owned(),
            Some("Invalid image".to_owned()),
            StatusCode::BAD_REQUEST,
        )
        .expect("render invalid upload");
        assert_eq!(stResponse.status(), StatusCode::BAD_REQUEST);
        assert!(sBody(stResponse).await.contains("Ошибка! Invalid image"));
    }
}

/// Exact `EditProfileChecker.checkLoadUserpic` policy. This must run on both
/// GET and POST because hiding the form alone does not protect the mutation.
pub(crate) async fn bCanLoadUserpic(
    stState: &AppState,
    stUser: &crate::models::UserSummary,
) -> Result<bool> {
    CUserpicService::new(
        CUserpicPgRepository::new(stState.pool.clone()),
        stState.config.upload_dir.clone(),
    )
    .bCanUpload(stUser.id)
    .await
}

async fn vCheckLoadUserpic(stState: &AppState, stUser: &crate::models::UserSummary) -> Result<()> {
    if !bCanLoadUserpic(stState, stUser).await? {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

#[derive(Template)]
#[template(path = "deregister.html")]
struct StDeregisterTemplate {
    csrf_token: String,
    captcha_site_key: String,
    errors: Vec<String>,
    accept_block: bool,
    accept_oneway: bool,
}

#[derive(Template)]
#[template(path = "action_done.html")]
struct StDeregisterDoneTemplate {
    message: String,
    big_message: Option<String>,
    link: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeregisterForm {
    pub password: Option<String>,
    #[serde(alias = "accept_block")]
    #[serde(rename = "acceptBlock")]
    pub accept_block: Option<String>,
    #[serde(alias = "accept_oneway")]
    #[serde(rename = "acceptOneway")]
    pub accept_oneway: Option<String>,
    #[serde(rename = "h-captcha-response")]
    pub captcha_response: Option<String>,
}

pub async fn deregister_form(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let cService = CUserAccountService::new(CUserAccountPgRepository::new(state.pool.clone()));
    cService.vCheckDeregister(user.id).await?;
    render_deregister_page(&state, csrf_token, Vec::new(), false, false)
}

pub async fn deregister_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    Form(form): Form<DeregisterForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let cService = CUserAccountService::new(CUserAccountPgRepository::new(state.pool.clone()));
    cService.vCheckDeregister(user.id).await?;

    let bAcceptBlock = form.accept_block.is_some();
    let bAcceptOneway = form.accept_oneway.is_some();
    let mut vecErrors = Vec::new();
    if !bAcceptBlock {
        vecErrors.push("Вы не согласились с блокировкой аккаунта".to_owned());
    }
    if !bAcceptOneway {
        vecErrors.push("Вы не согласились с невозможностью восстановления аккаунта".to_owned());
    }
    let bPasswordMatches = cService
        .bPasswordMatches(user.id, form.password.as_deref().unwrap_or(""))
        .await?;
    if !bPasswordMatches {
        vecErrors.push("Неверный пароль".to_owned());
    }
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    if let Err(sError) = crate::application::auth::sValidateCaptcha(
        &state.config,
        &state.http,
        form.captcha_response.as_deref(),
        &sRemoteIp,
    )
    .await
    {
        vecErrors.push(sError);
    }

    if !vecErrors.is_empty() {
        return Ok(render_deregister_page(
            &state,
            csrf_token,
            vecErrors,
            bAcceptBlock,
            bAcceptOneway,
        )?
        .into_response());
    }

    cService.vDeregister(user.id).await?;
    Ok(Html(
        StDeregisterDoneTemplate {
            message: "Удаление пользователя прошло успешно.".to_owned(),
            big_message: None,
            link: None,
        }
        .render()?,
    )
    .into_response())
}

fn render_deregister_page(
    state: &AppState,
    csrf_token: String,
    errors: Vec<String>,
    accept_block: bool,
    accept_oneway: bool,
) -> Result<Html<String>> {
    Ok(Html(
        StDeregisterTemplate {
            csrf_token,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
            errors,
            accept_block,
            accept_oneway,
        }
        .render()?,
    ))
}

async fn user_exists_or_similar(state: &AppState, nick: &str) -> Result<bool> {
    let exists: Option<i32> =
        sqlx::query_scalar("SELECT id FROM users WHERE lower(nick)=lower($1)")
            .bind(nick)
            .fetch_optional(&state.pool)
            .await?;
    if exists.is_some() {
        return Ok(true);
    }
    let similar: Option<i32> = sqlx::query_scalar(
        r#"SELECT id FROM users
           WHERE score>=200 AND lastlogin>CURRENT_TIMESTAMP - interval '3 years'
             AND levenshtein_less_equal(lower(nick), lower($1), 1)<=1
           LIMIT 1"#,
    )
    .bind(nick)
    .fetch_optional(&state.pool)
    .await?;
    Ok(similar.is_some())
}

pub fn valid_login_name_for_java(nick: &str) -> bool {
    let nick = nick.to_lowercase();
    if nick.is_empty() || nick.len() >= 80 {
        return false;
    }
    let mut chars = nick.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub async fn forum_page_or_archive(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((group, id_or_year, page_or_month)): Path<(String, String, String)>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<axum::response::Response> {
    if let Some(page) = page_or_month.strip_prefix("page") {
        let page: i64 = page.parse().map_err(|_| AppError::NotFound)?;
        let id: i32 = id_or_year.parse().map_err(|_| AppError::NotFound)?;
        let sRemoteIp = crate::security::stClientIp(
            stPeerAddress.ip(),
            &headers,
            &state.config.trusted_proxy_cidrs,
        )
        .to_string();
        return crate::routes::topics::render_topic_page(
            state,
            "forum",
            group,
            id,
            page,
            current_user,
            csrf_token,
            sRemoteIp,
        )
        .await;
    }

    let year: i32 = id_or_year.parse().map_err(|_| AppError::NotFound)?;
    let month: i32 = page_or_month.parse().map_err(|_| AppError::NotFound)?;
    Ok(forum_archive_month(
        State(state),
        Path((group, year, month)),
        Query(q),
        CurrentUser(current_user),
    )
    .await?
    .into_response())
}

fn validate_year_month(year: i32, month: i32) -> Result<()> {
    if !(1990..=3000).contains(&year) {
        return Err(AppError::BadRequest("указан некорректный год".into()));
    }
    if !(1..=12).contains(&month) {
        return Err(AppError::BadRequest("указан некорректный месяц".into()));
    }
    Ok(())
}

fn section_from_uri(uri: &Uri) -> Option<&'static str> {
    match uri.path().trim_start_matches('/').split('/').next()? {
        "forum" => Some("forum"),
        "news" => Some("news"),
        "articles" => Some("articles"),
        "gallery" => Some("gallery"),
        "polls" => Some("polls"),
        _ => None,
    }
}

#[derive(Deserialize)]
pub struct MemoryForm {
    pub msgid: Option<i32>,
    pub watch: Option<bool>,
    pub id: Option<i32>,
    pub remove: Option<String>,
}

/// MemoriesController.add/remove: "favorite" (watch=false) and "watch"
/// (watch=true) are independent rows per topic - `add` upserts the row for
/// the requested `watch` value only, `remove` deletes one specific row by
/// its own id (never the whole userid+topic pair), matching the frontend
/// contract in `static/js/lor/memories.js` (`{msgid,watch}` to add,
/// `{id}` to remove, JSON `{id,count}`/bare count responses).
pub async fn memories(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<MemoryForm>,
) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };

    if form.remove.is_some() {
        let Some(id) = form.id else {
            return Err(AppError::BadRequest("missing id".into()));
        };
        let row: Option<(i32, i32, bool)> =
            sqlx::query_as("SELECT userid, topic, watch FROM memories WHERE id=$1")
                .bind(id)
                .fetch_optional(&state.pool)
                .await?;
        let Some((owner_id, topic_id, watch)) = row else {
            return Ok(Json(serde_json::json!(-1)));
        };
        if owner_id != user.id {
            return Err(AppError::Forbidden);
        }
        sqlx::query("DELETE FROM memories WHERE id=$1")
            .bind(id)
            .execute(&state.pool)
            .await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM memories WHERE topic=$1 AND watch=$2")
                .bind(topic_id)
                .bind(watch)
                .fetch_one(&state.pool)
                .await?;
        return Ok(Json(serde_json::json!(count)));
    }

    let msgid = form
        .msgid
        .ok_or_else(|| AppError::BadRequest("missing msgid".into()))?;
    let watch = form.watch.unwrap_or(false);
    let deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1")
        .bind(msgid)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    if deleted {
        return Err(AppError::BadRequest("Тема удалена".into()));
    }
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO memories(userid,topic,watch) VALUES($1,$2,$3) ON CONFLICT(userid,topic,watch) DO UPDATE SET topic=EXCLUDED.topic RETURNING id",
    )
    .bind(user.id).bind(msgid).bind(watch)
    .fetch_one(&state.pool)
    .await?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM memories WHERE topic=$1 AND watch=$2")
            .bind(msgid)
            .bind(watch)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(serde_json::json!({"id": id, "count": count})))
}

#[derive(Debug, Deserialize, Default)]
pub struct StUserFilterQuery {
    #[serde(rename = "newFavoriteTagName")]
    pub optNewFavoriteTagName: Option<String>,
    #[serde(rename = "newIgnoreTagName")]
    pub optNewIgnoreTagName: Option<String>,
}

#[derive(Debug)]
struct StIgnoredUserRow {
    iId: i32,
    sNick: String,
    optRemark: Option<String>,
}

#[derive(Template)]
#[template(path = "user_filter.html")]
struct StUserFilterTemplate {
    vecIgnoredUsers: Vec<StIgnoredUserRow>,
    vecFavoriteTags: Vec<String>,
    vecIgnoreTags: Vec<String>,
    bModerator: bool,
    optNewFavoriteTagName: Option<String>,
    optNewIgnoreTagName: Option<String>,
    vecFavoriteErrors: Vec<String>,
    vecIgnoreErrors: Vec<String>,
    sCsrfToken: String,
}

async fn stRenderUserFilter(
    stState: &AppState,
    stUser: &crate::models::UserSummary,
    stQuery: StUserFilterQuery,
    vecFavoriteErrors: Vec<String>,
    vecIgnoreErrors: Vec<String>,
    sCsrfToken: String,
) -> Result<Response> {
    let vecIgnoredUsers = sqlx::query_as::<_, (i32, String, Option<String>)>(
        r#"SELECT u.id,u.nick,r.remark_text
             FROM ignore_list il
             JOIN users u ON u.id=il.ignored
             LEFT JOIN user_remarks r ON r.user_id=il.userid AND r.ref_user_id=il.ignored
            WHERE il.userid=$1 ORDER BY u.nick"#,
    )
    .bind(stUser.id)
    .fetch_all(&stState.pool)
    .await?
    .into_iter()
    .map(|(iId, sNick, optRemark)| StIgnoredUserRow {
        iId,
        sNick,
        optRemark,
    })
    .collect();
    let vecFavoriteTags = crate::routes::users::user_tags(stState, stUser.id, true).await?;
    let vecIgnoreTags = if stUser.canmod {
        Vec::new()
    } else {
        crate::routes::users::user_tags(stState, stUser.id, false).await?
    };
    let sHtml = StUserFilterTemplate {
        vecIgnoredUsers,
        vecFavoriteTags,
        vecIgnoreTags,
        bModerator: stUser.canmod,
        optNewFavoriteTagName: stQuery
            .optNewFavoriteTagName
            .filter(|sTag| crate::routes::tags::is_good_tag(sTag)),
        optNewIgnoreTagName: stQuery
            .optNewIgnoreTagName
            .filter(|sTag| crate::routes::tags::is_good_tag(sTag)),
        vecFavoriteErrors,
        vecIgnoreErrors,
        sCsrfToken,
    }
    .render()?;
    let mut stResponse = Html(sHtml).into_response();
    stResponse.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store, no-cache, must-revalidate".parse().unwrap(),
    );
    stResponse
        .headers_mut()
        .insert(header::PRAGMA, "no-cache".parse().unwrap());
    Ok(stResponse)
}

pub async fn user_filter(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Query(stQuery): Query<StUserFilterQuery>,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> Result<Response> {
    let stUser = optUser.ok_or(AppError::Forbidden)?;
    stRenderUserFilter(
        &stState,
        &stUser,
        stQuery,
        Vec::new(),
        Vec::new(),
        sCsrfToken,
    )
    .await
}

#[derive(Deserialize)]
pub struct UserTagForm {
    pub tag: Option<String>,
    #[serde(rename = "tagName")]
    pub tag_name: Option<String>,
    pub del: Option<String>,
    pub add: Option<String>,
}

pub async fn favorite_tag(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    stHeaders: HeaderMap,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    Form(form): Form<UserTagForm>,
) -> Result<Response> {
    save_or_delete_user_tag(stState, optUser, stHeaders, form, true, sCsrfToken).await
}

pub async fn ignore_tag(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    stHeaders: HeaderMap,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    Form(form): Form<UserTagForm>,
) -> Result<Response> {
    if optUser.as_ref().is_some_and(|stUser| stUser.canmod) {
        return Err(AppError::Forbidden);
    }
    save_or_delete_user_tag(stState, optUser, stHeaders, form, false, sCsrfToken).await
}

async fn save_or_delete_user_tag(
    stState: AppState,
    optUser: Option<crate::models::UserSummary>,
    stHeaders: HeaderMap,
    form: UserTagForm,
    bFavorite: bool,
    sCsrfToken: String,
) -> Result<Response> {
    let stUser = optUser.ok_or(AppError::Forbidden)?;
    let sRawTag = form
        .tag_name
        .or(form.tag)
        .unwrap_or_default()
        .trim()
        .to_string();
    if sRawTag.is_empty() {
        return Err(AppError::BadRequest("tagName is required".into()));
    }
    let bJson = bAcceptsJson(&stHeaders);
    let bDelete = form.del.is_some();
    if !bDelete && form.add.is_none() {
        return Err(AppError::NotFound);
    }
    let vecTags = if bJson || bDelete {
        vec![sRawTag.to_lowercase()]
    } else {
        crate::routes::tags::parse_tags(&sRawTag)
    };
    let mut vecErrors = Vec::new();
    let mut iLastCount = 0_i64;
    for sTag in &vecTags {
        if !crate::routes::tags::is_good_tag(sTag) {
            vecErrors.push(format!("Некорректный тег: '{sTag}'"));
            continue;
        }
        let sCounterFilter = if bFavorite && !bDelete {
            " AND counter>0"
        } else {
            ""
        };
        let sSql =
            format!("SELECT id FROM tags_values WHERE lower(value)=lower($1){sCounterFilter}");
        let optTagId: Option<i32> = sqlx::query_scalar(sqlx::AssertSqlSafe(sSql))
            .bind(sTag)
            .fetch_optional(&stState.pool)
            .await?;
        let Some(iTagId) = optTagId else {
            vecErrors.push(format!("Тег не найден: '{sTag}'"));
            continue;
        };
        if bDelete {
            sqlx::query("DELETE FROM user_tags WHERE user_id=$1 AND tag_id=$2 AND is_favorite=$3")
                .bind(stUser.id)
                .bind(iTagId)
                .bind(bFavorite)
                .execute(&stState.pool)
                .await?;
        } else {
            sqlx::query("INSERT INTO user_tags(user_id,tag_id,is_favorite) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                .bind(stUser.id)
                .bind(iTagId)
                .bind(bFavorite)
                .execute(&stState.pool)
                .await?;
        }
        iLastCount =
            sqlx::query_scalar("SELECT count(*) FROM user_tags WHERE tag_id=$1 AND is_favorite=$2")
                .bind(iTagId)
                .bind(bFavorite)
                .fetch_one(&stState.pool)
                .await?;
    }
    if bJson {
        if let Some(sError) = vecErrors.first() {
            return Ok(Json(json!({"error": sError})).into_response());
        }
        return Ok(Json(json!({"count": iLastCount})).into_response());
    }
    if vecErrors.is_empty() {
        return Ok((StatusCode::FOUND, [(header::LOCATION, "/user-filter")]).into_response());
    }
    let stQuery = if bFavorite {
        StUserFilterQuery {
            optNewFavoriteTagName: Some(sRawTag),
            optNewIgnoreTagName: None,
        }
    } else {
        StUserFilterQuery {
            optNewFavoriteTagName: None,
            optNewIgnoreTagName: Some(sRawTag),
        }
    };
    let (vecFavoriteErrors, vecIgnoreErrors) = if bFavorite {
        (vecErrors, Vec::new())
    } else {
        (Vec::new(), vecErrors)
    };
    stRenderUserFilter(
        &stState,
        &stUser,
        stQuery,
        vecFavoriteErrors,
        vecIgnoreErrors,
        sCsrfToken,
    )
    .await
}

#[derive(Deserialize)]
pub struct IgnoreUserForm {
    pub id: Option<i32>,
    pub nick: Option<String>,
    pub del: Option<String>,
    pub add: Option<String>,
}

pub async fn ignore_user(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<IgnoreUserForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    // UserFilterController.listAdd/listDel: the personal user-ignore list
    // has no moderator restriction at all - only ignore-*tags* is
    // moderator-restricted (moderators must see every tag), see
    // ignore_tag below.
    let ignored_id: i32 = if let Some(id) = form.id {
        id
    } else {
        let nick = form.nick.unwrap_or_default();
        sqlx::query_scalar("SELECT id FROM users WHERE lower(nick)=lower($1)")
            .bind(nick.trim())
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?
    };
    if ignored_id == user.id {
        return Err(AppError::BadRequest(
            "нельзя игнорировать самого себя".into(),
        ));
    }
    if form.del.is_some() {
        sqlx::query("DELETE FROM ignore_list WHERE userid=$1 AND ignored=$2")
            .bind(user.id)
            .bind(ignored_id)
            .execute(&state.pool)
            .await?;
    } else if form.add.is_some() {
        sqlx::query("INSERT INTO ignore_list(userid,ignored) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(user.id)
            .bind(ignored_id)
            .execute(&state.pool)
            .await?;
    } else {
        return Err(AppError::NotFound);
    }
    Ok((StatusCode::FOUND, [(header::LOCATION, "/user-filter")]).into_response())
}

fn bAcceptsJson(stHeaders: &HeaderMap) -> bool {
    stHeaders
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|stValue| stValue.to_str().ok())
        .any(|sValue| {
            sValue
                .split(',')
                .any(|sMediaType| sMediaType.trim().starts_with("application/json"))
        })
}

#[derive(Deserialize)]
pub struct StSetPostScoreQuery {
    pub msgid: Option<String>,
}

#[derive(Deserialize)]
pub struct StSetPostScoreForm {
    pub msgid: Option<String>,
    pub postscore: Option<String>,
    pub sticky: Option<String>,
    pub notop: Option<String>,
}

#[derive(Template)]
#[template(path = "set_post_score.html")]
struct StSetPostScoreTemplate {
    csrf_token: String,
    topic_id: i32,
    postscore: i32,
    sticky: bool,
    notop: bool,
    premoderated: bool,
}

#[derive(Template)]
#[template(path = "set_post_score_done.html")]
struct StSetPostScoreDoneTemplate {
    big_message: String,
    link: String,
}

#[derive(Template)]
#[template(path = "set_post_score_user_error.html")]
struct StSetPostScoreUserErrorTemplate {
    message: String,
}

fn bSpringRequestBoolean(optValue: Option<&str>, sName: &str) -> Result<bool> {
    match optValue.map(str::to_ascii_lowercase).as_deref() {
        None | Some("") | Some("false") | Some("off") | Some("no") | Some("0") => Ok(false),
        Some("true") | Some("on") | Some("yes") | Some("1") => Ok(true),
        Some(_) => Err(AppError::BadRequest(format!(
            "Некорректное значение параметра `{sName}`"
        ))),
    }
}

fn iSpringRequiredInt(optValue: Option<&str>, sName: &str) -> Result<i32> {
    optValue
        .ok_or_else(|| AppError::BadRequest(format!("Required parameter '{sName}' is missing")))?
        .parse::<i32>()
        .map_err(|_| AppError::BadRequest(format!("Failed to convert parameter '{sName}'")))
}

fn stTopicOptionsService(
    stState: &AppState,
) -> crate::application::topic::options::CTopicOptionsService<
    crate::infra::postgres::topic_options_repository::CTopicOptionsPgRepository,
    crate::infra::search_queue::CSearchQueueSender,
> {
    crate::application::topic::options::CTopicOptionsService::new(
        crate::infra::postgres::topic_options_repository::CTopicOptionsPgRepository::new(
            stState.pool.clone(),
        ),
        crate::infra::search_queue::CSearchQueueSender::new(
            stState.config.opensearch_url.as_deref(),
            &stState.config.upload_dir,
        ),
    )
}

fn stSetPostScoreUserErrorResponse(sMessage: String) -> Response {
    let sBody = StSetPostScoreUserErrorTemplate { message: sMessage }
        .render()
        .unwrap_or_else(|_| "Внутренняя ошибка сервера".to_owned());
    (StatusCode::INTERNAL_SERVER_ERROR, Html(sBody)).into_response()
}

pub async fn set_post_score_form(
    State(stState): State<AppState>,
    Query(stQuery): Query<StSetPostScoreQuery>,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let iTopicId = iSpringRequiredInt(stQuery.msgid.as_deref(), "msgid")?;
    let stOptions = stTopicOptionsService(&stState)
        .stForm(optUser.as_ref(), iTopicId)
        .await?;
    Ok(Html(
        StSetPostScoreTemplate {
            csrf_token: sCsrfToken,
            topic_id: stOptions.iTopicId,
            postscore: stOptions.iPostScore,
            sticky: stOptions.bSticky,
            notop: stOptions.bNoTop,
            premoderated: stOptions.bPremoderated,
        }
        .render()?,
    ))
}

pub async fn set_post_score(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Form(stForm): Form<StSetPostScoreForm>,
) -> Result<Response> {
    let iTopicId = iSpringRequiredInt(stForm.msgid.as_deref(), "msgid")?;
    let iPostScore = iSpringRequiredInt(stForm.postscore.as_deref(), "postscore")?;
    let bSticky = bSpringRequestBoolean(stForm.sticky.as_deref(), "sticky")?;
    let bNoTop = bSpringRequestBoolean(stForm.notop.as_deref(), "notop")?;
    let stOutcome = match stTopicOptionsService(&stState)
        .stSet(
            optUser.as_ref(),
            crate::domain::topic::options::StSetTopicOptions {
                iTopicId,
                iPostScore,
                bSticky,
                bNoTop,
            },
        )
        .await
    {
        Ok(stOutcome) => stOutcome,
        // UserErrorException is deliberately rendered by Java's common error
        // resolver with HTTP 500, while binding failures above remain the
        // separate Spring HTTP 400 contract.
        Err(AppError::BadRequest(sMessage)) => {
            return Ok(stSetPostScoreUserErrorResponse(sMessage));
        }
        Err(stError) => return Err(stError),
    };
    Ok(Html(
        StSetPostScoreDoneTemplate {
            big_message: stOutcome.sBigMessage,
            link: stOutcome.sCanonicalUrl,
        }
        .render()?,
    )
    .into_response())
}

#[cfg(test)]
mod set_post_score_tests {
    use super::*;

    #[test]
    fn spring_checkbox_values_and_empty_defaults_are_preserved() {
        for optValue in [
            None,
            Some(""),
            Some("false"),
            Some("off"),
            Some("no"),
            Some("0"),
        ] {
            assert!(!bSpringRequestBoolean(optValue, "sticky").unwrap());
        }
        for optValue in [Some("true"), Some("on"), Some("yes"), Some("1"), Some("ON")] {
            assert!(bSpringRequestBoolean(optValue, "sticky").unwrap());
        }
        assert!(matches!(
            bSpringRequestBoolean(Some("invalid"), "sticky"),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn spring_integer_binding_is_a_400_validation_error() {
        for optValue in [None, Some("x"), Some("2147483648")] {
            assert!(matches!(
                iSpringRequiredInt(optValue, "msgid"),
                Err(AppError::BadRequest(_))
            ));
        }
        assert_eq!(iSpringRequiredInt(Some("42"), "msgid").unwrap(), 42);
    }
}

#[derive(Deserialize)]
pub struct ImageForm {
    pub id: i32,
}

#[derive(Template)]
#[template(path = "delete_image.html")]
struct StDeleteImageTemplate {
    csrf_token: String,
    image_id: i32,
    topic_title: String,
    medium_url: String,
    original_url: String,
    medium_width: i32,
    medium_height: i32,
    max_width: i32,
    padding: f64,
    srcset: String,
    linked: bool,
}

pub async fn delete_image_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ImageForm>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let stForm = state.image_delete.stForm(&user, q.id, &sRemoteIp).await?;
    let stImage = stForm.stImage;
    Ok(Html(
        StDeleteImageTemplate {
            csrf_token,
            image_id: stImage.iId,
            topic_title: crate::domain::title::sTopicTitlePlainForDisplay(
                &stForm.stTarget.sTopicTitle,
            ),
            medium_url: stImage.sMediumUrl,
            original_url: stImage.sOriginalUrl,
            medium_width: stImage.iMediumWidth,
            medium_height: stImage.iMediumHeight,
            max_width: stImage.iWidth.min(2000),
            padding: 100.0 * f64::from(stImage.iMediumHeight) / f64::from(stImage.iMediumWidth),
            srcset: stImage.sSrcSet,
            linked: stForm.stTarget.bSectionImagePost
                || stImage.iWidth >= 1920
                || stImage.iHeight >= 1080,
        }
        .render()?,
    ))
}

pub async fn delete_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    CurrentUser(user): CurrentUser,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    Form(form): Form<ImageForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let sRedirect = state
        .image_delete
        .sDelete(&user, form.id, &sRemoteIp)
        .await?;
    Ok(stLegacyFoundRedirect(sRedirect))
}

#[derive(Deserialize)]
pub struct RemoveUserpicForm {
    pub id: Option<i32>,
}

pub async fn remove_userpic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<RemoveUserpicForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    // Java declares `id` as a required @RequestParam; it does not default to
    // the current user when the field is missing.
    let iTargetUserId = form.id.ok_or(AppError::NotFound)?;
    let cService = crate::application::user::CUserModerationService::new(
        crate::infra::postgres::user_moderation_repository::CUserModerationPgRepository::new(
            state.pool.clone(),
        ),
    );
    let sTargetNick = cService.sResetUserpic(&user, iTargetUserId).await?;
    Ok(crate::routes::admin::stProfileRedirect(&sTargetNick))
}

pub async fn reset_password_form(
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    crate::routes::auth::render_reset_password_form(csrf_token, None)
}
