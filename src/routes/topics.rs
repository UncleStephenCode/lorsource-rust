use crate::{auth::CurrentUser, application::topic::CTopicService, domain::topic::repository::{StEditTopic, StNewTopic}, error::{AppError, Result}, infra::postgres::topic_repository::CTopicPgRepository, markup, models::{CommentItem, PagerQuery, TopicDetail, TopicSummary, UserSummary}, pagination::Pager, state::AppState};
use askama::Template;
use axum::{extract::{Path, Query, State}, http::Uri, response::{Html, IntoResponse, Redirect, Response}, Form};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    pager: Pager,
    current_user: Option<crate::models::UserSummary>,
}

#[derive(Debug, Clone)]
struct CommentPageLink {
    page: i64,
    current: bool,
}

#[derive(Template)]
#[template(path = "topic.html")]
struct TopicTemplate {
    topic: TopicDetail,
    topic_html: String,
    comments: Vec<CommentView>,
    current_user: Option<UserSummary>,
    /// Non-empty only outside thread/deleted mode, when there's more than
    /// one page of comments (TopicController.buildPages).
    pages: Vec<CommentPageLink>,
    thread_mode: bool,
    thread_root: Option<i32>,
    show_deleted: bool,
    /// Java's `showDeletedButton`: only a moderator viewing the live
    /// (non-deleted-mode) page gets offered the toggle.
    show_deleted_button: bool,
    /// Comments hidden by the viewer's ignore list in the current filtered
    /// view (TopicController's hideSet) - `unfiltered_count` is Java's
    /// `unfilteredCount`, used to render "N скрыто, показать".
    filtered_count: usize,
    unfiltered_count: usize,
    filter_show: bool,
    csrf_token: String,
}

#[derive(Debug, Clone)]
struct CommentView {
    item: CommentItem,
    html: String,
}

