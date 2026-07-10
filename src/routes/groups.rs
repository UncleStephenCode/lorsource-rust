use crate::{error::Result, models::Group, state::AppState};
use askama::Template;
use axum::{extract::{Path, Query, State}, response::Html};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "groups.html")]
struct GroupsTemplate {
    title: String,
    groups: Vec<Group>,
}

#[derive(Deserialize)]
pub struct ArchiveQuery {
    year: Option<i32>,
    month: Option<i32>,
}

pub async fn forum_index(State(state): State<AppState>) -> Result<Html<String>> {
    let groups = list_groups_by_section(&state, Some("forum")).await?;
    Ok(Html(GroupsTemplate { title: "Форум".into(), groups }.render()?))
}

pub async fn group_page(State(state): State<AppState>, Path(group): Path<String>, Query(q): Query<crate::models::PagerQuery>, current_user: crate::auth::CurrentUser) -> crate::error::Result<Html<String>> {
    crate::routes::topics::section_group_topics(State(state), axum::http::Uri::from_static("/forum"), Path(group), Query(q), current_user).await
}

pub async fn group_archive(State(state): State<AppState>, Path(group): Path<String>, Query(_q): Query<ArchiveQuery>) -> Result<Html<String>> {
    let group = sqlx::query_as::<_, Group>(GROUP_SELECT_SQL.to_string() + " WHERE g.urlname=$1 GROUP BY g.id,s.id")
        .bind(group)
        .fetch_one(&state.pool)
        .await?;
    Ok(Html(GroupsTemplate { title: format!("Архив / {}", group.title), groups: vec![group] }.render()?))
}

pub async fn list_groups(state: &AppState) -> Result<Vec<Group>> {
    Ok(sqlx::query_as::<_, Group>(&(GROUP_SELECT_SQL.to_string() + " GROUP BY g.id,s.id ORDER BY s.id,g.title"))
        .fetch_all(&state.pool)
        .await?)
}

pub async fn list_groups_by_section(state: &AppState, section_prefix: Option<&str>) -> Result<Vec<Group>> {
    Ok(sqlx::query_as::<_, Group>(
        &(GROUP_SELECT_SQL.to_string()
            + " WHERE ($1::text IS NULL OR CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END=$1) GROUP BY g.id,s.id ORDER BY g.title"),
    )
    .bind(section_prefix)
    .fetch_all(&state.pool)
    .await?)
}

const GROUP_SELECT_SQL: &str = r#"
SELECT g.id, g.title, g.urlname, g.section, s.name AS section_name,
       CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
       g.info, g.longinfo, count(t.id) AS topics
FROM groups g
JOIN sections s ON s.id=g.section
LEFT JOIN topics t ON t.groupid=g.id AND NOT t.deleted
"#;
