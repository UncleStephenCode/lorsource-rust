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
    extract::{Multipart, Path, Query, State},
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect, Response},
    Form, Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use image::GenericImageView;
use serde::Deserialize;
use serde_json::json;

pub async fn gone() -> impl IntoResponse {
    (StatusCode::GONE, Html("Legacy endpoint is no longer available."))
}

pub async fn error_403() -> AppError { AppError::Forbidden }
pub async fn error_404() -> AppError { AppError::NotFound }

pub async fn exception_resolver() -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, Html("Exception resolver compatibility endpoint"))
}

#[derive(Template)]
#[template(path = "index.html")]
struct LegacyIndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    pager: Pager,
    current_user: Option<crate::models::UserSummary>,
}

#[derive(Deserialize)]
pub struct LegacyGroupQuery {
    pub group: i32,
    pub offset: Option<i64>,
}

pub async fn group_jsp(State(state): State<AppState>, Query(q): Query<LegacyGroupQuery>) -> Result<Redirect> {
    group_redirect(state, q, false).await
}

pub async fn group_lastmod_jsp(State(state): State<AppState>, Query(q): Query<LegacyGroupQuery>) -> Result<Redirect> {
    group_redirect(state, q, true).await
}

async fn group_redirect(state: AppState, q: LegacyGroupQuery, lastmod: bool) -> Result<Redirect> {
    let (section, group): (String, String) = sqlx::query_as(
        r#"SELECT CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END,
                  g.urlname
           FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1"#,
    )
    .bind(q.group)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let mut url = format!("/{section}/{group}");
    let mut params = Vec::new();
    if let Some(offset) = q.offset { params.push(format!("offset={offset}")); }
    if lastmod { params.push("lastmod=true".to_string()); }
    if !params.is_empty() { url.push('?'); url.push_str(&params.join("&")); }
    Ok(Redirect::to(&url))
}

#[derive(Deserialize)]
pub struct LegacySectionQuery { pub section: i32 }

pub async fn view_section_jsp(State(state): State<AppState>, Query(q): Query<LegacySectionQuery>) -> Result<Redirect> {
    let section: String = sqlx::query_scalar(
        r#"SELECT CASE name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(name) END
           FROM sections WHERE id=$1"#,
    )
    .bind(q.section)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let target = if section == "forum" { "/forum".to_string() } else { format!("/{section}/") };
    Ok(Redirect::to(&target))
}

#[derive(Deserialize)]
pub struct ViewNewsQuery { pub tag: Option<String> }

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
pub async fn markup_preview(CurrentUser(user): CurrentUser, Form(form): Form<PreviewForm>) -> Json<serde_json::Value> {
    let text = form.text.or(form.msg).or(form.message).unwrap_or_default();

    let markup_id = form.markup.as_deref().unwrap_or(crate::profile::DEFAULT_FORMAT_MODE);
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
    let html = markup::render_message(&text, Some(markup_id != "markdown"));
    Json(json!({"html": html}))
}

#[derive(Deserialize)]
pub struct CheckLoginQuery { pub nick: Option<String> }

