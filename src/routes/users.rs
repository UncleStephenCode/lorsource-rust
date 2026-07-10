use crate::{
    auth::CurrentUser,
    error::{AppError, Result},
    models::{PagerQuery, TopicSummary, UserSummary},
    pagination::Pager,
    profile::{ChoiceOption, NumberOption, ProfileSettings, ThemeOption},
    security,
    state::AppState,
};
use askama::Template;
use axum::{extract::{Path, Query, State}, response::{Html, Redirect}, Form};
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
}

impl UserProfileData {
    fn status(&self) -> &'static str {
        if self.blocked { "заблокирован" }
        else if !self.activated { "не активирован" }
        else if self.score >= 100 { "активный пользователь" }
        else { "новый пользователь" }
    }

    fn photo_url(&self) -> Option<String> {
        self.photo.as_ref().map(|p| if p.starts_with('/') || p.starts_with("http://") || p.starts_with("https://") {
            p.clone()
        } else {
            format!("/photos/{p}")
        })
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

pub async fn profile(State(state): State<AppState>, Path(nick): Path<String>, Query(q): Query<PagerQuery>, current: CurrentUser) -> Result<Html<String>> {
    render_profile(state, nick, q, current).await
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

    Ok(Html(UserTemplate { profile, stats, topics, favorite_tags, ignore_tags, drafts_count, is_owner, can_view_private }.render()?))
}

#[derive(Deserialize)]
pub struct WhoisQuery { nick: String }

pub async fn legacy_whois(Query(q): Query<WhoisQuery>) -> Redirect {
    Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&q.nick)))
}

pub async fn reactions(State(state): State<AppState>, Path(nick): Path<String>) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
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

pub async fn remarks(State(state): State<AppState>, Path(nick): Path<String>) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT u.nick, r.remark FROM user_remarks r JOIN users u ON u.id=r.userid WHERE r.who=$1 ORDER BY lower(u.nick)",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    let mut html = format!("<h1>Заметки о {}</h1><ul>", html_escape::encode_text(&user.nick));
    for (author, remark) in rows {
        html.push_str(&format!("<li><b>{}</b>: {}</li>", html_escape::encode_text(&author), html_escape::encode_text(&remark)));
    }
    html.push_str("</ul>");
    Ok(Html(html))
}

pub async fn get_user(state: &AppState, nick: &str) -> Result<UserSummary> {
    sqlx::query_as::<_, UserSummary>(
        "SELECT id,nick,name,score,max_score,photo,town,regdate,canmod,blocked,userinfo FROM users WHERE lower(nick)=lower($1)",
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
                  to_char(lastlogin, 'YYYY-MM-DD HH24:MI') AS lastlogin_text
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
}

pub async fn edit_profile_form(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let profile = get_user_profile(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    Ok(Html(EditProfileTemplate { user, profile }.render()?))
}

pub async fn edit_profile(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser, Form(form): axum::Form<ProfileForm>) -> Result<Redirect> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    sqlx::query("UPDATE users SET name=$2,town=$3,url=$4,userinfo=$5,email=$6 WHERE id=$1")
        .bind(user.id)
        .bind(form.name)
        .bind(form.town)
        .bind(form.url)
        .bind(form.userinfo)
        .bind(form.email)
        .execute(&state.pool)
        .await?;
    if let Some(password) = form.password.as_deref().filter(|s| !s.is_empty()) {
        if form.password2.as_deref() != Some(password) {
            return Err(AppError::BadRequest("пароли не совпадают".to_string()));
        }
        if password.chars().count() < 10 {
            return Err(AppError::BadRequest("пароль должен быть не короче 10 символов".to_string()));
        }
        let hash = security::password::hash(password).map_err(|e| AppError::BadRequest(format!("password hash error: {e}")))?;
        sqlx::query("UPDATE users SET passwd=$2 WHERE id=$1").bind(user.id).bind(hash).execute(&state.pool).await?;
    }
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&user.nick))))
}

pub async fn settings(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    let settings_text: Option<String> = sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
        .bind(user.id)
        .fetch_optional(&state.pool)
        .await?;
    let settings = ProfileSettings::from_hstore_text(settings_text);
    Ok(Html(SettingsTemplate {
        themes: settings.theme_options(),
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
    ensure_self_or_moderator(&current.0, &user)?;
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
    let remark: Option<String> = sqlx::query_scalar("SELECT remark FROM user_remarks WHERE userid=$1 AND who=$2")
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
    sqlx::query("INSERT INTO user_remarks(userid,who,remark) VALUES($1,$2,$3) ON CONFLICT(userid,who) DO UPDATE SET remark=EXCLUDED.remark")
        .bind(me.id).bind(target.id).bind(form.remark).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/people/{}/remarks", urlencoding::encode(&target.nick))))
}

pub async fn profile_wipe(State(state): State<AppState>, Path(nick): Path<String>, current: CurrentUser) -> Result<Redirect> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    sqlx::query("UPDATE users SET userinfo=NULL, photo=NULL, town=NULL, url=NULL WHERE id=$1").bind(user.id).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&user.nick))))
}

fn ensure_self_or_moderator(current: &Option<UserSummary>, target: &UserSummary) -> Result<()> {
    let Some(current) = current else { return Err(AppError::Forbidden); };
    if current.id == target.id || current.canmod { Ok(()) } else { Err(AppError::Forbidden) }
}