#[derive(Template)]
#[template(path = "topic_form.html")]
struct TopicFormTemplate {
    title: String,
    action: String,
    topic: Option<TopicDetail>,
    groups: Vec<crate::models::Group>,
    csrf_token: String,
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
    pub draft: Option<String>,
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

#[derive(Deserialize, Default)]
pub struct TopicViewQuery {
    pub cid: Option<i32>,
    /// Presence-based, like Java's `deleted != null` - any non-empty
    /// query string (`?deleted` or `?deleted=true`) requests the
    /// moderator-only deleted-comments view.
    pub deleted: Option<String>,
    /// "show" disables ignore-list-based comment hiding for this request.
    pub filter: Option<String>,
}

pub async fn topic_page(State(state): State<AppState>, uri: Uri, Path((group, id)): Path<(String, i32)>, Query(q): Query<TopicViewQuery>, CurrentUser(current_user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Response> {
    let section = section_from_uri(&uri).unwrap_or("forum");
    render_topic_view(state, section, group, id, 0, None, q, current_user, csrf_token).await
}

pub async fn topic_page_with_page(State(state): State<AppState>, uri: Uri, Path((group, id, page_marker)): Path<(String, i32, String)>, CurrentUser(current_user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Response> {
    let Some(page) = page_marker.strip_prefix("page") else { return Err(AppError::NotFound); };
    let page: i64 = page.parse().map_err(|_| AppError::NotFound)?;
    let section = section_from_uri(&uri).unwrap_or("forum");
    // Java's getMessagePage doesn't accept `cid`/`deleted`/`filter` at all -
    // only the base (page-less) route does.
    render_topic_view(state, section, group, id, page, None, TopicViewQuery::default(), current_user, csrf_token).await
}

pub async fn topic_thread(State(state): State<AppState>, uri: Uri, Path((group, id, thread_root)): Path<(String, i32, i32)>, CurrentUser(current_user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Response> {
    let section = section_from_uri(&uri).unwrap_or("forum");
    render_topic_view(state, section, group, id, 0, Some(thread_root), TopicViewQuery::default(), current_user, csrf_token).await
}

/// Called from legacy.rs's combined `/forum/{group}/{id_or_year}/{page_or_month}`
/// route once it's determined the third segment is `pageN`, not a year/month.
pub async fn render_topic_page(state: AppState, section: &'static str, group: String, id: i32, page: i64, current_user: Option<UserSummary>, csrf_token: String) -> Result<Response> {
    render_topic_view(state, section, group, id, page, None, TopicViewQuery::default(), current_user, csrf_token).await
}

async fn messages_per_page(state: &AppState, user: &Option<UserSummary>) -> i64 {
    match user {
        Some(u) => {
            let settings_text: Option<String> = sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                .bind(u.id)
                .fetch_optional(&state.pool)
                .await
                .ok()
                .flatten();
            crate::profile::ProfileSettings::from_hstore_text(settings_text).messages as i64
        }
        None => crate::profile::DEFAULT_MESSAGES as i64,
    }
}

/// TopicController.getMessageMain/getMessagePage/getMessageThread, merged
/// into one function parameterized by page/thread_root/query - mirrors
/// Java's shared private `getMessage` helper.
async fn render_topic_view(
    state: AppState,
    section: &'static str,
    group: String,
    id: i32,
    page: i64,
    thread_root: Option<i32>,
    query: TopicViewQuery,
    current_user: Option<UserSummary>,
    csrf_token: String,
) -> Result<Response> {
    let topic = get_topic(&state, id).await?;
    let is_moderator = current_user.as_ref().map(|u| u.canmod).unwrap_or(false);

    // GroupPermissionService.checkView / drafts: a draft or not-yet-committed
    // premoderated topic is only visible to its author or a moderator. A
    // deleted topic is likewise author/moderator-only - the previous
    // implementation never checked `topic.deleted` at all here, so a
    // deleted topic stayed fully visible to everyone.
    if topic.draft || topic.moderate || topic.deleted {
        let allowed = current_user.as_ref().map(|u| u.canmod || u.id == topic.author_id).unwrap_or(false);
        if !allowed {
            return Err(AppError::NotFound);
        }
    }

    // TopicController.getMessageMain: canonical redirect if the URL's
    // group/section don't match the topic's real ones.
    if topic.group_urlname != group || topic.section_prefix != section {
        return Ok(Redirect::to(&topic.topic_url()).into_response());
    }

    let want_deleted = query.deleted.is_some();
    if want_deleted && !is_moderator {
        return Ok(Redirect::to(&topic.topic_url()).into_response());
    }

    // `?cid=` jumps straight to the comment (resolving its page), bypassing
    // the rest of rendering entirely - matches Java's inline jumpMessage
    // short-circuit in getMessageMain. Only the base (page-less, non-thread)
    // route wires this in.
    if let Some(cid) = query.cid {
        if thread_root.is_none() && page == 0 {
            return resolve_comment_jump(&state, &topic, cid, is_moderator, &current_user).await;
        }
    }

    let topic_html = markup::render_message(&topic.message, topic.bbcode);

    let all_comments: Vec<CommentItem> = if want_deleted {
        topic_service(&state).vecListComments(id).await?
    } else {
        topic_service(&state).vecListComments(id).await?.into_iter().filter(|c| !c.deleted).collect()
    };

    // TopicController's hideSet: comments from ignored authors are dropped
    // from the rendered list (not just visually) unless `?filter=show`.
    let filter_show = query.filter.as_deref() == Some("show");
    let ignored_ids: Vec<i32> = match (&current_user, filter_show) {
        (Some(u), false) => sqlx::query_scalar("SELECT ignored FROM ignore_list WHERE userid=$1").bind(u.id).fetch_all(&state.pool).await.unwrap_or_default(),
        _ => vec![],
    };
    let unfiltered_count = all_comments.len();
    let visible_comments: Vec<CommentItem> = if ignored_ids.is_empty() {
        all_comments
    } else {
        all_comments.into_iter().filter(|c| !ignored_ids.contains(&c.author_id)).collect()
    };
    let filtered_count = visible_comments.len();

    let (page_comments, pages, thread_mode): (Vec<CommentItem>, Vec<CommentPageLink>, bool) = if let Some(root) = thread_root {
        let subtree = comment_subtree(&visible_comments, root);
        (subtree, vec![], true)
    } else if want_deleted {
        // Java's showDeleted path uses page=-1: render every comment on one
        // page, no pagination controls.
        (visible_comments, vec![], false)
    } else {
        let per_page = messages_per_page(&state, &current_user).await.max(1);
        let total_pages = (unfiltered_count as i64 + per_page - 1) / per_page.max(1);
        if page > 0 && page >= total_pages {
            let target_page = (total_pages - 1).max(0);
            let url = if target_page > 0 { format!("{}/page{target_page}", topic.topic_url()) } else { topic.topic_url() };
            return Ok(Redirect::to(&url).into_response());
        }
        let start = (page * per_page) as usize;
        let end = (start + per_page as usize).min(visible_comments.len());
        let slice = if start < visible_comments.len() { visible_comments[start..end].to_vec() } else { vec![] };
        let pages = if total_pages > 1 {
            (0..total_pages).map(|p| CommentPageLink { page: p, current: p == page }).collect()
        } else {
            vec![]
        };
        (slice, pages, false)
    };

    let comments: Vec<CommentView> = page_comments.into_iter().map(|item| CommentView { html: markup::render_message(&item.message, Some(true)), item }).collect();

    Ok(Html(TopicTemplate {
        topic,
        topic_html,
        comments,
        current_user,
        pages,
        thread_mode,
        thread_root,
        show_deleted: want_deleted,
        show_deleted_button: is_moderator && !want_deleted,
        filtered_count,
        unfiltered_count,
        filter_show,
        csrf_token,
    }.render()?).into_response())
}

/// Filters `comments` down to the subtree rooted at `root` (root itself
/// plus all descendants reachable through `replyto`), sorted by id -
/// matches CommentReadService.getCommentsSubtree.
fn comment_subtree(comments: &[CommentItem], root: i32) -> Vec<CommentItem> {
    let mut ids = std::collections::HashSet::new();
    ids.insert(root);
    // `comments` is ordered by postdate ASC, so a parent always appears
    // before its replies; a couple of passes is enough to reach a fixed
    // point without needing a real graph structure.
    loop {
        let before = ids.len();
        for c in comments {
            if let Some(parent) = c.replyto {
                if ids.contains(&parent) {
                    ids.insert(c.id);
                }
            }
        }
        if ids.len() == before {
            break;
        }
    }
    let mut subtree: Vec<CommentItem> = comments.iter().filter(|c| ids.contains(&c.id)).cloned().collect();
    subtree.sort_by_key(|c| c.id);
    subtree
}

/// TopicController's inline `jumpMessage(msgid, cid, skipDeleted)`: resolves
/// which page a comment lives on (among non-deleted comments) and redirects
/// there with a `#comment-N` anchor; falls back to the deleted-comments view
/// for a moderator if the comment isn't found live.
async fn resolve_comment_jump(state: &AppState, topic: &TopicDetail, cid: i32, is_moderator: bool, current_user: &Option<UserSummary>) -> Result<Response> {
    let live_comments: Vec<CommentItem> = topic_service(state).vecListComments(topic.id).await?.into_iter().filter(|c| !c.deleted).collect();
    if let Some(pos) = live_comments.iter().position(|c| c.id == cid) {
        let per_page = messages_per_page(state, current_user).await.max(1);
        let page = pos as i64 / per_page;
        let url = if page > 0 { format!("{}/page{page}#comment-{cid}", topic.topic_url()) } else { format!("{}#comment-{cid}", topic.topic_url()) };
        return Ok(Redirect::to(&url).into_response());
    }
    if is_moderator {
        let exists_deleted: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM comments WHERE id=$1 AND topic=$2 AND deleted)")
            .bind(cid)
            .bind(topic.id)
            .fetch_one(&state.pool)
            .await?;
        if exists_deleted {
            return Ok(Redirect::to(&format!("{}?deleted=true#comment-{cid}", topic.topic_url())).into_response());
        }
    }
    Err(AppError::NotFound)
}

pub async fn new_topic_form(State(state): State<AppState>, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    let groups = crate::routes::groups::list_groups(&state).await?;
    Ok(Html(TopicFormTemplate { title: "Новая тема".into(), action: "/add.jsp".into(), topic: None, groups, csrf_token }.render()?))
}

/// AddTopicController.MaxMessageLength (anonymous posting isn't supported by
/// Rust's session model, so only the registered-user limit applies).
const TOPIC_MAX_MESSAGE_LENGTH: usize = 65536;

pub async fn create_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if form.msg.chars().count() > TOPIC_MAX_MESSAGE_LENGTH {
        return Err(AppError::BadRequest("Слишком большое сообщение".into()));
    }
    if form.title.trim().is_empty() {
        return Err(AppError::BadRequest("заголовок сообщения не может быть пустым".into()));
    }
    let is_draft = form.draft.as_deref().is_some_and(|v| v == "true" || v == "on" || v == "1");
    let premoderated: bool = sqlx::query_scalar("SELECT s.moderate FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1")
        .bind(form.group)
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or(false);

    let mut tx = state.pool.begin().await?;
    let service = topic_service(&state);
    let id = service.iNextMessageId(&mut tx).await?;
    service.vInsertTopicMessage(&mut tx, id, &form.msg).await?;
    service.vInsertTopic(&mut tx, StNewTopic {
        iMsgId: id,
        iGroupId: form.group,
        iUserId: user.id,
        sTitle: form.title.trim(),
        optUrl: form.url.as_deref().filter(|sValue| !sValue.trim().is_empty()),
        optLinkText: form.linktext.as_deref().filter(|sValue| !sValue.trim().is_empty()),
        bDraft: is_draft,
        bPremoderated: premoderated,
    }).await?;
    service.vReplaceTags(&mut tx, id, form.tags.as_deref()).await?;
    tx.commit().await?;
    notify_topic_created(&state, id, user.id, &form.msg).await?;
    crate::search_index::index_topic(&state, id, false).await;
    // The topic-view gate (render_topic) lets the author through even while
    // draft/pending, so redirecting straight to the topic works for both
    // cases - Java instead shows a dedicated "add-done-moderated" interim
    // page for the premoderated case, which isn't replicated here.
    let topic = get_topic(&state, id).await?;
    Ok(Redirect::to(&topic.topic_url()))
}

pub async fn edit_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    let topic = get_topic(&state, q.msgid).await?;
    let groups = crate::routes::groups::list_groups(&state).await?;
    Ok(Html(TopicFormTemplate { title: "Редактировать тему".into(), action: "/edit.jsp".into(), topic: Some(topic), groups, csrf_token }.render()?))
}

/// Simplified from EditTopicChecker.checkContentEdit/checkEditByAuthor:
/// author (or moderator, unconditional bypass) may edit within a 14-day
/// window from posting, or at any time while still a draft. The corrector
/// role, premoderated-section/articles commitDate nuances, and the
/// postscore==NO_COMMENTS lock aren't modeled by Rust's session yet - this
/// intentionally errs toward Java's baseline author/moderator gate rather
/// than leaving the endpoint wide open.
const TOPIC_EDIT_WINDOW_DAYS: i64 = 14;

pub async fn edit_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let id = form.id.ok_or_else(|| AppError::BadRequest("missing topic id".into()))?;
    let meta = load_topic_delete_meta(&state, id).await?;
    if meta.deleted {
        return Err(AppError::BadRequest("нельзя править удаленные топики".into()));
    }
    let editable_by_author = meta.author_id == user.id
        && (meta.draft || chrono::Utc::now() <= meta.postdate + chrono::Duration::days(TOPIC_EDIT_WINDOW_DAYS));
    if !user.canmod && !editable_by_author {
        return Err(AppError::Forbidden);
    }

    let mut tx = state.pool.begin().await?;
    let service = topic_service(&state);
    service.vUpdateTopicMessage(&mut tx, id, &form.msg).await?;
    service.vUpdateTopicHeader(&mut tx, StEditTopic {
        iMsgId: id,
        sTitle: form.title.trim(),
        optUrl: form.url,
        optLinkText: form.linktext,
    }).await?;
    service.vReplaceTags(&mut tx, id, form.tags.as_deref()).await?;
    tx.commit().await?;
    crate::search_index::index_topic(&state, id, false).await;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")))
}

#[derive(Deserialize)]
pub struct TopicActionForm { pub msgid: i32, pub resolve: Option<String>, pub reason: Option<String>, pub bonus: Option<i32> }

/// Matches GroupPermissionService.DeletePeriod: an author may delete their
/// own (non-draft, non-premoderated-and-committed) topic for 3 hours after
/// posting, and only while it has no comments. Moderators bypass this.
const TOPIC_DELETE_WINDOW_HOURS: i64 = 3;

struct TopicDeleteMeta {
    author_id: i32,
    deleted: bool,
    postdate: chrono::DateTime<chrono::Utc>,
    draft: bool,
    premoderated: bool,
    commited: bool,
    comment_count: i64,
}

async fn load_topic_delete_meta(state: &AppState, msgid: i32) -> Result<TopicDeleteMeta> {
    let row: (i32, bool, chrono::DateTime<chrono::Utc>, bool, bool, bool, i64) = sqlx::query_as(
        r#"SELECT t.userid, t.deleted, t.postdate, COALESCE(t.draft,false), s.moderate,
                  (t.commitdate IS NOT NULL), t.stat1::bigint
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(TopicDeleteMeta {
        author_id: row.0,
        deleted: row.1,
        postdate: row.2,
        draft: row.3,
        premoderated: row.4,
        commited: row.5,
        comment_count: row.6,
    })
}

pub async fn delete_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let meta = load_topic_delete_meta(&state, form.msgid).await?;
    if meta.deleted {
        return Err(AppError::BadRequest("сообщение уже удалено".into()));
    }

    let deletable_by_author = meta.author_id == user.id && (
        meta.draft || (
            !(meta.premoderated && meta.commited)
                && meta.comment_count == 0
                && chrono::Utc::now() <= meta.postdate + chrono::Duration::hours(TOPIC_DELETE_WINDOW_HOURS)
        )
    );
    if !user.canmod && !deletable_by_author {
        return Err(AppError::Forbidden);
    }

    let bonus = if user.canmod && user.id != meta.author_id && !meta.draft {
        form.bonus.unwrap_or(0).clamp(0, 20)
    } else {
        0
    };
    let reason = form.reason.clone().unwrap_or_default();

    topic_service(&state).vSetDeleted(form.msgid, true).await?;
    sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES($1,$2,$3,now(),$4) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now(), bonus=EXCLUDED.bonus")
        .bind(form.msgid).bind(user.id).bind(&reason).bind(bonus).execute(&state.pool).await?;
    if bonus != 0 {
        sqlx::query("UPDATE users SET score=GREATEST(score-$2,0) WHERE id=$1").bind(meta.author_id).bind(bonus).execute(&state.pool).await?;
    }
    crate::routes::comments::notify_deleted(&state, meta.author_id, user.id, Some(form.msgid), None, &reason).await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    Ok(Redirect::to("/"))
}

pub async fn undelete_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if !user.canmod {
        return Err(AppError::Forbidden);
    }
    let meta = load_topic_delete_meta(&state, form.msgid).await?;
    if !meta.deleted {
        return Err(AppError::BadRequest("сообщение не удалено".into()));
    }
    topic_service(&state).vSetDeleted(form.msgid, false).await?;
    sqlx::query("DELETE FROM del_info WHERE msgid=$1").bind(form.msgid).execute(&state.pool).await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    // Java: `new ModelAndView(new RedirectView(topic.getLink))` - a topic
    // (not a comment), so no ?cid= here.
    let topic = get_topic(&state, form.msgid).await?;
    Ok(Redirect::to(&topic.topic_url()))
}

pub async fn resolve_topic_get(State(state): State<AppState>, Query(form): Query<TopicActionForm>, CurrentUser(user): CurrentUser) -> Result<Redirect> {
    do_resolve_topic(&state, user, form).await
}

pub async fn resolve_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    do_resolve_topic(&state, user, form).await
}

async fn do_resolve_topic(state: &AppState, user: Option<crate::models::UserSummary>, form: TopicActionForm) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let Some((author_id, group_resolvable)) = topic_service(state).optResolveMeta(form.msgid).await? else {
        return Err(AppError::NotFound);
    };
    if !group_resolvable {
        return Err(AppError::Forbidden);
    }
    if !user.canmod && user.id != author_id {
        return Err(AppError::Forbidden);
    }
    let resolved = form.resolve.as_deref().map(|value| value == "yes");
    topic_service(state).vSetResolved(form.msgid, resolved).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn list_topics(state: &AppState, section: Option<&str>, group: Option<&str>, offset: i64, limit: i64) -> Result<Vec<TopicSummary>> {
    topic_service(state).vecListTopics(section, group, offset, limit).await
}

pub async fn get_topic(state: &AppState, id: i32) -> Result<TopicDetail> {
    topic_service(state).stGetTopic(id).await
}


fn topic_service(state: &AppState) -> CTopicService<CTopicPgRepository> {
    CTopicService::new(CTopicPgRepository::new(state.pool.clone()))
}

/// TopicService.sendEvents: on topic creation, notify (a) users mentioned
/// via @nick in the body (REF), and (b) users who favorited one of the
/// topic's tags (TAG) - excluding anyone already notified via (a), matching
/// Java's `tagUsers.filterNot(userRefIds.contains)`. Java also records
/// `topic_users_notified` to suppress duplicate notifications across a
/// later edit; this port doesn't re-run sendEvents on edit at all, so that
/// bookkeeping table is skipped as unnecessary for now.
async fn notify_topic_created(state: &AppState, topic_id: i32, author_id: i32, message: &str) -> Result<()> {
    let mentioned_nicks = markup::extract_mentions(message);
    let mut notified: Vec<i32> = if mentioned_nicks.is_empty() {
        vec![]
    } else {
        sqlx::query_scalar(
            r#"SELECT u.id FROM users u
               WHERE lower(u.nick) = ANY($1) AND u.id <> $2
                 AND NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=u.id AND il.ignored=$2)"#,
        )
        .bind(mentioned_nicks.iter().map(|n| n.to_lowercase()).collect::<Vec<_>>())
        .bind(author_id)
        .fetch_all(&state.pool)
        .await?
    };
    for &mentioned_id in &notified {
        sqlx::query("INSERT INTO user_events(userid,type,private,message_id) VALUES($1,'REF',false,$2)")
            .bind(mentioned_id)
            .bind(topic_id)
            .execute(&state.pool)
            .await?;
    }

