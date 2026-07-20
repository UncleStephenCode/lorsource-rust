use crate::{application::forum::CForumService, auth::CurrentUser, error::{AppError, Result}, infra::postgres::forum_repository::CForumPgRepository, models::{Group, TopicSummary}, state::AppState};
use askama::Template;
use axum::{extract::{Path, Query, State}, http::Method, response::{Html, IntoResponse, Redirect, Response}};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "groups.html")]
struct GroupsTemplate {
    title: String,
    tech: Vec<Group>,
    other: Vec<Group>,
}

#[derive(Template)]
#[template(path = "group_topics.html")]
struct GroupTopicsTemplate {
    title: String,
    group: Group,
    topics: Vec<TopicSummary>,
    quick_groups: Vec<crate::routes::topics::QuickGroupLink>,
    new_url: String,
    active_url: String,
    archive_url: String,
    add_url: Option<String>,
    add_reason: String,
    lastmod: bool,
    prev_url: Option<String>,
    next_url: Option<String>,
    is_moderator: bool,
}

pub async fn forum_index(State(state): State<AppState>) -> Result<Html<String>> {
    let groups = forum_service(&state).vecListForumGroups().await?;
    let (other, tech): (Vec<_>, Vec<_>) = groups.into_iter().partition(|group| [8404, 4068, 9326, 19405].contains(&group.id));
    Ok(Html(GroupsTemplate { title: "Форум".into(), tech, other }.render()?))
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
    #[serde(rename = "showDeleted")]
    pub show_deleted: Option<bool>,
}

const MAX_GROUP_OFFSET: i64 = 300;

pub async fn group_page(State(state): State<AppState>, method: Method, Path(group_urlname): Path<String>, Query(q): Query<ForumGroupQuery>, CurrentUser(user): CurrentUser) -> Result<Response> {
    let offset = q.offset.unwrap_or(0);
    if offset < 0 {
        return Err(AppError::BadRequest("offset не может быть отрицательным".into()));
    }
    if offset > MAX_GROUP_OFFSET {
        return Ok(Redirect::to(&format!("/forum/{}/archive", urlencoding::encode(&group_urlname))).into_response());
    }

    // GroupController.forum: showDeleted требует авторизации +
    // canViewAllDeletedTopics (score>=50, не заморожен - без отдельной
    // льготы для модераторов, ровно как в оригинале) и применяется только
    // на POST - обычный GET с этим флагом отбрасывается редиректом на
    // страницу без него.
    let show_deleted_requested = q.show_deleted.unwrap_or(false);
    if show_deleted_requested {
        if user.is_none() {
            return Err(AppError::Forbidden);
        }
        if !crate::routes::topics::can_view_all_deleted_topics(&state, &user).await? {
            return Err(AppError::Forbidden);
        }
        if method != Method::POST {
            return Ok(Redirect::to(&format!("/forum/{}", urlencoding::encode(&group_urlname))).into_response());
        }
    }
    let show_deleted = show_deleted_requested;

    let group = find_group(&state, &group_urlname).await?;
    if group.section_prefix != "forum" { return Err(AppError::NotFound); }
    let group_id = group.id;

    let tag_id: Option<i32> = if let Some(tag) = q.tag.as_deref().filter(|t| !t.is_empty()) {
        let id: Option<i32> = sqlx::query_scalar("SELECT id FROM tags_values WHERE lower(value)=lower($1)").bind(tag).fetch_optional(&state.pool).await?;
        if id.is_none() {
            return Err(AppError::NotFound);
        }
        id
    } else {
        None
    };

    // GroupListDao.load: showIgnored=true (или анонимус) отключает
    // фильтрацию по ignore_list текущего пользователя.
    let show_ignored = q.show_ignored.unwrap_or(false);
    let ignore_user_id: Option<i32> = if show_ignored { None } else { user.as_ref().map(|u| u.id) };

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
           WHERE t.groupid=$1 AND NOT t.sticky AND (NOT t.deleted OR $5) AND NOT t.draft AND NOT t.moderate AND {date_filter}
             AND ($4::int IS NULL OR t.id IN (SELECT msgid FROM tags WHERE tagid=$4))
             AND ($6::int IS NULL OR NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=$6 AND il.ignored=u.id))
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY {order_by}
           OFFSET $2 LIMIT $3"#,
        date_filter = date_filter,
        order_by = order_by,
    );
    let mut topics = sqlx::query_as::<_, TopicSummary>(&sql)
        .bind(group_id).bind(offset).bind(limit).bind(tag_id).bind(show_deleted).bind(ignore_user_id)
        .fetch_all(&state.pool).await?;

    if offset == 0 && !lastmod {
        let sticky_sql = r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
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
                 AND ($2::int IS NULL OR t.id IN (SELECT msgid FROM tags WHERE tagid=$2))
               GROUP BY t.id,u.id,g.id,s.id
               ORDER BY t.postdate DESC
               LIMIT 100"#;
        let mut sticky = sqlx::query_as::<_, TopicSummary>(sticky_sql)
            .bind(group_id).bind(tag_id)
            .fetch_all(&state.pool).await?;
        sticky.extend(topics);
        topics = sticky;
    }

    let restriction: i32 = sqlx::query_scalar("SELECT GREATEST(COALESCE(g.restrict_topics,-9999),s.restrict_score) FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1")
        .bind(group_id).fetch_one(&state.pool).await?;
    let add_reason = crate::routes::topics::posting_reason_for_port(&state, restriction, &user).await?;
    let tag_suffix = q.tag.as_deref().filter(|tag| !tag.is_empty()).map(|tag| format!("tag={}", urlencoding::encode(tag)));
    let new_url = group_mode_url(&group_urlname, false, tag_suffix.as_deref(), None, show_ignored);
    let active_url = group_mode_url(&group_urlname, true, tag_suffix.as_deref(), None, show_ignored);
    let archive_url = format!("/forum/{}/archive", urlencoding::encode(&group_urlname));
    let add_url = add_reason.is_none().then(|| {
        let mut url = format!("/add.jsp?group={group_id}");
        if let Some(tag) = q.tag.as_deref().filter(|tag| !tag.is_empty()) {
            url.push_str("&tags=");
            url.push_str(&urlencoding::encode(tag));
        }
        url
    });
    let quick_groups = list_groups_by_section(&state, Some("forum")).await?.into_iter().map(|item| crate::routes::topics::QuickGroupLink {
        title: item.title,
        url: group_mode_url(&item.urlname, lastmod, tag_suffix.as_deref(), None, show_ignored),
        selected: item.id == group_id,
    }).collect();
    let non_sticky_count = topics.iter().filter(|topic| !topic.sticky).count() as i64;
    let prev_url = (offset > 0).then(|| group_mode_url(&group_urlname, lastmod, tag_suffix.as_deref(), Some((offset - limit).max(0)), show_ignored));
    let next_url = (non_sticky_count >= limit).then(|| group_mode_url(&group_urlname, lastmod, tag_suffix.as_deref(), Some(offset + limit), show_ignored));
    let title = format!("Форум — {}", group.title);
    let is_moderator = user.as_ref().is_some_and(|current| current.canmod);

    Ok(Html(GroupTopicsTemplate { title, group, topics, quick_groups, new_url, active_url, archive_url, add_url, add_reason: add_reason.unwrap_or_default(), lastmod, prev_url, next_url, is_moderator }.render()?).into_response())
}

