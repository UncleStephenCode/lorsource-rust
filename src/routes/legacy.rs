use crate::{
    auth::CurrentUser,
    error::{AppError, Result},
    markup,
    models::{CommentItem, PagerQuery, TopicSummary},
    pagination::Pager,
    state::AppState,
};
use askama::Template;
use axum::{
    Form, Json,
    extract::{ConnectInfo, Multipart, Path, Query, State},
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use image::GenericImageView;
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;

pub async fn error_403() -> AppError {
    AppError::Forbidden
}
pub async fn error_404() -> AppError {
    AppError::NotFound
}

pub async fn exception_resolver() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html("Exception resolver compatibility endpoint"),
    )
}

#[derive(Template)]
#[template(path = "index.html")]
struct LegacyIndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    news: Vec<crate::routes::topics::NewsTopicView>,
    pager: Pager,
    main_page: bool,
    tracker_layout: bool,
    navigation: Option<crate::routes::topics::TopicListNavigation>,
}

#[derive(Deserialize)]
pub struct LegacyGroupQuery {
    pub group: i32,
    pub offset: Option<i64>,
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
    let (section, group): (String, String) = sqlx::query_as(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END,
                  g.urlname
           FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1"#,
    )
    .bind(q.group)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let mut url = format!("/{section}/{group}");
    let mut params = Vec::new();
    if let Some(offset) = q.offset {
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
    pub section: i32,
}

pub async fn view_section_jsp(
    State(state): State<AppState>,
    Query(q): Query<LegacySectionQuery>,
) -> Result<Redirect> {
    let section: String = sqlx::query_scalar(
        r#"SELECT CASE id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(name) END
           FROM sections WHERE id=$1"#,
    )
    .bind(q.section)
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

pub async fn view_news_jsp(Query(q): Query<ViewNewsQuery>) -> Redirect {
    if let Some(tag) = q.tag {
        Redirect::to(&format!("/tag/{}", urlencoding::encode(&tag)))
    } else {
        Redirect::to("/news/")
    }
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
    CurrentUser(user): CurrentUser,
    Form(form): Form<PreviewForm>,
) -> Json<serde_json::Value> {
    let text = form.text.or(form.msg).or(form.message).unwrap_or_default();

    let markup_id = form
        .markup
        .as_deref()
        .unwrap_or(crate::profile::DEFAULT_FORMAT_MODE);
    if !crate::profile::is_format_mode(markup_id) {
        return Json(json!({"error": "Недопустимый режим разметки"}));
    }
    let _ = &user; // allowed_formats is identical for anon/registered in this port (see profile::FORMAT_MODES)

    if text.is_empty() {
        return Json(json!({"html": ""}));
    }
    if text.chars().count() > 65_536 {
        return Json(json!({"error": "Слишком длинный текст"}));
    }
    let stored_markup = match markup_id {
        "markdown" => "MARKDOWN",
        "ntobr" => "BBCODE_ULB",
        "lorcode" => "BBCODE_TEX",
        _ => "PLAIN",
    };
    let html = markup::render_message_with_markup(&text, Some(stored_markup), None);
    Json(json!({"html": html}))
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
    pub(crate) months: Vec<ArchiveMonthLink>,
}

pub(crate) struct ArchiveMonthLink {
    pub(crate) year: i32,
    pub(crate) month_name: &'static str,
    pub(crate) count: i64,
    pub(crate) url: String,
}

/// ArchiveDao.getArchiveStats: rather than maintaining a separate
/// `monthly_stats` side-table (unpopulated in this port - no triggers
/// write to it), the year/month breakdown is computed live from `topics`.
/// Functionally equivalent to Java's precomputed table for this dataset
/// size; matches `list_archive_topics`' own visibility filter (`NOT
/// deleted`) so the counts always agree with what the drill-down page
/// actually lists.
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
    CurrentUser(_current_user): CurrentUser,
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
    let months = rows
        .into_iter()
        .map(|(y, m, c)| ArchiveMonthLink {
            year: y,
            month_name: month_name(m),
            count: c,
            url: format!("/{section}/archive/{y}/{m}"),
        })
        .collect();
    Ok(Html(
        ArchiveIndexTemplate {
            title: format!("{section_name} - Архив"),
            heading: section_name.to_string(),
            back_url: format!("/{section}/"),
            back_label: "Лента".to_string(),
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
            pager,
            main_page: false,
            tracker_layout: false,
            navigation: None,
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

/// Java's EditHistoryController.canViewHistory requires an authenticated
/// viewer in every branch (moderator, author, or "any logged-in user on a
/// non-expired topic") - anonymous visitors are always rejected. Rust's
/// "expired" (archived-topic) concept isn't modeled yet, so this collapses
/// to "must be logged in", which closes the actual disclosure hole (history
/// text, including deleted/edited content, was previously world-readable).
pub async fn topic_history(
    State(state): State<AppState>,
    uri: Uri,
    Path((_group, id)): Path<(String, i32)>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>> {
    if user.is_none() {
        return Err(AppError::Forbidden);
    }
    render_history(&state, section_from_uri(&uri).unwrap_or("forum"), id, None).await
}

pub async fn comment_history(
    State(state): State<AppState>,
    uri: Uri,
    Path((_group, _id, commentid)): Path<(String, i32, i32)>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>> {
    if user.is_none() {
        return Err(AppError::Forbidden);
    }
    render_history(
        &state,
        section_from_uri(&uri).unwrap_or("forum"),
        commentid,
        Some(commentid),
    )
    .await
}

async fn render_history(
    state: &AppState,
    section: &str,
    msgid: i32,
    commentid: Option<i32>,
) -> Result<Html<String>> {
    let rows = sqlx::query_as::<
        _,
        (
            i32,
            String,
            String,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"SELECT e.id, u.nick, COALESCE(e.oldtitle,''), e.oldmessage, e.editdate
           FROM edit_info e JOIN users u ON u.id=e.editor
           WHERE e.msgid=$1
           ORDER BY e.editdate DESC LIMIT 50"#,
    )
    .bind(msgid)
    .fetch_all(&state.pool)
    .await?;

    let mut html = format!("<h1>История изменений {section} #{msgid}</h1>");
    if let Some(commentid) = commentid {
        html.push_str(&format!("<p>Комментарий: #{commentid}</p>"));
    }
    if rows.is_empty() {
        html.push_str("<p class=\"muted\">История изменений пуста.</p>");
    } else {
        html.push_str("<ul>");
        for (_id, editor, old_title, old_message, editdate) in rows {
            html.push_str(&format!(
                "<li><b>{}</b> · {}<br><small>{}</small><pre>{}</pre></li>",
                html_escape::encode_text(&editor),
                editdate,
                html_escape::encode_text(&old_title),
                html_escape::encode_text(old_message.as_deref().unwrap_or(""))
            ));
        }
        html.push_str("</ul>");
    }
    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct ShowCommentsQuery {
    pub nick: String,
}

pub async fn show_comments_jsp(Query(q): Query<ShowCommentsQuery>) -> Redirect {
    Redirect::to(&format!(
        "/search.jsp?range=COMMENTS&user={}&sort=DATE",
        urlencoding::encode(&q.nick)
    ))
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
        let body = render_replies_feed(&state, &target_nick, &events, is_atom);
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

    let mut html = format!(
        "<h1>Уведомления {}</h1><p class=\"muted\">Просмотр от имени модератора {}.</p><ul class=\"notifications-list\">",
        html_escape::encode_text(&nick),
        html_escape::encode_text(&current.nick)
    );
    for e in &events {
        html.push_str(&format!(
            "<li{}><a href=\"{}\">{}</a> <small>{} · {}</small></li>",
            if e.unread { " class=\"unread\"" } else { "" },
            e.link(),
            html_escape::encode_text(&e.subj),
            e.event_date,
            html_escape::encode_text(&e.event_type),
        ));
    }
    if events.is_empty() {
        html.push_str("<li class=\"muted\">Нет уведомлений</li>");
    }
    html.push_str("</ul>");
    Ok(Html(html).into_response())
}

fn render_replies_feed(
    state: &AppState,
    nick: &str,
    events: &[crate::routes::api::NotificationEvent],
    atom: bool,
) -> String {
    let title = format!("Ответы пользователю {nick}");
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
            body.push_str(&format!(
                "<entry><title>{}</title><link href=\"{link}\"/><id>{link}</id><updated>{}</updated></entry>",
                html_escape::encode_text(&e.subj), e.event_date.to_rfc3339(),
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
            body.push_str(&format!(
                "<item><title>{}</title><link>{link}</link><guid>{link}</guid><pubDate>{}</pubDate></item>",
                html_escape::encode_text(&e.subj), e.event_date.to_rfc2822(),
            ));
        }
        body.push_str("</channel></rss>");
        body
    }
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
                  c.postdate, u.id AS author_id, u.nick AS author, c.deleted
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

fn sRenderDeletedComment(stPrepared: &StPreparedDeletedComment) -> String {
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
    let sTitle = if !stComment.title.trim().is_empty() {
        format!("<h1>{}</h1>", html_escape::encode_text(&stComment.title))
    } else {
        String::new()
    };
    format!(
        "<article class=\"msg\" id=\"comment-{id}\">{deleted_title}<div class=\"msg-container\"><div class=\"msg_body\"><div class=\"msg-text\">{title}{body}</div><div class=\"sign\"><a href=\"/people/{author_url}/profile\">{author}</a>, {date}</div></div></div></article>",
        id = stComment.id,
        deleted_title = sDeletedTitle,
        title = sTitle,
        body =
            markup::render_message_with_markup(&stComment.message, Some(&stComment.markup), None),
        author_url = urlencoding::encode(&stComment.author),
        author = html_escape::encode_text(&stComment.author),
        date = stComment.postdate,
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
    let mut sHtml = format!(
        "<h1>Просмотр удаленного комментария</h1><nav><a class=\"btn btn-default\" href=\"{}\">Перейти в топик</a></nav><div class=\"messages\">",
        html_escape::encode_double_quoted_attribute(&sBackLink)
    );
    for stParent in &vecChain {
        sHtml.push_str("<h2>Ответ на:</h2>");
        sHtml.push_str(&sRenderDeletedComment(stParent));
    }
    if !vecChain.is_empty() {
        sHtml.push_str("<h2>Удаленный комментарий:</h2>");
    }
    sHtml.push_str(&sRenderDeletedComment(&StPreparedDeletedComment {
        stComment,
        optDeleteInfo: Some((sDeletedBy, sDeleteReason)),
    }));
    sHtml.push_str("</div>");
    Ok(Html(sHtml))
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
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Html<String> {
    render_activation_form(
        q.nick.as_deref().unwrap_or(""),
        q.activation.as_deref().unwrap_or(""),
        q.error.as_deref(),
        &csrf_token,
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
            )
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
                )
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
            )
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
        )
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
) -> Html<String> {
    let error_html = error
        .map(|e| format!("<p class=\"error\">{}</p>", html_escape::encode_text(e)))
        .unwrap_or_default();
    Html(format!(
        r#"
<h1>Активация аккаунта</h1>
{error_html}
<form method="post" action="/activate" class="form">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="action" value="activate">
  <label>Ник <input name="nick" value="{nick}" required></label>
  <label>Пароль <input name="passwd" type="password" required></label>
  <label>Код активации <input name="activation" value="{activation}" required></label>
  <button type="submit">Активировать</button>
</form>
"#,
        nick = html_escape::encode_double_quoted_attribute(nick),
        activation = html_escape::encode_double_quoted_attribute(activation)
    ))
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
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    Ok(Html(format!(
        r#"
<h1>Загрузить userpic для {nick}</h1>
<form method="post" action="/addphoto.jsp" enctype="multipart/form-data" class="form">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <label>Файл PNG/JPEG/WEBP, 50–300 px, до 100 KiB <input type="file" name="file" accept="image/png,image/jpeg,image/webp" required></label>
  <button type="submit">Загрузить</button>
</form>
"#,
        nick = html_escape::encode_text(&user.nick),
        csrf_token = html_escape::encode_double_quoted_attribute(&csrf_token),
    )))
}

pub async fn upload_userpic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    mut multipart: Multipart,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let mut upload: Option<(String, bytes::Bytes)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("ошибка multipart: {e}")))?
    {
        let is_file = field.name() == Some("file");
        let filename = field.file_name().unwrap_or("userpic").to_string();
        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("ошибка чтения файла: {e}")))?;
        if is_file {
            upload = Some((filename, data));
            break;
        }
    }
    let (_original_name, bytes) =
        upload.ok_or_else(|| AppError::BadRequest("изображение не задано".into()))?;
    let extension = validate_userpic_bytes(&bytes)?;
    let filename = format!("{}-{}.{}", user.id, uuid::Uuid::new_v4(), extension);
    let dir = format!("{}/photos", state.config.upload_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::Anyhow(e.into()))?;
    let path = format!("{dir}/{filename}");
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| AppError::Anyhow(e.into()))?;
    sqlx::query("UPDATE users SET photo=$2 WHERE id=$1")
        .bind(user.id)
        .bind(&filename)
        .execute(&state.pool)
        .await?;
    crate::audit::log_user_action(
        &state.pool,
        user.id,
        user.id,
        "set_userpic",
        &[("file", filename.as_str())],
    )
    .await?;
    Ok(Redirect::to(&format!(
        "/people/{}/profile?nocache={}",
        urlencoding::encode(&user.nick),
        uuid::Uuid::new_v4()
    )))
}

fn validate_userpic_bytes(data: &[u8]) -> Result<&'static str> {
    const MAX_FILE_SIZE: usize = 100 * 1024;
    const MIN_IMAGE_SIZE: u32 = 50;
    const MAX_IMAGE_SIZE: u32 = 300;
    if data.is_empty() {
        return Err(AppError::BadRequest("изображение не задано".into()));
    }
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::BadRequest(
            "Сбой загрузки изображения: слишком большой файл".into(),
        ));
    }
    let format = image::guess_format(data).map_err(|_| {
        AppError::BadRequest("Сбой загрузки изображения: неизвестный формат".into())
    })?;
    let extension =
        match format {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::WebP => "webp",
            _ => return Err(AppError::BadRequest(
                "Сбой загрузки изображения: неподдерживаемый или потенциально анимированный формат"
                    .into(),
            )),
        };
    let image = image::load_from_memory_with_format(data, format)
        .map_err(|e| AppError::BadRequest(format!("Сбой загрузки изображения: {e}")))?;
    let (width, height) = image.dimensions();
    if !(MIN_IMAGE_SIZE..=MAX_IMAGE_SIZE).contains(&width)
        || !(MIN_IMAGE_SIZE..=MAX_IMAGE_SIZE).contains(&height)
    {
        return Err(AppError::BadRequest(
            "Сбой загрузки изображения: недопустимые размеры фотографии".into(),
        ));
    }
    Ok(extension)
}

