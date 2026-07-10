use crate::{error::{AppError, Result}, state::AppState};
use axum::{extract::{Query, State}, response::Redirect, Form};
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
