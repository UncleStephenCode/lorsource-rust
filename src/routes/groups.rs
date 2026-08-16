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
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, Method},
    response::{Html, IntoResponse, Response},
};
use serde::{Deserialize, Deserializer};
use std::net::SocketAddr;

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
    next_offset: i64,
    is_moderator: bool,
    old_tracker: bool,
    show_ignored: bool,
    show_deleted: bool,
    show_deleted_button: bool,
    csrf_token: String,
    frozen_until: Option<chrono::DateTime<chrono::Utc>>,
    active_tags: Vec<crate::routes::topics::ActiveTagLink>,
    tag_name: Option<String>,
    tag_title: String,
    tag_url: String,
    group_url: String,
    first_page: bool,
}

#[derive(Debug)]
struct GroupTopicView {
    topic: TopicSummary,
    last_author: String,
    last_postdate: chrono::DateTime<chrono::Utc>,
    target_url: String,
    comments: i32,
    comments_closed: bool,
    topic_author_blocked: bool,
    last_author_blocked: bool,
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
    topic_author_blocked: bool,
    last_author_blocked: bool,
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
                  COALESCE(t.postscore,-9999) AS postscore,
                  COALESCE(tu.blocked,false) AS topic_author_blocked,
                  COALESCE(lu.blocked,tu.blocked,false) AS last_author_blocked
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
                topic_author_blocked: stActivity.topic_author_blocked,
                last_author_blocked: stActivity.last_author_blocked,
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
    #[serde(default, deserialize_with = "optDeserializeGroupOffset")]
    pub offset: Option<i64>,
    #[serde(default, deserialize_with = "optDeserializeGroupBoolean")]
    pub lastmod: Option<bool>,
    pub tag: Option<String>,
    #[serde(
        default,
        rename = "showignored",
        deserialize_with = "optDeserializeGroupBoolean"
    )]
    pub show_ignored: Option<bool>,
    #[serde(
        default,
        rename = "showDeleted",
        deserialize_with = "optDeserializeGroupBoolean"
    )]
    pub show_deleted: Option<bool>,
}

const MAX_GROUP_OFFSET: i64 = 300;