#[derive(Deserialize)]
pub struct DeregisterForm {
    pub password: String,
    pub accept_block: Option<String>,
    pub acceptBlock: Option<String>,
    pub accept_oneway: Option<String>,
    pub acceptOneway: Option<String>,
}

pub async fn deregister_form(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    ensure_deregister_allowed(&state, &user).await?;
    Ok(Html(format!(
        r#"
<h1>Удаление аккаунта {nick}</h1>
<p>Операция соответствует исходной логике: аккаунт блокируется, профиль очищается, восстановление через эту форму не предусмотрено.</p>
<form method="post" action="/deregister.jsp" class="form">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <label>Пароль <input name="password" type="password" required></label>
  <label><input type="checkbox" name="acceptBlock" value="true" required> Я согласен с блокировкой аккаунта</label>
  <label><input type="checkbox" name="acceptOneway" value="true" required> Я понимаю, что действие необратимо</label>
  <button type="submit">Удалить аккаунт</button>
</form>
"#,
        nick = html_escape::encode_text(&user.nick)
    )))
}

pub async fn deregister_post(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Form(form): Form<DeregisterForm>,
) -> Result<impl IntoResponse> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    ensure_deregister_allowed(&state, &user).await?;
    if form.accept_block.or(form.acceptBlock).is_none() {
        return Err(AppError::BadRequest(
            "Вы не согласились с блокировкой аккаунта".into(),
        ));
    }
    if form.accept_oneway.or(form.acceptOneway).is_none() {
        return Err(AppError::BadRequest(
            "Вы не согласились с невозможностью восстановления аккаунта".into(),
        ));
    }
    let ok = matches!(
        crate::auth::verify_login(&state.pool, &user.nick, &form.password).await?,
        crate::auth::LoginOutcome::Success(_)
    );
    if !ok {
        return Err(AppError::BadRequest("Неверный пароль".into()));
    }
    sqlx::query(
        "UPDATE users SET name='', url='', town='', userinfo='', photo=NULL, blocked=true WHERE id=$1",
    )
    .bind(user.id)
    .execute(&state.pool)
    .await?;
    crate::audit::log_user_action(
        &state.pool,
        user.id,
        user.id,
        "block_user",
        &[("reason", "deregister")],
    )
    .await?;
    Ok((
        jar.remove(
            Cookie::build((crate::security::remember_me::COOKIE_NAME, ""))
                .path("/")
                .build(),
        )
        .remove(Cookie::build(("lor_session", "")).path("/").build()),
        Html("<h1>Удаление пользователя прошло успешно.</h1>".to_string()),
    )
        .into_response())
}

