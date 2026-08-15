use crate::{
    application::forum::CForumService,
    auth::CurrentUser,
    error::{AppError, Result},
    infra::postgres::forum_repository::CForumPgRepository,
    models::{Group, TopicSummary},
    state::AppState,
};
use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::Method,
    response::{Html, IntoResponse, Redirect, Response},
};
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
    longinfo_html: Option<String>,
    topics: Vec<GroupTopicView>,
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

#[derive(Debug)]
struct GroupTopicView {
    topic: TopicSummary,
    last_author: String,
    last_postdate: chrono::DateTime<chrono::Utc>,
    target_url: String,
    comments: i32,
    comments_closed: bool,
}

impl GroupTopicView {
    fn vecTags(&self) -> Vec<String> {
        self.topic.vecTags()
    }
}

#[derive(Debug, sqlx::FromRow)]
struct GroupTopicActivityRow {
    topic_id: i32,
    last_comment_id: Option<i32>,
    last_author: String,
    last_postdate: chrono::DateTime<chrono::Utc>,
    postscore: i32,
}

// SectionController.NonTech in the original.  GroupDao returns groups by id;
// forum.jsp preserves that order inside each partition, while the quick-jump
// list uses SectionController.groupsSorted (technical first, then these ids).
const VEC_NON_TECH_GROUP_IDS: [i32; 4] = [8404, 4068, 9326, 19405];

fn bNonTechGroup(iGroupId: i32) -> bool {
    VEC_NON_TECH_GROUP_IDS.contains(&iGroupId)
}

async fn vecPrepareGroupTopics(
    stState: &AppState,
    vecTopics: Vec<TopicSummary>,
    optIgnoreUserId: Option<i32>,
    iMessages: i32,
    bLastmod: bool,
) -> Result<Vec<GroupTopicView>> {
    if vecTopics.is_empty() {
        return Ok(Vec::new());
    }
    let vecTopicIds: Vec<i32> = vecTopics.iter().map(|stTopic| stTopic.id).collect();
    let vecActivity = sqlx::query_as::<_, GroupTopicActivityRow>(
        r#"SELECT t.id AS topic_id,lc.id AS last_comment_id,
                  COALESCE(lu.nick,tu.nick) AS last_author,
                  COALESCE(lc.postdate,t.postdate) AS last_postdate,
                  COALESCE(t.postscore,-9999) AS postscore
           FROM topics t
           JOIN users tu ON tu.id=t.userid
           LEFT JOIN LATERAL (
             SELECT c.id,c.userid,c.postdate
               FROM comments c
              WHERE c.topic=t.id AND NOT c.deleted
                AND ($2::int IS NULL OR t.sticky OR NOT EXISTS (
                  SELECT ignored FROM ignore_list WHERE userid=$2
                  INTERSECT SELECT get_branch_authors(c.id)
                ))
              ORDER BY c.postdate DESC
              LIMIT 1
           ) lc ON t.postscore IS DISTINCT FROM 10002
           LEFT JOIN users lu ON lu.id=lc.userid
           WHERE t.id=ANY($1)"#,
    )
    .bind(&vecTopicIds)
    .bind(optIgnoreUserId)
    .fetch_all(&stState.pool)
    .await?;
    let mapActivity: std::collections::HashMap<i32, GroupTopicActivityRow> = vecActivity
        .into_iter()
        .map(|stRow| (stRow.topic_id, stRow))
        .collect();
    let iMessages = iMessages.max(1);
    Ok(vecTopics
        .into_iter()
        .filter_map(|stTopic| {
            let stActivity = mapActivity.get(&stTopic.id)?;
            let iPages = ((stTopic.comments.max(0) + iMessages - 1) / iMessages).max(0);
            let sCanonical = stTopic.sTopicUrl();
            let iLastCommentId = stActivity.last_comment_id.unwrap_or(0);
            let sTargetUrl = if !bLastmod {
                sCanonical
            } else if iPages > 1 {
                format!("{sCanonical}/page{}?lastmod={iLastCommentId}", iPages - 1)
            } else {
                format!("{sCanonical}?lastmod={iLastCommentId}")
            };
            Some(GroupTopicView {
                comments: if stActivity.postscore == 10002 {
                    0
                } else {
                    stTopic.comments
                },
                comments_closed: stActivity.postscore >= 10000,
                last_author: stActivity.last_author.clone(),
                last_postdate: stActivity.last_postdate,
                target_url: sTargetUrl,
                topic: stTopic,
            })
        })
        .collect())
}