pub async fn group_page(
    State(state): State<AppState>,
    method: Method,
    Path(group_urlname): Path<String>,
    Query(mut q): Query<ForumGroupQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    headers: HeaderMap,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    stRequest: Request,
) -> Result<Response> {
    if method == Method::POST {
        let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
        if let Some(sValue) = crate::form::get(&vecParameters, "showDeleted") {
            q.show_deleted = Some(bGroupFormBoolean(sValue, "showDeleted")?);
        }
        if let Some(sValue) = crate::form::get(&vecParameters, "offset") {
            q.offset = Some(iGroupOffset(sValue)?);
        }
        if let Some(sValue) = crate::form::get(&vecParameters, "lastmod") {
            q.lastmod = Some(bGroupFormBoolean(sValue, "lastmod")?);
        }
        if let Some(sValue) = crate::form::get(&vecParameters, "showignored") {
            q.show_ignored = Some(bGroupFormBoolean(sValue, "showignored")?);
        }
        if let Some(sValue) = crate::form::get(&vecParameters, "tag") {
            q.tag = Some(sValue.to_owned());
        }
    }
    let offset = q.offset.unwrap_or(0);
    // GroupController resolves section/group before authorization, offset and
    // tag semantics, so an unknown group remains a 404 for every valid bind.
    let group = find_group_by_section(&state, "forum", &group_urlname).await?;
    let group_id = group.id;

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
            return Ok(crate::routes::stFoundRedirect(format!(
                "/forum/{}",
                urlencoding::encode(&group_urlname)
            )));
        }
    }
    let show_deleted = show_deleted_requested;

    // A permitted GET showDeleted request redirects before isFirstPage runs
    // in Java. POST and ordinary GET requests validate/limit the offset here.
    if offset < 0 {
        return Err(AppError::BadParameter(
            "Bad format of 'offset' offset не может быть отрицательным".into(),
        ));
    }
    if offset > MAX_GROUP_OFFSET {
        return Ok(crate::routes::stFoundRedirect(format!(
            "/forum/{}/archive",
            urlencoding::encode(&group_urlname)
        )));
    }

    // TagService.getTagInfo(tag, skipZero=true): the request parameter is
    // significant even when it is empty, TagName validation happens before
    // the lookup, and TagDao performs an exact (case-sensitive) canonical-tag
    // lookup while hiding zero-counter rows.  Synonyms are not resolved here.
    let (tag_id, tag_name) = match q.tag.as_deref() {
        None => (None, None),
        Some(tag) if crate::routes::tags::is_good_tag(tag) => {
            let optTag = sqlx::query_as::<_, (i32, String)>(
                "SELECT id,value FROM tags_values WHERE value=$1 AND counter>0",
            )
            .bind(tag)
            .fetch_optional(&state.pool)
            .await?;
            let Some((iTagId, sCanonicalName)) = optTag else {
                return Err(AppError::NotFound);
            };
            (Some(iTagId), Some(sCanonicalName))
        }
        Some(_) => return Err(AppError::NotFound),
    };
    let dtActiveTagsDeadline = crate::search_index::dtActiveTagsDeadline();
    let optActiveTagsTask = if group.id == 4068 {
        None
    } else {
        Some(crate::search_index::hSpawnActiveTopTags(
            state.clone(),
            "forum".to_owned(),
            Some(group.urlname.clone()),
            dtActiveTagsDeadline,
        ))
    };

    let show_ignored = q.show_ignored.unwrap_or(false);
    let lastmod = q.lastmod.unwrap_or(false);
    // getGroupTrackerTopics hardcodes showIgnored=false even though the
    // controller keeps the requested value in the JSP model. Normal group
    // mode alone honours showignored=true for its data query.
    let ignore_user_id =
        optGroupQueryIgnoreUserId(user.as_ref().map(|stUser| stUser.id), lastmod, show_ignored);
    // Likewise the tracker DAO always hides deleted topics while the model
    // still reflects the submitted showDeleted flag.
    let bQueryShowDeleted = bGroupQueryShowDeleted(lastmod, show_deleted);
    let bViewerAuthorized = user.is_some();

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

    let order_by = if lastmod {
        "GREATEST(t.postdate,COALESCE(lc_visible.postdate,t.postdate)) DESC"
    } else {
        "t.postdate DESC"
    };
    let date_filter = if lastmod {
        "t.lastmod > now() - interval '6 months' AND \
         (t.postdate > now() - interval '6 months' OR \
          lc_visible.postdate > now() - interval '6 months')"
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
           WHERE t.groupid=$1 AND ($7 OR NOT t.sticky) AND (NOT t.deleted OR $5) AND NOT t.draft
             AND (t.moderate OR NOT s.moderate) AND {date_filter}
             AND ($8 OR t.open_warnings <= 2)
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
        .bind(bQueryShowDeleted)
        .bind(ignore_user_id)
        .bind(lastmod)
        .bind(bViewerAuthorized)
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
                 AND ($3 OR t.open_warnings <= 2)
               ORDER BY t.postdate DESC
               LIMIT 100"#;
        let mut sticky = sqlx::query_as::<_, TopicSummary>(sticky_sql)
            .bind(group_id)
            .bind(tag_id)
            .bind(bViewerAuthorized)
            .fetch_all(&state.pool)
            .await?;
        sticky.extend(topics);
        topics = sticky;
    }

    let topics =
        vecPrepareGroupTopics(&state, topics, ignore_user_id, stProfile.messages, lastmod).await?;

    let restriction: i32 = sqlx::query_scalar("SELECT GREATEST(COALESCE(g.restrict_topics,-9999),COALESCE(s.restrict_topics,-9999)) FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1")
        .bind(group_id).fetch_one(&state.pool).await?;
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let add_reason =
        crate::routes::topics::posting_reason_for_port(&state, restriction, &user, &sRemoteIp)
            .await?;
    let tag_suffix = tag_name
        .as_deref()
        .map(|tag| format!("tag={}", urlencoding::encode(tag)));
    let new_url = sGroupNavigationUrl(&group_urlname, false, tag_suffix.as_deref());
    let active_url = sGroupNavigationUrl(&group_urlname, true, tag_suffix.as_deref());
    let archive_url = format!("/forum/{}/archive", urlencoding::encode(&group_urlname));
    let add_url = add_reason.is_none().then(|| {
        let mut url = format!("/add.jsp?group={group_id}");
        if let Some(tag) = tag_name.as_deref() {
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
            url: sGroupQuickUrl(&item.urlname, lastmod),
            selected: item.id == group_id,
        })
        .collect();
    let prev_url = (offset > 0).then(|| {
        sGroupPagerUrl(
            &group_urlname,
            lastmod,
            tag_suffix.as_deref(),
            Some((offset - limit).max(0)),
            show_ignored,
        )
    });
    let next_url = bGroupHasNext(offset, main_topics_count, limit).then(|| {
        sGroupPagerUrl(
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
    let show_deleted_button =
        !lastmod && crate::routes::topics::can_view_all_deleted_topics(&state, &user).await?;
    let frozen_until = if let Some(stUser) = user.as_ref() {
        sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT frozen_until FROM users WHERE id=$1",
        )
        .bind(stUser.id)
        .fetch_optional(&state.pool)
        .await?
        .flatten()
        .filter(|dtFrozenUntil| *dtFrozenUntil > chrono::Utc::now())
    } else {
        None
    };
    let active_tags = match optActiveTagsTask {
        None => Vec::new(),
        Some(hTask) => match crate::search_index::enJoinActiveTagsTask(hTask).await {
            crate::search_index::EnActiveTagsTaskOutcome::Tags(vecTags) => vecTags
                .into_iter()
                .map(|name| crate::routes::topics::ActiveTagLink {
                    url: crate::application::tag::sTagSectionUrl(&name, group.section, 0),
                    name,
                })
                .collect(),
            crate::search_index::EnActiveTagsTaskOutcome::SearchError(error)
            | crate::search_index::EnActiveTagsTaskOutcome::JoinError(error) => {
                tracing::warn!(%error, group = %group.urlname, "unable to find active group tags");
                Vec::new()
            }
            crate::search_index::EnActiveTagsTaskOutcome::TimedOut => {
                tracing::warn!(group = %group.urlname, "active group tags search timed out");
                Vec::new()
            }
        },
    };
    let tag_title = tag_name
        .as_deref()
        .map(capitalize_first)
        .unwrap_or_default();
    let tag_url = tag_name.as_deref().map_or_else(String::new, |tag| {
        format!("/tag/{}", urlencoding::encode(tag))
    });
    let title = tag_name.as_ref().map_or(title, |tag| {
        format!("Форум — {} (тег {})", group.title, capitalize_first(tag))
    });
    let group_url = format!("/forum/{}", urlencoding::encode(&group_urlname));

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
            next_offset: offset + limit,
            is_moderator,
            old_tracker: stProfile.old_tracker,
            show_ignored,
            show_deleted,
            show_deleted_button,
            csrf_token,
            frozen_until,
            active_tags,
            tag_name,
            tag_title,
            tag_url,
            group_url,
            first_page: offset == 0,
        }
        .render()?,
    )
    .into_response())
}

