use crate::{
    auth::CurrentUser,
    error::{AppError, Result},
    markup,
    models::{PagerQuery, TopicSummary, UserSummary},
    pagination::Pager,
    profile::{ChoiceOption, NumberOption, ProfileSettings, ThemeOption},
    security,
    state::AppState,
};
use askama::Template;
use axum::{extract::{Path, Query, State}, response::{Html, IntoResponse, Redirect}, Form};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, sqlx::FromRow)]
struct UserProfileData {
    id: i32,
    nick: String,
    name: Option<String>,
    score: i32,
    max_score: i32,
    photo: Option<String>,
    town: Option<String>,
    userinfo: Option<String>,
    url: Option<String>,
    email: Option<String>,
    canmod: bool,
    candel: bool,
    corrector: bool,
    blocked: bool,
    activated: bool,
    regdate_text: Option<String>,
    lastlogin_text: Option<String>,
    userinfo_markup: Option<String>,
}

impl UserProfileData {
    fn status(&self) -> &'static str {
        if self.blocked { "заблокирован" }
        else if !self.activated { "не активирован" }
        else if self.score >= 100 { "активный пользователь" }
        else { "новый пользователь" }
    }

}

#[derive(Debug, Clone)]
struct UserStats {
    topic_count: i64,
    comment_count: i64,
    first_topic: Option<String>,
    last_topic: Option<String>,
    first_comment: Option<String>,
    last_comment: Option<String>,
}

#[derive(Debug, Clone)]
struct BanInfo {
    bandate_text: String,
    reason: String,
    moderator_nick: String,
}

#[derive(Debug, Clone)]
struct UserLogEntry {
    action: String,
    date_text: String,
    actor_nick: String,
}

#[derive(Template)]
#[template(path = "user.html")]
struct UserTemplate {
    profile: UserProfileData,
    stats: UserStats,
    topics: Vec<TopicSummary>,
    favorite_tags: Vec<String>,
    ignore_tags: Vec<String>,
    drafts_count: i64,
    is_owner: bool,
    can_view_private: bool,
    /// Pre-rendered, sanitized HTML for `profile.userinfo` - see
    /// `render_profile`. Never render `profile.userinfo` directly with
    /// `|safe`; it's raw user input.
    userinfo_html: Option<String>,
    ban_info: Option<BanInfo>,
    frozen_until_text: Option<String>,
    is_frozen: bool,
    blockable: bool,
    freezable: bool,
    other_accounts: Vec<String>,
    user_log: Vec<UserLogEntry>,
    invited_users: Vec<String>,
    /// `UserService.getUserpic(user, viewer.avatarMode, misteryMan=true)` -
    /// always renders as an `<img>`, falling back to a 1x1 transparent gif
    /// (`DisabledUserpic`) rather than a "no photo" box when the viewer has
    /// avatars disabled or the target has neither a local photo nor email.
    userpic_url: String,
}

#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    user: UserSummary,
    settings: ProfileSettings,
    themes: Vec<ThemeOption>,
    avatars: Vec<ChoiceOption>,
    tracker_modes: Vec<ChoiceOption>,
    format_modes: Vec<ChoiceOption>,
    topic_values: Vec<NumberOption>,
    message_values: Vec<NumberOption>,
}

#[derive(Template)]
#[template(path = "edit_profile.html")]
struct EditProfileTemplate {
    user: UserSummary,
    profile: UserProfileData,
}

#[derive(Deserialize)]
pub struct UserTopicFeedQuery {
    pub offset: Option<i64>,
    pub section: Option<i32>,
}

