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
    Ok(Html(format!("<h1>Реакции {}</h1><p>Модуль реакций вынесен в routes/api.rs и готов к расширению.</p>", user.nick)))
}

pub async fn remarks(State(state): State<AppState>, Path(nick): Path<String>) -> Result<Html<String>> {
    let user = get_user(&state, &nick).await?;
    Ok(Html(format!("<h1>Заметки о {}</h1><p>Персональные заметки оставлены как совместимый маршрут.</p>", user.nick)))
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
