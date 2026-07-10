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
    extract::{Path, Query, State},
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect},
    Form, Json,
};
use serde::Deserialize;
use serde_json::json;

/// Route-level compatibility placeholder.
///
/// It makes unported legacy URLs explicit in the Rust router, so coverage and
/// HTTP compatibility tests can distinguish "route is known but behaviour is
/// pending" from accidental 404s.
pub async fn not_implemented() -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Html("Legacy endpoint is mapped but the business logic has not been ported yet."))
}

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

pub async fn markup_preview(Form(form): Form<PreviewForm>) -> Json<serde_json::Value> {
    let text = form.text.or(form.msg).or(form.message).unwrap_or_default();
    if text.len() > 65_536 {
        return Json(json!({"error": "Слишком длинный текст"}));
    }
    let html = markup::render_message(&text, Some(form.markup.as_deref().unwrap_or("lorcode") != "plain"));
    Json(json!({"html": html}))
}

pub async fn check_login(CurrentUser(user): CurrentUser) -> Json<serde_json::Value> {
    Json(match user {
        Some(user) => json!({"loggedIn": true, "id": user.id, "nick": user.nick, "moderator": user.canmod}),
        None => json!({"loggedIn": false}),
    })
}

pub async fn yandex_tableau(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "version": 1,
        "api_version": 1,
        "layout": {"logo": format!("{}/static/app.css", state.config.public_url), "color": "#385e8e", "show_title": true},
    }))
}

pub async fn help_page(Path(page): Path<String>) -> Result<Html<String>> {
    let title = html_escape::encode_text(&page.replace('-', " "));
    Ok(Html(format!(
        "<h1>Справка: {title}</h1><p>Страница справки сохранена как legacy-compatible endpoint. Контент можно перенести из старых JSP/Markdown-ресурсов отдельной итерацией.</p>"
    )))
}

pub async fn archive_section(State(state): State<AppState>, uri: Uri, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    render_archive(state, Some(section), None, None, None, q, current_user).await
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

pub async fn topic_thread_redirect(uri: Uri, Path((group, id, thread_root)): Path<(String, i32, i32)>) -> Redirect {
    let section = section_from_uri(&uri).unwrap_or("forum");
    Redirect::to(&format!("/{section}/{group}/{id}#comment-{thread_root}"))
}

pub async fn topic_history(State(state): State<AppState>, uri: Uri, Path((_group, id)): Path<(String, i32)>) -> Result<Html<String>> {
    render_history(&state, section_from_uri(&uri).unwrap_or("forum"), id, None).await
}

pub async fn comment_history(State(state): State<AppState>, uri: Uri, Path((_group, _id, commentid)): Path<(String, i32, i32)>) -> Result<Html<String>> {
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
pub struct ShowRepliesQuery { pub nick: Option<String>, pub output: Option<String> }

pub async fn show_replies_jsp(CurrentUser(user): CurrentUser, Query(q): Query<ShowRepliesQuery>) -> impl IntoResponse {
    if q.output.is_some() {
        return Json(json!({"items": [], "nick": q.nick.or_else(|| user.as_ref().map(|u| u.nick.clone()))})).into_response();
    }
    Redirect::to("/notifications").into_response()
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

pub async fn notifications_click() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

fn validate_year_month(year: i32, month: i32) -> Result<()> {
    if !(1990..=3000).contains(&year) { return Err(AppError::BadRequest("указан некорректный год".into())); }
    if !(1..=12).contains(&month) { return Err(AppError::BadRequest("указан некорректный месяц".into())); }
    Ok(())
}

fn section_from_uri(uri: &Uri) -> Option<&'static str> {
    uri.path().trim_start_matches('/').split('/').next().and_then(|s| match s {
        "forum" | "news" | "articles" | "gallery" | "polls" => Some(s),
        _ => None,
    })
}

#[derive(Deserialize)]
pub struct MemoryForm {
    pub topic: i32,
    pub watch: Option<bool>,
    pub notify: Option<bool>,
    pub action: Option<String>,
}

pub async fn memories(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<MemoryForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if form.action.as_deref() == Some("delete") {
        sqlx::query("DELETE FROM memories WHERE userid=$1 AND topic=$2").bind(user.id).bind(form.topic).execute(&state.pool).await?;
    } else {
        sqlx::query(
            "INSERT INTO memories(userid,topic,watch,notify) VALUES($1,$2,$3,$4) ON CONFLICT(userid,topic) DO UPDATE SET watch=EXCLUDED.watch, notify=EXCLUDED.notify",
        )
        .bind(user.id).bind(form.topic).bind(form.watch.unwrap_or(false)).bind(form.notify.unwrap_or(false)).execute(&state.pool).await?;
    }
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.topic)))
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
pub struct UserTagForm { pub tag: String }

