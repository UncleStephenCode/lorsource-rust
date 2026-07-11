use crate::{auth::CurrentUser, error::{AppError, Result}, models::TopicSummary, state::AppState};
use axum::{extract::{Query, State}, response::{Html, IntoResponse, Redirect}, Json};
use askama::Template;
use serde::Deserialize;
use serde_json::json;

/// Maps UserEventFilterEnum's `getName` (lowercase enum case) to its `dbType`.
fn filter_db_type(filter: &str) -> Option<&'static str> {
    match filter {
        "answers" => Some("REPLY"),
        "favorites" => Some("WATCH"),
        "deleted" => Some("DEL"),
        "reference" => Some("REF"),
        "tag" => Some("TAG"),
        "reaction" => Some("REACTION"),
        "warning" => Some("WARNING"),
        _ => None, // "all" or unrecognized
    }
}

#[derive(Debug, sqlx::FromRow)]
struct NotificationEvent {
    id: i32,
    event_date: chrono::DateTime<chrono::Utc>,
    subj: String,
    msgid: i32,
    cid: Option<i32>,
    unread: bool,
    event_type: String,
    section_prefix: String,
    group_urlname: String,
}

impl NotificationEvent {
    fn link(&self) -> String {
        let anchor = self.cid.map(|id| format!("?cid={id}")).unwrap_or_default();
        format!("/{}/{}/{}{anchor}", self.section_prefix, self.group_urlname, self.msgid)
    }
}

#[derive(Deserialize)]
pub struct NotificationsQuery {
    pub filter: Option<String>,
    pub offset: Option<i64>,
}

/// UserEventController.showNotifications - requires auth, lists user_events
/// for the current user (newest first), with an "answers/favorites/deleted/
/// reference/tag/reaction/warning" filter and offset pagination.
pub async fn notifications(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<NotificationsQuery>) -> Result<Html<String>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    // Reflected-XSS guard: normalize to a known filter name instead of
    // echoing the raw `?filter=` value back into the page (it's spliced
    // into an href below).
    let requested_filter = q.filter.unwrap_or_else(|| "all".to_string());
    let filter = if requested_filter == "all" || filter_db_type(&requested_filter).is_some() {
        requested_filter
    } else {
        "all".to_string()
    };
    let db_type = filter_db_type(&filter);
    let offset = q.offset.unwrap_or(0).max(0);
    let limit = 20i64;

    let events = sqlx::query_as::<_, NotificationEvent>(
        r#"SELECT e.id, e.event_date, t.title AS subj, t.id AS msgid, e.comment_id AS cid, e.unread, e.type::text AS event_type,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                  g.urlname AS group_urlname
           FROM user_events e
           JOIN topics t ON t.id=e.message_id
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE e.userid=$1 AND ($2::text IS NULL OR e.type::text=$2)
           ORDER BY e.id DESC LIMIT $3 OFFSET $4"#,
    )
    .bind(user.id)
    .bind(db_type)
    .bind(limit + 1)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let has_more = events.len() as i64 > limit;
    let events: Vec<_> = events.into_iter().take(limit as usize).collect();
    let top_id = events.iter().map(|e| e.id).max();

    let mut html = format!("<h1>Уведомления {}</h1>", html_escape::encode_text(&user.nick));
    html.push_str("<nav class=\"filters\">");
    for (label, value) in [("все", "all"), ("ответы", "answers"), ("отслеживаемое", "favorites"), ("упоминания", "reference"), ("теги", "tag"), ("реакции", "reaction"), ("предупреждения", "warning")] {
        let active = if value == filter { " class=\"active\"" } else { "" };
        html.push_str(&format!("<a{active} href=\"/notifications?filter={value}\">{label}</a> "));
    }
    html.push_str("</nav>");

    if let Some(top_id) = top_id {
        html.push_str(&format!(
            "<form method=\"post\" action=\"/notifications\"><input type=\"hidden\" name=\"topId\" value=\"{top_id}\"><button type=\"submit\">Отметить всё прочитанным</button></form>"
        ));
    }

    html.push_str("<ul class=\"notifications-list\">");
    for e in &events {
        let unread_class = if e.unread { " class=\"unread\"" } else { "" };
        html.push_str(&format!(
            "<li{unread_class}><a href=\"{link}\">{subj}</a> <small>{date} · {etype}</small></li>",
            link = e.link(),
            subj = html_escape::encode_text(&e.subj),
            date = e.event_date,
            etype = html_escape::encode_text(&e.event_type),
        ));
    }
    if events.is_empty() {
        html.push_str("<li class=\"muted\">Нет уведомлений</li>");
    }
    html.push_str("</ul>");

    if has_more {
        html.push_str(&format!("<a href=\"/notifications?filter={filter}&offset={}\">Далее »</a>", offset + limit));
    }

    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct NotificationsResetForm {
    #[serde(rename = "topId")]
    pub top_id: i32,
}