async fn ensure_deregister_allowed(
    state: &AppState,
    user: &crate::models::UserSummary,
) -> Result<()> {
    if user.max_score.unwrap_or(0) < 100 {
        return Err(AppError::Forbidden);
    }
    if user.canmod {
        return Err(AppError::Forbidden);
    }
    if user.blocked.unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    if frozen_until
        .map(|u| u > chrono::Utc::now())
        .unwrap_or(false)
    {
        return Err(AppError::Forbidden);
    }
    Ok(())
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

pub async fn user_filter(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let tags = sqlx::query_as::<_, (String, bool)>(
        "SELECT tv.value, ut.is_favorite FROM user_tags ut JOIN tags_values tv ON tv.id=ut.tag_id WHERE ut.user_id=$1 ORDER BY tv.value",
    ).bind(user.id).fetch_all(&state.pool).await?;
    let ignored = sqlx::query_as::<_, (String,)>(
        "SELECT u.nick FROM ignore_list il JOIN users u ON u.id=il.ignored WHERE il.userid=$1 ORDER BY u.nick",
    ).bind(user.id).fetch_all(&state.pool).await?;
    Ok(Json(
        json!({"tags": tags.into_iter().map(|(tag, favorite)| json!({"tag": tag, "favorite": favorite})).collect::<Vec<_>>(), "ignoredUsers": ignored.into_iter().map(|(nick,)| nick).collect::<Vec<_>>() }),
    ))
}

#[derive(Deserialize)]
pub struct UserTagForm {
    pub tag: Option<String>,
    #[serde(rename = "tagName")]
    pub tag_name: Option<String>,
    pub del: Option<String>,
}

pub async fn favorite_tag(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<UserTagForm>,
) -> Result<Json<serde_json::Value>> {
    save_or_delete_user_tag(state, user, form, true).await
}

pub async fn ignore_tag(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<UserTagForm>,
) -> Result<Json<serde_json::Value>> {
    if user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    save_or_delete_user_tag(state, user, form, false).await
}

async fn save_or_delete_user_tag(
    state: AppState,
    user: Option<crate::models::UserSummary>,
    form: UserTagForm,
    is_favorite: bool,
) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let tag = form
        .tag_name
        .or(form.tag)
        .unwrap_or_default()
        .trim()
        .to_string();
    if tag.is_empty() {
        return Err(AppError::BadRequest("tagName is required".into()));
    }

    let tag_id: i32 = if form.del.is_some() {
        sqlx::query_scalar("SELECT id FROM tags_values WHERE lower(value)=lower($1)")
            .bind(&tag)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?
    } else {
        sqlx::query_scalar(
            "INSERT INTO tags_values(value,counter) VALUES($1,0) ON CONFLICT(value) DO UPDATE SET value=EXCLUDED.value RETURNING id",
        )
        .bind(&tag)
        .fetch_one(&state.pool)
        .await?
    };

    if form.del.is_some() {
        sqlx::query("DELETE FROM user_tags WHERE user_id=$1 AND tag_id=$2 AND is_favorite=$3")
            .bind(user.id)
            .bind(tag_id)
            .bind(is_favorite)
            .execute(&state.pool)
            .await?;
    } else {
        sqlx::query("INSERT INTO user_tags(user_id,tag_id,is_favorite) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(user.id)
            .bind(tag_id)
            .bind(is_favorite)
            .execute(&state.pool)
            .await?;
    }

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_tags WHERE tag_id=$1 AND is_favorite=$2")
            .bind(tag_id)
            .bind(is_favorite)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(
        json!({"count": count, "tag": tag, "favorite": is_favorite}),
    ))
}

