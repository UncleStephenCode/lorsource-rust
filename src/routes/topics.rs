use crate::{auth::CurrentUser, error::{AppError, Result}, markup, models::{PagerQuery, TopicDetail, TopicSummary}, pagination::Pager, state::AppState};
use askama::Template;
use axum::{extract::{Path, Query, State}, http::Uri, response::{Html, Redirect}, Form};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    pager: Pager,
    current_user: Option<crate::models::UserSummary>,
}

#[derive(Template)]
#[template(path = "topic.html")]
struct TopicTemplate {
    topic: TopicDetail,
    topic_html: String,
    comments: Vec<CommentView>,
    current_user: Option<crate::models::UserSummary>,
}

#[derive(Debug, Clone)]
struct CommentView {
    item: crate::models::CommentItem,
    html: String,
}

#[derive(Template)]
#[template(path = "topic_form.html")]
struct TopicFormTemplate {
    title: String,
    action: String,
    topic: Option<TopicDetail>,
    groups: Vec<crate::models::Group>,
}

#[derive(Deserialize)]
pub struct TopicForm {
    pub id: Option<i32>,
    pub group: i32,
    pub title: String,
    pub msg: String,
    pub url: Option<String>,
    pub linktext: Option<String>,
    pub tags: Option<String>,
}

pub async fn index(State(state): State<AppState>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, None, None, pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: "Последние темы".into(), topics, pager, current_user }.render()?))
}

pub async fn lenta(State(state): State<AppState>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some("forum"), None, pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: "Форум / лента".into(), topics, pager, current_user }.render()?))
}

pub async fn section_topics(State(state): State<AppState>, uri: Uri, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some(section), None, pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: section_title(section).to_string(), topics, pager, current_user }.render()?))
}

pub async fn section_group_topics(State(state): State<AppState>, uri: Uri, Path(group): Path<String>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, Some(section), Some(&group), pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: format!("{} / {}", section_title(section), group), topics, pager, current_user }.render()?))
}

pub async fn legacy_show_topics(State(state): State<AppState>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_topics(&state, None, None, pager.offset, pager.limit).await?;
    Ok(Html(IndexTemplate { title: "show-topics.jsp".into(), topics, pager, current_user }.render()?))
}

#[derive(Deserialize)]
pub struct ViewMessageQuery { msgid: i32 }

pub async fn legacy_view_message(Query(q): Query<ViewMessageQuery>) -> Redirect {
    Redirect::to(&format!("/jump-message.jsp?msgid={}", q.msgid))
}

pub async fn topic_page(State(state): State<AppState>, Path((_group, id)): Path<(String, i32)>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    render_topic(state, id, current_user).await
}

pub async fn topic_page_with_page(State(state): State<AppState>, Path((_group, id, _page)): Path<(String, i32, i64)>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    render_topic(state, id, current_user).await
}

async fn render_topic(state: AppState, id: i32, current_user: Option<crate::models::UserSummary>) -> Result<Html<String>> {
    let topic = get_topic(&state, id).await?;
    let topic_html = markup::render_message(&topic.message, topic.bbcode);
    let items = sqlx::query_as::<_, crate::models::CommentItem>(
        r#"SELECT c.id, c.topic, c.replyto, c.title, m.message, c.postdate, u.id AS author_id, u.nick AS author, c.deleted
           FROM comments c
           JOIN msgbase m ON m.id=c.id
           JOIN users u ON u.id=c.userid
           WHERE c.topic=$1 AND NOT c.topic_deleted
           ORDER BY c.postdate ASC"#,
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let comments = items.into_iter().map(|item| CommentView { html: markup::render_message(&item.message, Some(true)), item }).collect();
    Ok(Html(TopicTemplate { topic, topic_html, comments, current_user }.render()?))
}

pub async fn new_topic_form(State(state): State<AppState>) -> Result<Html<String>> {
    let groups = crate::routes::groups::list_groups(&state).await?;
    Ok(Html(TopicFormTemplate { title: "Новая тема".into(), action: "/add.jsp".into(), topic: None, groups }.render()?))
}

pub async fn create_topic(State(state): State<AppState>, Form(form): Form<TopicForm>) -> Result<Redirect> {
    let mut tx = state.pool.begin().await?;
    let id: i32 = sqlx::query_scalar("SELECT nextval('s_msgid')::int").fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO msgbase(id, message, bbcode) VALUES ($1, $2, true)")
        .bind(id)
        .bind(&form.msg)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"INSERT INTO topics(id, groupid, userid, title, url, postdate, linktext, stat1, stat2, lastmod)
           VALUES ($1,$2,1,$3,$4,now(),$5,0,0,now())"#,
    )
    .bind(id)
    .bind(form.group)
    .bind(form.title.trim())
    .bind(form.url.as_deref().filter(|s| !s.trim().is_empty()))
    .bind(form.linktext.as_deref().filter(|s| !s.trim().is_empty()))
    .execute(&mut *tx)
    .await?;
    if let Some(tags) = form.tags.as_deref() { upsert_tags(&mut tx, id, tags).await?; }
    tx.commit().await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")))
}

pub async fn edit_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>) -> Result<Html<String>> {
    let topic = get_topic(&state, q.msgid).await?;
    let groups = crate::routes::groups::list_groups(&state).await?;
    Ok(Html(TopicFormTemplate { title: "Редактировать тему".into(), action: "/edit.jsp".into(), topic: Some(topic), groups }.render()?))
}