fn bGroupFormBoolean(sValue: &str, sName: &str) -> Result<bool> {
    match sValue.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" | "" => Ok(false),
        _ => Err(AppError::BadParameter(format!("Bad format of '{sName}'"))),
    }
}

fn optDeserializeGroupBoolean<'de, D>(
    stDeserializer: D,
) -> std::result::Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let optValue = Option::<String>::deserialize(stDeserializer)?;
    optValue
        .map(|sValue| {
            bGroupFormBoolean(&sValue, "boolean")
                .map_err(|_| serde::de::Error::custom("invalid boolean parameter"))
        })
        .transpose()
}

fn iGroupOffset(sValue: &str) -> Result<i64> {
    let sValue = sValue.trim();
    if sValue.is_empty() {
        return Ok(0);
    }
    sValue
        .parse::<i64>()
        .map_err(|_| AppError::BadParameter("Bad format of 'offset'".into()))
}

fn optDeserializeGroupOffset<'de, D>(
    stDeserializer: D,
) -> std::result::Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let optValue = Option::<String>::deserialize(stDeserializer)?;
    optValue
        .map(|sValue| {
            iGroupOffset(&sValue).map_err(|_| serde::de::Error::custom("invalid offset parameter"))
        })
        .transpose()
}

fn capitalize_first(sValue: &str) -> String {
    let mut chars = sValue.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

fn bGroupHasNext(iOffset: i64, iMainTopicCount: usize, iTopicsPerPage: i64) -> bool {
    iOffset < MAX_GROUP_OFFSET && iMainTopicCount as i64 == iTopicsPerPage
}

fn optGroupQueryIgnoreUserId(
    optViewerId: Option<i32>,
    bLastmod: bool,
    bShowIgnored: bool,
) -> Option<i32> {
    optViewerId.filter(|_| bLastmod || !bShowIgnored)
}

fn bGroupQueryShowDeleted(bLastmod: bool, bRequestedShowDeleted: bool) -> bool {
    bRequestedShowDeleted && !bLastmod
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

/// group-new.jsp New/Active navigation retains the selected tag, but never
/// carries the ignore filter or page offset into the other ordering mode.
fn sGroupNavigationUrl(sGroup: &str, bLastmod: bool, optTagQuery: Option<&str>) -> String {
    group_mode_url(sGroup, bLastmod, optTagQuery, None, false)
}

/// The quick group selector preserves only the New/Active ordering mode.
fn sGroupQuickUrl(sGroup: &str, bLastmod: bool) -> String {
    group_mode_url(sGroup, bLastmod, None, None, false)
}

/// Previous/next links retain all three group-list filters.
fn sGroupPagerUrl(
    sGroup: &str,
    bLastmod: bool,
    optTagQuery: Option<&str>,
    optOffset: Option<i64>,
    bShowIgnored: bool,
) -> String {
    group_mode_url(sGroup, bLastmod, optTagQuery, optOffset, bShowIgnored)
}

pub async fn group_archive(
    State(state): State<AppState>,
    Path(group_name): Path<String>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
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
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let add_reason =
        crate::routes::topics::posting_reason_for_port(&state, restriction, &user, &sRemoteIp)
            .await?;
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
        ForumGroupQuery, GroupTopicView, GroupTopicsTemplate, bGroupFormBoolean, bGroupHasNext,
        bGroupQueryShowDeleted, bNonTechGroup, optGroupQueryIgnoreUserId, optPreparedGroupLongInfo,
        sGroupNavigationUrl, sGroupPagerUrl, sGroupQuickUrl,
    };
    use crate::models::{Group, TopicSummary};
    use askama::Template;
    use chrono::{TimeZone, Utc};

    #[test]
    fn group_template_keeps_old_tracker_filters_and_status_controls() {
        let sTemplate = include_str!("../../templates/group_topics.html");
        for sFragment in [
            "{% if old_tracker %}",
            "class=\"message-table\"",
            "name=\"showignored\"",
            "name=\"showDeleted\" value=\"true\"",
            "name=\"csrf\" value=\"{{ csrf_token }}\"",
            "frozen_until",
            "active_tags",
            "topic_author_blocked",
            "last_author_blocked",
        ] {
            assert!(
                sTemplate.contains(sFragment),
                "missing original group DOM/status hook: {sFragment}"
            );
        }

        assert!(sTemplate.contains(
            "<form action=\"{{ group_url }}\" method=\"POST\">\n  <input type=\"hidden\" name=\"csrf\""
        ));
        assert!(
            sTemplate
                .contains("<input type=\"hidden\" name=\"offset\" value=\"{{ next_offset }}\">")
        );
        assert!(!sTemplate.contains("<form action=\"{{ url }}\" method=\"POST\">"));
        assert!(!sTemplate.contains("<input type=\"hidden\" name=\"tag\""));
    }

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
    fn group_links_preserve_only_the_parameters_used_by_group_new_jsp() {
        assert_eq!(
            sGroupNavigationUrl("linux-hardware", true, Some("tag=usb%20c")),
            "/forum/linux-hardware?lastmod=true&tag=usb%20c"
        );
        assert_eq!(
            sGroupQuickUrl("linux-hardware", true),
            "/forum/linux-hardware?lastmod=true"
        );
        assert_eq!(
            sGroupPagerUrl("linux-hardware", true, Some("tag=usb%20c"), Some(100), true,),
            "/forum/linux-hardware?lastmod=true&tag=usb%20c&showignored=true&offset=100"
        );
    }

    #[test]
    fn group_boolean_binding_matches_spring_custom_boolean_editor() {
        for sValue in ["true", "TRUE", " on ", "Yes", "1"] {
            assert!(bGroupFormBoolean(sValue, "lastmod").unwrap());
        }
        for sValue in ["false", "FALSE", " off ", "No", "0", ""] {
            assert!(!bGroupFormBoolean(sValue, "lastmod").unwrap());
        }
        for sValue in ["t", "f", "maybe"] {
            assert!(bGroupFormBoolean(sValue, "lastmod").is_err());
        }

        let stAliases: ForumGroupQuery =
            serde_urlencoded::from_str("lastmod=YES&showignored=on&showDeleted=0").unwrap();
        assert_eq!(stAliases.lastmod, Some(true));
        assert_eq!(stAliases.show_ignored, Some(true));
        assert_eq!(stAliases.show_deleted, Some(false));

        let stDefaults: ForumGroupQuery =
            serde_urlencoded::from_str("offset=&lastmod=&showignored=&showDeleted=").unwrap();
        assert_eq!(stDefaults.offset, Some(0));
        assert_eq!(stDefaults.lastmod, Some(false));
        assert_eq!(stDefaults.show_ignored, Some(false));
        assert_eq!(stDefaults.show_deleted, Some(false));
        let stTrimmedOffset: ForumGroupQuery =
            serde_urlencoded::from_str("offset=%207%20").unwrap();
        assert_eq!(stTrimmedOffset.offset, Some(7));
        assert!(serde_urlencoded::from_str::<ForumGroupQuery>("offset=nope").is_err());
        assert!(serde_urlencoded::from_str::<ForumGroupQuery>("lastmod=t").is_err());
    }

    #[test]
    fn group_tag_filter_is_exact_nonzero_and_rejects_an_empty_parameter() {
        assert!(!crate::routes::tags::is_good_tag(""));
        let sSource = include_str!("groups.rs");
        let sHandler = sSource
            .split(concat!("pub async fn ", "group_page("))
            .nth(1)
            .unwrap()
            .split("fn bGroupFormBoolean")
            .next()
            .unwrap();
        assert!(sHandler.contains("SELECT id,value FROM tags_values WHERE value=$1 AND counter>0"));
        assert!(!sHandler.contains(concat!("lower(value)", "=lower($1)")));
        assert!(!sHandler.contains("tags_synonyms"));
        assert!(sHandler.contains("Some(_) => return Err(AppError::NotFound)"));
        assert!(sHandler.contains("Some(sCanonicalName)"));
    }

    #[test]
    fn active_tags_start_before_group_page_work_and_keep_the_java_deadline() {
        let sSource = include_str!("groups.rs");
        let sHandler = sSource
            .split(concat!("pub async fn ", "group_page("))
            .nth(1)
            .expect("group handler")
            .split("fn bGroupFormBoolean")
            .next()
            .expect("end of group handler");

        let iDeadline = sHandler
            .find("dtActiveTagsDeadline()")
            .expect("absolute active-tags deadline");
        let iSpawn = sHandler
            .find("hSpawnActiveTopTags(")
            .expect("underlying cache-warming task");
        let iProfile = sHandler
            .find("SELECT settings::text FROM user_settings")
            .expect("viewer profile query");
        let iTopics = sHandler
            .find("let mut topics = sqlx::query_as")
            .expect("main topic query");
        let iAwait = sHandler
            .find("enJoinActiveTagsTask(")
            .expect("deadline await");
        assert!(iDeadline < iSpawn && iSpawn < iProfile && iProfile < iTopics && iTopics < iAwait);
        assert!(!sHandler.contains("group.id == 4068 || offset != 0"));
        assert!(!sHandler.contains("tokio::time::timeout("));
    }

    #[test]
    fn lastmod_data_query_ignores_requested_deleted_and_ignore_overrides() {
        assert_eq!(optGroupQueryIgnoreUserId(Some(42), true, true), Some(42));
        assert_eq!(optGroupQueryIgnoreUserId(Some(42), false, true), None);
        assert_eq!(optGroupQueryIgnoreUserId(None, true, false), None);
        assert!(!bGroupQueryShowDeleted(true, true));
        assert!(bGroupQueryShowDeleted(false, true));

        let sSource = include_str!("groups.rs");
        let sHandler = sSource
            .split(concat!("pub async fn ", "group_page("))
            .nth(1)
            .unwrap()
            .split("fn bGroupFormBoolean")
            .next()
            .unwrap();
        assert!(sHandler.contains("AND ($7 OR NOT t.sticky)"));
        assert!(sHandler.contains("AND ($8 OR t.open_warnings <= 2)"));
        assert!(sHandler.contains("AND ($3 OR t.open_warnings <= 2)"));
        assert!(sHandler.contains(".bind(bQueryShowDeleted)"));
        assert!(sHandler.contains(".bind(bViewerAuthorized)"));
        assert!(sHandler.contains("t.lastmod > now() - interval '6 months'"));
        assert!(sHandler.contains("lc_visible.postdate > now() - interval '6 months'"));
        assert!(
            !sHandler.contains("COALESCE(t.lastmod, t.postdate) > now() - interval '6 months'")
        );
        let iGroupLookup = sHandler
            .find("let group = find_group_by_section")
            .expect("group lookup");
        let iDeletedCheck = sHandler
            .find("let show_deleted_requested")
            .expect("deleted flag check");
        let iOffsetValidation = sHandler.find("if offset < 0").expect("offset validation");
        assert!(iGroupLookup < iDeletedCheck);
        assert!(iDeletedCheck < iOffsetValidation);
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
                topic_author_blocked: false,
                last_author_blocked: false,
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
            next_offset: 50,
            is_moderator: false,
            old_tracker: false,
            show_ignored: false,
            show_deleted: false,
            show_deleted_button: false,
            csrf_token: "token".to_owned(),
            frozen_until: None,
            active_tags: Vec::new(),
            tag_name: None,
            tag_title: String::new(),
            tag_url: String::new(),
            group_url: "/forum/general".to_owned(),
            first_page: true,
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
