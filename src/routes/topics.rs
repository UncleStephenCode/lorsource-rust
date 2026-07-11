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
    poll: Option<PollView>,
    image: Option<TopicImageView>,
    /// Shown to the author/moderator of an imagepost (gallery) topic that
    /// has no main image yet.
    show_add_image_link: bool,
    topic_reactions_html: String,
}

#[derive(Debug, Clone)]
struct CommentView {
    item: CommentItem,
    html: String,
    reactions_html: String,
}

/// poll-form.tag rendered server-side: a topic's poll (if any), with vote
/// counts/percentages and whether the current viewer may still vote.
/// `can_vote` doesn't pre-check expiry (Topic.expired isn't loaded by
/// `get_topic` here) - `/vote.jsp` itself still rejects an expired poll,
/// so a stale "Голосовать" button just surfaces that error instead of
/// silently vanishing a beat early.
#[derive(Debug, Clone)]
struct PollView {
    voteid: i32,
    multiselect: bool,
    variants: Vec<PollVariantView>,
    total_votes: i32,
    user_voted: bool,
    can_vote: bool,
}

#[derive(Debug, Clone)]
struct PollVariantView {
    id: i32,
    label: String,
    votes: i32,
    pct: i32,
}

/// ImageService/Image.getMedium: the topic's main gallery image, if any.
struct TopicImageView {
    medium_url: String,
    original_url: String,
    width: i32,
    height: i32,
}

async fn load_topic_image(state: &AppState, topic_id: i32) -> Result<Option<TopicImageView>> {
    let row: Option<(String, String, i32, i32)> = sqlx::query_as(
        "SELECT medium, original, width, height FROM images WHERE topic=$1 AND primary_image AND NOT deleted LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|(medium, original, width, height)| TopicImageView {
        medium_url: format!("/gallery-uploads/{medium}"),
        original_url: format!("/gallery-uploads/{original}"),
        width,
        height,
    }))
}

/// ReactionService.allowInteract: logged in, not frozen, not the target's
/// own author, target (topic/comment) not deleted, topic not expired, and
/// (for a comment) the topic's comments aren't hidden.
#[allow(clippy::too_many_arguments)]
fn reactions_allow_interact(current_user: &Option<UserSummary>, frozen: bool, topic_expired: bool, target_author_id: i32, target_deleted: bool, comments_hidden: bool) -> bool {
    match current_user {
        Some(u) => u.id != target_author_id && !frozen && !topic_expired && !target_deleted && !comments_hidden,
        None => false,
    }
}

/// reactions.tag rendered server-side as a raw HTML fragment (consistent
/// with this port's other hand-built widgets, e.g. `/notifications`) -
/// non-zero reactions are always shown, all-zero ones collapse behind a
/// "»" toggle (or the whole widget is hidden if every reaction is at zero).
/// reactions.tag's exact nesting turns out to leave brand-new content (zero
/// reactions ever) with no discoverable way to add the first one at all:
/// the outer div gets `class="zero-reactions"` (CSS `display: none`) purely
/// from `emptyMap`, and the only two things that could reveal it - the "?"
/// full-log link and the "»" collapse toggle - are *both* separately gated
/// on `not emptyMap` too. Reproducing that literally would make this port's
/// inline widget just as unreachable for every new post, which defeats the
/// point of adding it, so this renders every button whenever the viewer may
/// interact (no zero-count collapsing) instead of replicating that gate.
fn render_reactions_widget(
    topic_id: i32,
    comment_id: Option<i32>,
    reaction_users: &[(String, i32, String, i32)],
    current_user_id: Option<i32>,
    allow_interact: bool,
    csrf_token: &str,
) -> String {
    let is_anonymous = current_user_id.is_none();
    let anon_class = if is_anonymous { " reaction-anonymous" } else { "" };
    let disabled = if allow_interact { "" } else { " disabled" };

    struct Btn { emoji: String, count: i64, clicked: bool, tooltip: String }
    let mut buttons = Vec::new();
    for (emoji, description) in crate::routes::api::REACTIONS {
        let mut users: Vec<&(String, i32, String, i32)> = reaction_users.iter().filter(|(r, ..)| r == emoji).collect();
        users.sort_by(|a, b| b.3.cmp(&a.3));
        let count = users.len() as i64;
        let clicked = current_user_id.map(|uid| users.iter().any(|(_, u, ..)| *u == uid)).unwrap_or(false);
        let top: Vec<&str> = users.iter().take(3).map(|(_, _, nick, _)| nick.as_str()).collect();
        let more = if users.len() > 3 { "..." } else { "" };
        let tooltip = format!("Реакция \"{description}\": {}{more}", top.join(" "));
        buttons.push(Btn { emoji: emoji.to_string(), count, clicked, tooltip });
    }

    let mut html = format!(
        "<div class=\"reactions\"><form class=\"reactions-form\" action=\"/reactions\" method=\"post\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"topic\" value=\"{topic_id}\">",
        html_escape::encode_text(csrf_token),
    );
    if let Some(cid) = comment_id {
        html.push_str(&format!("<input type=\"hidden\" name=\"comment\" value=\"{cid}\">"));
    }
    for b in &buttons {
        if b.count == 0 && !allow_interact {
            // A viewer who can't react at all (anonymous, the target's own
            // author, ...) only sees reactions someone has actually left.
            continue;
        }
        let value = format!("{}-{}", b.emoji, !b.clicked);
        let clicked_class = if b.clicked { " btn-primary" } else { "" };
        html.push_str(&format!(
            "<button name=\"reaction\" value=\"{}\" class=\"reaction{clicked_class}{anon_class}\" title=\"{}\"{disabled}>{} <span class=\"reaction-count\">{}</span></button>",
            html_escape::encode_text(&value), html_escape::encode_text(&b.tooltip), html_escape::encode_text(&b.emoji), b.count,
        ));
    }
    html.push_str("</form></div>");
    html
}