pub async fn forum_index(State(state): State<AppState>) -> Result<Html<String>> {
    let groups = forum_service(&state).vecListForumGroups().await?;
    let (other, tech): (Vec<_>, Vec<_>) = groups
        .into_iter()
        .partition(|group| bNonTechGroup(group.id));
    Ok(Html(
        GroupsTemplate {
            title: "Форум".into(),
            tech,
            other,
        }
        .render()?,
    ))
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

pub async fn group_page(
    State(state): State<AppState>,
    method: Method,
    Path(group_urlname): Path<String>,
    Query(q): Query<ForumGroupQuery>,
    CurrentUser(user): CurrentUser,
) -> Result<Response> {
    let offset = q.offset.unwrap_or(0);
    if offset < 0 {
        return Err(AppError::BadRequest(
            "offset не может быть отрицательным".into(),
        ));
    }
    if offset > MAX_GROUP_OFFSET {
        return Ok(Redirect::to(&format!(
            "/forum/{}/archive",
            urlencoding::encode(&group_urlname)
        ))
        .into_response());
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
            return Ok(
                Redirect::to(&format!("/forum/{}", urlencoding::encode(&group_urlname)))
                    .into_response(),
            );
        }
    }
    let show_deleted = show_deleted_requested;

    let group = find_group_by_section(&state, "forum", &group_urlname).await?;
    let group_id = group.id;

    let tag_id: Option<i32> = if let Some(tag) = q.tag.as_deref().filter(|t| !t.is_empty()) {
        let id: Option<i32> =
            sqlx::query_scalar("SELECT id FROM tags_values WHERE lower(value)=lower($1)")
                .bind(tag)
                .fetch_optional(&state.pool)
                .await?;
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
    let ignore_user_id: Option<i32> = if show_ignored {
        None
    } else {
        user.as_ref().map(|u| u.id)
    };

    // GroupListDao uses the current viewer profile, including the anonymous
    // DefaultProfile, rather than the process-wide PAGE_SIZE setting.
    let stProfile = if let Some(stUser) = user.as_ref() {
        let optSettings: Option<String> =
            sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                .bind(stUser.id)
                .fetch_optional(&state.pool)
                .await?;
        crate::profile::ProfileSettings::from_hstore_text(optSettings)
    } else {
        crate::profile::ProfileSettings::default()
    };
    let limit = i64::from(stProfile.topics);
    let lastmod = q.lastmod.unwrap_or(false);

    let order_by = if lastmod {
        "GREATEST(t.postdate,COALESCE(lc_visible.postdate,t.postdate)) DESC"
    } else {
        "t.postdate DESC"
    };
    let date_filter = if lastmod {
        "COALESCE(t.lastmod, t.postdate) > now() - interval '6 months'"
    } else {
        "t.postdate > now() - interval '6 months'"
    };

    let sql = format!(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  'forum' AS section_prefix,
                  t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                  (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                     FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                    WHERE tg.msgid=t.id) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN LATERAL (
             SELECT c.id,c.postdate
               FROM comments c
              WHERE c.topic=t.id AND NOT c.deleted
                AND ($6::int IS NULL OR NOT EXISTS (
                  SELECT ignored FROM ignore_list WHERE userid=$6
                  INTERSECT SELECT get_branch_authors(c.id)
                ))
              ORDER BY c.postdate DESC
              LIMIT 1
           ) lc_visible ON t.postscore IS DISTINCT FROM 10002
           WHERE t.groupid=$1 AND NOT t.sticky AND (NOT t.deleted OR $5) AND NOT t.draft
             AND (t.moderate OR NOT s.moderate) AND {date_filter}
             AND ($4::int IS NULL OR t.id IN (SELECT msgid FROM tags WHERE tagid=$4))
             AND ($6::int IS NULL OR NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=$6 AND il.ignored=u.id))
             AND ($6::int IS NULL OR NOT (
               EXISTS (
                 SELECT 1 FROM tags topic_tag
                 JOIN user_tags ignored_tag ON ignored_tag.tag_id=topic_tag.tagid
                 WHERE topic_tag.msgid=t.id AND ignored_tag.user_id=$6
                   AND NOT ignored_tag.is_favorite
               )
               AND NOT EXISTS (
                 SELECT 1 FROM tags topic_tag
                 JOIN user_tags favorite_tag ON favorite_tag.tag_id=topic_tag.tagid
                 WHERE topic_tag.msgid=t.id AND favorite_tag.user_id=$6
                   AND favorite_tag.is_favorite
               )
             ))
           ORDER BY {order_by}
           OFFSET $2 LIMIT $3"#,
        date_filter = date_filter,
        order_by = order_by,
    );
    let mut topics = sqlx::query_as::<_, TopicSummary>(sqlx::AssertSqlSafe(sql))
        .bind(group_id)
        .bind(offset)
        .bind(limit)
        .bind(tag_id)
        .bind(show_deleted)
        .bind(ignore_user_id)
        .fetch_all(&state.pool)
        .await?;
    let main_topics_count = topics.len();

    if offset == 0 && !lastmod {
        let sticky_sql = r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                      g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                      s.id AS section_id, s.name AS section_name,
                      'forum' AS section_prefix,
                      t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                      (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                         FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                        WHERE tg.msgid=t.id) AS tags
               FROM topics t
               JOIN users u ON u.id=t.userid
               JOIN groups g ON g.id=t.groupid
               JOIN sections s ON s.id=g.section
               WHERE t.groupid=$1 AND t.sticky AND NOT t.deleted AND NOT t.draft
                 AND (t.moderate OR NOT s.moderate)
                 AND ($2::int IS NULL OR t.id IN (SELECT msgid FROM tags WHERE tagid=$2))
               ORDER BY t.postdate DESC
               LIMIT 100"#;
        let mut sticky = sqlx::query_as::<_, TopicSummary>(sticky_sql)
            .bind(group_id)
            .bind(tag_id)
            .fetch_all(&state.pool)
            .await?;
        sticky.extend(topics);
        topics = sticky;
    }

    let topics =
        vecPrepareGroupTopics(&state, topics, ignore_user_id, stProfile.messages, lastmod).await?;

    let restriction: i32 = sqlx::query_scalar("SELECT GREATEST(COALESCE(g.restrict_topics,-9999),COALESCE(s.restrict_topics,-9999)) FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1")
        .bind(group_id).fetch_one(&state.pool).await?;
    let add_reason =
        crate::routes::topics::posting_reason_for_port(&state, restriction, &user).await?;
    let tag_suffix = q
        .tag
        .as_deref()
        .filter(|tag| !tag.is_empty())
        .map(|tag| format!("tag={}", urlencoding::encode(tag)));
    let new_url = group_mode_url(
        &group_urlname,
        false,
        tag_suffix.as_deref(),
        None,
        show_ignored,
    );
    let active_url = group_mode_url(
        &group_urlname,
        true,
        tag_suffix.as_deref(),
        None,
        show_ignored,
    );
    let archive_url = format!("/forum/{}/archive", urlencoding::encode(&group_urlname));
    let add_url = add_reason.is_none().then(|| {
        let mut url = format!("/add.jsp?group={group_id}");
        if let Some(tag) = q.tag.as_deref().filter(|tag| !tag.is_empty()) {
            url.push_str("&tags=");
            url.push_str(&urlencoding::encode(tag));
        }
        url
    });
    let mut vecQuickGroups = list_groups_by_section(&state, Some("forum")).await?;
    vecQuickGroups.sort_by_key(|stGroup| (bNonTechGroup(stGroup.id), stGroup.id));
    let quick_groups = vecQuickGroups
        .into_iter()
        .map(|item| crate::routes::topics::QuickGroupLink {
            title: item.title,
            url: group_mode_url(
                &item.urlname,
                lastmod,
                tag_suffix.as_deref(),
                None,
                show_ignored,
            ),
            selected: item.id == group_id,
        })
        .collect();
    let prev_url = (offset > 0).then(|| {
        group_mode_url(
            &group_urlname,
            lastmod,
            tag_suffix.as_deref(),
            Some((offset - limit).max(0)),
            show_ignored,
        )
    });
    let next_url = bGroupHasNext(offset, main_topics_count, limit).then(|| {
        group_mode_url(
            &group_urlname,
            lastmod,
            tag_suffix.as_deref(),
            Some(offset + limit),
            show_ignored,
        )
    });
    let title = format!("Форум — {}", group.title);
    let longinfo_html = optPreparedGroupLongInfo(group.longinfo.as_deref());
    let is_moderator = user.as_ref().is_some_and(|current| current.canmod);

    Ok(Html(
        GroupTopicsTemplate {
            title,
            group,
            longinfo_html,
            topics,
            quick_groups,
            new_url,
            active_url,
            archive_url,
            add_url,
            add_reason: add_reason.unwrap_or_default(),
            lastmod,
            prev_url,
            next_url,
            is_moderator,
        }
        .render()?,
    )
    .into_response())
}

