use crate::{application::forum::CForumService, error::Result, infra::postgres::forum_repository::CForumPgRepository, models::Group, state::AppState};
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
    let groups = forum_service(&state).vecListForumGroups().await?;
    Ok(Html(GroupsTemplate { title: "Форум".into(), groups }.render()?))
}

pub async fn group_page(State(state): State<AppState>, Path(group): Path<String>, Query(q): Query<crate::models::PagerQuery>, current_user: crate::auth::CurrentUser) -> crate::error::Result<Html<String>> {
    crate::routes::topics::section_group_topics(State(state), axum::http::Uri::from_static("/forum"), Path(group), Query(q), current_user).await
}

pub async fn group_archive(State(state): State<AppState>, Path(group): Path<String>, Query(_q): Query<ArchiveQuery>) -> Result<Html<String>> {
    let group = forum_service(&state).stArchiveGroup(&group).await?;
    Ok(Html(GroupsTemplate { title: format!("Архив / {}", group.title), groups: vec![group] }.render()?))
}

pub async fn list_groups(state: &AppState) -> Result<Vec<Group>> {
    forum_service(state).vecListGroups().await
}

pub async fn list_groups_by_section(state: &AppState, section_prefix: Option<&str>) -> Result<Vec<Group>> {
    forum_service(state).vecListGroupsBySection(section_prefix).await
}

fn forum_service(state: &AppState) -> CForumService<CForumPgRepository> {
    CForumService::new(CForumPgRepository::new(state.pool.clone()))
}