pub async fn favorite_tag(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<UserTagForm>) -> Result<Json<serde_json::Value>> {
    save_user_tag(state, user, form.tag, true).await
}

pub async fn ignore_tag(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<UserTagForm>) -> Result<Json<serde_json::Value>> {
    save_user_tag(state, user, form.tag, false).await
}

async fn save_user_tag(state: AppState, user: Option<crate::models::UserSummary>, tag: String, is_favorite: bool) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let tag_id: i32 = sqlx::query_scalar(
        "INSERT INTO tags_values(value,counter) VALUES($1,0) ON CONFLICT(value) DO UPDATE SET value=EXCLUDED.value RETURNING id",
    ).bind(tag.trim()).fetch_one(&state.pool).await?;
    sqlx::query("INSERT INTO user_tags(userid,tag_id,is_favorite) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(user.id).bind(tag_id).bind(is_favorite).execute(&state.pool).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct IgnoreUserForm { pub nick: String }

pub async fn ignore_user(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<IgnoreUserForm>) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let ignored_id: i32 = sqlx::query_scalar("SELECT id FROM users WHERE lower(nick)=lower($1)").bind(form.nick.trim()).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    sqlx::query("INSERT INTO ignore_list(userid,ignored) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(user.id).bind(ignored_id).execute(&state.pool).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct LegacyMsgIdQuery { pub msgid: i32 }

#[derive(Deserialize)]
pub struct ScoreForm { pub msgid: i32, pub score: Option<i32>, pub postscore: Option<i32> }

pub async fn set_post_score_form(Query(q): Query<LegacyMsgIdQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Изменить score темы #{}</h1>
<form method="post" action="/setpostscore.jsp">
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

pub async fn delete_image_form(Query(q): Query<ImageForm>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Удалить изображение #{}</h1>
<form method="post" action="/delete_image"><input type="hidden" name="id" value="{}"><button type="submit">Удалить</button></form>
"#, q.id, q.id)))
}

pub async fn delete_image(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<ImageForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    sqlx::query("UPDATE images SET deleted=true WHERE id=$1 AND (userid=$2 OR EXISTS (SELECT 1 FROM users WHERE id=$2 AND canmod))")
        .bind(form.id).bind(user.id).execute(&state.pool).await?;
    Ok(Redirect::to("/gallery/"))
}

pub async fn remove_userpic(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    sqlx::query("UPDATE users SET photo=NULL WHERE id=$1").bind(user.id).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&user.nick))))
}

pub async fn reset_password_form(CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    Ok(Html(r#"
<h1>Сбросить пароль</h1>
<form method="post" action="/reset-password" class="form">
<label>Ник <input name="nick" required></label>
<label>Новый пароль <input name="passwd" type="password" required minlength="6"></label>
<button type="submit">Сохранить</button>
</form>
"#.to_string()))
}

#[derive(Deserialize)]
pub struct ResetPasswordForm { pub nick: String, pub passwd: String }

pub async fn reset_password(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<ResetPasswordForm>) -> Result<Redirect> {
    let Some(current) = user else { return Err(AppError::Forbidden); };
    let target: (i32, String) = sqlx::query_as("SELECT id,nick FROM users WHERE lower(nick)=lower($1)")
        .bind(form.nick.trim()).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    if current.id != target.0 && !current.canmod { return Err(AppError::Forbidden); }
    let hash = crate::security::password::hash(&form.passwd).map_err(|e| AppError::Anyhow(e.into()))?;
    sqlx::query("UPDATE users SET passwd=$2 WHERE id=$1").bind(target.0).bind(hash).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&target.1))))
}