async fn reset_unread_events(state: &AppState, user_id: i32, top_id: i32) -> Result<()> {
    sqlx::query("UPDATE user_events SET unread=false WHERE userid=$1 AND unread AND id<=$2")
        .bind(user_id)
        .bind(top_id)
        .execute(&state.pool)
        .await?;
    sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=$1")
        .bind(user_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// POST /notifications (UserEventController.resetNotifications) - HTML flow,
/// redirects back to the notifications page.
pub async fn notifications_mark_read(State(state): State<AppState>, CurrentUser(user): CurrentUser, axum::Form(form): axum::Form<NotificationsResetForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    reset_unread_events(&state, user.id, form.top_id).await?;
    Ok(Redirect::to("/notifications"))
}

/// GET /notifications-count (UserEventApiController.getEventsCount) - bare
/// JSON integer, not an object, matching the Java `Json` response shape.
pub async fn notifications_count(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let count: i32 = sqlx::query_scalar("SELECT unread_events FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!(count)))
}

/// POST /notifications-reset (UserEventApiController.resetNotifications) -
/// the JSON-API twin of the HTML `notifications_mark_read` above.
pub async fn notifications_reset(State(state): State<AppState>, CurrentUser(user): CurrentUser, axum::Form(form): axum::Form<NotificationsResetForm>) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    reset_unread_events(&state, user.id, form.top_id).await?;
    Ok(Json(json!("ok")))
}

#[derive(Debug, serde::Deserialize)]
pub struct TrackerQuery {
    pub offset: Option<i64>,
    pub filter: Option<String>,
}

#[derive(Template)]
#[template(path = "tracker.html")]
struct TrackerTemplate {
    title: String,
    filter: String,
    topics: Vec<crate::models::TopicSummary>,
    prev_link: Option<String>,
    next_link: Option<String>,
}

pub async fn tracker_old_redirect(Query(q): Query<TrackerQuery>) -> Redirect {
    match q.filter {
        Some(filter) if !filter.trim().is_empty() && filter != "all" => {
            Redirect::to(&format!("/tracker/?filter={}", urlencoding::encode(&filter)))
        }
        _ => Redirect::to("/tracker/"),
    }
}

/// Matches TrackerFilterEnum.NonTech (SectionController.NonTech): these are
/// real production group ids on the upstream Java site (a "Talks" group and
/// a few others) - hardcoded the same way upstream hardcodes them, for
/// compatibility with a migrated real DB. Harmless no-op against fresh dev
/// seed data, which doesn't use these ids.
const TRACKER_NON_TECH_GROUPS: &[i32] = &[8404, 4068, 9326, 19405];
const TRACKER_TALKS_GROUP: i32 = 8404;
/// Forum section id (matches Section.Forum / seed data: 'Форум').
const TRACKER_TECH_SECTION_ID: i32 = 2;

fn tracker_filter_group_clause(filter: &str) -> String {
    let non_tech = TRACKER_NON_TECH_GROUPS.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
    match filter {
        "notalks" => format!("AND t.groupid <> {TRACKER_TALKS_GROUP} AND NOT t.notop"),
        "main" => format!("AND t.groupid NOT IN ({non_tech}) AND NOT t.notop"),
        "tech" => format!("AND t.groupid NOT IN ({non_tech}) AND NOT t.notop AND s.id = {TRACKER_TECH_SECTION_ID}"),
        _ => String::new(),
    }
}

/// Simplified from GroupListDao.getTrackerTopics: real topic tracker
/// semantics (filter by TrackerFilterEnum, default to the viewer's saved
/// trackerMode, sorted by most recent activity in the last 7 days) - the
/// previous handler filtered by *section name* instead, an unrelated
/// concept, and never read the user's saved preference at all. The exact
/// ignore-list-aware last-comment subquery from Java isn't replicated here;
/// this orders by COALESCE(lastmod, postdate) as a practical proxy for
/// "most recent activity".
pub async fn tracker(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<TrackerQuery>) -> Result<Html<String>> {
    if q.offset.unwrap_or(0) < 0 || q.offset.unwrap_or(0) > 300 {
        return Err(AppError::BadRequest("Некорректное значение offset".into()));
    }
    let offset = q.offset.unwrap_or(0).clamp(0, 300);
    let limit = state.config.page_size.max(1);

    let default_filter: String = if let Some(u) = &user {
        sqlx::query_scalar::<_, Option<String>>("SELECT settings->'trackerMode' FROM user_settings WHERE id=$1")
            .bind(u.id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
            .filter(|v: &String| v == "all" || v == "main")
            .unwrap_or_else(|| "main".to_string())
    } else {
        "main".to_string()
    };
    let filter = q.filter.filter(|f| ["all", "main", "notalks", "tech"].contains(&f.as_str())).unwrap_or_else(|| default_filter.clone());

    let is_moderator = user.as_ref().map(|u| u.canmod).unwrap_or(false);
    let show_uncommitted = filter == "all" || is_moderator;

    let sql = format!(
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
           WHERE NOT t.draft AND NOT t.deleted
             AND COALESCE(t.lastmod, t.postdate) > now() - interval '7 days'
             {uncommitted}
             {group_clause}
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY COALESCE(t.lastmod, t.postdate) DESC
           OFFSET $1 LIMIT $2"#,
        uncommitted = if show_uncommitted { "" } else { "AND NOT t.moderate" },
        group_clause = tracker_filter_group_clause(&filter),
    );
    let topics = sqlx::query_as::<_, TopicSummary>(&sql)
        .bind(offset)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?;

    let filter_label = match filter.as_str() {
        "main" => "основные",
        "notalks" => "без talks",
        "tech" => "тех. форум",
        _ => "все",
    };
    let title = if filter == default_filter { "Активные топики".to_string() } else { format!("Активные топики ({filter_label})") };
    let extra = if filter == default_filter { String::new() } else { format!("filter={}", urlencoding::encode(&filter)) };
    let next_link = if topics.len() as i64 == limit && offset < 300 {
        let sep = if extra.is_empty() { "" } else { "&" };
        Some(format!("/tracker/?offset={}{}{}", offset + limit, sep, extra))
    } else {
        None
    };
    let prev_link = if offset >= limit {
        let new_offset = offset - limit;
        if extra.is_empty() {
            Some(if new_offset == 0 { "/tracker/".to_string() } else { format!("/tracker/?offset={new_offset}") })
        } else {
            Some(if new_offset == 0 { format!("/tracker/?{extra}") } else { format!("/tracker/?offset={new_offset}&{extra}") })
        }
    } else {
        None
    };
    Ok(Html(TrackerTemplate { title, filter, topics, prev_link, next_link }.render()?))
}

pub async fn top10_boxlet(State(state): State<AppState>) -> Result<Html<String>> {
    let topics = crate::routes::topics::list_topics(&state, None, None, 0, 10).await?;
    let mut html = String::from("<ul class=\"boxlet\">");
    for t in topics { html.push_str(&format!("<li><a href=\"{}\">{}</a></li>", t.topic_url(), html_escape::encode_text(&t.title))); }
    html.push_str("</ul>");
    Ok(Html(html))
}

pub async fn articles_boxlet(State(state): State<AppState>) -> Result<Html<String>> {
    let topics = crate::routes::topics::list_topics(&state, Some("articles"), None, 0, 10).await?;
    let mut html = String::from("<ul class=\"boxlet\">");
    for t in topics { html.push_str(&format!("<li><a href=\"{}\">{}</a></li>", t.topic_url(), html_escape::encode_text(&t.title))); }
    html.push_str("</ul>");
    Ok(Html(html))
}

pub async fn poll_boxlet(State(state): State<AppState>) -> Result<Html<String>> {
    let topics = crate::routes::topics::list_topics(&state, Some("polls"), None, 0, 5).await?;
    let mut html = String::from("<ul class=\"boxlet\">");
    for t in topics { html.push_str(&format!("<li><a href=\"{}\">{}</a></li>", t.topic_url(), html_escape::encode_text(&t.title))); }
    html.push_str("</ul>");
    Ok(Html(html))
}

#[derive(serde::Deserialize)]
pub struct ReactionQuery { pub topic: Option<i32>, pub comment: Option<i32>, pub msgid: Option<i32> }

#[derive(serde::Deserialize)]
pub struct ReactionForm { pub topic: Option<i32>, pub comment: Option<i32>, pub msgid: Option<i32>, pub reaction: Option<String>, pub value: Option<bool> }

fn parse_reaction_action(raw: Option<String>, value: Option<bool>) -> (String, bool) {
    let raw = raw.unwrap_or_else(|| "+1-true".to_string());
    if let Some((reaction, action)) = raw.rsplit_once('-') {
        if action == "true" || action == "false" {
            return (reaction.to_string(), action == "true");
        }
    }
    (raw, value.unwrap_or(true))
}

async fn resolve_reaction_target(pool: &sqlx::PgPool, topic: Option<i32>, comment: Option<i32>, msgid: Option<i32>) -> Result<(i32, Option<i32>)> {
    if let Some(comment_id) = comment {
        let topic_id: i32 = sqlx::query_scalar("SELECT topic FROM comments WHERE id=$1")
            .bind(comment_id)
            .fetch_optional(pool)
            .await?
            .ok_or(crate::error::AppError::NotFound)?;
        return Ok((topic_id, Some(comment_id)));
    }

    let topic_id = topic.or(msgid).ok_or_else(|| crate::error::AppError::BadRequest("missing topic/comment".into()))?;
    Ok((topic_id, None))
}

async fn reaction_target_link(pool: &sqlx::PgPool, topic_id: i32, comment_id: Option<i32>) -> Result<String> {
    let prefix: Option<(String, String)> = sqlx::query_as(
        r#"SELECT CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END,
                  g.urlname
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(pool)
    .await?;
    let Some((section, group)) = prefix else { return Ok("/".to_string()); };
    let anchor = comment_id.map(|id| format!("?cid={id}")).unwrap_or_default();
    Ok(format!("/{section}/{group}/{topic_id}{anchor}"))
}

/// ReactionController.commentReaction/topicReaction (GET, non-ajax): an
/// anonymous visitor is redirected straight to the topic/comment; a logged
/// in user gets an HTML breakdown of who reacted with what. The previous
/// handler always returned raw JSON regardless of auth state or Accept
/// header, which isn't what a plain browser GET (e.g. from a bookmarked
/// link or the non-JS reaction UI) expects.
pub async fn reactions_get(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<ReactionQuery>) -> Result<axum::response::Response> {
    let (topic_id, comment_id) = resolve_reaction_target(&state.pool, q.topic, q.comment, q.msgid).await?;
    let link = reaction_target_link(&state.pool, topic_id, comment_id).await?;

    let Some(_user) = user else {
        return Ok(Redirect::to(&link).into_response());
    };

    let rows = sqlx::query_as::<_, (i32, String, String, chrono::DateTime<chrono::Utc>)>(
        r#"SELECT rl.origin_user, u.nick, rl.reaction, rl.set_date
           FROM reactions_log rl JOIN users u ON u.id=rl.origin_user
           WHERE rl.topic_id=$1 AND (($2::int IS NULL AND rl.comment_id IS NULL) OR rl.comment_id=$2)
           ORDER BY rl.set_date"#,
    )
    .bind(topic_id)
    .bind(comment_id)
    .fetch_all(&state.pool)
    .await?;

    let mut html = format!("<h1>Реакции</h1><p><a href=\"{link}\">Перейти к {}</a></p><ul>", if comment_id.is_some() { "комментарию" } else { "теме" });
    for (_uid, nick, reaction, date) in &rows {
        html.push_str(&format!(
            "<li>{} <b>{}</b> · {date}</li>",
            html_escape::encode_text(reaction),
            html_escape::encode_text(nick),
        ));
    }
    if rows.is_empty() {
        html.push_str("<li class=\"muted\">Нет реакций</li>");
    }
    html.push_str("</ul>");
    Ok(Html(html).into_response())
}

const ALLOWED_REACTIONS: &[&str] = &[
    "👍", "👎", "😊", "😱", "🤦", "🔥", "🤔", "🤡", "☕☕", "🪗", "😢", "🚮", "🎉", "🤬",
];

async fn check_reaction_allowed(pool: &sqlx::PgPool, user_id: i32, topic_id: i32, comment_id: Option<i32>, set: bool, reaction: &str) -> Result<()> {
    if set && !ALLOWED_REACTIONS.contains(&reaction) {
        return Err(crate::error::AppError::Forbidden);
    }
    if set {
        let recent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM reactions_log WHERE origin_user=$1 AND set_date > CURRENT_TIMESTAMP - interval '10 minutes'",
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?;
        if recent >= 5 {
            return Err(crate::error::AppError::TooManyRequests("Попробуйте позже".into()));
        }
    }

    let (author_id, topic_deleted, topic_expired, comment_deleted): (i32, bool, bool, Option<bool>) = if let Some(comment_id) = comment_id {
        sqlx::query_as(
            r#"SELECT c.userid,
                      t.deleted,
                      (t.postdate + s.expire < now()) AS expired,
                      c.deleted
               FROM comments c
               JOIN topics t ON t.id=c.topic
               JOIN groups g ON g.id=t.groupid
               JOIN sections s ON s.id=g.section
               WHERE c.id=$1 AND t.id=$2"#,
        )
        .bind(comment_id)
        .bind(topic_id)
        .fetch_optional(pool)
        .await?
        .ok_or(crate::error::AppError::NotFound)?
    } else {
        let (author_id, deleted, expired): (i32, bool, bool) = sqlx::query_as(
            r#"SELECT t.userid, t.deleted, (t.postdate + s.expire < now()) AS expired
               FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
        )
        .bind(topic_id)
        .fetch_optional(pool)
        .await?
        .ok_or(crate::error::AppError::NotFound)?;
        (author_id, deleted, expired, None)
    };

    if user_id == author_id || topic_deleted || topic_expired || comment_deleted.unwrap_or(false) {
        return Err(crate::error::AppError::Forbidden);
    }
    Ok(())
}

struct SetReactionResult {
    topic_id: i32,
    comment_id: Option<i32>,
    reaction: String,
    count: i64,
}

async fn do_set_reaction(state: &AppState, user_id: i32, form: ReactionForm) -> Result<SetReactionResult> {
    let (topic_id, comment_id) = resolve_reaction_target(&state.pool, form.topic, form.comment, form.msgid).await?;
    let (reaction, set) = parse_reaction_action(form.reaction, form.value);
    check_reaction_allowed(&state.pool, user_id, topic_id, comment_id, set, &reaction).await?;

    if set {
        if let Some(comment_id) = comment_id {
            sqlx::query("UPDATE comments SET reactions = reactions || jsonb_build_object($2::text, $3::text) WHERE id=$1")
                .bind(comment_id).bind(user_id).bind(&reaction).execute(&state.pool).await?;
        } else {
            sqlx::query("UPDATE topics SET reactions = reactions || jsonb_build_object($2::text, $3::text) WHERE id=$1")
                .bind(topic_id).bind(user_id).bind(&reaction).execute(&state.pool).await?;
        }
        sqlx::query(
            r#"INSERT INTO reactions_log(origin_user,topic_id,comment_id,reaction,set_date)
               VALUES($1,$2,$3,$4,now())
               ON CONFLICT (topic_id, comment_id, origin_user)
               DO UPDATE SET set_date=now(), reaction=EXCLUDED.reaction"#,
        )
        .bind(user_id).bind(topic_id).bind(comment_id).bind(&reaction).execute(&state.pool).await?;
    } else {
        if let Some(comment_id) = comment_id {
            sqlx::query("UPDATE comments SET reactions = reactions - $2::text WHERE id=$1")
                .bind(comment_id).bind(user_id.to_string()).execute(&state.pool).await?;
        } else {
            sqlx::query("UPDATE topics SET reactions = reactions - $2::text WHERE id=$1")
                .bind(topic_id).bind(user_id.to_string()).execute(&state.pool).await?;
        }
        sqlx::query(
            r#"DELETE FROM reactions_log
               WHERE origin_user=$1 AND topic_id=$2 AND (($3::int IS NULL AND comment_id IS NULL) OR comment_id=$3)"#,
        )
        .bind(user_id).bind(topic_id).bind(comment_id).execute(&state.pool).await?;
    }

    let count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM reactions_log
           WHERE topic_id=$1 AND (($2::int IS NULL AND comment_id IS NULL) OR comment_id=$2) AND reaction=$3"#,
    )
    .bind(topic_id).bind(comment_id).bind(&reaction).fetch_one(&state.pool).await?;

    Ok(SetReactionResult { topic_id, comment_id, reaction, count })
}

/// ReactionController.setCommentReaction/setTopicReaction (POST, non-ajax
/// form submit) - redirects back to the topic/comment, matching Java's
/// RedirectView. The previous handler always returned JSON here too, which
/// breaks a plain `<form method=post>` submit (no fetch/XHR).
pub async fn reactions_post(State(state): State<AppState>, CurrentUser(user): CurrentUser, axum::Form(form): axum::Form<ReactionForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let result = do_set_reaction(&state, user.id, form).await?;
    let link = reaction_target_link(&state.pool, result.topic_id, result.comment_id).await?;
    Ok(Redirect::to(&link))
}

/// ReactionController.setCommentReactionAjax/setTopicReactionAjax (POST /reactions/ajax).
pub async fn reactions_post_ajax(State(state): State<AppState>, CurrentUser(user): CurrentUser, axum::Form(form): axum::Form<ReactionForm>) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let result = do_set_reaction(&state, user.id, form).await?;
    Ok(Json(json!({"count": result.count})))
}

#[derive(serde::Deserialize)]
pub struct VoteForm {
    /// Poll id (`voteid` in the original VoteController).
    pub voteid: i32,
    /// Selected variant ids. The original form submits this field as repeated `vote`.
    #[serde(default)]
    pub vote: Vec<i32>,
}

pub async fn vote(State(state): State<AppState>, CurrentUser(user): CurrentUser, axum::Form(form): axum::Form<VoteForm>) -> Result<axum::response::Redirect> {
    let Some(user) = user else { return Err(crate::error::AppError::Forbidden); };
    if form.vote.is_empty() {
        return Err(crate::error::AppError::BadRequest("ничего не выбрано".into()));
    }

    let Some((topic_id, multiselect, section_prefix, group_urlname, expired)) = sqlx::query_as::<_, (i32, bool, String, String, bool)>(
        r#"SELECT p.topic, p.multiselect,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                  g.urlname,
                  (t.postdate + s.expire < now()) AS expired
           FROM polls p
           JOIN topics t ON t.id=p.topic
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE p.id=$1 AND NOT t.moderate AND NOT t.deleted AND NOT t.draft"#,
    )
    .bind(form.voteid)
    .fetch_optional(&state.pool)
    .await? else {
        return Err(crate::error::AppError::BadRequest("опрос не найден или ещё не подтверждён".into()));
    };

    if expired {
        return Err(crate::error::AppError::BadRequest("Опрос завершен".into()));
    }
    if !multiselect && form.vote.len() != 1 {
        return Err(crate::error::AppError::BadRequest("этот опрос допускает только один вариант ответа".into()));
    }

    let already_voted: i64 = sqlx::query_scalar("SELECT count(vote) FROM vote_users WHERE vote=$1 AND userid=$2")
        .bind(form.voteid)
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    if already_voted == 0 {
        for variant_id in form.vote {
            let Some(valid_variant) = sqlx::query_scalar::<_, i32>("SELECT id FROM polls_variants WHERE id=$1 AND vote=$2")
                .bind(variant_id)
                .bind(form.voteid)
                .fetch_optional(&state.pool)
                .await? else {
                    return Err(crate::error::AppError::BadRequest("неправильный вариант ответа".into()));
                };
            let inserted = sqlx::query(
                "INSERT INTO vote_users(vote, userid, variant_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
            )
            .bind(form.voteid)
            .bind(user.id)
            .bind(valid_variant)
            .execute(&state.pool)
            .await?
            .rows_affected();
            if inserted > 0 {
                sqlx::query("UPDATE polls_variants SET votes=votes+1 WHERE id=$1")
                    .bind(valid_variant)
                    .execute(&state.pool)
                    .await?;
            }
        }
    }

    Ok(axum::response::Redirect::to(&format!("/{section_prefix}/{group_urlname}/{topic_id}")))
}