fn bGroupHasNext(iOffset: i64, iMainTopicCount: usize, iTopicsPerPage: i64) -> bool {
    iOffset < MAX_GROUP_OFFSET && iMainTopicCount as i64 == iTopicsPerPage
}

fn optPreparedGroupLongInfo(optSource: Option<&str>) -> Option<String> {
    optSource
        .map(|sSource| crate::markup::render_message_with_markup(sSource, Some("MARKDOWN"), None))
}

fn group_mode_url(
    group: &str,
    lastmod: bool,
    tag_query: Option<&str>,
    offset: Option<i64>,
    show_ignored: bool,
) -> String {
    let mut params = Vec::new();
    if lastmod {
        params.push("lastmod=true".to_string());
    }
    if let Some(tag) = tag_query {
        params.push(tag.to_string());
    }
    if show_ignored {
        params.push("showignored=true".to_string());
    }
    if let Some(offset) = offset.filter(|offset| *offset > 0) {
        params.push(format!("offset={offset}"));
    }
    let mut url = format!("/forum/{}", urlencoding::encode(group));
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }
    url
}

pub async fn group_archive(
    State(state): State<AppState>,
    Path(group_name): Path<String>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>> {
    let group = forum_service(&state)
        .stGroupBySectionAndUrlName("forum", &group_name)
        .await?;
    let rows =
        crate::routes::legacy::list_archive_year_months(&state, Some("forum"), Some(&group_name))
            .await?;
    let months = rows
        .into_iter()
        .map(|(y, m, c)| crate::routes::legacy::ArchiveMonthLink {
            year: y,
            month_name: crate::routes::legacy::month_name(m),
            count: c,
            url: format!("/forum/{group_name}/{y}/{m}/"),
        })
        .collect();
    let restriction: i32 = sqlx::query_scalar(
        "SELECT GREATEST(COALESCE(g.restrict_topics,-9999),COALESCE(s.restrict_topics,-9999)) FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1",
    )
    .bind(group.id)
    .fetch_one(&state.pool)
    .await?;
    let add_reason =
        crate::routes::topics::posting_reason_for_port(&state, restriction, &user).await?;
    Ok(Html(
        crate::routes::legacy::ArchiveIndexTemplate {
            title: format!("Форум - {} - Архив", group.title),
            heading: format!("Форум «{}»", group.title),
            back_url: format!("/forum/{group_name}"),
            back_label: "Новые".to_string(),
            active_url: Some(format!("/forum/{group_name}?lastmod=true")),
            archive_url: format!("/forum/{group_name}/archive/"),
            section_id: group.section,
            section_urlname: "forum".to_string(),
            group_urlname: Some(group_name.clone()),
            uncommitted_count: 0,
            add_url: add_reason
                .is_none()
                .then(|| format!("/add.jsp?group={}", group.id)),
            add_reason: add_reason.unwrap_or_default(),
            months,
        }
        .render()?,
    ))
}

pub async fn find_group_by_section(
    state: &AppState,
    section_prefix: &str,
    urlname: &str,
) -> Result<Group> {
    forum_service(state)
        .stGroupBySectionAndUrlName(section_prefix, urlname)
        .await
        .map_err(|error| match error {
            AppError::Sqlx(sqlx::Error::RowNotFound) => AppError::NotFound,
            other => other,
        })
}

pub async fn list_groups_by_section(
    state: &AppState,
    section_prefix: Option<&str>,
) -> Result<Vec<Group>> {
    forum_service(state)
        .vecListGroupsBySection(section_prefix)
        .await
}

fn forum_service(state: &AppState) -> CForumService<CForumPgRepository> {
    CForumService::new(CForumPgRepository::new(state.pool.clone()))
}

#[cfg(test)]
mod tests {
    use super::{
        GroupTopicView, GroupTopicsTemplate, bGroupHasNext, bNonTechGroup, group_mode_url,
        optPreparedGroupLongInfo,
    };
    use crate::models::{Group, TopicSummary};
    use askama::Template;
    use chrono::{TimeZone, Utc};

    #[test]
    fn group_longinfo_is_rendered_as_sanitized_markdown() {
        let sHtml = optPreparedGroupLongInfo(Some(
            "**важно** <script>alert(1)</script> [x](javascript:alert(2))",
        ))
        .expect("rendered long info");
        assert!(sHtml.contains("<strong>важно</strong>"));
        assert!(!sHtml.contains("<script"));
        assert!(!sHtml.contains("javascript:"));
    }

    #[test]
    fn group_next_link_uses_viewer_page_size_and_java_offset_ceiling() {
        assert!(bGroupHasNext(250, 50, 50));
        assert!(!bGroupHasNext(250, 49, 50));
        assert!(!bGroupHasNext(300, 50, 50));
    }

    #[test]
    fn forum_group_partition_and_quick_jump_match_section_controller() {
        assert!(!bNonTechGroup(126));
        assert!(bNonTechGroup(4068));
        assert!(bNonTechGroup(8404));
        assert!(bNonTechGroup(9326));

        let mut vecGroupIds = [8404, 9326, 1339, 4068, 126];
        vecGroupIds.sort_by_key(|iGroupId| (bNonTechGroup(*iGroupId), *iGroupId));
        assert_eq!(vecGroupIds, [126, 1339, 4068, 8404, 9326]);
    }

    #[test]
    fn group_pager_keeps_activity_tag_and_ignore_parameters() {
        assert_eq!(
            group_mode_url("linux-hardware", true, Some("tag=usb%20c"), Some(100), true,),
            "/forum/linux-hardware?lastmod=true&tag=usb%20c&showignored=true&offset=100"
        );
    }

    #[test]
    fn forum_group_uses_group_rows_instead_of_tracker_rows() {
        let dtPostdate = Utc.with_ymd_and_hms(2026, 8, 14, 12, 0, 0).unwrap();
        let stGroup = Group {
            id: 4068,
            title: "General".to_owned(),
            urlname: "general".to_owned(),
            section: 2,
            section_name: "Форум".to_owned(),
            section_prefix: "forum".to_owned(),
            info: None,
            longinfo: None,
            topics: 1,
            topics_per_day: 1,
        };
        let stTopic = TopicSummary {
            id: 42,
            title: "Тестовая тема".to_owned(),
            url: None,
            postdate: dtPostdate,
            lastmod: Some(dtPostdate),
            author_id: 1,
            author: "author".to_owned(),
            group_id: stGroup.id,
            group_title: stGroup.title.clone(),
            group_urlname: stGroup.urlname.clone(),
            section_id: stGroup.section,
            section_name: stGroup.section_name.clone(),
            section_prefix: stGroup.section_prefix.clone(),
            comments: 3,
            deleted: false,
            sticky: false,
            resolved: None,
            tags: Some("rust,lor".to_owned()),
        };
        let sHtml = GroupTopicsTemplate {
            title: "Форум — General".to_owned(),
            group: stGroup,
            longinfo_html: None,
            topics: vec![GroupTopicView {
                topic: stTopic,
                last_author: "commenter".to_owned(),
                last_postdate: dtPostdate,
                target_url: "/forum/general/42".to_owned(),
                comments: 3,
                comments_closed: false,
            }],
            quick_groups: Vec::new(),
            new_url: "/forum/general".to_owned(),
            active_url: "/forum/general?lastmod=true".to_owned(),
            archive_url: "/forum/general/archive".to_owned(),
            add_url: None,
            add_reason: "forbidden".to_owned(),
            lastmod: false,
            prev_url: None,
            next_url: None,
            is_moderator: false,
        }
        .render()
        .expect("group template renders");

        assert!(sHtml.contains("class=\"tracker\""));
        assert!(sHtml.contains("href=\"/forum/general/42\" class=\"group-item\""));
        for sClass in [
            "tracker-count",
            "tracker-title",
            "tracker-tags",
            "tracker-last",
        ] {
            assert!(
                sHtml.contains(&format!("class=\"{sClass}\"")),
                "missing original group row cell: {sClass}"
            );
        }
        assert!(!sHtml.contains("class=\"tracker-item\""));
        assert!(!sHtml.contains("class=\"tracker-src\""));
    }
}