/// UserTopicListController.showUserTopics: `/people/{nick}` (bare, no
/// suffix) is the user's topic feed, a distinct page from the profile at
/// `/people/{nick}/profile` - the previous handler aliased this straight to
/// the profile page. Optional `?section=` filter, 404s if the feed is
/// empty (matches Java exactly, including on a valid user with zero posts).
pub async fn topic_feed(State(state): State<AppState>, Path(nick): Path<String>, Query(q): Query<UserTopicFeedQuery>) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let pager = Pager::new(q.offset.unwrap_or(0).max(0), state.config.page_size);

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
           WHERE u.id=$1 AND NOT t.deleted AND NOT COALESCE(t.draft,false) AND NOT t.moderate
             {section_clause}
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY t.postdate DESC OFFSET $2 LIMIT $3"#,
        section_clause = if q.section.is_some() { "AND s.id=$4" } else { "" },
    );
    let mut query = sqlx::query_as::<_, TopicSummary>(&sql).bind(user.id).bind(pager.offset).bind(pager.limit);
    if let Some(section) = q.section {
        query = query.bind(section);
    }
    let topics = query.fetch_all(&state.pool).await?;

    if topics.is_empty() {
        return Err(AppError::NotFound);
    }

    Ok(Html(simple_topic_list(&format!("Сообщения {}", user.nick), &topics)))
}

pub async fn profile_full(State(state): State<AppState>, Path(nick): Path<String>, Query(q): Query<PagerQuery>, current: CurrentUser) -> Result<Html<String>> {
    render_profile(state, nick, q, current).await
}

async fn render_profile(state: AppState, nick: String, q: PagerQuery, current: CurrentUser) -> Result<Html<String>> {
    let profile = get_user_profile(&state, &nick).await?;
    if profile.blocked && current.0.is_none() {
        return Err(AppError::Forbidden);
    }
    if !profile.activated && !current.0.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::NotFound);
    }

    let pager = Pager::new(q.offset.or(q.page.map(|p| p.saturating_sub(1) * state.config.page_size)).unwrap_or(0), state.config.page_size);
    let topics = user_topics(&state, profile.id, pager.offset, pager.limit).await?;
    let stats = user_stats(&state, profile.id).await?;
    let favorite_tags = user_tags(&state, profile.id, true).await?;
    let ignore_tags = user_tags(&state, profile.id, false).await?;
    let drafts_count = count_drafts(&state, profile.id).await.unwrap_or(0);
    let is_owner = current.0.as_ref().map(|u| u.id == profile.id).unwrap_or(false);
    let is_moderator = current.0.as_ref().map(|u| u.canmod).unwrap_or(false);
    let can_view_private = is_owner || is_moderator;
    // Was rendered with Askama's `|safe` straight from the raw DB column -
    // stored XSS via the "about me" field (POST /people/{nick}/edit). Route
    // it through the same sanitizing markup pipeline as comments/topics.
    let is_markdown = profile.userinfo_markup.as_deref().map(|m| m.to_uppercase().contains("MARKDOWN")).unwrap_or(false);
    let userinfo_html = profile.userinfo.as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|text| markup::render_message(text, Some(!is_markdown)));

    // Moderation info: matches WhoisController's banInfo/isFrozen/
    // blockable/freezable/otherUsers/userlog fields, which the previous
    // implementation didn't surface at all - a moderator had no way to see
    // ban/freeze history or other accounts sharing an email from the
    // profile page itself.
    let ban_info = if profile.blocked {
        sqlx::query_as::<_, (chrono::NaiveDateTime, String, String)>(
            r#"SELECT b.bandate, b.reason, u.nick FROM ban_info b JOIN users u ON u.id=b.ban_by WHERE b.userid=$1"#,
        )
        .bind(profile.id)
        .fetch_optional(&state.pool)
        .await?
        .map(|(bandate, reason, moderator_nick)| BanInfo { bandate_text: bandate.to_string(), reason, moderator_nick })
    } else {
        None
    };

    let frozen_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1").bind(profile.id).fetch_optional(&state.pool).await?.flatten();
    let is_frozen = frozen_until.map(|u| u > chrono::Utc::now()).unwrap_or(false);
    let frozen_until_text = is_frozen.then(|| frozen_until.unwrap().to_string());

    // UserService.isBlockable/isFreezable: reuse the exact same rules
    // enforced server-side in usermod.jsp so the profile page never shows
    // a button that would then 403.
    let blockable = current.0.as_ref().map(|u| crate::routes::admin::is_blockable(profile.id, profile.canmod, u)).unwrap_or(false);
    let freezable = current.0.as_ref().map(|u| u.canmod && !profile.canmod).unwrap_or(false);

    let other_accounts = if is_moderator {
        match profile.email.as_deref().filter(|e| !e.is_empty()) {
            Some(email) => sqlx::query_scalar("SELECT nick FROM users WHERE lower(email)=lower($1) AND id<>$2 ORDER BY nick")
                .bind(email).bind(profile.id).fetch_all(&state.pool).await.unwrap_or_default(),
            None => vec![],
        }
    } else {
        vec![]
    };

    let user_log = if is_owner || is_moderator {
        sqlx::query_as::<_, (String, chrono::DateTime<chrono::Utc>, String)>(
            r#"SELECT l.action::text, l.action_date, u.nick
               FROM user_log l JOIN users u ON u.id=l.action_userid
               WHERE l.userid=$1 ORDER BY l.action_date DESC LIMIT 20"#,
        )
        .bind(profile.id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(action, date, actor_nick)| UserLogEntry { action, date_text: date.to_string(), actor_nick })
        .collect()
    } else {
        vec![]
    };

    // UserService.getUserpic: avatar fallback style is the *viewer's*
    // profile setting, not the target's.
    let viewer_avatar_mode = match &current.0 {
        Some(viewer) => {
            let settings_text: Option<String> = sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                .bind(viewer.id).fetch_optional(&state.pool).await?;
            crate::profile::ProfileSettings::from_hstore_text(settings_text).avatar
        }
        None => crate::profile::DEFAULT_AVATAR.to_string(),
    };
    let userpic_url = crate::profile::userpic_url(
        &viewer_avatar_mode,
        true,
        profile.id == crate::routes::comments::ANONYMOUS_USER_ID,
        profile.photo.as_deref(),
        profile.email.as_deref(),
    ).unwrap_or_else(|| crate::profile::DISABLED_USERPIC.to_string());

    // UserService.getAllInvitedUsers / WhoisController "invitedUsers":
    // shown to everyone, not gated to owner/moderator, matching the original.
    let invited_users: Vec<String> = sqlx::query_scalar(
        r#"SELECT u.nick FROM user_invites i JOIN users u ON u.id=i.invited_user
           WHERE i.owner=$1 AND i.invited_user IS NOT NULL ORDER BY i.issue_date"#,
    )
    .bind(profile.id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    Ok(Html(UserTemplate {
        profile, stats, topics, favorite_tags, ignore_tags, drafts_count, is_owner, can_view_private, userinfo_html,
        ban_info, frozen_until_text, is_frozen, blockable, freezable, other_accounts, user_log, invited_users, userpic_url,
    }.render()?))
}