/// All reactions for the topic in one query (topic-level rows have
/// `comment_id IS NULL`), so per-comment widgets don't each hit the DB.
async fn load_all_reactions(state: &AppState, topic_id: i32) -> Result<Vec<(Option<i32>, String, i32, String, i32)>> {
    Ok(sqlx::query_as(
        r#"SELECT rl.comment_id, rl.reaction, rl.origin_user, u.nick, COALESCE(u.score,0)
           FROM reactions_log rl JOIN users u ON u.id=rl.origin_user
           WHERE rl.topic_id=$1"#,
    )
    .bind(topic_id)
    .fetch_all(&state.pool)
    .await?)
}

async fn load_poll_view(state: &AppState, topic_id: i32, deleted: bool, current_user: &Option<UserSummary>) -> Result<Option<PollView>> {
    let Some((poll_id, multiselect)): Option<(i32, bool)> = sqlx::query_as("SELECT id, multiselect FROM polls WHERE topic=$1").bind(topic_id).fetch_optional(&state.pool).await? else {
        return Ok(None);
    };
    let rows: Vec<(i32, String, i32)> = sqlx::query_as("SELECT id, label, votes FROM polls_variants WHERE vote=$1 ORDER BY id").bind(poll_id).fetch_all(&state.pool).await?;
    let total_votes: i32 = rows.iter().map(|(_, _, v)| *v).sum();
    let variants = rows.into_iter().map(|(id, label, votes)| PollVariantView {
        id, label, votes,
        pct: if total_votes > 0 { (votes * 100) / total_votes } else { 0 },
    }).collect();
    let user_voted = match current_user {
        Some(u) => sqlx::query_scalar::<_, i64>("SELECT count(*) FROM vote_users WHERE vote=$1 AND userid=$2").bind(poll_id).bind(u.id).fetch_one(&state.pool).await? > 0,
        None => false,
    };
    Ok(Some(PollView {
        voteid: poll_id,
        multiselect,
        variants,
        total_votes,
        user_voted,
        can_vote: current_user.is_some() && !user_voted && !deleted,
    }))
}

#[derive(Template)]
#[template(path = "topic_form.html")]
struct TopicFormTemplate {
    title: String,
    action: String,
    topic: Option<TopicDetail>,
    groups: Vec<crate::models::Group>,
    csrf_token: String,
    /// Existing (id, label) poll variants - empty for a brand-new topic.
    poll_variants: Vec<(i32, String)>,
    /// Number of blank "add new variant" rows to render: `Poll.MaxPollSize`
    /// (15) for a new topic, `EditTopicRequest.newPoll`'s size (3) for edit.
    poll_new_rows: Vec<()>,
    poll_multiselect: bool,
}

pub struct TopicForm {
    pub id: Option<i32>,
    pub group: i32,
    pub title: String,
    pub msg: String,
    pub url: Option<String>,
    pub linktext: Option<String>,
    pub tags: Option<String>,
    pub draft: Option<String>,
    /// Poll variant labels, positionally paired with `variant_id` - matches
    /// Java's AddTopicRequest.poll (create, all ids implicitly 0/new) and
    /// EditTopicRequest.poll+newPoll (edit: real ids for existing variants,
    /// 0 for new ones), just flattened into two parallel repeated-field
    /// vectors instead of a bracket-indexed map, since serde_urlencoded has
    /// no map/array-index syntax.
    pub poll: Vec<String>,
    pub variant_id: Vec<i32>,
    pub multiselect: Option<String>,
}

