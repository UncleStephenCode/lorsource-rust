use crate::{error::{AppError, Result}, models::{PagerQuery, TopicSummary, UserSummary}, pagination::Pager, state::AppState};
use askama::Template;
use axum::{extract::{Path, Query, State}, response::{Html, Redirect}};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "user.html")]
struct UserTemplate {
    user: UserSummary,
    topics: Vec<TopicSummary>,
    full: bool,
}

pub async fn profile(State(state): State<AppState>, Path(nick): Path<String>, Query(q): Query<PagerQuery>) -> Result<Html<String>> {
    render_profile(state, nick, q, false).await
}

pub async fn profile_full(State(state): State<AppState>, Path(nick): Path<String>, Query(q): Query<PagerQuery>) -> Result<Html<String>> {
    render_profile(state, nick, q, true).await
}

async fn render_profile(state: AppState, nick: String, q: PagerQuery, full: bool) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = sqlx::query_as::<_, TopicSummary>(
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
           WHERE u.id=$1 AND NOT t.deleted
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY t.postdate DESC OFFSET $2 LIMIT $3"#,
    )
    .bind(user.id)
    .bind(pager.offset)
    .bind(pager.limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(Html(UserTemplate { user, topics, full }.render()?))
}

#[derive(Deserialize)]
pub struct WhoisQuery { nick: String }

pub async fn legacy_whois(Query(q): Query<WhoisQuery>) -> Redirect {
    Redirect::to(&format!("/people/{}", urlencoding::encode(&q.nick)))
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
        html.push_str(&format!("<li>{date}: <a href="/jump-message.jsp?msgid={topic}{target}">{}</a></li>", html_escape::encode_text(&reaction)));
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
    Ok(Html(UserTemplate { user, topics, full: true }.render()?))
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
    Ok(Html(UserTemplate { user, topics, full: true }.render()?))
}

async fn render_user_topic_list(state: AppState, nick: String, q: PagerQuery, _title: &str, predicate: &str) -> Result<Html<String>> {
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
    Ok(Html(UserTemplate { user, topics, full: true }.render()?))
}

#[derive(Deserialize)]
pub struct ProfileForm {
    pub name: Option<String>,
    pub town: Option<String>,
    pub url: Option<String>,
    pub userinfo: Option<String>,
}

pub async fn edit_profile_form(State(state): State<AppState>, Path(nick): Path<String>, current: crate::auth::CurrentUser) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    Ok(Html(format!(r#"
<h1>Редактировать профиль {nick}</h1>
<form method="post" action="/people/{nick}/edit" class="form wide">
  <label>Имя <input name="name" value="{name}"></label>
  <label>Город <input name="town" value="{town}"></label>
  <label>О себе <textarea name="userinfo" rows="10">{userinfo}</textarea></label>
  <button type="submit">Сохранить</button>
</form>
"#, nick=html_escape::encode_double_quoted_attribute(&user.nick), name=html_escape::encode_double_quoted_attribute(user.name.as_deref().unwrap_or("")), town=html_escape::encode_double_quoted_attribute(user.town.as_deref().unwrap_or("")), userinfo=html_escape::encode_text(user.userinfo.as_deref().unwrap_or("")))))
}

pub async fn edit_profile(State(state): State<AppState>, Path(nick): Path<String>, current: crate::auth::CurrentUser, Form(form): axum::Form<ProfileForm>) -> Result<Redirect> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    sqlx::query("UPDATE users SET name=$2,town=$3,url=$4,userinfo=$5 WHERE id=$1")
        .bind(user.id)
        .bind(form.name)
        .bind(form.town)
        .bind(form.url)
        .bind(form.userinfo)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&user.nick))))
}

pub async fn settings(State(state): State<AppState>, Path(nick): Path<String>, current: crate::auth::CurrentUser) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    let settings: Option<String> = sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
        .bind(user.id)
        .fetch_optional(&state.pool)
        .await?;
    Ok(Html(format!("<h1>Настройки {}</h1><pre>{}</pre>", html_escape::encode_text(&user.nick), html_escape::encode_text(settings.as_deref().unwrap_or("")))))
}

pub async fn save_settings(State(state): State<AppState>, Path(nick): Path<String>, current: crate::auth::CurrentUser, Form(form): axum::Form<std::collections::HashMap<String, String>>) -> Result<Redirect> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    let (keys, values): (Vec<String>, Vec<String>) = form.into_iter().unzip();
    sqlx::query(
        "INSERT INTO user_settings(id,settings) VALUES($1,hstore($2::text[],$3::text[])) ON CONFLICT(id) DO UPDATE SET settings=EXCLUDED.settings",
    )
    .bind(user.id)
    .bind(keys)
    .bind(values)
    .execute(&state.pool)
    .await?;
    Ok(Redirect::to(&format!("/people/{}/settings", urlencoding::encode(&user.nick))))
}

#[derive(Deserialize)]
pub struct RemarkForm { pub remark: String }

pub async fn remark_form(State(state): State<AppState>, Path(nick): Path<String>, current: crate::auth::CurrentUser) -> Result<Html<String>> {
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

pub async fn save_remark(State(state): State<AppState>, Path(nick): Path<String>, current: crate::auth::CurrentUser, Form(form): axum::Form<RemarkForm>) -> Result<Redirect> {
    let target = get_user(&state, &nick).await?;
    let Some(me) = current.0 else { return Err(AppError::Forbidden); };
    sqlx::query("INSERT INTO user_remarks(userid,who,remark) VALUES($1,$2,$3) ON CONFLICT(userid,who) DO UPDATE SET remark=EXCLUDED.remark")
        .bind(me.id).bind(target.id).bind(form.remark).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/people/{}/remarks", urlencoding::encode(&target.nick))))
}

pub async fn profile_wipe(State(state): State<AppState>, Path(nick): Path<String>, current: crate::auth::CurrentUser) -> Result<Redirect> {
    let user = get_user(&state, &nick).await?;
    ensure_self_or_moderator(&current.0, &user)?;
    sqlx::query("UPDATE users SET userinfo=NULL, photo=NULL, town=NULL, url=NULL WHERE id=$1").bind(user.id).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&user.nick))))
}

fn ensure_self_or_moderator(current: &Option<UserSummary>, target: &UserSummary) -> Result<()> {
    let Some(current) = current else { return Err(AppError::Forbidden); };
    if current.id == target.id || current.canmod { Ok(()) } else { Err(AppError::Forbidden) }
}