pub async fn edit_topic(State(state): State<AppState>, Form(form): Form<TopicForm>) -> Result<Redirect> {
    let id = form.id.ok_or_else(|| AppError::BadRequest("missing topic id".into()))?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE msgbase SET message=$2 WHERE id=$1").bind(id).bind(&form.msg).execute(&mut *tx).await?;
    sqlx::query("UPDATE topics SET title=$2, url=$3, linktext=$4, lastmod=now() WHERE id=$1")
        .bind(id).bind(form.title.trim()).bind(form.url).bind(form.linktext).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM tags WHERE msgid=$1").bind(id).execute(&mut *tx).await?;
    if let Some(tags) = form.tags.as_deref() { upsert_tags(&mut tx, id, tags).await?; }
    tx.commit().await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")))
}

#[derive(Deserialize)]
pub struct TopicActionForm { pub msgid: i32 }

pub async fn delete_topic(State(state): State<AppState>, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    sqlx::query("UPDATE topics SET deleted=true WHERE id=$1").bind(form.msgid).execute(&state.pool).await?;
    Ok(Redirect::to("/"))
}

pub async fn undelete_topic(State(state): State<AppState>, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    sqlx::query("UPDATE topics SET deleted=false WHERE id=$1").bind(form.msgid).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn resolve_topic(State(state): State<AppState>, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    sqlx::query("UPDATE topics SET resolved=COALESCE(NOT resolved, true) WHERE id=$1").bind(form.msgid).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn list_topics(state: &AppState, section: Option<&str>, group: Option<&str>, offset: i64, limit: i64) -> Result<Vec<TopicSummary>> {
    let rows = sqlx::query_as::<_, TopicSummary>(
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
             AND NOT t.deleted
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY t.sticky DESC, COALESCE(t.lastmod,t.postdate) DESC
           OFFSET $3 LIMIT $4"#,
    )
    .bind(section)
    .bind(group)
    .bind(offset)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows)
}

pub async fn get_topic(state: &AppState, id: i32) -> Result<TopicDetail> {
    Ok(sqlx::query_as::<_, TopicDetail>(
        r#"SELECT t.id, t.title, m.message, m.bbcode, t.url, t.linktext, t.postdate, t.lastmod,
                  u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                  string_agg(tv.value, ',' ORDER BY tv.value) AS tags
           FROM topics t
           JOIN msgbase m ON m.id=t.id
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN tags tg ON tg.msgid=t.id
           LEFT JOIN tags_values tv ON tv.id=tg.tagid
           WHERE t.id=$1
           GROUP BY t.id,m.id,u.id,g.id,s.id"#,
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?)
}

async fn upsert_tags(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, msgid: i32, tags: &str) -> Result<()> {
    for tag in tags.split(',').map(str::trim).filter(|t| !t.is_empty()).take(20) {
        let tagid: i32 = sqlx::query_scalar(
            r#"INSERT INTO tags_values(value,counter) VALUES ($1,1)
               ON CONFLICT(value) DO UPDATE SET counter=tags_values.counter+1
               RETURNING id"#,
        )
        .bind(tag)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query("INSERT INTO tags(msgid, tagid) VALUES ($1,$2) ON CONFLICT DO NOTHING")
            .bind(msgid)
            .bind(tagid)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

fn section_from_uri(uri: &Uri) -> Option<&'static str> {
    uri.path().trim_start_matches('/').split('/').next().and_then(|s| match s {
        "forum" | "news" | "articles" | "gallery" | "polls" => Some(s),
        _ => None,
    })
}

fn section_title(section: &str) -> &'static str {
    match section {
        "forum" => "Форум",
        "news" => "Новости",
        "articles" => "Статьи",
        "gallery" => "Галерея",
        "polls" => "Опросы",
        _ => "Темы",
    }
}

pub async fn delete_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Удалить тему #{}</h1>
<form method="post" action="/delete.jsp">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Удалить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn undelete_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Восстановить тему #{}</h1>
<form method="post" action="/undelete">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Восстановить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn commit_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Подтвердить тему #{}</h1>
<form method="post" action="/commit.jsp">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Подтвердить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn commit_topic(State(state): State<AppState>, Form(form): Form<TopicActionForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if !user.canmod { return Err(AppError::Forbidden); }
    sqlx::query("UPDATE topics SET moderate=false, commitby=$2, commitdate=now(), lastmod=now() WHERE id=$1")
        .bind(form.msgid).bind(user.id).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn uncommit_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Отменить подтверждение темы #{}</h1>
<form method="post" action="/uncommit.jsp">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Отменить подтверждение</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn uncommit(State(state): State<AppState>, Form(form): Form<TopicActionForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    sqlx::query("UPDATE topics SET moderate=true, commitby=NULL, commitdate=NULL, lastmod=now() WHERE id=$1")
        .bind(form.msgid).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

#[derive(Deserialize)]
pub struct MoveTopicForm { pub msgid: i32, pub moveto: i32 }

pub async fn move_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    let topic = get_topic(&state, q.msgid).await?;
    let groups = crate::routes::groups::list_groups(&state).await?;
    let mut options = String::new();
    for g in groups {
        let selected = if g.id == topic.group_id { " selected" } else { "" };
        options.push_str(&format!("<option value=\"{}\"{}>{} / {}</option>", g.id, selected, html_escape::encode_text(&g.section_name), html_escape::encode_text(&g.title)));
    }
    Ok(Html(format!(r#"
<h1>Переместить тему #{}</h1>
<form method="post" action="/mt.jsp">
  <input type="hidden" name="msgid" value="{}">
  <select name="moveto">{}</select>
  <button type="submit">Переместить</button>
</form>
"#, q.msgid, q.msgid, options)))
}

pub async fn move_topic(State(state): State<AppState>, Form(form): Form<MoveTopicForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    sqlx::query("UPDATE topics SET groupid=$2,lastmod=now() WHERE id=$1")
        .bind(form.msgid).bind(form.moveto).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn premoderated_move_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, user: CurrentUser) -> Result<Html<String>> {
    move_topic_form(State(state), Query(q), user).await
}