#[derive(Deserialize)]
pub struct WhoisQuery { nick: String }

pub async fn legacy_whois(Query(q): Query<WhoisQuery>) -> Redirect {
    Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&q.nick)))
}

pub async fn reactions(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    let rows = sqlx::query_as::<_, (i32, Option<i32>, chrono::DateTime<chrono::Utc>, String)>(
        "SELECT topic_id, comment_id, set_date, reaction FROM reactions_log WHERE origin_user=$1 ORDER BY set_date DESC LIMIT 100",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    let mut html = format!("<h1>Реакции {}</h1><ul>", html_escape::encode_text(&user.nick));
    for (topic, comment, date, reaction) in rows {
        let target = comment.map(|id| format!("#comment-{id}")).unwrap_or_default();
        html.push_str(&format!(r#"<li>{date}: <a href="/jump-message.jsp?msgid={topic}{target}">{}</a></li>"#, html_escape::encode_text(&reaction)));
    }
    html.push_str("</ul>");
    Ok(Html(html))
}

pub async fn remarks(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser) -> Result<Html<String>> {
    // Java's ShowRemarkController only ever shows the logged-in user's OWN
    // remarks about other people (keyed by user_id = viewer), never other
    // people's remarks about the profile being viewed - it is a private
    // notebook, not a public annotation feed. `nick` must equal the viewer.
    let Some(me) = current.0 else { return Err(AppError::Forbidden); };
    if !me.nick.eq_ignore_ascii_case(&nick) {
        return Err(AppError::Forbidden);
    }
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT u.nick, r.remark_text FROM user_remarks r JOIN users u ON u.id=r.ref_user_id WHERE r.user_id=$1 ORDER BY lower(u.nick)",
    )
    .bind(me.id)
    .fetch_all(&state.pool)
    .await?;
    let mut html = format!("<h1>Заметки {}</h1><ul>", html_escape::encode_text(&me.nick));
    for (target, remark) in rows {
        html.push_str(&format!("<li><b>{}</b>: {}</li>", html_escape::encode_text(&target), html_escape::encode_text(&remark)));
    }
    html.push_str("</ul>");
    Ok(Html(html))
}

pub async fn get_user(state: &AppState, nick: &str) -> Result<UserSummary> {
    sqlx::query_as::<_, UserSummary>(
        "SELECT id,nick,name,score,max_score,photo,town,regdate,canmod,COALESCE(candel,false) AS candel,blocked,userinfo FROM users WHERE lower(nick)=lower($1)",
    )
    .bind(nick)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

async fn get_user_profile(state: &AppState, nick: &str) -> Result<UserProfileData> {
    sqlx::query_as::<_, UserProfileData>(
        r#"SELECT id, nick, name,
                  COALESCE(score,0) AS score,
                  COALESCE(max_score,0) AS max_score,
                  photo, town, userinfo, url, email,
                  COALESCE(canmod,false) AS canmod,
                  COALESCE(candel,false) AS candel,
                  COALESCE(corrector,false) AS corrector,
                  COALESCE(blocked,false) AS blocked,
                  COALESCE(activated,true) AS activated,
                  to_char(regdate, 'YYYY-MM-DD HH24:MI') AS regdate_text,
                  to_char(lastlogin, 'YYYY-MM-DD HH24:MI') AS lastlogin_text,
                  userinfo_markup::text AS userinfo_markup
           FROM users WHERE lower(nick)=lower($1)"#,
    )
    .bind(nick)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

async fn user_topics(state: &AppState, user_id: i32, offset: i64, limit: i64) -> Result<Vec<TopicSummary>> {
    Ok(sqlx::query_as::<_, TopicSummary>(
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
           WHERE u.id=$1 AND NOT t.deleted AND NOT COALESCE(t.draft,false)
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY t.postdate DESC OFFSET $2 LIMIT $3"#,
    )
    .bind(user_id)
    .bind(offset)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?)
}

async fn user_stats(state: &AppState, user_id: i32) -> Result<UserStats> {
    let (topic_count, first_topic, last_topic): (i64, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT count(*)::bigint, to_char(min(postdate), 'YYYY-MM-DD HH24:MI'), to_char(max(postdate), 'YYYY-MM-DD HH24:MI') FROM topics WHERE userid=$1 AND NOT COALESCE(deleted,false) AND NOT COALESCE(draft,false)",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    let (comment_count, first_comment, last_comment): (i64, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT count(*)::bigint, to_char(min(postdate), 'YYYY-MM-DD HH24:MI'), to_char(max(postdate), 'YYYY-MM-DD HH24:MI') FROM comments WHERE userid=$1 AND NOT COALESCE(deleted,false)",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(UserStats { topic_count, comment_count, first_topic, last_topic, first_comment, last_comment })
}

async fn user_tags(state: &AppState, user_id: i32, favorite: bool) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT tv.value FROM user_tags ut JOIN tags_values tv ON tv.id=ut.tag_id WHERE ut.userid=$1 AND ut.is_favorite=$2 ORDER BY tv.value",
    )
    .bind(user_id)
    .bind(favorite)
    .fetch_all(&state.pool)
    .await?)
}

async fn count_drafts(state: &AppState, user_id: i32) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*)::bigint FROM topics WHERE userid=$1 AND COALESCE(draft,false)")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await?)
}