/// `axum::Form` can't deserialize the repeated `poll`/`variant_id` keys into
/// `Vec` fields (see `crate::form`), so this form is parsed from the raw
/// body by hand instead.
fn parse_topic_form(pairs: &[(String, String)]) -> Result<TopicForm> {
    use crate::form::{get, get_all};
    Ok(TopicForm {
        id: get(pairs, "id").and_then(|v| v.parse().ok()),
        group: get(pairs, "group").and_then(|v| v.parse().ok()).ok_or_else(|| AppError::BadRequest("missing group".into()))?,
        title: get(pairs, "title").unwrap_or("").to_string(),
        msg: get(pairs, "msg").unwrap_or("").to_string(),
        url: get(pairs, "url").map(|s| s.to_string()),
        linktext: get(pairs, "linktext").map(|s| s.to_string()),
        tags: get(pairs, "tags").map(|s| s.to_string()),
        draft: get(pairs, "draft").map(|s| s.to_string()),
        poll: get_all(pairs, "poll").into_iter().map(|s| s.to_string()).collect(),
        variant_id: get_all(pairs, "variant_id").into_iter().filter_map(|s| s.parse().ok()).collect(),
        multiselect: get(pairs, "multiselect").map(|s| s.to_string()),
    })
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

const VIEW_ALL_SECTION_PREFIX_CASE: &str = "CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END";
const GALLERY_SECTION_ID: i32 = 3;

#[derive(Debug, Clone, sqlx::FromRow)]
struct ViewAllSection {
    id: i32,
    name: String,
    restrict_score: i32,
    section_prefix: String,
}

impl ViewAllSection {
    /// Section.uncommitedName
    fn uncommited_name(&self) -> String {
        if self.id == GALLERY_SECTION_ID { "Неподтверждённые галереи".to_string() } else { format!("Неподтверждённые {}", self.name.to_lowercase()) }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct DeletedTopicRow {
    subj: String,
    nick: String,
    msgid: i32,
    reason: Option<String>,
    postdate: chrono::DateTime<chrono::Utc>,
    deldate: Option<chrono::NaiveDateTime>,
    bonus: Option<i32>,
}

impl DeletedTopicRow {
    fn reason_display(&self) -> &str {
        self.reason.as_deref().unwrap_or_default()
    }

    fn deldate_display(&self) -> String {
        self.deldate.map(|dt| dt.to_string()).unwrap_or_default()
    }
}

#[derive(Template)]
#[template(path = "view_all.html")]
struct ViewAllTemplate {
    title: String,
    section: Option<ViewAllSection>,
    uncommitted_counts: Vec<(ViewAllSection, i64)>,
    uncommitted: i64,
    add_link: Option<String>,
    add_link_reason: Option<String>,
    messages: Vec<TopicSummary>,
    deleted_topics: Vec<DeletedTopicRow>,
    show_dates: bool,
    show_gallery_notice: bool,
    current_user: Option<UserSummary>,
}

#[derive(Deserialize)]
pub struct ViewAllQuery {
    pub section: Option<i32>,
}

const POSTSCORE_UNRESTRICTED: i32 = -9999;
const POSTSCORE_REGISTERED_ONLY: i32 = -50;
const POSTSCORE_MODERATORS_ONLY: i32 = 10000;
const POSTSCORE_NO_COMMENTS: i32 = 10001;
const POSTSCORE_HIDE_COMMENTS: i32 = 10002;

/// Simplified AddTopicChecker.checkTopicPosting: only the postscore/karma
/// threshold and moderator-only/no-comments cases are modeled - frozen-user
/// and IP-block restrictions aren't checked here since this is just a UI
/// hint (the "Добавить" button), not an enforcement gate, and topic
/// creation itself doesn't enforce postscore restrictions yet either.
fn topic_posting_reason(restriction: i32, user: &Option<UserSummary>) -> Option<String> {
    let anonymous = user.is_none();
    let score = user.as_ref().and_then(|u| u.score).unwrap_or(0);
    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);
    match restriction {
        POSTSCORE_UNRESTRICTED => None,
        POSTSCORE_MODERATORS_ONLY => if is_moderator { None } else { Some("только для модераторов".to_string()) },
        POSTSCORE_REGISTERED_ONLY => if anonymous { Some("только для зарегистрированных".to_string()) } else { None },
        POSTSCORE_NO_COMMENTS | POSTSCORE_HIDE_COMMENTS => Some("постинг запрещен".to_string()),
        _ => if anonymous || score < restriction { Some(format!("только для зарегистрированных, score>={restriction}")) } else { None },
    }
}

/// UncommitedTopicsController/view-all.jsp: the premoderation queue -
/// public (no auth required, matching Java's `MaybeAuthorized`), lists
/// topics awaiting commit in premoderated sections plus recently deleted
/// ones, with an add-topic shortcut gated on posting permission.
pub async fn view_all(State(state): State<AppState>, Query(q): Query<ViewAllQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    let section: Option<ViewAllSection> = if let Some(sid) = q.section.filter(|&id| id != 0) {
        let sql = format!("SELECT s.id, s.name, s.restrict_score, {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix FROM sections s WHERE s.id=$1");
        Some(sqlx::query_as::<_, ViewAllSection>(&sql).bind(sid).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?)
    } else {
        None
    };

    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);

    let sql = format!(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix,
                  t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                  string_agg(tv.value, ',' ORDER BY tv.value) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN tags tg ON tg.msgid=t.id
           LEFT JOIN tags_values tv ON tv.id=tg.tagid
           WHERE NOT t.deleted AND NOT t.draft AND t.moderate AND s.moderate
             AND t.postdate >= now() - interval '3 months'
             AND ($1::int IS NULL OR s.id=$1)
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY t.postdate DESC"#
    );
    let messages = sqlx::query_as::<_, TopicSummary>(&sql).bind(section.as_ref().map(|s| s.id)).fetch_all(&state.pool).await?;
    let uncommitted = messages.len() as i64;

    let bad_reason_filter = if is_moderator { "" } else { "AND di.reason != '' AND di.reason NOT IN ('Блокировка пользователя с удалением сообщений','4.6 Спам')" };
    let sql = format!(
        r#"SELECT t.title AS subj, u.nick, t.id AS msgid, di.reason, t.postdate, di.deldate, di.bonus
           FROM topics t, groups g, users u, sections s, del_info di
           WHERE s.id=g.section AND t.userid=u.id AND t.groupid=g.id AND s.moderate AND t.deleted
             AND di.msgid=t.id AND t.userid != di.delby
             AND di.deldate > now() - interval '2 weeks'
             AND ($1::int IS NULL OR s.id=$1)
             {bad_reason_filter}
           ORDER BY di.deldate DESC LIMIT 20"#
    );
    let deleted_topics = sqlx::query_as::<_, DeletedTopicRow>(&sql).bind(section.as_ref().map(|s| s.id)).fetch_all(&state.pool).await?;

    let uncommitted_counts: Vec<(ViewAllSection, i64)> = if section.is_none() {
        let sql = format!(
            r#"SELECT s.id, s.name, s.restrict_score, {VIEW_ALL_SECTION_PREFIX_CASE} AS section_prefix, count(t.*) AS cnt
               FROM sections s
               JOIN groups g ON g.section=s.id
               JOIN topics t ON t.groupid=g.id
               WHERE s.moderate AND NOT t.draft AND NOT t.deleted AND t.moderate
                 AND t.postdate >= now() - interval '3 months'
               GROUP BY s.id
               ORDER BY s.id"#
        );
        sqlx::query_as::<_, (i32, String, i32, String, i64)>(&sql)
            .fetch_all(&state.pool)
            .await?
            .into_iter()
            .map(|(id, name, restrict_score, section_prefix, cnt)| (ViewAllSection { id, name, restrict_score, section_prefix }, cnt))
            .collect()
    } else {
        Vec::new()
    };

    let restriction = match &section {
        Some(s) => s.restrict_score,
        None => sqlx::query_scalar::<_, i32>("SELECT COALESCE(min(restrict_score), 0) FROM sections").fetch_one(&state.pool).await?,
    };
    let (add_link, add_link_reason) = match topic_posting_reason(restriction, &user) {
        None => (Some(match &section { Some(s) => format!("/add-section.jsp?section={}", s.id), None => "/add-section.jsp".to_string() }), None),
        Some(reason) => (None, Some(reason)),
    };

    let title = section.as_ref().map(|s| s.uncommited_name()).unwrap_or_else(|| "Просмотр неподтверждённых сообщений".to_string());
    let show_gallery_notice = section.as_ref().map(|s| s.id == GALLERY_SECTION_ID).unwrap_or(true);

    Ok(Html(ViewAllTemplate {
        title,
        section,
        uncommitted_counts,
        uncommitted,
        add_link,
        add_link_reason,
        messages,
        deleted_topics,
        show_dates: is_moderator,
        show_gallery_notice,
        current_user: user,
    }.render()?))
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
    let can_view_deleted_comments = allow_view_all_deleted_comments(&state, topic.id, &current_user).await?;
    if want_deleted && !can_view_deleted_comments {
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

    // ReactionService.allowInteract's expired/comments-hidden/frozen inputs,
    // fetched once up front so per-comment widgets don't each hit the DB.
    let (topic_expired, topic_postscore): (bool, i32) = sqlx::query_as(
        "SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire, t.postscore FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1",
    )
    .bind(topic.id)
    .fetch_one(&state.pool)
    .await?;
    let comments_hidden = topic_postscore == POSTSCORE_HIDE_COMMENTS;
    let reactor_frozen = match &current_user {
        Some(u) => sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>("SELECT frozen_until FROM users WHERE id=$1")
            .bind(u.id).fetch_one(&state.pool).await?.map(|t| t > chrono::Utc::now()).unwrap_or(false),
        None => false,
    };
    let all_reactions = load_all_reactions(&state, topic.id).await?;
    let current_user_id = current_user.as_ref().map(|u| u.id);

    let comments: Vec<CommentView> = page_comments.into_iter().map(|item| {
        let html = markup::render_message(&item.message, Some(true));
        let rows: Vec<(String, i32, String, i32)> = all_reactions.iter()
            .filter(|(cid, ..)| *cid == Some(item.id))
            .map(|(_, r, u, n, s)| (r.clone(), *u, n.clone(), *s))
            .collect();
        let allow_interact = reactions_allow_interact(&current_user, reactor_frozen, topic_expired, item.author_id, item.deleted, comments_hidden);
        let reactions_html = render_reactions_widget(topic.id, Some(item.id), &rows, current_user_id, allow_interact, &csrf_token);
        CommentView { item, html, reactions_html }
    }).collect();

    let topic_reaction_rows: Vec<(String, i32, String, i32)> = all_reactions.iter()
        .filter(|(cid, ..)| cid.is_none())
        .map(|(_, r, u, n, s)| (r.clone(), *u, n.clone(), *s))
        .collect();
    let topic_allow_interact = reactions_allow_interact(&current_user, reactor_frozen, topic_expired, topic.author_id, topic.deleted, false);
    let topic_reactions_html = render_reactions_widget(topic.id, None, &topic_reaction_rows, current_user_id, topic_allow_interact, &csrf_token);

    let poll = load_poll_view(&state, topic.id, topic.deleted, &current_user).await?;
    let image = if topic.section_prefix == "gallery" { load_topic_image(&state, topic.id).await? } else { None };
    let can_edit_topic = current_user.as_ref().map(|u| u.canmod || u.id == topic.author_id).unwrap_or(false);
    let show_add_image_link = topic.section_prefix == "gallery" && image.is_none() && can_edit_topic;

    Ok(Html(TopicTemplate {
        topic,
        topic_html,
        comments,
        current_user,
        pages,
        thread_mode,
        thread_root,
        show_deleted: want_deleted,
        show_deleted_button: can_view_deleted_comments && !want_deleted,
        filtered_count,
        unfiltered_count,
        filter_show,
        csrf_token,
        poll,
        image,
        show_add_image_link,
        topic_reactions_html,
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

/// Poll.MaxPollSize
const POLL_MAX_VARIANTS: usize = 15;
/// EditTopicRequest.newPoll's default array size.
const POLL_NEW_VARIANT_SLOTS: usize = 3;

pub async fn new_topic_form(State(state): State<AppState>, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    let groups = crate::routes::groups::list_groups(&state).await?;
    Ok(Html(TopicFormTemplate {
        title: "Новая тема".into(),
        action: "/add.jsp".into(),
        topic: None,
        groups,
        csrf_token,
        poll_variants: Vec::new(),
        poll_new_rows: vec![(); POLL_MAX_VARIANTS],
        poll_multiselect: false,
    }.render()?))
}

/// AddTopicController.MaxMessageLength (anonymous posting isn't supported by
/// Rust's session model, so only the registered-user limit applies).
const TOPIC_MAX_MESSAGE_LENGTH: usize = 65536;

pub async fn create_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, body: axum::body::Bytes) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let pairs = crate::form::parse_pairs(&body)?;
    let form = parse_topic_form(&pairs)?;
    if form.msg.chars().count() > TOPIC_MAX_MESSAGE_LENGTH {
        return Err(AppError::BadRequest("Слишком большое сообщение".into()));
    }
    if form.title.trim().is_empty() {
        return Err(AppError::BadRequest("заголовок сообщения не может быть пустым".into()));
    }
    let is_draft = form.draft.as_deref().is_some_and(|v| v == "true" || v == "on" || v == "1");
    let (premoderated, poll_allowed, imagepost): (bool, bool, bool) = sqlx::query_as("SELECT s.moderate, s.vote, s.imagepost FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1")
        .bind(form.group)
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or((false, false, false));

    // AddTopicRequestValidator.validateTags/AddTopicController: every
    // topic needs 1-5 valid tags, and creating a brand-new tag (one that
    // doesn't already exist as a value or synonym) needs either a
    // premoderated section or score>=200 (GroupPermissionService.canCreateTag).
    let tags = crate::routes::tags::parse_and_validate_tags(form.tags.as_deref().unwrap_or(""))
        .map_err(AppError::BadRequest)?;
    crate::routes::tags::check_can_create_new_tags(&state, &tags, &user, premoderated).await?;

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
    if poll_allowed {
        // AddTopicController.preparePollPreview/TopicService.addMessage:
        // every submitted variant_id is 0 (new) on creation.
        let variant_ids = vec![0; form.poll.len()];
        save_poll(&mut tx, id, form.multiselect.is_some(), &variant_ids, &form.poll).await?;
    }
    tx.commit().await?;
    notify_topic_created(&state, id, user.id, &form.msg).await?;
    crate::search_index::index_topic(&state, id, false).await;
    // AddTopicController normally requires the image up front for an
    // imagepost section; this port instead lets the topic post first and
    // sends the author straight to the upload step, matching Java's own
    // MultipartFile-based flow closely enough while avoiding a multipart
    // main form (which would drop CSRF protection on every other field -
    // see `src/csrf.rs`).
    if imagepost {
        return Ok(Redirect::to(&format!("/addphoto-topic.jsp?msgid={id}")));
    }
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
    // PollDao.getPollByTopicId/EditTopicController: pre-fill existing
    // variants (blank if the topic has no poll yet, e.g. a topic moved
    // into the Опросы section after creation) plus a handful of empty
    // slots for adding new ones.
    let poll_row: Option<(i32, bool)> = sqlx::query_as("SELECT id, multiselect FROM polls WHERE topic=$1").bind(q.msgid).fetch_optional(&state.pool).await?;
    let (poll_variants, poll_multiselect) = match poll_row {
        Some((poll_id, multiselect)) => {
            let variants = sqlx::query_as::<_, (i32, String)>("SELECT id, label FROM polls_variants WHERE vote=$1 ORDER BY id").bind(poll_id).fetch_all(&state.pool).await?;
            (variants, multiselect)
        }
        None => (Vec::new(), false),
    };
    Ok(Html(TopicFormTemplate {
        title: "Редактировать тему".into(),
        action: "/edit.jsp".into(),
        topic: Some(topic),
        groups,
        csrf_token,
        poll_variants,
        poll_new_rows: vec![(); POLL_NEW_VARIANT_SLOTS],
        poll_multiselect,
    }.render()?))
}

/// Simplified from EditTopicChecker.checkContentEdit/checkEditByAuthor:
/// author (or moderator, unconditional bypass) may edit within a 14-day
/// window from posting, or at any time while still a draft. The corrector
/// role, premoderated-section/articles commitDate nuances, and the
/// postscore==NO_COMMENTS lock aren't modeled by Rust's session yet - this
/// intentionally errs toward Java's baseline author/moderator gate rather
/// than leaving the endpoint wide open.
const TOPIC_EDIT_WINDOW_DAYS: i64 = 14;

pub async fn edit_topic(State(state): State<AppState>, CurrentUser(user): CurrentUser, body: axum::body::Bytes) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let pairs = crate::form::parse_pairs(&body)?;
    let form = parse_topic_form(&pairs)?;
    let id = form.id.ok_or_else(|| AppError::BadRequest("missing topic id".into()))?;
    let meta = load_topic_delete_meta(&state, id).await?;
    if meta.deleted {
        return Err(AppError::BadRequest("нельзя править удаленные топики".into()));
    }
    // EditTopicChecker.checkEditByAuthor: a draft is always editable by its
    // author; a committed, premoderated (non-Articles) topic is
    // *permanently* locked for the author, regardless of any deadline;
    // otherwise the 14-day window applies, measured from `commitDate` for
    // a committed Articles topic and from `postdate` everywhere else.
    const ARTICLES_SECTION_ID: i32 = 4;
    let permanently_locked = meta.commited && meta.premoderated && meta.section_id != ARTICLES_SECTION_ID;
    let deadline_base = if meta.commited && meta.section_id == ARTICLES_SECTION_ID {
        meta.commitdate.map(|d| d.and_utc()).unwrap_or(meta.postdate)
    } else {
        meta.postdate
    };
    let editable_by_author = meta.author_id == user.id
        && (meta.draft || (!permanently_locked && chrono::Utc::now() <= deadline_base + chrono::Duration::days(TOPIC_EDIT_WINDOW_DAYS)));
    if !user.canmod && !editable_by_author {
        return Err(AppError::Forbidden);
    }

    // EditTopicRequestValidator.validateTags: same rule as topic creation.
    let tags = crate::routes::tags::parse_and_validate_tags(form.tags.as_deref().unwrap_or(""))
        .map_err(AppError::BadRequest)?;
    crate::routes::tags::check_can_create_new_tags(&state, &tags, &user, meta.premoderated).await?;

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
    if meta.poll_allowed && !form.variant_id.is_empty() {
        save_poll(&mut tx, id, form.multiselect.is_some(), &form.variant_id, &form.poll).await?;
    }
    tx.commit().await?;
    crate::search_index::index_topic(&state, id, false).await;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")))
}

#[derive(Deserialize)]
pub struct TopicActionForm { pub msgid: i32, pub resolve: Option<String>, pub reason: Option<String>, pub bonus: Option<i32> }

/// Matches GroupPermissionService.DeletePeriod: an author may delete their
/// own (non-draft, non-premoderated-and-committed) topic for 3 days after
/// posting, and only while it has no comments. Moderators bypass this.
const TOPIC_DELETE_WINDOW_HOURS: i64 = 72;

struct TopicDeleteMeta {
    author_id: i32,
    deleted: bool,
    postdate: chrono::DateTime<chrono::Utc>,
    commitdate: Option<chrono::NaiveDateTime>,
    draft: bool,
    premoderated: bool,
    commited: bool,
    comment_count: i64,
    section_id: i32,
    poll_allowed: bool,
}

/// GroupPermissionService.canViewAllDeletedTopics: a listing-level "show me
/// deleted topics too" gate, distinct from (and much looser than) the
/// per-topic `ViewDeletedScore=200` in `check_topic_viewable` - any
/// authorized, non-frozen user with score>=50 qualifies, not just
/// moderators. No `SlowModeChecker` equivalent exists in this port, so
/// that extra restriction is not modeled.
pub(crate) async fn can_view_all_deleted_topics(state: &AppState, user: &Option<UserSummary>) -> Result<bool> {
    const CAN_VIEW_ALL_DELETED_SCORE: i32 = 50;
    let Some(user) = user else { return Ok(false); };
    // Java's canViewAllDeletedTopics has no isModerator special-case at
    // all - the score+frozen check applies uniformly, moderators included.
    if user.score.unwrap_or(0) < CAN_VIEW_ALL_DELETED_SCORE {
        return Ok(false);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1").bind(user.id).fetch_optional(&state.pool).await?.flatten();
    Ok(!frozen_until.map(|u| u > chrono::Utc::now()).unwrap_or(false))
}

/// TopicPermissionService.allowViewAllDeletedComments: the `?deleted=`
/// gate on a topic's own page - narrower than `can_view_all_deleted_topics`
/// (score>=200, not 50) but *does* bypass for moderators, unlike that one.
/// No `SlowModeChecker` equivalent exists in this port.
pub(crate) async fn allow_view_all_deleted_comments(state: &AppState, topic_id: i32, user: &Option<UserSummary>) -> Result<bool> {
    if user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Ok(true);
    }
    const POSTSCORE_MODERATORS_ONLY: i32 = 10000;
    const POSTSCORE_NO_COMMENTS: i32 = 10001;
    const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
    let Some((expired, draft, postscore)): Option<(bool, bool, i32)> = sqlx::query_as(
        r#"SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire, COALESCE(t.draft,false), t.postscore
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?
    else {
        return Ok(false);
    };
    let topic_forbidden = expired || draft || matches!(postscore, POSTSCORE_MODERATORS_ONLY | POSTSCORE_NO_COMMENTS | POSTSCORE_HIDE_COMMENTS);
    if topic_forbidden {
        return Ok(false);
    }
    let Some(user) = user else { return Ok(false); };
    if user.score.unwrap_or(0) < VIEW_DELETED_SCORE {
        return Ok(false);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1").bind(user.id).fetch_optional(&state.pool).await?.flatten();
    if frozen_until.map(|u| u > chrono::Utc::now()).unwrap_or(false) {
        return Ok(false);
    }
    let score_loss: i32 = sqlx::query_scalar(
        r#"SELECT COALESCE((SELECT sum(bonus) FROM del_info JOIN comments ON comments.id=del_info.msgid
             WHERE bonus IS NOT NULL AND bonus<>0 AND comments.userid<>2 AND comments.deleted AND topic=$1), 0)::int"#,
    )
    .bind(topic_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(score_loss < 150)
}

/// TopicPermissionService.ViewDeletedScore/ViewAfterDeleteDays/TopicMaxWarnings.
const VIEW_DELETED_SCORE: i32 = 200;
const VIEW_AFTER_DELETE_DAYS: i64 = 14;
const TOPIC_MAX_WARNINGS: i32 = 2;

/// TopicPermissionService.checkView: whether `user` may view this specific
/// topic given its deleted/draft/expired/open-warnings state. Moderators
/// always pass (mirrors `!session.moderator` guarding the whole body in
/// Java). Used both for the standalone topic view and, transitively, for
/// anything that needs the same "can view a deleted topic" rule (reactions
/// viewer, forum/group `showDeleted` gate).
pub(crate) async fn check_topic_viewable(state: &AppState, topic_id: i32, user: &Option<UserSummary>) -> Result<()> {
    if user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Ok(());
    }
    let row: Option<(bool, bool, bool, i32, i32, bool)> = sqlx::query_as(
        r#"SELECT t.deleted, COALESCE(t.draft,false),
                  NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire AS expired,
                  t.userid, t.open_warnings, u.canmod
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           JOIN users u ON u.id=t.userid
           WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((deleted, draft, expired, author_id, open_warnings, author_is_moderator)) = row else {
        return Err(AppError::NotFound);
    };

    let view_by_author = user.as_ref().map(|u| u.id == author_id).unwrap_or(false);

    if deleted {
        if expired {
            return Err(AppError::NotFound);
        }
        if user.is_none() {
            return Err(AppError::NotFound);
        }
        if !view_by_author {
            let current = user.as_ref().unwrap();
            let deldate: Option<chrono::NaiveDateTime> = sqlx::query_scalar("SELECT deldate FROM del_info WHERE msgid=$1").bind(topic_id).fetch_optional(&state.pool).await?.flatten();
            let delete_expired = deldate.map(|d| d.and_utc() < chrono::Utc::now() - chrono::Duration::days(VIEW_AFTER_DELETE_DAYS)).unwrap_or(true);
            if delete_expired {
                return Err(AppError::NotFound);
            }
            let frozen_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1").bind(current.id).fetch_optional(&state.pool).await?.flatten();
            if frozen_until.map(|u| u > chrono::Utc::now()).unwrap_or(false) {
                return Err(AppError::Forbidden);
            }
            if current.score.unwrap_or(0) < VIEW_DELETED_SCORE {
                return Err(AppError::NotFound);
            }
            if author_is_moderator {
                return Err(AppError::NotFound);
            }
        }
    }

    if draft {
        if expired {
            return Err(AppError::NotFound);
        }
        if !view_by_author {
            return Err(AppError::NotFound);
        }
    }

    if user.is_none() && open_warnings > TOPIC_MAX_WARNINGS {
        return Err(AppError::NotFound);
    }

    Ok(())
}

async fn load_topic_delete_meta(state: &AppState, msgid: i32) -> Result<TopicDeleteMeta> {
    let row: (i32, bool, chrono::DateTime<chrono::Utc>, Option<chrono::NaiveDateTime>, bool, bool, bool, i64, i32, bool) = sqlx::query_as(
        r#"SELECT t.userid, t.deleted, t.postdate, t.commitdate, COALESCE(t.draft,false), s.moderate,
                  (t.commitdate IS NOT NULL), t.stat1::bigint, s.id, s.vote
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
        commitdate: row.3,
        draft: row.4,
        premoderated: row.5,
        commited: row.6,
        comment_count: row.7,
        section_id: row.8,
        poll_allowed: row.9,
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
    // GroupPermissionService.isDeletable: an administrator always passes;
    // otherwise try the author path first, and only fall back to
    // isDeletableByModerator (which itself refuses a committed
    // premoderated topic more than a month old, admin-only past that
    // point) when the author path fails and the actor is a moderator.
    let deletable = if user.candel {
        true
    } else if deletable_by_author {
        true
    } else if user.canmod {
        !meta.premoderated
            || !meta.commited
            || chrono::Utc::now() <= meta.postdate + chrono::Duration::days(30)
    } else {
        false
    };
    if !deletable {
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
    // GroupPermissionService.isUndeletable: an administrator can always
    // undelete; a plain moderator only while the topic isn't expired, or -
    // if it is - within 14 days of the deletion itself.
    if !user.candel {
        let expired = crate::routes::comments::is_topic_expired(&state, form.msgid).await?;
        if expired {
            let deldate: Option<chrono::NaiveDateTime> = sqlx::query_scalar("SELECT deldate FROM del_info WHERE msgid=$1").bind(form.msgid).fetch_optional(&state.pool).await?.flatten();
            let recently_deleted = deldate.map(|d| d.and_utc() > chrono::Utc::now() - chrono::Duration::days(14)).unwrap_or(false);
            if !recently_deleted {
                return Err(AppError::Forbidden);
            }
        }
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

/// PollDao.createPoll/updatePoll unified into one helper: creates the
/// topic's poll row on first call, then on every call reconciles
/// `polls_variants` against the submitted (variant_id, label) pairs -
/// `variant_id==0` inserts a new variant, an existing id with an empty
/// label deletes it, an existing id with a non-empty label updates it.
/// `variant_id` is scoped to `vote=voteid` in every UPDATE/DELETE so a
/// forged id from another poll can't be touched.
async fn save_poll(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, topic_id: i32, multiselect: bool, variant_ids: &[i32], labels: &[String]) -> Result<()> {
    let existing: Option<i32> = sqlx::query_scalar("SELECT id FROM polls WHERE topic=$1").bind(topic_id).fetch_optional(&mut **tx).await?;
    let voteid = match existing {
        Some(id) => {
            sqlx::query("UPDATE polls SET multiselect=$1 WHERE id=$2").bind(multiselect).bind(id).execute(&mut **tx).await?;
            id
        }
        None => {
            let id: i32 = sqlx::query_scalar("SELECT nextval('vote_id')::int").fetch_one(&mut **tx).await?;
            sqlx::query("INSERT INTO polls(id, multiselect, topic) VALUES($1,$2,$3)").bind(id).bind(multiselect).bind(topic_id).execute(&mut **tx).await?;
            id
        }
    };
    for (variant_id, label) in variant_ids.iter().zip(labels.iter()) {
        let label = label.trim();
        if *variant_id == 0 {
            if !label.is_empty() {
                sqlx::query("INSERT INTO polls_variants(id, vote, label) VALUES(nextval('votes_id'), $1, $2)").bind(voteid).bind(label).execute(&mut **tx).await?;
            }
        } else if label.is_empty() {
            sqlx::query("DELETE FROM polls_variants WHERE id=$1 AND vote=$2").bind(variant_id).bind(voteid).execute(&mut **tx).await?;
        } else {
            sqlx::query("UPDATE polls_variants SET label=$1 WHERE id=$2 AND vote=$3").bind(label).bind(variant_id).bind(voteid).execute(&mut **tx).await?;
        }
    }
    Ok(())
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

/// EditTopicChecker.checkCommit: moderators or correctors may commit a
/// news topic, but a corrector may not commit their own - moderators have
/// no such restriction.
fn check_commit_allowed(user: &UserSummary, topic_author_id: i32) -> Result<()> {
    if !user.canmod && !user.corrector {
        return Err(AppError::Forbidden);
    }
    if user.corrector && !user.canmod && user.id == topic_author_id {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub async fn commit_topic_form(State(state): State<AppState>, Query(q): Query<ViewMessageQuery>, CurrentUser(user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1").bind(q.msgid).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    check_commit_allowed(&user, author_id)?;
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
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1").bind(form.msgid).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    check_commit_allowed(&user, author_id)?;
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