fn group_mode_url(group: &str, lastmod: bool, tag_query: Option<&str>, offset: Option<i64>, show_ignored: bool) -> String {
    let mut params = Vec::new();
    if lastmod { params.push("lastmod=true".to_string()); }
    if let Some(tag) = tag_query { params.push(tag.to_string()); }
    if show_ignored { params.push("showignored=true".to_string()); }
    if let Some(offset) = offset.filter(|offset| *offset > 0) { params.push(format!("offset={offset}")); }
    let mut url = format!("/forum/{}", urlencoding::encode(group));
    if !params.is_empty() { url.push('?'); url.push_str(&params.join("&")); }
    url
}

pub async fn group_archive(State(state): State<AppState>, Path(group_name): Path<String>) -> Result<Html<String>> {
    let group = forum_service(&state).stArchiveGroup(&group_name).await?;
    let rows = crate::routes::legacy::list_archive_year_months(&state, Some("forum"), Some(&group_name)).await?;
    let months = rows.into_iter().map(|(y, m, c)| crate::routes::legacy::ArchiveMonthLink {
        year: y, month_name: crate::routes::legacy::month_name(m), count: c, url: format!("/forum/{group_name}/{y}/{m}"),
    }).collect();
    Ok(Html(crate::routes::legacy::ArchiveIndexTemplate {
        title: format!("Форум - {} - Архив", group.title),
        heading: format!("Форум «{}»", group.title),
        back_url: format!("/forum/{group_name}"),
        back_label: "Новые".to_string(),
        months,
    }.render()?))
}

pub async fn list_groups(state: &AppState) -> Result<Vec<Group>> {
    forum_service(state).vecListGroups().await
}

pub async fn find_group(state: &AppState, urlname: &str) -> Result<Group> {
    forum_service(state).stArchiveGroup(urlname).await.map_err(|error| match error {
        AppError::Sqlx(sqlx::Error::RowNotFound) => AppError::NotFound,
        other => other,
    })
}

pub async fn list_groups_by_section(state: &AppState, section_prefix: Option<&str>) -> Result<Vec<Group>> {
    forum_service(state).vecListGroupsBySection(section_prefix).await
}

fn forum_service(state: &AppState) -> CForumService<CForumPgRepository> {
    CForumService::new(CForumPgRepository::new(state.pool.clone()))
}
