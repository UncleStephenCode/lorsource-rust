use crate::{error::Result, models::{PagerQuery, TagItem, TopicSummary}, pagination::Pager, state::AppState};
use askama::Template;
use axum::{extract::{Path, Query, State}, response::Html};

#[derive(Template)]
#[template(path = "tags.html")]
struct TagsTemplate {
    title: String,
    tags: Vec<TagItem>,
}

#[derive(Template)]
#[template(path = "index.html")]
struct TagTopicsTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    pager: Pager,
    current_user: Option<crate::models::UserSummary>,
}

pub async fn all_tags(State(state): State<AppState>) -> Result<Html<String>> {
    let tags = sqlx::query_as::<_, TagItem>("SELECT value,counter FROM tags_values ORDER BY lower(value) LIMIT 1000")
        .fetch_all(&state.pool).await?;
    Ok(Html(TagsTemplate { title: "Метки".into(), tags }.render()?))
}

pub async fn tags_by_letter(State(state): State<AppState>, Path(first_letter): Path<String>) -> Result<Html<String>> {
    let prefix = format!("{}%", first_letter);
    let tags = sqlx::query_as::<_, TagItem>("SELECT value,counter FROM tags_values WHERE lower(value) LIKE lower($1) ORDER BY lower(value) LIMIT 1000")
        .bind(prefix).fetch_all(&state.pool).await?;
    Ok(Html(TagsTemplate { title: format!("Метки: {first_letter}"), tags }.render()?))
}

pub async fn tag_page(State(state): State<AppState>, Path(tag): Path<String>, Query(q): Query<PagerQuery>, current_user: crate::auth::CurrentUser) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = sqlx::query_as::<_, TopicSummary>(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                  string_agg(tv2.value, ',' ORDER BY tv2.value) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           JOIN tags tg ON tg.msgid=t.id
           JOIN tags_values tv ON tv.id=tg.tagid
           LEFT JOIN tags tg2 ON tg2.msgid=t.id
           LEFT JOIN tags_values tv2 ON tv2.id=tg2.tagid
           WHERE lower(tv.value)=lower($1) AND NOT t.deleted
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY t.postdate DESC OFFSET $2 LIMIT $3"#,
    )
    .bind(&tag).bind(pager.offset).bind(pager.limit).fetch_all(&state.pool).await?;
    Ok(Html(TagTopicsTemplate { title: format!("Метка: {tag}"), topics, pager, current_user: current_user.0 }.render()?))
}

#[derive(serde::Deserialize)]
pub struct TagRenameForm { pub old: String, pub new: String }

pub async fn change_form(crate::auth::CurrentUser(user): crate::auth::CurrentUser) -> crate::error::Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(crate::error::AppError::Forbidden); }
    Ok(Html(r#"
<h1>Переименовать метку</h1>
<form method="post" action="/tags/change" class="form">
<label>Старая <input name="old" required></label>
<label>Новая <input name="new" required></label>
<button type="submit">Переименовать</button>
</form>
"#.to_string()))
}

pub async fn change_tag(State(state): State<AppState>, crate::auth::CurrentUser(user): crate::auth::CurrentUser, axum::Form(form): axum::Form<TagRenameForm>) -> crate::error::Result<axum::response::Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(crate::error::AppError::Forbidden); }
    sqlx::query("UPDATE tags_values SET value=$2 WHERE lower(value)=lower($1)")
        .bind(form.old.trim())
        .bind(form.new.trim())
        .execute(&state.pool)
        .await?;
    Ok(axum::response::Redirect::to(&format!("/tag/{}", urlencoding::encode(form.new.trim()))))
}

#[derive(serde::Deserialize)]
pub struct TagDeleteForm { pub tag: String }

pub async fn delete_form(crate::auth::CurrentUser(user): crate::auth::CurrentUser) -> crate::error::Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(crate::error::AppError::Forbidden); }
    Ok(Html(r#"
<h1>Удалить метку</h1>
<form method="post" action="/tags/delete" class="form">
<label>Метка <input name="tag" required></label>
<button type="submit">Удалить</button>
</form>
"#.to_string()))
}

pub async fn delete_tag(State(state): State<AppState>, crate::auth::CurrentUser(user): crate::auth::CurrentUser, axum::Form(form): axum::Form<TagDeleteForm>) -> crate::error::Result<axum::response::Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(crate::error::AppError::Forbidden); }
    let tag_id: Option<i32> = sqlx::query_scalar("SELECT id FROM tags_values WHERE lower(value)=lower($1)")
        .bind(form.tag.trim()).fetch_optional(&state.pool).await?;
    if let Some(tag_id) = tag_id {
        sqlx::query("DELETE FROM tags WHERE tagid=$1").bind(tag_id).execute(&state.pool).await?;
        sqlx::query("DELETE FROM tags_values WHERE id=$1").bind(tag_id).execute(&state.pool).await?;
    }
    Ok(axum::response::Redirect::to("/tags"))
}