pub async fn deleted_topics(State(state): State<AppState>, Path(nick): Path<String>, Query(q): Query<PagerQuery>) -> Result<Html<String>> {
    render_user_topic_list(state, nick, q, "Удалённые темы", "t.deleted").await
}

pub async fn drafts(State(state): State<AppState>, Path(nick): Path<String>, Query(q): Query<PagerQuery>) -> Result<Html<String>> {
    render_user_topic_list(state, nick, q, "Черновики", "t.draft").await
}

pub async fn favs(State(state): State<AppState>, Path(nick): Path<String>, Query(q): Query<PagerQuery>) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = sqlx::query_as::<_, TopicSummary>(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, au.id AS author_id, au.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                  string_agg(tv.value, ',' ORDER BY tv.value) AS tags
           FROM memories mem
           JOIN topics t ON t.id=mem.topic
           JOIN users au ON au.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN tags tg ON tg.msgid=t.id
           LEFT JOIN tags_values tv ON tv.id=tg.tagid
           WHERE mem.userid=$1 AND NOT t.deleted
           GROUP BY t.id,au.id,g.id,s.id,mem.add_date
           ORDER BY mem.add_date DESC OFFSET $2 LIMIT $3"#,
    )
    .bind(user.id)
    .bind(pager.offset)
    .bind(pager.limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(Html(simple_topic_list(&format!("Избранное {}", user.nick), &topics)))
}