#[derive(Deserialize)]
pub struct IgnoreUserForm {
    pub id: Option<i32>,
    pub nick: Option<String>,
    pub del: Option<String>,
}

pub async fn ignore_user(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<IgnoreUserForm>,
) -> Result<Json<serde_json::Value>> {
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
    } else {
        sqlx::query("INSERT INTO ignore_list(userid,ignored) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(user.id)
            .bind(ignored_id)
            .execute(&state.pool)
            .await?;
    }
    Ok(Json(
        json!({"ok": true, "ignored": ignored_id, "deleted": form.del.is_some()}),
    ))
}

#[derive(Deserialize)]
pub struct LegacyMsgIdQuery {
    pub msgid: i32,
}

#[derive(Deserialize)]
pub struct ScoreForm {
    pub msgid: i32,
    pub score: Option<i32>,
    pub postscore: Option<i32>,
}

pub async fn set_post_score_form(
    Query(q): Query<LegacyMsgIdQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    Ok(Html(format!(
        r#"
<h1>Изменить score темы #{}</h1>
<form method="post" action="/setpostscore.jsp">
<input type="hidden" name="csrf" value="{csrf_token}">
<input type="hidden" name="msgid" value="{}">
<input name="score" type="number" value="0">
<button type="submit">Сохранить</button>
</form>
"#,
        q.msgid, q.msgid
    )))
}

