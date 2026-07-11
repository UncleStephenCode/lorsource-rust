use crate::{auth::CurrentUser, error::{AppError, Result}, markup, state::AppState};
use axum::{extract::{Path, Query, State}, response::{Html, Redirect}, Form};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct JumpQuery { pub msgid: i32 }

pub async fn jump_message(State(state): State<AppState>, Query(q): Query<JumpQuery>) -> Result<Redirect> {
    if let Some((section, group, topic_id, comment_id)) = locate_topic_or_comment(&state, q.msgid).await? {
        let anchor = comment_id.map(|id| format!("#comment-{id}")).unwrap_or_default();
        Ok(Redirect::to(&format!("/{section}/{group}/{topic_id}{anchor}")))
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

pub async fn add_comment(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<CommentForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let id = insert_comment(&state, user.id, form).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={id}")))
}

pub async fn add_comment_ajax(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<CommentForm>) -> Result<axum::Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let id = insert_comment(&state, user.id, form).await?;
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

/// Default upstream config (config.properties.dist): comments are editable
/// only by their author, within 30 minutes of posting, only if they have no
/// replies yet, and only once the author has score >= 45.
/// comment.isModeratorAllowedToEdit defaults to false, so moderators do not
/// get a bypass here in the default configuration.
const COMMENT_EDIT_WINDOW_MINUTES: i64 = 30;
const COMMENT_EDIT_MIN_SCORE: i32 = 45;

pub async fn edit_comment(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<EditCommentForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let row: (i32, i32, bool, chrono::DateTime<chrono::Utc>, bool) = sqlx::query_as(
        r#"SELECT c.topic, c.userid, c.deleted, c.postdate,
                  EXISTS(SELECT 1 FROM comments r WHERE r.replyto=c.id) AS has_replies
           FROM comments c WHERE c.id=$1"#,
    )
    .bind(form.msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (topic_id, author_id, deleted, postdate, has_replies) = row;
    let topic_deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1").bind(topic_id).fetch_one(&state.pool).await?;

    if deleted || topic_deleted {
        return Err(AppError::BadRequest("тема или комментарий удалены".into()));
    }
    if user.id != author_id {
        return Err(AppError::Forbidden);
    }
    if has_replies {
        return Err(AppError::BadRequest("редактирование комментариев с ответами запрещено".into()));
    }
    if user.score.unwrap_or(0) < COMMENT_EDIT_MIN_SCORE {
        return Err(AppError::Forbidden);
    }
    if chrono::Utc::now() > postdate + chrono::Duration::minutes(COMMENT_EDIT_WINDOW_MINUTES) {
        return Err(AppError::BadRequest("истек срок редактирования".into()));
    }

    sqlx::query("UPDATE msgbase SET message=$2 WHERE id=$1").bind(form.msgid).bind(&form.msg).execute(&state.pool).await?;
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
pub struct CommentAction {
    pub msgid: i32,
    pub reason: Option<String>,
    pub bonus: Option<i32>,
}

/// Matches TopicPermissionService.DeletePeriod: authors may delete their own
/// comment for 3 hours after posting (and only if nobody has replied yet).
/// Moderators bypass this window entirely.
const COMMENT_DELETE_WINDOW_HOURS: i64 = 3;

pub async fn delete_comment(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<CommentAction>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let row: (i32, i32, bool, chrono::DateTime<chrono::Utc>, bool) = sqlx::query_as(
        r#"SELECT c.topic, c.userid, c.deleted, c.postdate,
                  EXISTS(SELECT 1 FROM comments r WHERE r.replyto=c.id) AS has_replies
           FROM comments c WHERE c.id=$1"#,
    )
    .bind(form.msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (topic_id, author_id, deleted, postdate, has_replies) = row;
    if deleted {
        return Err(AppError::BadRequest("комментарий уже удален".into()));
    }
    let topic_deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1").bind(topic_id).fetch_one(&state.pool).await?;

    let deletable = user.canmod || {
        let within_window = chrono::Utc::now() <= postdate + chrono::Duration::hours(COMMENT_DELETE_WINDOW_HOURS);
        user.id == author_id && !has_replies && !topic_deleted && within_window
    };
    if !deletable {
        return Err(AppError::Forbidden);
    }

    let bonus = if user.canmod && user.id != author_id {
        form.bonus.unwrap_or(0).clamp(0, 20)
    } else {
        0
    };
    let reason = form.reason.clone().unwrap_or_default();

    sqlx::query("UPDATE comments SET deleted=true WHERE id=$1").bind(form.msgid).execute(&state.pool).await?;
    sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES($1,$2,$3,now(),$4) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now(), bonus=EXCLUDED.bonus")
        .bind(form.msgid).bind(user.id).bind(&reason).bind(bonus).execute(&state.pool).await?;
    if bonus != 0 {
        sqlx::query("UPDATE users SET score=GREATEST(score-$2,0) WHERE id=$1").bind(author_id).bind(bonus).execute(&state.pool).await?;
    }
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

pub async fn undelete_comment(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<CommentAction>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if !user.canmod {
        return Err(AppError::Forbidden);
    }
    let row: (i32, bool) = sqlx::query_as("SELECT topic, deleted FROM comments WHERE id=$1")
        .bind(form.msgid)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let (topic_id, deleted) = row;
    if !deleted {
        return Err(AppError::BadRequest("комментарий не удален".into()));
    }
    let topic_deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1").bind(topic_id).fetch_one(&state.pool).await?;
    if topic_deleted {
        return Err(AppError::Forbidden);
    }
    // Mirrors TopicPermissionService.isUndeletable: a comment cannot be
    // undeleted if its own author is the one who deleted it (self-moderation
    // is respected, only another moderator's deletion can be reversed).
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM comments WHERE id=$1").bind(form.msgid).fetch_one(&state.pool).await?;
    let delby: Option<i32> = sqlx::query_scalar("SELECT delby FROM del_info WHERE msgid=$1").bind(form.msgid).fetch_optional(&state.pool).await?;
    if delby == Some(author_id) {
        return Err(AppError::Forbidden);
    }

    sqlx::query("UPDATE comments SET deleted=false WHERE id=$1").bind(form.msgid).execute(&state.pool).await?;
    sqlx::query("DELETE FROM del_info WHERE msgid=$1").bind(form.msgid).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

async fn insert_comment(state: &AppState, user_id: i32, form: CommentForm) -> Result<i32> {
    let mut tx = state.pool.begin().await?;
    let id: i32 = sqlx::query_scalar("SELECT nextval('s_msgid')::int").fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO msgbase(id, message, bbcode) VALUES($1,$2,true)").bind(id).bind(&form.msg).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO comments(id, topic, userid, title, postdate, replyto) VALUES($1,$2,$3,$4,now(),$5)",
    )
    .bind(id)
    .bind(form.topic)
    .bind(user_id)
    .bind(form.title.unwrap_or_else(|| "Комментарий".into()))
    .bind(form.replyto)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE topics SET stat1=stat1+1,lastmod=now() WHERE id=$1").bind(form.topic).execute(&mut *tx).await?;

    // Matches CommentCreateService.notifyReply / UserEventDao.insertCommentWatchNotification:
    // notify the parent comment's author (REPLY) and topic watchers (WATCH),
    // skipping the commenter themselves and anyone who has the commenter ignored.
    let mut notified: Vec<i32> = Vec::new();

    let mut parent_author: Option<i32> = None;
    if let Some(replyto) = form.replyto {
        if let Some(parent_userid) = sqlx::query_scalar::<_, i32>("SELECT userid FROM comments WHERE id=$1")
            .bind(replyto)
            .fetch_optional(&mut *tx)
            .await?
        {
            parent_author = Some(parent_userid);
            if parent_userid != user_id {
                let ignored: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ignore_list WHERE userid=$1 AND ignored=$2)")
                    .bind(parent_userid)
                    .bind(user_id)
                    .fetch_one(&mut *tx)
                    .await?;
                if !ignored {
                    sqlx::query("INSERT INTO user_events(userid,type,private,message_id,comment_id) VALUES($1,'REPLY',false,$2,$3)")
                        .bind(parent_userid)
                        .bind(form.topic)
                        .bind(id)
                        .execute(&mut *tx)
                        .await?;
                    notified.push(parent_userid);
                }
            }
        }
    }

    let watchers: Vec<i32> = sqlx::query_scalar(
        r#"SELECT m.userid FROM memories m
           WHERE m.topic=$1 AND m.watch AND m.userid<>$2 AND m.userid<>COALESCE($3,0)
             AND NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=m.userid AND il.ignored=$2)"#,
    )
    .bind(form.topic)
    .bind(user_id)
    .bind(parent_author)
    .fetch_all(&mut *tx)
    .await?;
    for watcher in &watchers {
        sqlx::query("INSERT INTO user_events(userid,type,private,message_id,comment_id) VALUES($1,'WATCH',false,$2,$3)")
            .bind(watcher)
            .bind(form.topic)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        notified.push(*watcher);
    }

    if !notified.is_empty() {
        notified.sort_unstable();
        notified.dedup();
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=ANY($1)")
            .bind(&notified)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(id)
}

async fn locate_topic_or_comment(state: &AppState, msgid: i32) -> Result<Option<(String, String, i32, Option<i32>)>> {
    let row = sqlx::query_as::<_, (String, String, i32, Option<i32>)>(
        r#"SELECT CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section,
                  g.urlname, t.id, NULL::integer AS comment_id
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1
           UNION ALL
           SELECT CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section,
                  g.urlname, t.id, c.id AS comment_id
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