pub async fn tracked(State(state): State<AppState>, Path(nick): Path<String>, Query(q): Query<PagerQuery>) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = sqlx::query_as::<_, TopicSummary>(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, au.id AS author_id, au.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                  string_agg(tv.value, ',' ORDER BY tv.value) AS tags
           FROM memories mem
           JOIN topics t ON t.id=mem.topic
           JOIN users au ON au.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN tags tg ON tg.msgid=t.id
           LEFT JOIN tags_values tv ON tv.id=tg.tagid
           WHERE mem.userid=$1 AND mem.watch AND NOT t.deleted
           GROUP BY t.id,au.id,g.id,s.id,mem.add_date
           ORDER BY mem.add_date DESC OFFSET $2 LIMIT $3"#,
    )
    .bind(user.id)
    .bind(pager.offset)
    .bind(pager.limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(Html(simple_topic_list(&format!("Отслеживаемое {}", user.nick), &topics)))
}

async fn render_user_topic_list(state: AppState, nick: String, q: PagerQuery, title: &str, predicate: &str) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let sql = format!(r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
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
           WHERE u.id=$1 AND {predicate}
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY t.postdate DESC OFFSET $2 LIMIT $3"#);
    let topics = sqlx::query_as::<_, TopicSummary>(&sql)
        .bind(user.id)
        .bind(pager.offset)
        .bind(pager.limit)
        .fetch_all(&state.pool)
        .await?;
    Ok(Html(simple_topic_list(&format!("{title} {}", user.nick), &topics)))
}

fn simple_topic_list(title: &str, topics: &[TopicSummary]) -> String {
    let mut html = format!("<h1>{}</h1><div class=\"topic-list\">", html_escape::encode_text(title));
    for t in topics {
        html.push_str(&format!("<article class=\"topic-card\"><h3><a href=\"{}\">{}</a></h3><div class=\"meta\">{} · {} комментариев</div></article>", t.topic_url(), html_escape::encode_text(&t.title), t.postdate, t.comments));
    }
    html.push_str("</div>");
    html
}

#[derive(Deserialize)]
pub struct ProfileForm {
    pub name: Option<String>,
    pub town: Option<String>,
    pub url: Option<String>,
    pub email: Option<String>,
    pub userinfo: Option<String>,
    pub password: Option<String>,
    pub password2: Option<String>,
    pub oldpass: Option<String>,
}

pub async fn edit_profile_form(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let profile = get_user_profile(&state, &nick).await?;
    ensure_self(&current.0, &user)?;
    Ok(Html(EditProfileTemplate { user, profile }.render()?))
}