pub async fn set_post_score(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ScoreForm>,
) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    let score = form.score.or(form.postscore).unwrap_or(0);
    sqlx::query("UPDATE topics SET postscore=$2,lastmod=now() WHERE id=$1")
        .bind(form.msgid)
        .bind(score)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to(&format!(
        "/jump-message.jsp?msgid={}",
        form.msgid
    )))
}

#[derive(Deserialize)]
pub struct ImageForm {
    pub id: i32,
}

pub async fn delete_image_form(
    Query(q): Query<ImageForm>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    if user.is_none() {
        return Err(AppError::Forbidden);
    }
    Ok(Html(format!(
        r#"
<h1>Удалить изображение #{}</h1>
<form method="post" action="/delete_image"><input type="hidden" name="csrf" value="{csrf_token}"><input type="hidden" name="id" value="{}"><button type="submit">Удалить</button></form>
"#,
        q.id, q.id
    )))
}

pub async fn delete_image(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ImageForm>,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let Some((topic_id, author_id, is_main, group_urlname)): Option<(i32, i32, bool, String)> =
        sqlx::query_as(
            r#"SELECT i.topic, t.userid, i.main, g.urlname
           FROM images i JOIN topics t ON t.id=i.topic JOIN groups g ON g.id=t.groupid
           WHERE i.id=$1"#,
        )
        .bind(form.id)
        .fetch_optional(&state.pool)
        .await?
    else {
        return Err(AppError::NotFound);
    };
    if !user.canmod && user.id != author_id {
        return Err(AppError::Forbidden);
    }
    // Matches DeleteImageController.checkDelete: a gallery section's main
    // image can't be deleted through this endpoint at all - the previous
    // handler had no such guard.
    if is_main {
        return Err(AppError::Forbidden);
    }
    sqlx::query("UPDATE images SET deleted=true WHERE id=$1")
        .bind(form.id)
        .execute(&state.pool)
        .await?;
    sqlx::query("UPDATE topics SET lastmod=now() WHERE id=$1")
        .bind(topic_id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to(&format!(
        "/gallery/{}/{}",
        urlencoding::encode(&group_urlname),
        topic_id
    )))
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