    let tag_favoriters: Vec<i32> = sqlx::query_scalar(
        r#"SELECT DISTINCT ut.userid FROM user_tags ut
           JOIN tags tg ON tg.tagid=ut.tag_id
           WHERE tg.msgid=$1 AND ut.is_favorite AND ut.userid<>$2 AND NOT ut.userid=ANY($3)"#,
    )
    .bind(topic_id)
    .bind(author_id)
    .bind(&notified)
    .fetch_all(&state.pool)
    .await?;
    for &tag_userid in &tag_favoriters {
        sqlx::query("INSERT INTO user_events(userid,type,private,message_id) VALUES($1,'TAG',false,$2)")
            .bind(tag_userid)
            .bind(topic_id)
            .execute(&state.pool)
            .await?;
    }
    notified.extend(tag_favoriters);

    if !notified.is_empty() {
        notified.sort_unstable();
        notified.dedup();
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=ANY($1)")
            .bind(&notified)
            .execute(&state.pool)
            .await?;
    }
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

pub async fn delete_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Удалить тему #{}</h1>
<form method="post" action="/delete.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Удалить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn undelete_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Восстановить тему #{}</h1>
<form method="post" action="/undelete">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Восстановить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn commit_topic_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Подтвердить тему #{}</h1>
<form method="post" action="/commit.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Подтвердить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn commit_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if !user.canmod { return Err(AppError::Forbidden); }
    topic_service(&state).vCommitTopic(form.msgid, user.id).await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn uncommit_form(Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Отменить подтверждение темы #{}</h1>
<form method="post" action="/uncommit.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Отменить подтверждение</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn uncommit(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<TopicActionForm>) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    topic_service(&state).vUncommitTopic(form.msgid).await?;
    crate::search_index::index_topic(&state, form.msgid, true).await;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

#[derive(Deserialize)]
pub struct MoveTopicForm { pub msgid: i32, pub moveto: i32 }

pub async fn move_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
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
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <select name="moveto">{}</select>
  <button type="submit">Переместить</button>
</form>
"#, q.msgid, q.msgid, options)))
}

pub async fn move_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<MoveTopicForm>) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    topic_service(&state).vMoveTopic(form.msgid, form.moveto).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn premoderated_move_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, user: CurrentUser, csrf: crate::csrf::CsrfToken) -> Result<Html<String>> {
    move_topic_form(State(state), Query(q), user, csrf).await
}