pub async fn check_login(State(state): State<AppState>, Query(q): Query<CheckLoginQuery>) -> Result<Json<serde_json::Value>> {
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
pub async fn yandex_tableau(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Ok(Json(json!({}))); };
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

pub async fn help_page(State(state): State<AppState>, Path(page): Path<String>) -> Result<Html<String>> {
    let Some(title) = help_page_title(&page) else { return Err(AppError::NotFound); };
    let path = format!("{}/help/{page}", state.config.static_dir);
    let source = tokio::fs::read_to_string(&path).await.map_err(|_| AppError::NotFound)?;
    let html = markup::render_message(&source, Some(false));
    Ok(Html(format!("<h1>{}</h1>{html}", html_escape::encode_text(title))))
}

const MONTH_NAMES: [&str; 12] = ["Январь", "Февраль", "Март", "Апрель", "Май", "Июнь", "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь"];

pub(crate) fn month_name(month: i32) -> &'static str {
    MONTH_NAMES.get((month - 1) as usize).copied().unwrap_or("?")
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
    pub(crate) month: i32,
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
pub(crate) async fn list_archive_year_months(state: &AppState, section: Option<&str>, group: Option<&str>) -> Result<Vec<(i32, i32, i64)>> {
    Ok(sqlx::query_as::<_, (i32, i32, i64)>(
        r#"SELECT EXTRACT(YEAR FROM t.postdate)::int AS y, EXTRACT(MONTH FROM t.postdate)::int AS m, count(*) AS c
           FROM topics t
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE ($1::text IS NULL OR CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END = $1)
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

pub async fn archive_section(State(state): State<AppState>, uri: Uri, CurrentUser(_current_user): CurrentUser) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let section_name = match section { "news" => "Новости", "forum" => "Форум", "gallery" => "Галерея", "articles" => "Статьи", "polls" => "Опросы", _ => "Темы" };
    let rows = list_archive_year_months(&state, Some(section), None).await?;
    let months = rows.into_iter().map(|(y, m, c)| ArchiveMonthLink { year: y, month: m, month_name: month_name(m), count: c, url: format!("/{section}/archive/{y}/{m}") }).collect();
    Ok(Html(ArchiveIndexTemplate {
        title: format!("{section_name} - Архив"),
        heading: section_name.to_string(),
        back_url: format!("/{section}/"),
        back_label: "Лента".to_string(),
        months,
    }.render()?))
}

pub async fn archive_section_month(State(state): State<AppState>, uri: Uri, Path((year, month)): Path<(i32, i32)>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    validate_year_month(year, month)?;
    let section = section_from_uri(&uri).unwrap_or("news");
    render_archive(state, Some(section), None, Some(year), Some(month), q, current_user).await
}

pub async fn forum_archive_month(State(state): State<AppState>, Path((group, year, month)): Path<(String, i32, i32)>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    validate_year_month(year, month)?;
    render_archive(state, Some("forum"), Some(group), Some(year), Some(month), q, current_user).await
}

async fn render_archive(
    state: AppState,
    section: Option<&str>,
    group: Option<String>,
    year: Option<i32>,
    month: Option<i32>,
    q: PagerQuery,
    current_user: Option<crate::models::UserSummary>,
) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_archive_topics(&state, section, group.as_deref(), year, month, pager.offset, pager.limit).await?;
    let title = match (section, group.as_deref(), year, month) {
        (Some(sec), Some(group), Some(y), Some(m)) => format!("Архив: {sec}/{group}, {y:04}-{m:02}"),
        (Some(sec), _, Some(y), Some(m)) => format!("Архив: {sec}, {y:04}-{m:02}"),
        (Some(sec), _, _, _) => format!("Архив: {sec}"),
        _ => "Архив".to_string(),
    };
    Ok(Html(LegacyIndexTemplate { title, topics, pager, current_user }.render()?))
}

async fn list_archive_topics(state: &AppState, section: Option<&str>, group: Option<&str>, year: Option<i32>, month: Option<i32>, offset: i64, limit: i64) -> Result<Vec<TopicSummary>> {
    Ok(sqlx::query_as::<_, TopicSummary>(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                  string_agg(tv.value, ',' ORDER BY tv.value) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN tags tg ON tg.msgid=t.id
           LEFT JOIN tags_values tv ON tv.id=tg.tagid
           WHERE ($1::text IS NULL OR CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END = $1)
             AND ($2::text IS NULL OR g.urlname=$2)
             AND ($3::int IS NULL OR EXTRACT(YEAR FROM t.postdate)::int=$3)
             AND ($4::int IS NULL OR EXTRACT(MONTH FROM t.postdate)::int=$4)
             AND NOT t.deleted
           GROUP BY t.id,u.id,g.id,s.id
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
pub async fn topic_history(State(state): State<AppState>, uri: Uri, Path((_group, id)): Path<(String, i32)>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    render_history(&state, section_from_uri(&uri).unwrap_or("forum"), id, None).await
}

pub async fn comment_history(State(state): State<AppState>, uri: Uri, Path((_group, _id, commentid)): Path<(String, i32, i32)>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    render_history(&state, section_from_uri(&uri).unwrap_or("forum"), commentid, Some(commentid)).await
}

async fn render_history(state: &AppState, section: &str, msgid: i32, commentid: Option<i32>) -> Result<Html<String>> {
    let rows = sqlx::query_as::<_, (i32, String, String, Option<String>, chrono::NaiveDateTime)>(
        r#"SELECT e.id, u.nick, COALESCE(e.oldtitle,''), e.oldmessage, e.editdate
           FROM edit_info e JOIN users u ON u.id=e.editor
           WHERE e.msgid=$1
           ORDER BY e.editdate DESC LIMIT 50"#,
    )
    .bind(msgid)
    .fetch_all(&state.pool)
    .await?;

    let mut html = format!("<h1>История изменений {section} #{msgid}</h1>");
    if let Some(commentid) = commentid { html.push_str(&format!("<p>Комментарий: #{commentid}</p>")); }
    if rows.is_empty() {
        html.push_str("<p class=\"muted\">История изменений пуста.</p>");
    } else {
        html.push_str("<ul>");
        for (_id, editor, old_title, old_message, editdate) in rows {
            html.push_str(&format!("<li><b>{}</b> · {}<br><small>{}</small><pre>{}</pre></li>",
                html_escape::encode_text(&editor), editdate,
                html_escape::encode_text(&old_title),
                html_escape::encode_text(old_message.as_deref().unwrap_or(""))));
        }
        html.push_str("</ul>");
    }
    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct ShowCommentsQuery { pub nick: String }

pub async fn show_comments_jsp(Query(q): Query<ShowCommentsQuery>) -> Redirect {
    Redirect::to(&format!("/search.jsp?range=COMMENTS&user={}&sort=DATE", urlencoding::encode(&q.nick)))
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
pub async fn show_replies_jsp(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<ShowRepliesQuery>) -> Result<Response> {
    if let Some(output) = q.output.as_deref() {
        let nick = q.nick.clone().unwrap_or_default();
        if !valid_login_name_for_java(&nick) {
            return Err(AppError::BadRequest("некорректное имя пользователя".into()));
        }
        let target: Option<(i32, String)> = sqlx::query_as("SELECT id, nick FROM users WHERE lower(nick)=lower($1)")
            .bind(&nick)
            .fetch_optional(&state.pool)
            .await?;
        let Some((target_id, target_nick)) = target else { return Err(AppError::NotFound); };
        let view_by_owner = user.as_ref().map(|u| u.nick.eq_ignore_ascii_case(&target_nick)).unwrap_or(false);
        let db_type = q.filter.as_deref().and_then(crate::routes::api::filter_db_type);
        let events = crate::routes::api::fetch_events(&state, target_id, db_type, view_by_owner, 200, 0).await?;

        let is_atom = output == "atom";
        let body = render_replies_feed(&state, &target_nick, &events, is_atom);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            (if is_atom { "application/atom+xml; charset=utf-8" } else { "application/rss+xml; charset=utf-8" }).parse().unwrap(),
        );
        // Java sets `Expires: now + 90s` on this feed endpoint.
        let expires = (chrono::Utc::now() + chrono::Duration::seconds(90)).to_rfc2822();
        headers.insert(axum::http::header::EXPIRES, expires.parse().unwrap());
        return Ok((headers, body).into_response());
    }

    let Some(nick) = q.nick.clone() else {
        if user.is_none() { return Err(AppError::Forbidden); }
        return Ok(Redirect::to("/notifications").into_response());
    };
    if !valid_login_name_for_java(&nick) {
        return Err(AppError::BadRequest("некорректное имя пользователя".into()));
    }
    let Some(current) = user else { return Err(AppError::Forbidden); };
    if current.nick.eq_ignore_ascii_case(&nick) {
        return Ok(Redirect::to("/notifications").into_response());
    }
    if !current.canmod {
        return Err(AppError::Forbidden);
    }

    let target_id: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE lower(nick)=lower($1)").bind(&nick).fetch_optional(&state.pool).await?;
    let Some(target_id) = target_id else { return Err(AppError::NotFound); };
    let db_type = q.filter.as_deref().and_then(crate::routes::api::filter_db_type);
    let offset = q.offset.unwrap_or(0).max(0);
    let events = crate::routes::api::fetch_events(&state, target_id, db_type, true, 20, offset).await?;

    let mut html = format!("<h1>Уведомления {}</h1><p class=\"muted\">Просмотр от имени модератора {}.</p><ul class=\"notifications-list\">", html_escape::encode_text(&nick), html_escape::encode_text(&current.nick));
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

fn render_replies_feed(state: &AppState, nick: &str, events: &[crate::routes::api::NotificationEvent], atom: bool) -> String {
    let title = format!("Ответы пользователю {nick}");
    if atom {
        let mut body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>{}</title><link href="{}/show-replies.jsp?nick={}&amp;output=atom" rel="self"/><id>{}/show-replies.jsp?nick={}</id>"#,
            html_escape::encode_text(&title), state.config.public_url, urlencoding::encode(nick), state.config.public_url, urlencoding::encode(nick),
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
            html_escape::encode_text(&title), state.config.public_url, urlencoding::encode(nick), html_escape::encode_text(&title),
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

pub async fn view_deleted(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    let comments = sqlx::query_as::<_, CommentItem>(
        r#"SELECT c.id, c.topic, c.replyto, c.title, m.message, c.postdate, u.id AS author_id, u.nick AS author, c.deleted
           FROM comments c JOIN msgbase m ON m.id=c.id JOIN users u ON u.id=c.userid
           WHERE c.deleted ORDER BY c.postdate DESC LIMIT 100"#,
    )
    .fetch_all(&state.pool)
    .await?;
    let mut html = "<h1>Удалённые комментарии</h1>".to_string();
    for c in comments {
        html.push_str(&format!("<article id=\"comment-{}\"><h3>{}</h3><p>{} · topic #{}</p><div>{}</div></article>",
            c.id, html_escape::encode_text(&c.title), html_escape::encode_text(&c.author), c.topic,
            markup::render_message(&c.message, Some(true))));
    }
    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct NotificationsClickForm {
    #[serde(rename = "firstId")]
    pub first_id: i32,
    #[serde(rename = "lastId")]
    pub last_id: i32,
}

async fn topic_link(state: &AppState, topic_id: i32, comment_id: Option<i32>) -> Result<String> {
    let prefix: Option<(String, String)> = sqlx::query_as(
        r#"SELECT CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END,
                  g.urlname
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((section, group)) = prefix else { return Ok("/notifications".to_string()); };
    let anchor = comment_id.map(|id| format!("?cid={id}")).unwrap_or_default();
    Ok(format!("/{section}/{group}/{topic_id}{anchor}"))
}

/// Simplified from UserEventController.processClickNotifications: verifies
/// both events belong to the current user, marks the id range read, and
/// returns the link to the first event's topic/comment. The FAVORITES/
/// REACTION grouped-range validation (isValidClickRange) isn't replicated -
/// out of scope for a first pass, tracked as a follow-up.
async fn process_notifications_click(state: &AppState, user_id: i32, form: &NotificationsClickForm) -> Result<String> {
    let first: Option<(i32,)> = sqlx::query_as("SELECT userid FROM user_events WHERE id=$1").bind(form.first_id).fetch_optional(&state.pool).await?;
    let last: Option<(i32, bool, i32, Option<i32>)> = sqlx::query_as(
        "SELECT userid, unread, message_id, comment_id FROM user_events WHERE id=$1",
    )
    .bind(form.last_id)
    .fetch_optional(&state.pool)
    .await?;

    let (Some((first_owner,)), Some((last_owner, last_unread, _, _))) = (first, last) else {
        return Ok("/notifications".to_string());
    };
    if user_id != first_owner || user_id != last_owner {
        return Err(AppError::Forbidden);
    }

    if last_unread {
        let (lo, hi) = (form.first_id.min(form.last_id), form.first_id.max(form.last_id));
        sqlx::query("UPDATE user_events SET unread=false WHERE userid=$1 AND unread AND id BETWEEN $2 AND $3")
            .bind(user_id).bind(lo).bind(hi).execute(&state.pool).await?;
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=$1")
            .bind(user_id).execute(&state.pool).await?;
    }

    let first_target: Option<(i32, Option<i32>)> = sqlx::query_as("SELECT message_id, comment_id FROM user_events WHERE id=$1")
        .bind(form.first_id)
        .fetch_optional(&state.pool)
        .await?;
    match first_target {
        Some((topic_id, comment_id)) => topic_link(state, topic_id, comment_id).await,
        None => Ok("/notifications".to_string()),
    }
}

pub async fn notifications_click(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<NotificationsClickForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let url = process_notifications_click(&state, user.id, &form).await?;
    Ok(Redirect::to(&url))
}

pub async fn notifications_click_ajax(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<NotificationsClickForm>) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let url = process_notifications_click(&state, user.id, &form).await?;
    Ok(Json(json!({"url": url})))
}

#[derive(Deserialize)]
pub struct ActivationQuery {
    pub nick: Option<String>,
    pub activation: Option<String>,
    pub error: Option<String>,
}

pub async fn activate_form(Query(q): Query<ActivationQuery>, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Html<String> {
    render_activation_form(q.nick.as_deref().unwrap_or(""), q.activation.as_deref().unwrap_or(""), q.error.as_deref(), &csrf_token)
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
    Form(form): Form<ActivationForm>,
) -> Result<impl IntoResponse> {
    if form.action.is_some() {
        let nick = form.nick.as_deref().unwrap_or("").trim();
        let password = form.passwd.as_deref().unwrap_or("");
        let Some((id, db_nick, email, regdate, activated)) = sqlx::query_as::<_, (i32, String, Option<String>, Option<chrono::NaiveDateTime>, bool)>(
            "SELECT id,nick,email,regdate,activated FROM users WHERE lower(nick)=lower($1)",
        )
        .bind(nick)
        .fetch_optional(&state.pool)
        .await? else {
            return Ok(render_activation_form(nick, &form.activation, Some("Пользователь не найден"), &csrf_token).into_response());
        };

        if activated {
            return Ok(Redirect::to("/").into_response());
        }

        if matches!(crate::auth::verify_login(&state.pool, nick, password).await?, crate::auth::LoginOutcome::Failed) {
            // verify_login deliberately refuses inactive users, so do a direct password check here.
            let encoded: Option<String> = sqlx::query_scalar("SELECT passwd FROM users WHERE id=$1")
                .bind(id)
                .fetch_one(&state.pool)
                .await?;
            if !encoded.as_deref().map(|hash| crate::security::password::verify(password, hash)).unwrap_or(false) {
                return Ok(render_activation_form(nick, &form.activation, Some("Неправильный логин или пароль"), &csrf_token).into_response());
            }
        }

        if !verify_activation_code(&state, &db_nick, email.as_deref().unwrap_or(""), regdate, &form.activation) {
            return Ok(render_activation_form(nick, &form.activation, Some("Неправильный код активации"), &csrf_token).into_response());
        }

        sqlx::query("UPDATE users SET activated=true,lastlogin=now() WHERE id=$1")
            .bind(id)
            .execute(&state.pool)
            .await?;
        crate::audit::log_user_action(&state.pool, id, id, "register", &[]).await?;
        let cookie = Cookie::build(("lor_session", crate::auth::make_session(id, &state.config.cookie_secret)))
            .path("/")
            .http_only(true)
            .secure(crate::security::is_secure_request(&headers))
            .same_site(SameSite::Lax)
            .build();
        return Ok((jar.add(cookie), Redirect::to("/")).into_response());
    }

    let Some(user) = current_user else { return Err(AppError::Forbidden); };
    let Some((email, regdate)) = sqlx::query_as::<_, (Option<String>, Option<chrono::NaiveDateTime>)>(
        "SELECT new_email,regdate FROM users WHERE id=$1",
    )
    .bind(user.id)
    .fetch_optional(&state.pool)
    .await? else { return Err(AppError::NotFound); };
    let Some(new_email) = email else { return Err(AppError::BadRequest("new_email == null".into())); };

    if !verify_activation_code(&state, &user.nick, &new_email, regdate, &form.activation) {
        return Ok(render_activation_form(&user.nick, &form.activation, Some("Неправильный код активации"), &csrf_token).into_response());
    }
    sqlx::query("UPDATE users SET email=new_email,new_email=NULL WHERE id=$1")
        .bind(user.id)
        .execute(&state.pool)
        .await?;
    crate::audit::log_user_action(&state.pool, user.id, user.id, "accept_new_email", &[]).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&user.nick))).into_response())
}

fn render_activation_form(nick: &str, activation: &str, error: Option<&str>, csrf_token: &str) -> Html<String> {
    let error_html = error.map(|e| format!("<p class=\"error\">{}</p>", html_escape::encode_text(e))).unwrap_or_default();
    Html(format!(r#"
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
"#, nick = html_escape::encode_double_quoted_attribute(nick), activation = html_escape::encode_double_quoted_attribute(activation)))
}

fn verify_activation_code(state: &AppState, nick: &str, email: &str, regdate: Option<chrono::NaiveDateTime>, supplied: &str) -> bool {
    if state.config.enable_dev_bypasses && supplied == "dev-activate" {
        return true;
    }
    let Some(regdate) = regdate else { return false; };
    let payload = format!("{nick}:{email}:{}:activate", regdate.and_utc().timestamp_millis());
    let expected = crate::security::hmac_sha256_hex(&state.config.site_secret, &payload);
    crate::security::verify_hash(&expected, supplied)
}

pub async fn addphoto_form(CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    Ok(Html(format!(r#"
<h1>Загрузить userpic для {nick}</h1>
<form method="post" action="/addphoto.jsp" enctype="multipart/form-data" class="form">
  <label>Файл PNG/JPEG/WEBP, 50–300 px, до 100 KiB <input type="file" name="file" accept="image/png,image/jpeg,image/webp" required></label>
  <button type="submit">Загрузить</button>
</form>
"#, nick = html_escape::encode_text(&user.nick))))
}

pub async fn upload_userpic(State(state): State<AppState>, CurrentUser(user): CurrentUser, mut multipart: Multipart) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let mut upload: Option<(String, bytes::Bytes)> = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::BadRequest(format!("ошибка multipart: {e}")))? {
        let is_file = field.name() == Some("file");
        let filename = field.file_name().unwrap_or("userpic").to_string();
        let data = field.bytes().await.map_err(|e| AppError::BadRequest(format!("ошибка чтения файла: {e}")))?;
        if is_file {
            upload = Some((filename, data));
            break;
        }
    }
    let (_original_name, bytes) = upload.ok_or_else(|| AppError::BadRequest("изображение не задано".into()))?;
    let extension = validate_userpic_bytes(&bytes)?;
    let filename = format!("{}-{}.{}", user.id, uuid::Uuid::new_v4(), extension);
    let dir = format!("{}/photos", state.config.upload_dir);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| AppError::Anyhow(e.into()))?;
    let path = format!("{dir}/{filename}");
    tokio::fs::write(&path, &bytes).await.map_err(|e| AppError::Anyhow(e.into()))?;
    sqlx::query("UPDATE users SET photo=$2 WHERE id=$1")
        .bind(user.id)
        .bind(&filename)
        .execute(&state.pool)
        .await?;
    crate::audit::log_user_action(&state.pool, user.id, user.id, "set_userpic", &[("file", filename.as_str())]).await?;
    Ok(Redirect::to(&format!("/people/{}/profile?nocache={}", urlencoding::encode(&user.nick), uuid::Uuid::new_v4())))
}

fn validate_userpic_bytes(data: &[u8]) -> Result<&'static str> {
    const MAX_FILE_SIZE: usize = 100 * 1024;
    const MIN_IMAGE_SIZE: u32 = 50;
    const MAX_IMAGE_SIZE: u32 = 300;
    if data.is_empty() {
        return Err(AppError::BadRequest("изображение не задано".into()));
    }
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::BadRequest("Сбой загрузки изображения: слишком большой файл".into()));
    }
    let format = image::guess_format(data).map_err(|_| AppError::BadRequest("Сбой загрузки изображения: неизвестный формат".into()))?;
    let extension = match format {
        image::ImageFormat::Png => "png",
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::WebP => "webp",
        _ => return Err(AppError::BadRequest("Сбой загрузки изображения: неподдерживаемый или потенциально анимированный формат".into())),
    };
    let image = image::load_from_memory_with_format(data, format).map_err(|e| AppError::BadRequest(format!("Сбой загрузки изображения: {e}")))?;
    let (width, height) = image.dimensions();
    if width < MIN_IMAGE_SIZE || width > MAX_IMAGE_SIZE || height < MIN_IMAGE_SIZE || height > MAX_IMAGE_SIZE {
        return Err(AppError::BadRequest("Сбой загрузки изображения: недопустимые размеры фотографии".into()));
    }
    Ok(extension)
}

/// Image.MaxFileSize/MinDimension/MaxDimension (Java uses a 4-size srcset
/// instead - this port's `images` table has concrete original/medium/
/// thumbnail columns, so three fixed sizes are stored instead of Java's
/// dynamic srcset).
const GALLERY_MAX_FILE_SIZE: usize = 8 * 1024 * 1024;
const GALLERY_MIN_DIMENSION: u32 = 400;
const GALLERY_MAX_DIMENSION: u32 = 5120;
const GALLERY_SIZES: [(&str, u32); 3] = [("original", 2000), ("medium", 800), ("thumbnail", 200)];

#[derive(Deserialize)]
pub struct AddPhotoTopicQuery {
    pub msgid: i32,
}

/// Loads (author_id, section.imagepost) for a topic, or 404s. Both the GET
/// form and the POST handler need this same author/section check.
async fn topic_for_photo_upload(state: &AppState, msgid: i32) -> Result<(i32, bool, String, String)> {
    sqlx::query_as(
        r#"SELECT t.userid, s.imagepost,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END,
                  g.urlname
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
    )
    .bind(msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

/// No `csrf` hidden field, matching `addphoto_form`'s existing precedent:
/// a multipart POST body isn't inspected by the CSRF middleware (see
/// `src/csrf.rs`), so a token here would be decorative.
pub async fn addphoto_topic_form(State(state): State<AppState>, Query(q): Query<AddPhotoTopicQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let (author_id, imagepost, _section_prefix, _group_urlname) = topic_for_photo_upload(&state, q.msgid).await?;
    if !imagepost {
        return Err(AppError::BadRequest("этот раздел не поддерживает изображения".into()));
    }
    if !user.canmod && user.id != author_id {
        return Err(AppError::Forbidden);
    }
    Ok(Html(format!(r#"
<h1>Загрузить изображение</h1>
<form method="post" action="/addphoto-topic.jsp" enctype="multipart/form-data" class="form">
  <input type="hidden" name="msgid" value="{}">
  <label>Файл PNG/JPEG/WEBP, {}-{} px, до {} МиБ <input type="file" name="file" accept="image/png,image/jpeg,image/webp" required></label>
  <button type="submit">Загрузить</button>
</form>
"#, q.msgid, GALLERY_MIN_DIMENSION, GALLERY_MAX_DIMENSION, GALLERY_MAX_FILE_SIZE / 1024 / 1024)))
}

fn validate_gallery_image_bytes(data: &[u8]) -> Result<image::DynamicImage> {
    if data.is_empty() {
        return Err(AppError::BadRequest("изображение не задано".into()));
    }
    if data.len() > GALLERY_MAX_FILE_SIZE {
        return Err(AppError::BadRequest("Сбой загрузки изображения: слишком большой файл".into()));
    }
    let format = image::guess_format(data).map_err(|_| AppError::BadRequest("Сбой загрузки изображения: неизвестный формат".into()))?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP) {
        return Err(AppError::BadRequest("Сбой загрузки изображения: неподдерживаемый или потенциально анимированный формат".into()));
    }
    let img = image::load_from_memory_with_format(data, format).map_err(|e| AppError::BadRequest(format!("Сбой загрузки изображения: {e}")))?;
    let (width, height) = img.dimensions();
    if width < GALLERY_MIN_DIMENSION || height < GALLERY_MIN_DIMENSION {
        return Err(AppError::BadRequest("Сбой загрузки изображения: изображение слишком маленькое".into()));
    }
    if width > GALLERY_MAX_DIMENSION || height > GALLERY_MAX_DIMENSION {
        return Err(AppError::BadRequest("Сбой загрузки изображения: изображение слишком большое".into()));
    }
    Ok(img)
}

/// Never upscales - `resize` bounds by `max_side` on the longer dimension,
/// but an image already smaller than that is returned as-is.
fn resize_capped(img: &image::DynamicImage, max_side: u32) -> image::DynamicImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= max_side {
        img.clone()
    } else {
        img.resize(max_side, max_side, image::imageops::FilterType::Lanczos3)
    }
}

fn encode_jpeg(img: &image::DynamicImage) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Jpeg)
        .map_err(|e| AppError::Anyhow(e.into()))?;
    Ok(buf)
}

/// ImageService.saveImage: only the single "main" gallery image is
/// supported (Java also allows several additional images per topic - not
/// implemented here). Sets `topics.image` only if it isn't already set, so
/// re-uploading doesn't silently orphan the previous main image's row.
pub async fn upload_topic_photo(State(state): State<AppState>, CurrentUser(user): CurrentUser, mut multipart: Multipart) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let mut msgid: Option<i32> = None;
    let mut upload: Option<bytes::Bytes> = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::BadRequest(format!("ошибка multipart: {e}")))? {
        match field.name() {
            Some("msgid") => {
                let text = field.text().await.map_err(|e| AppError::BadRequest(format!("ошибка чтения msgid: {e}")))?;
                msgid = text.trim().parse().ok();
            }
            Some("file") => {
                upload = Some(field.bytes().await.map_err(|e| AppError::BadRequest(format!("ошибка чтения файла: {e}")))?);
            }
            _ => {}
        }
    }
    let msgid = msgid.ok_or_else(|| AppError::BadRequest("missing msgid".into()))?;
    let bytes = upload.ok_or_else(|| AppError::BadRequest("изображение не задано".into()))?;

    let (author_id, imagepost, section_prefix, group_urlname) = topic_for_photo_upload(&state, msgid).await?;
    if !imagepost {
        return Err(AppError::BadRequest("этот раздел не поддерживает изображения".into()));
    }
    if !user.canmod && user.id != author_id {
        return Err(AppError::Forbidden);
    }

    let img = validate_gallery_image_bytes(&bytes)?;
    let (width, height) = img.dimensions();

    let image_id: i32 = sqlx::query_scalar("SELECT nextval('images_id_seq')::int").fetch_one(&state.pool).await?;
    let dir = format!("{}/gallery/{image_id}", state.config.upload_dir);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| AppError::Anyhow(e.into()))?;
    for (name, max_side) in GALLERY_SIZES {
        let resized = resize_capped(&img, max_side);
        let jpeg = encode_jpeg(&resized)?;
        tokio::fs::write(format!("{dir}/{name}.jpg"), &jpeg).await.map_err(|e| AppError::Anyhow(e.into()))?;
    }

    sqlx::query(
        "INSERT INTO images(id, userid, topic, original, medium, thumbnail, width, height, primary_image) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,true)",
    )
    .bind(image_id)
    .bind(user.id)
    .bind(msgid)
    .bind(format!("{image_id}/original.jpg"))
    .bind(format!("{image_id}/medium.jpg"))
    .bind(format!("{image_id}/thumbnail.jpg"))
    .bind(width as i32)
    .bind(height as i32)
    .execute(&state.pool)
    .await?;

    sqlx::query("UPDATE topics SET image=$1, lastmod=now() WHERE id=$2 AND image IS NULL")
        .bind(image_id)
        .bind(msgid)
        .execute(&state.pool)
        .await?;

    Ok(Redirect::to(&format!("/{section_prefix}/{group_urlname}/{msgid}")))
}

#[derive(Deserialize)]
pub struct DeregisterForm {
    pub password: String,
    pub accept_block: Option<String>,
    pub acceptBlock: Option<String>,
    pub accept_oneway: Option<String>,
    pub acceptOneway: Option<String>,
}

pub async fn deregister_form(State(state): State<AppState>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    ensure_deregister_allowed(&state, &user).await?;
    Ok(Html(format!(r#"
<h1>Удаление аккаунта {nick}</h1>
<p>Операция соответствует исходной логике: аккаунт блокируется, профиль очищается, восстановление через эту форму не предусмотрено.</p>
<form method="post" action="/deregister.jsp" class="form">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <label>Пароль <input name="password" type="password" required></label>
  <label><input type="checkbox" name="acceptBlock" value="true" required> Я согласен с блокировкой аккаунта</label>
  <label><input type="checkbox" name="acceptOneway" value="true" required> Я понимаю, что действие необратимо</label>
  <button type="submit">Удалить аккаунт</button>
</form>
"#, nick = html_escape::encode_text(&user.nick))))
}

pub async fn deregister_post(State(state): State<AppState>, jar: CookieJar, CurrentUser(user): CurrentUser, Form(form): Form<DeregisterForm>) -> Result<impl IntoResponse> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    ensure_deregister_allowed(&state, &user).await?;
    if form.accept_block.or(form.acceptBlock).is_none() {
        return Err(AppError::BadRequest("Вы не согласились с блокировкой аккаунта".into()));
    }
    if form.accept_oneway.or(form.acceptOneway).is_none() {
        return Err(AppError::BadRequest("Вы не согласились с невозможностью восстановления аккаунта".into()));
    }
    let ok = matches!(crate::auth::verify_login(&state.pool, &user.nick, &form.password).await?, crate::auth::LoginOutcome::Success(_));
    if !ok {
        return Err(AppError::BadRequest("Неверный пароль".into()));
    }
    sqlx::query(
        "UPDATE users SET name='', url='', town='', userinfo='', photo=NULL, blocked=true WHERE id=$1",
    )
    .bind(user.id)
    .execute(&state.pool)
    .await?;
    crate::audit::log_user_action(&state.pool, user.id, user.id, "block_user", &[("reason", "deregister")]).await?;
    Ok((jar.remove(Cookie::from("lor_session")), Html("<h1>Удаление пользователя прошло успешно.</h1>".to_string())).into_response())
}

async fn ensure_deregister_allowed(state: &AppState, user: &crate::models::UserSummary) -> Result<()> {
    if user.max_score.unwrap_or(0) < 100 {
        return Err(AppError::Forbidden);
    }
    if user.canmod {
        return Err(AppError::Forbidden);
    }
    if user.blocked.unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_optional(&state.pool)
        .await?
        .flatten();
    if frozen_until.map(|u| u > chrono::Utc::now()).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

async fn user_exists_or_similar(state: &AppState, nick: &str) -> Result<bool> {
    let exists: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE lower(nick)=lower($1)")
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
    Path((group, id_or_year, page_or_month)): Path<(String, String, String)>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<axum::response::Response> {
    if let Some(page) = page_or_month.strip_prefix("page") {
        let page: i64 = page.parse().map_err(|_| AppError::NotFound)?;
        let id: i32 = id_or_year.parse().map_err(|_| AppError::NotFound)?;
        return crate::routes::topics::render_topic_page(state, "forum", group, id, page, current_user, csrf_token).await;
    }

    let year: i32 = id_or_year.parse().map_err(|_| AppError::NotFound)?;
    let month: i32 = page_or_month.parse().map_err(|_| AppError::NotFound)?;
    Ok(forum_archive_month(State(state), Path((group, year, month)), Query(q), CurrentUser(current_user)).await?.into_response())
}

fn validate_year_month(year: i32, month: i32) -> Result<()> {
    if !(1990..=3000).contains(&year) { return Err(AppError::BadRequest("указан некорректный год".into())); }
    if !(1..=12).contains(&month) { return Err(AppError::BadRequest("указан некорректный месяц".into())); }
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
    pub add: Option<String>,
    pub remove: Option<String>,
}

/// MemoriesController.add/remove: "favorite" (watch=false) and "watch"
/// (watch=true) are independent rows per topic - `add` upserts the row for
/// the requested `watch` value only, `remove` deletes one specific row by
/// its own id (never the whole userid+topic pair), matching the frontend
/// contract in `static/js/lor/memories.js` (`{msgid,watch}` to add,
/// `{id}` to remove, JSON `{id,count}`/bare count responses).
pub async fn memories(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<MemoryForm>) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };

    if form.remove.is_some() {
        let Some(id) = form.id else { return Err(AppError::BadRequest("missing id".into())); };
        let row: Option<(i32, i32, bool)> = sqlx::query_as("SELECT userid, topic, watch FROM memories WHERE id=$1").bind(id).fetch_optional(&state.pool).await?;
        let Some((owner_id, topic_id, watch)) = row else {
            return Ok(Json(serde_json::json!(-1)));
        };
        if owner_id != user.id {
            return Err(AppError::Forbidden);
        }
        sqlx::query("DELETE FROM memories WHERE id=$1").bind(id).execute(&state.pool).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM memories WHERE topic=$1 AND watch=$2").bind(topic_id).bind(watch).fetch_one(&state.pool).await?;
        return Ok(Json(serde_json::json!(count)));
    }

    let msgid = form.msgid.ok_or_else(|| AppError::BadRequest("missing msgid".into()))?;
    let watch = form.watch.unwrap_or(false);
    let deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1").bind(msgid).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    if deleted {
        return Err(AppError::BadRequest("Тема удалена".into()));
    }
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO memories(userid,topic,watch) VALUES($1,$2,$3) ON CONFLICT(userid,topic,watch) DO UPDATE SET topic=EXCLUDED.topic RETURNING id",
    )
    .bind(user.id).bind(msgid).bind(watch)
    .fetch_one(&state.pool)
    .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM memories WHERE topic=$1 AND watch=$2").bind(msgid).bind(watch).fetch_one(&state.pool).await?;
    Ok(Json(serde_json::json!({"id": id, "count": count})))
}

pub async fn user_filter(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let tags = sqlx::query_as::<_, (String, bool)>(
        "SELECT tv.value, ut.is_favorite FROM user_tags ut JOIN tags_values tv ON tv.id=ut.tag_id WHERE ut.userid=$1 ORDER BY tv.value",
    ).bind(user.id).fetch_all(&state.pool).await?;
    let ignored = sqlx::query_as::<_, (String,)>(
        "SELECT u.nick FROM ignore_list il JOIN users u ON u.id=il.ignored WHERE il.userid=$1 ORDER BY u.nick",
    ).bind(user.id).fetch_all(&state.pool).await?;
    Ok(Json(json!({"tags": tags.into_iter().map(|(tag, favorite)| json!({"tag": tag, "favorite": favorite})).collect::<Vec<_>>(), "ignoredUsers": ignored.into_iter().map(|(nick,)| nick).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
pub struct UserTagForm {
    pub tag: Option<String>,
    #[serde(rename = "tagName")]
    pub tag_name: Option<String>,
    pub add: Option<String>,
    pub del: Option<String>,
}

pub async fn favorite_tag(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<UserTagForm>) -> Result<Json<serde_json::Value>> {
    save_or_delete_user_tag(state, user, form, true).await
}

pub async fn ignore_tag(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<UserTagForm>) -> Result<Json<serde_json::Value>> {
    if user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    save_or_delete_user_tag(state, user, form, false).await
}

async fn save_or_delete_user_tag(state: AppState, user: Option<crate::models::UserSummary>, form: UserTagForm, is_favorite: bool) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let tag = form.tag_name.or(form.tag).unwrap_or_default().trim().to_string();
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
        sqlx::query("DELETE FROM user_tags WHERE userid=$1 AND tag_id=$2 AND is_favorite=$3")
            .bind(user.id)
            .bind(tag_id)
            .bind(is_favorite)
            .execute(&state.pool)
            .await?;
    } else {
        sqlx::query("INSERT INTO user_tags(userid,tag_id,is_favorite) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(user.id)
            .bind(tag_id)
            .bind(is_favorite)
            .execute(&state.pool)
            .await?;
    }

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_tags WHERE tag_id=$1 AND is_favorite=$2")
        .bind(tag_id)
        .bind(is_favorite)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!({"count": count, "tag": tag, "favorite": is_favorite})))
}

#[derive(Deserialize)]
pub struct IgnoreUserForm {
    pub id: Option<i32>,
    pub nick: Option<String>,
    pub add: Option<String>,
    pub del: Option<String>,
}

pub async fn ignore_user(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<IgnoreUserForm>) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
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
        return Err(AppError::BadRequest("нельзя игнорировать самого себя".into()));
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
    Ok(Json(json!({"ok": true, "ignored": ignored_id, "deleted": form.del.is_some()})))
}

#[derive(Deserialize)]
pub struct LegacyMsgIdQuery { pub msgid: i32 }

#[derive(Deserialize)]
pub struct ScoreForm { pub msgid: i32, pub score: Option<i32>, pub postscore: Option<i32> }

pub async fn set_post_score_form(Query(q): Query<LegacyMsgIdQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Изменить score темы #{}</h1>
<form method="post" action="/setpostscore.jsp">
<input type="hidden" name="csrf" value="{csrf_token}">
<input type="hidden" name="msgid" value="{}">
<input name="score" type="number" value="0">
<button type="submit">Сохранить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn set_post_score(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<ScoreForm>) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    let score = form.score.or(form.postscore).unwrap_or(0);
    sqlx::query("UPDATE topics SET postscore=$2,lastmod=now() WHERE id=$1").bind(form.msgid).bind(score).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

#[derive(Deserialize)]
pub struct ImageForm { pub id: i32 }

pub async fn delete_image_form(Query(q): Query<ImageForm>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Удалить изображение #{}</h1>
<form method="post" action="/delete_image"><input type="hidden" name="csrf" value="{csrf_token}"><input type="hidden" name="id" value="{}"><button type="submit">Удалить</button></form>
"#, q.id, q.id)))
}

pub async fn delete_image(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<ImageForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let Some((topic_id, author_id, is_main, group_urlname)): Option<(i32, i32, bool, String)> = sqlx::query_as(
        r#"SELECT i.topic, t.userid, (t.image = i.id), g.urlname
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
    sqlx::query("UPDATE images SET deleted=true WHERE id=$1").bind(form.id).execute(&state.pool).await?;
    sqlx::query("UPDATE topics SET lastmod=now() WHERE id=$1").bind(topic_id).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/gallery/{}/{}", urlencoding::encode(&group_urlname), topic_id)))
}

#[derive(Deserialize)]
pub struct RemoveUserpicForm { pub id: Option<i32> }

pub async fn remove_userpic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<RemoveUserpicForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let target_id = form.id.unwrap_or(user.id);
    if target_id != user.id && !user.canmod {
        return Err(AppError::Forbidden);
    }
    let target_nick: String = sqlx::query_scalar("SELECT nick FROM users WHERE id=$1")
        .bind(target_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    sqlx::query("UPDATE users SET photo=NULL WHERE id=$1").bind(target_id).execute(&state.pool).await?;
    crate::audit::log_user_action(&state.pool, target_id, user.id, "reset_userpic", &[]).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&target_nick))))
}

pub async fn reset_password_form(crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    Ok(Html(format!(r#"
<h1>Сбросить пароль</h1>
<form method="post" action="/reset-password" class="form">
<input type="hidden" name="csrf" value="{csrf_token}">
<label>Ник <input name="nick" required></label>
<label>Код из письма <input name="code" required></label>
<button type="submit">Сбросить пароль</button>
</form>
"#)))
}

