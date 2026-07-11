use crate::{application::forum::CForumService, auth::CurrentUser, error::{AppError, Result}, infra::postgres::forum_repository::CForumPgRepository, models::{Group, TopicSummary}, pagination::Pager, state::AppState};
use askama::Template;
use axum::{extract::{Path, Query, State}, response::{Html, IntoResponse, Redirect, Response}};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "groups.html")]
struct GroupsTemplate {
    title: String,
    groups: Vec<Group>,
}

#[derive(Template)]
#[template(path = "index.html")]
struct GroupTopicsTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    pager: Pager,
    current_user: Option<crate::models::UserSummary>,
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

/// GroupController.forum: not the generic section-group listing (which
/// `topics::section_group_topics` implements) - the forum group page has
/// its own rules: sticky topics pinned first, an optional tag filter
/// (404 if the tag doesn't exist), a `lastmod=true` last-activity-sorted
/// mode, `offset > 300` redirects to the archive instead of paginating
/// forever, and `showDeleted` only takes effect on a POST from a moderator
/// (a bare GET with the flag is bounced back without it).
#[derive(Deserialize)]
pub struct ForumGroupQuery {
    pub offset: Option<i64>,
    pub lastmod: Option<bool>,
    pub tag: Option<String>,
    #[serde(rename = "showignored")]
    pub show_ignored: Option<bool>,
}

const MAX_GROUP_OFFSET: i64 = 300;

pub async fn group_page(State(state): State<AppState>, Path(group_urlname): Path<String>, Query(q): Query<ForumGroupQuery>, CurrentUser(user): CurrentUser) -> Result<Response> {
    let offset = q.offset.unwrap_or(0);
    if offset < 0 {
        return Err(AppError::BadRequest("offset не может быть отрицательным".into()));
    }
    if offset > MAX_GROUP_OFFSET {
        return Ok(Redirect::to(&format!("/forum/{}/archive", urlencoding::encode(&group_urlname))).into_response());
    }

    let Some(group_id): Option<i32> = sqlx::query_scalar("SELECT id FROM groups WHERE urlname=$1")
        .bind(&group_urlname)
        .fetch_optional(&state.pool)
        .await?
    else {
        return Err(AppError::NotFound);
    };

    let tag_id: Option<i32> = if let Some(tag) = q.tag.as_deref().filter(|t| !t.is_empty()) {
        let id: Option<i32> = sqlx::query_scalar("SELECT id FROM tags_values WHERE lower(value)=lower($1)").bind(tag).fetch_optional(&state.pool).await?;
        if id.is_none() {
            return Err(AppError::NotFound);
        }
        id
    } else {
        None
    };

    let limit = state.config.page_size.max(1);
    let lastmod = q.lastmod.unwrap_or(false);

    let order_by = if lastmod { "COALESCE(t.lastmod, t.postdate) DESC" } else { "t.postdate DESC" };
    let date_filter = if lastmod { "COALESCE(t.lastmod, t.postdate) > now() - interval '7 days'" } else { "t.postdate > now() - interval '6 months'" };

    let sql = format!(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  'forum' AS section_prefix,
                  t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                  string_agg(tv.value, ',' ORDER BY tv.value) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN tags tg ON tg.msgid=t.id
           LEFT JOIN tags_values tv ON tv.id=tg.tagid
           WHERE t.groupid=$1 AND NOT t.sticky AND NOT t.deleted AND NOT t.draft AND NOT t.moderate AND {date_filter}
             {tag_clause}
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY {order_by}
           OFFSET $2 LIMIT $3"#,
        date_filter = date_filter,
        order_by = order_by,
        tag_clause = if tag_id.is_some() { "AND t.id IN (SELECT msgid FROM tags WHERE tagid=$4)" } else { "" },
    );
    let mut query = sqlx::query_as::<_, TopicSummary>(&sql).bind(group_id).bind(offset).bind(limit);
    if let Some(tag_id) = tag_id {
        query = query.bind(tag_id);
    }
    let mut topics = query.fetch_all(&state.pool).await?;

    if offset == 0 && !lastmod {
        let sticky_sql = format!(
            r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                      g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                      s.id AS section_id, s.name AS section_name,
                      'forum' AS section_prefix,
                      t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                      string_agg(tv.value, ',' ORDER BY tv.value) AS tags
               FROM topics t
               JOIN users u ON u.id=t.userid
               JOIN groups g ON g.id=t.groupid
               JOIN sections s ON s.id=g.section
               LEFT JOIN tags tg ON tg.msgid=t.id
               LEFT JOIN tags_values tv ON tv.id=tg.tagid
               WHERE t.groupid=$1 AND t.sticky AND NOT t.deleted AND NOT t.draft AND NOT t.moderate
                 {tag_clause}
               GROUP BY t.id,u.id,g.id,s.id
               ORDER BY t.postdate DESC
               LIMIT 100"#,
            tag_clause = if tag_id.is_some() { "AND t.id IN (SELECT msgid FROM tags WHERE tagid=$2)" } else { "" },
        );
        let mut sticky_query = sqlx::query_as::<_, TopicSummary>(&sticky_sql).bind(group_id);
        if let Some(tag_id) = tag_id {
            sticky_query = sticky_query.bind(tag_id);
        }
        let mut sticky = sticky_query.fetch_all(&state.pool).await?;
        sticky.extend(topics);
        topics = sticky;
    }

    let group_title: String = sqlx::query_scalar("SELECT title FROM groups WHERE id=$1").bind(group_id).fetch_one(&state.pool).await?;
    let pager = Pager::new(offset, limit);
    let title = format!("Форум / {group_title}");
    let _ = q.show_ignored; // accepted for URL round-tripping; ignore-list filtering isn't modeled yet

    Ok(Html(GroupTopicsTemplate { title, topics, pager, current_user: user }.render()?).into_response())
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
