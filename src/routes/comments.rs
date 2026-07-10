use crate::{auth::CurrentUser, error::{AppError, Result}, markup, state::AppState};
use axum::{extract::{Path, Query, State}, response::{Html, Redirect}, Form};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct JumpQuery { pub msgid: i32 }

pub async fn jump_message(State(state): State<AppState>, Query(q): Query<JumpQuery>) -> Result<Redirect> {
    if let Some((section, group, topic_id)) = locate_topic_or_comment(&state, q.msgid).await? {
        Ok(Redirect::to(&format!("/{section}/{group}/{topic_id}#comment-{}", q.msgid)))
    } else {
        Err(AppError::NotFound)
    }
}

#[derive(Deserialize)]
pub struct CommentForm {
    pub topic: i32,
    pub replyto: Option<i32>,
    pub title: Option<String>,
    pub msg: String,
}


pub async fn add_comment_form(State(state): State<AppState>, Query(q): Query<CommentFormQuery>) -> Result<Html<String>> {
    let topic = crate::routes::topics::get_topic(&state, q.topic).await?;
    let reply_input = q.replyto.map(|id| format!(r#"<input type="hidden" name="replyto" value="{id}">"#)).unwrap_or_default();
    Ok(Html(format!(r#"
<h1>Добавить комментарий</h1>
<p><a href="{url}">{title}</a></p>
<form method="post" action="/add_comment.jsp" class="form wide">
  <input type="hidden" name="topic" value="{topic_id}">
  {reply_input}
  <label>Заголовок <input name="title" value="Комментарий"></label>
  <label>Комментарий <textarea name="msg" rows="12" required></textarea></label>
  <button type="submit">Отправить</button>
</form>
"#, url = topic.topic_url(), title = html_escape::encode_text(&topic.title), topic_id = topic.id)))
}

#[derive(Deserialize)]
pub struct CommentFormQuery {
    pub topic: i32,
    pub replyto: Option<i32>,
}

pub async fn add_comment(State(state): State<AppState>, Form(form): Form<CommentForm>) -> Result<Redirect> {
    let id = insert_comment(&state, form).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")))
}

pub async fn add_comment_ajax(State(state): State<AppState>, Form(form): Form<CommentForm>) -> Result<axum::Json<serde_json::Value>> {
    let id = insert_comment(&state, form).await?;
    Ok(axum::Json(serde_json::json!({"id": id, "ok": true})))
}

pub async fn comment_message(Query(q): Query<JumpQuery>) -> Redirect {
    Redirect::to(&format!("/jump-message.jsp?msgid={}", q.msgid))
}

pub async fn edit_comment_form() -> Result<&'static str> {
    Ok("Редактирование комментариев реализовано как POST /edit_comment")
}

#[derive(Deserialize)]
pub struct EditCommentForm { pub msgid: i32, pub msg: String, pub title: Option<String> }

pub async fn edit_comment(State(state): State<AppState>, Form(form): Form<EditCommentForm>) -> Result<Redirect> {
    sqlx::query("UPDATE msgbase SET message=$2 WHERE id=$1").bind(form.msgid).bind(form.msg).execute(&state.pool).await?;
    if let Some(title) = form.title {
        sqlx::query("UPDATE comments SET title=$2 WHERE id=$1").bind(form.msgid).bind(title).execute(&state.pool).await?;
    }
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}


pub async fn delete_comment_form(State(state): State<AppState>, Query(q): Query<JumpQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    let row: (i32, String, String) = sqlx::query_as(
        "SELECT c.topic, c.title, u.nick FROM comments c JOIN users u ON u.id=c.userid WHERE c.id=$1",
    )
    .bind(q.msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Html(format!(r#"
<h1>Удалить комментарий #{}</h1>
<p>Тема #{} · {} · автор {}</p>
<form method="post" action="/delete_comment.jsp">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Удалить</button>
</form>
"#, q.msgid, row.0, html_escape::encode_text(&row.1), html_escape::encode_text(&row.2), q.msgid)))
}

pub async fn undelete_comment_form(Query(q): Query<JumpQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Восстановить комментарий #{}</h1>
<form method="post" action="/undelete_comment">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Восстановить</button>
</form>
"#, q.msgid, q.msgid)))
}

#[derive(Deserialize)]
pub struct CommentAction { pub msgid: i32 }

pub async fn delete_comment(State(state): State<AppState>, Form(form): Form<CommentAction>) -> Result<Redirect> {
    sqlx::query("UPDATE comments SET deleted=true WHERE id=$1").bind(form.msgid).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn undelete_comment(State(state): State<AppState>, Form(form): Form<CommentAction>) -> Result<Redirect> {
    sqlx::query("UPDATE comments SET deleted=false WHERE id=$1").bind(form.msgid).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

async fn insert_comment(state: &AppState, form: CommentForm) -> Result<i32> {
    let mut tx = state.pool.begin().await?;
    let id: i32 = sqlx::query_scalar("SELECT nextval('s_msgid')::int").fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO msgbase(id, message, bbcode) VALUES($1,$2,true)").bind(id).bind(&form.msg).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO comments(id, topic, userid, title, postdate, replyto) VALUES($1,$2,1,$3,now(),$4)",
    )
    .bind(id)
    .bind(form.topic)
    .bind(form.title.unwrap_or_else(|| "Комментарий".into()))
    .bind(form.replyto)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE topics SET stat1=stat1+1,lastmod=now() WHERE id=$1").bind(form.topic).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(id)
}

async fn locate_topic_or_comment(state: &AppState, msgid: i32) -> Result<Option<(String, String, i32)>> {
    let row = sqlx::query_as::<_, (String, String, i32)>(
        r#"SELECT CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section,
                  g.urlname, t.id
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1
           UNION ALL
           SELECT CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section,
                  g.urlname, t.id
           FROM comments c JOIN topics t ON t.id=c.topic JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE c.id=$1
           LIMIT 1"#,
    )
    .bind(msgid)
    .fetch_optional(&state.pool)
    .await?;
    Ok(row)
}


pub async fn deleted_comments_by_user(State(state): State<AppState>, Path(nick): Path<String>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    let comments = sqlx::query_as::<_, crate::models::CommentItem>(
        r#"SELECT c.id, c.topic, c.replyto, c.title, m.message, c.postdate, u.id AS author_id, u.nick AS author, c.deleted
           FROM comments c
           JOIN msgbase m ON m.id=c.id
           JOIN users u ON u.id=c.userid
           WHERE lower(u.nick)=lower($1) AND c.deleted
           ORDER BY c.postdate DESC LIMIT 100"#,
    )
    .bind(&nick)
    .fetch_all(&state.pool)
    .await?;
    let mut html = format!("<h1>Удалённые комментарии {}</h1>", html_escape::encode_text(&nick));
    for c in comments {
        html.push_str(&format!("<article id=\"comment-{}\"><h3>{}</h3><p>topic #{} · {}</p><div>{}</div></article>",
            c.id, html_escape::encode_text(&c.title), c.topic, c.postdate, markup::render_message(&c.message, Some(true))));
    }
    Ok(Html(html))
}