pub async fn edit_profile(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser, Form(form): axum::Form<ProfileForm>) -> Result<impl axum::response::IntoResponse> {
    let user = get_user(&state, &nick).await?;
    // Java's EditProfileController is strictly self-service (no moderator
    // override) and requires the current password before touching anything.
    ensure_self(&current.0, &user)?;

    let oldpass = form.oldpass.as_deref().unwrap_or("");
    if oldpass.is_empty() {
        return Err(AppError::BadRequest("Для изменения регистрации нужен ваш пароль".into()));
    }
    let current_hash: Option<String> = sqlx::query_scalar("SELECT passwd FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_optional(&state.pool)
        .await?;
    if !current_hash.as_deref().map(|hash| security::password::verify(oldpass, hash)).unwrap_or(false) {
        return Err(AppError::BadRequest("Неверный пароль".into()));
    }

    if let Some(password) = form.password.as_deref().filter(|s| !s.is_empty()) {
        if password.eq_ignore_ascii_case(&user.nick) {
            return Err(AppError::BadRequest("пароль не может совпадать с логином".to_string()));
        }
        if form.password2.as_deref() != Some(password) {
            return Err(AppError::BadRequest("пароли не совпадают".to_string()));
        }
        if password.chars().count() < 10 {
            return Err(AppError::BadRequest("пароль должен быть не короче 10 символов".to_string()));
        }
        let hash = security::password::hash(password).map_err(|e| AppError::BadRequest(format!("password hash error: {e}")))?;
        sqlx::query("UPDATE users SET passwd=$2 WHERE id=$1").bind(user.id).bind(hash).execute(&state.pool).await?;
    }

    // Email changes are staged into new_email and only take effect once the
    // user follows the activation-code link (see legacy::activate_post),
    // matching Java's UserDao.setNewEmail / acceptNewEmail split - the
    // previous handler wrote straight to `email` with no confirmation at all.
    let current_email: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_optional(&state.pool)
        .await?;
    let requested_email = form.email.as_deref().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty());
    let pending_email = requested_email.filter(|e| Some(e.as_str()) != current_email.as_deref());

    if let Some(ref new_email) = pending_email {
        let taken: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE lower(email)=$1 AND id<>$2")
            .bind(new_email)
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?;
        if taken.is_some() {
            return Err(AppError::BadRequest("такой email уже используется".into()));
        }
    }

    sqlx::query("UPDATE users SET name=$2,town=$3,url=$4,userinfo=$5,new_email=COALESCE($6,new_email) WHERE id=$1")
        .bind(user.id)
        .bind(form.name)
        .bind(form.town)
        .bind(form.url)
        .bind(form.userinfo)
        .bind(&pending_email)
        .execute(&state.pool)
        .await?;

    if pending_email.is_some() {
        Ok(Html(
            "<h1>Обновление регистрации прошло успешно</h1><p>Ожидайте письма с кодом активации смены email.</p>".to_string(),
        ).into_response())
    } else {
        Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&user.nick))).into_response())
    }
}

pub async fn settings(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    // Java's EditSettingsController is strictly self-service, no moderator override.
    ensure_self(&current.0, &user)?;
    let settings_text: Option<String> = sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
        .bind(user.id)
        .fetch_optional(&state.pool)
        .await?;
    let settings = ProfileSettings::from_hstore_text(settings_text);
    Ok(Html(SettingsTemplate {
        themes: settings.theme_options(user.score.unwrap_or(0)),
        avatars: settings.avatar_options(),
        tracker_modes: settings.tracker_options(),
        format_modes: settings.format_options(),
        topic_values: settings.topic_options(),
        message_values: settings.message_options(),
        user,
        settings,
    }.render()?))
}

pub async fn save_settings(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser, Form(form): axum::Form<HashMap<String, String>>) -> Result<Redirect> {
    let user = get_user(&state, &nick).await?;
    ensure_self(&current.0, &user)?;
    let settings = ProfileSettings::from_form(&form);
    let (keys, values) = settings.to_hstore_arrays();
    sqlx::query(
        "INSERT INTO user_settings(id,settings) VALUES($1,hstore($2::text[],$3::text[])) ON CONFLICT(id) DO UPDATE SET settings=EXCLUDED.settings",
    )
    .bind(user.id)
    .bind(keys)
    .bind(values)
    .execute(&state.pool)
    .await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&user.nick))))
}

#[derive(Deserialize)]
pub struct RemarkForm { pub remark: String }

pub async fn remark_form(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser) -> Result<Html<String>> {
    let target = get_user(&state, &nick).await?;
    let Some(me) = current.0 else { return Err(AppError::Forbidden); };
    if me.id == target.id {
        return Err(AppError::BadRequest("Нельзя оставить заметку самому себе".into()));
    }
    let remark: Option<String> = sqlx::query_scalar("SELECT remark_text FROM user_remarks WHERE user_id=$1 AND ref_user_id=$2")
        .bind(me.id).bind(target.id).fetch_optional(&state.pool).await?;
    Ok(Html(format!(r#"
<h1>Заметка о {}</h1>
<form method="post" action="/people/{}/remark">
<textarea name="remark" rows="8">{}</textarea>
<button type="submit">Сохранить</button>
</form>
"#, html_escape::encode_text(&target.nick), urlencoding::encode(&target.nick), html_escape::encode_text(remark.as_deref().unwrap_or("")))))
}

pub async fn save_remark(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser, Form(form): axum::Form<RemarkForm>) -> Result<Redirect> {
    let target = get_user(&state, &nick).await?;
    let Some(me) = current.0 else { return Err(AppError::Forbidden); };
    if me.id == target.id {
        return Err(AppError::BadRequest("Нельзя оставить заметку самому себе".into()));
    }
    let text: String = form.remark.chars().take(255).collect();
    if text.trim().is_empty() {
        sqlx::query("DELETE FROM user_remarks WHERE user_id=$1 AND ref_user_id=$2")
            .bind(me.id).bind(target.id).execute(&state.pool).await?;
    } else {
        sqlx::query(
            "INSERT INTO user_remarks(user_id,ref_user_id,remark_text) VALUES($1,$2,$3) ON CONFLICT(user_id,ref_user_id) DO UPDATE SET remark_text=EXCLUDED.remark_text",
        )
        .bind(me.id).bind(target.id).bind(text).execute(&state.pool).await?;
    }
    Ok(Redirect::to(&format!("/people/{}/remarks", urlencoding::encode(&me.nick))))
}

/// Java's `/people/{nick}/profile/wipe` is GET/HEAD-only and purely a
/// moderator confirmation view (`UserModificationController.wipe`) - the
/// actual destructive action lives behind a separate POST to
/// `/usermod.jsp?action=block-n-delete-comments`. The previous Rust handler
/// collapsed both into one plain GET that any logged-in user (self included)
/// could trigger with no confirmation step - fixed to match: moderator-only,
/// no side effects, renders a form that posts to the real action endpoint.
pub async fn profile_wipe(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser) -> Result<Html<String>> {
    let moderator = current.0.as_ref().filter(|u| u.canmod).ok_or(AppError::Forbidden)?;
    let user = get_user(&state, &nick).await?;
    if !crate::routes::admin::is_blockable(user.id, user.canmod, moderator) {
        return Err(AppError::Forbidden);
    }
    if user.blocked.unwrap_or(false) {
        return Err(AppError::BadRequest("Пользователь уже блокирован".into()));
    }
    let comment_count: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM comments WHERE userid=$1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Html(format!(r#"
<h1>Заблокировать и удалить сообщения {nick}</h1>
<p>Комментариев будет удалено: {comment_count}</p>
<form method="post" action="/usermod.jsp">
  <input type="hidden" name="action" value="block-n-delete-comments">
  <input type="hidden" name="id" value="{id}">
  <label>Причина <input name="reason"></label>
  <button type="submit">Заблокировать и удалить</button>
</form>
"#, nick = html_escape::encode_text(&user.nick), id = user.id)))
}

fn ensure_self_or_moderator(current: &Option<UserSummary>, target: &UserSummary) -> Result<()> {
    let Some(current) = current else { return Err(AppError::Forbidden); };
    if current.id == target.id || current.canmod { Ok(()) } else { Err(AppError::Forbidden) }
}

/// Strictly self-service, no moderator override - matches Java controllers
/// (e.g. EditProfileController) that reject even moderators editing someone
/// else's registration through this path.
fn ensure_self(current: &Option<UserSummary>, target: &UserSummary) -> Result<()> {
    let Some(current) = current else { return Err(AppError::Forbidden); };
    if current.id == target.id { Ok(()) } else { Err(AppError::Forbidden) }
}
