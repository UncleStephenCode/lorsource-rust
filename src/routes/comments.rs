use crate::{
    auth::CurrentUser,
    error::{AppError, Result},
    markup,
    state::AppState,
};
use askama::Template;
use axum::{
    Form,
    extract::{Path, Query, State},
    response::{Html, Redirect},
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct JumpQuery {
    pub msgid: i32,
}

pub async fn jump_message(
    State(state): State<AppState>,
    Query(q): Query<JumpQuery>,
) -> Result<Redirect> {
    if let Some((section, group, topic_id, comment_id)) =
        locate_topic_or_comment(&state, q.msgid).await?
    {
        let anchor = comment_id
            .map(|id| format!("#comment-{id}"))
            .unwrap_or_default();
        Ok(Redirect::to(&format!(
            "/{section}/{group}/{topic_id}{anchor}"
        )))
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

#[derive(Template)]
#[template(path = "comment_form.html")]
struct CommentFormTemplate {
    topic_id: i32,
    topic_title: String,
    topic_url: String,
    replyto: Option<i32>,
    csrf_token: String,
    format_mode: String,
    format_title: String,
}

async fn comment_format(state: &AppState, user_id: i32) -> Result<(String, String, String)> {
    let settings_text: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    let mode = crate::profile::ProfileSettings::from_hstore_text(settings_text).format_mode;
    let title = crate::profile::FORMAT_MODES
        .iter()
        .find(|(id, _, _)| *id == mode)
        .map(|(_, title, _)| *title)
        .unwrap_or("Markdown")
        .to_string();
    let markup = match mode.as_str() {
        "markdown" => "MARKDOWN",
        "ntobr" => "BBCODE_ULB",
        "plain" => "PLAIN",
        _ => "BBCODE_TEX",
    };
    Ok((mode, title, markup.to_string()))
}

pub async fn add_comment_form(
    State(state): State<AppState>,
    Query(q): Query<CommentFormQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let topic = crate::routes::topics::get_topic(&state, q.topic).await?;
    let (format_mode, format_title, _) = match user {
        Some(user) => comment_format(&state, user.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    let topic_url = topic.topic_url();
    Ok(Html(
        CommentFormTemplate {
            topic_id: topic.id,
            topic_title: topic.title,
            topic_url,
            replyto: q.replyto.filter(|id| *id > 0),
            csrf_token,
            format_mode,
            format_title,
        }
        .render()?,
    ))
}

#[derive(Deserialize)]
pub struct CommentFormQuery {
    pub topic: i32,
    pub replyto: Option<i32>,
}

/// Java redirects comment actions to `topic.getLink + "?cid=" + msgid`
/// (see AddCommentController.scala:132, EditCommentController, DeleteCommentController)
/// rather than through a jump/redirect endpoint. Reuses the topic/comment
/// lookup already needed by `/jump-message.jsp` so both stay consistent.
async fn comment_link(state: &AppState, comment_id: i32) -> Result<String> {
    match locate_topic_or_comment(state, comment_id).await? {
        Some((section, group, topic_id, _)) => {
            Ok(format!("/{section}/{group}/{topic_id}?cid={comment_id}"))
        }
        None => Ok(format!("/jump-message.jsp?msgid={comment_id}")),
    }
}

pub async fn add_comment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<CommentForm>,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    check_comment_posting_allowed(&state, &user, form.topic).await?;
    let (_, _, markup) = comment_format(&state, user.id).await?;
    let id = insert_comment(&state, user.id, form, &markup).await?;
    Ok(Redirect::to(&comment_link(&state, id).await?))
}

pub async fn add_comment_ajax(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<CommentForm>,
) -> Result<axum::Json<serde_json::Value>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    check_comment_posting_allowed(&state, &user, form.topic).await?;
    let (_, _, markup) = comment_format(&state, user.id).await?;
    let id = insert_comment(&state, user.id, form, &markup).await?;
    let url = comment_link(&state, id).await?;
    Ok(axum::Json(
        serde_json::json!({"id": id, "ok": true, "url": url}),
    ))
}

/// Section.getCommentPostscore: Forum/News are unrestricted by section;
/// Articles/Gallery/Polls/anything else default to a registered-with-score
/// floor. Section ids per db/migrations/0002_seed.sql (1=Новости, 2=Форум,
/// 3=Галерея, 5=Опросы, 6=Статьи).
fn section_comment_postscore(section_id: i32) -> i32 {
    match section_id {
        1 | 2 => -9999,
        _ => 45,
    }
}

const TOPIC_MAX_WARNINGS: i32 = 2;

type TyCommentPostingRow = (
    bool,
    bool,
    bool,
    chrono::DateTime<chrono::Utc>,
    Option<chrono::DateTime<chrono::Utc>>,
    bool,
    i32,
    i32,
    i32,
    i32,
    i32,
);

/// TopicPermissionService.isCommentsAllowedByUser + checkCommentsAllowed:
/// combines topic state (deleted/expired/draft), user state
/// (blocked/frozen), and a postscore computed as the *max* across six
/// independent restriction sources - matches Java's `getPostscore` exactly
/// except `getAllowAnonymousPostscore` (no anonymous-posting model here,
/// so that source is always POSTSCORE_UNRESTRICTED).
async fn check_comment_posting_allowed(
    state: &AppState,
    user: &crate::models::UserSummary,
    topic_id: i32,
) -> Result<()> {
    let row: Option<TyCommentPostingRow> = sqlx::query_as(
        r#"SELECT t.deleted, t.draft,
                  NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire AS expired,
                  t.postdate, t.commitdate, t.sticky, COALESCE(t.postscore, -9999),
                  g.restrict_comments, s.id AS section_id,
                  t.stat1 AS comment_count, t.open_warnings
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((
        deleted,
        draft,
        expired,
        _postdate,
        _commitdate,
        sticky,
        topic_postscore,
        restrict_comments,
        section_id,
        comment_count,
        open_warnings,
    )) = row
    else {
        return Err(AppError::NotFound);
    };
    if deleted {
        return Err(AppError::BadRequest(
            "Нельзя добавлять комментарии к удаленному сообщению".into(),
        ));
    }
    if draft {
        return Err(AppError::BadRequest(
            "Нельзя добавлять комментарии к черновику".into(),
        ));
    }
    if expired {
        return Err(AppError::BadRequest("Сообщение уже устарело".into()));
    }

    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    let is_frozen = frozen_until
        .map(|u| u > chrono::Utc::now())
        .unwrap_or(false);
    if user.blocked.unwrap_or(false) || is_frozen {
        return Err(AppError::Forbidden);
    }

    let comment_count_restriction = if !sticky {
        if comment_count > 3000 {
            200
        } else if comment_count > 2000 {
            100
        } else if comment_count > 1000 {
            50
        } else {
            -9999
        }
    } else {
        -9999
    };
    // DeleteInfoDao.scoreLoss sums `-bonus` because Java stores the penalty
    // as a non-positive score *delta*; this port's `del_info.bonus` is the
    // opposite sign (a positive "points removed" count, see every INSERT
    // INTO del_info in topics.rs/comments.rs/admin.rs), so summing `bonus`
    // directly gives the same positive "loss" total.
    let score_loss: i32 = sqlx::query_scalar(
        r#"SELECT COALESCE((SELECT sum(bonus) FROM del_info JOIN comments ON comments.id=del_info.msgid
             WHERE bonus IS NOT NULL AND bonus<>0 AND comments.userid<>2 AND comments.deleted AND topic=$1), 0)::int"#,
    )
    .bind(topic_id)
    .fetch_one(&state.pool)
    .await?;
    let score_loss_postscore = if !sticky && !expired {
        if score_loss >= 150 {
            100
        } else if score_loss >= 100 {
            50
        } else {
            -9999
        }
    } else {
        -9999
    };
    let open_warnings_postscore = if open_warnings > TOPIC_MAX_WARNINGS {
        100
    } else {
        -9999
    };

    let postscore = [
        topic_postscore,
        restrict_comments,
        section_comment_postscore(section_id),
        comment_count_restriction,
        score_loss_postscore,
        open_warnings_postscore,
    ]
    .into_iter()
    .max()
    .unwrap_or(-9999);

    const POSTSCORE_UNRESTRICTED: i32 = -9999;
    const POSTSCORE_MOD_AUTHOR: i32 = 9999;
    const POSTSCORE_MODERATORS_ONLY: i32 = 10000;
    const POSTSCORE_NO_COMMENTS: i32 = 10001;
    const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
    const POSTSCORE_REGISTERED_ONLY: i32 = -50;

    if postscore == POSTSCORE_NO_COMMENTS || postscore == POSTSCORE_HIDE_COMMENTS {
        return Err(AppError::Forbidden);
    }
    if postscore == POSTSCORE_UNRESTRICTED {
        return Ok(());
    }
    if user.canmod {
        return Ok(());
    }
    if postscore == POSTSCORE_REGISTERED_ONLY {
        return Ok(());
    }
    if postscore == POSTSCORE_MODERATORS_ONLY {
        return Err(AppError::Forbidden);
    }
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM topics WHERE id=$1")
        .bind(topic_id)
        .fetch_one(&state.pool)
        .await?;
    let view_by_author = user.id == author_id;
    if postscore == POSTSCORE_MOD_AUTHOR {
        return if view_by_author {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        };
    }
    if view_by_author || user.score.unwrap_or(0) >= postscore {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

pub async fn comment_message(Query(q): Query<CommentFormQuery>) -> Redirect {
    Redirect::to(&format!("/add_comment.jsp?topic={}", q.topic))
}

pub async fn edit_comment_form() -> Result<&'static str> {
    Ok("Редактирование комментариев реализовано как POST /edit_comment")
}

#[derive(Deserialize)]
pub struct EditCommentForm {
    pub msgid: i32,
    pub msg: String,
    pub title: Option<String>,
}

/// Default upstream config (config.properties.dist): comments are editable
/// only by their author, within 30 minutes of posting, only if they have no
/// replies yet, and only once the author has score >= 45.
/// comment.isModeratorAllowedToEdit defaults to false, so moderators do not
/// get a bypass here in the default configuration.
const COMMENT_EDIT_WINDOW_MINUTES: i64 = 30;
const COMMENT_EDIT_MIN_SCORE: i32 = 45;

/// TopicDao's `expired` column: `!sticky && COALESCE(commitdate,postdate) <
/// now()-sections.expire`. Shared by comment edit/delete/undelete, which
/// all gate on the topic's own expiry, not just the comment's age.
pub(crate) async fn is_topic_expired(state: &AppState, topic_id: i32) -> Result<bool> {
    Ok(sqlx::query_scalar(
        r#"SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_one(&state.pool)
    .await?)
}

pub async fn edit_comment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<EditCommentForm>,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let row: (i32, i32, bool, chrono::DateTime<chrono::Utc>, bool) = sqlx::query_as(
        r#"SELECT c.topic, c.userid, c.deleted, c.postdate,
                  EXISTS(SELECT 1 FROM comments r WHERE r.replyto=c.id AND NOT r.deleted) AS has_replies
           FROM comments c WHERE c.id=$1"#,
    )
    .bind(form.msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (topic_id, author_id, deleted, postdate, has_replies) = row;
    let topic_deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1")
        .bind(topic_id)
        .fetch_one(&state.pool)
        .await?;

    if deleted || topic_deleted {
        return Err(AppError::BadRequest("тема или комментарий удалены".into()));
    }
    // TopicPermissionService.checkCommentsAllowed, also enforced for edits
    // via isCommentEditableNow: an expired topic can't be commented on OR
    // have its comments edited, by anyone (moderators included).
    if is_topic_expired(&state, topic_id).await? {
        return Err(AppError::BadRequest("сообщение уже устарело".into()));
    }
    if user.id != author_id {
        return Err(AppError::Forbidden);
    }
    if has_replies {
        return Err(AppError::BadRequest(
            "редактирование комментариев с ответами запрещено".into(),
        ));
    }
    if user.score.unwrap_or(0) < COMMENT_EDIT_MIN_SCORE {
        return Err(AppError::Forbidden);
    }
    if chrono::Utc::now() > postdate + chrono::Duration::minutes(COMMENT_EDIT_WINDOW_MINUTES) {
        return Err(AppError::BadRequest("истек срок редактирования".into()));
    }

    sqlx::query("UPDATE msgbase SET message=$2 WHERE id=$1")
        .bind(form.msgid)
        .bind(&form.msg)
        .execute(&state.pool)
        .await?;
    if let Some(title) = form.title {
        sqlx::query("UPDATE comments SET title=$2 WHERE id=$1")
            .bind(form.msgid)
            .bind(title)
            .execute(&state.pool)
            .await?;
    }
    crate::search_index::index_comment(&state, form.msgid).await;
    Ok(Redirect::to(&comment_link(&state, form.msgid).await?))
}

pub async fn delete_comment_form(
    State(state): State<AppState>,
    Query(q): Query<JumpQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let row: (i32, String, String) = sqlx::query_as(
        "SELECT c.topic, c.title, u.nick FROM comments c JOIN users u ON u.id=c.userid WHERE c.id=$1",
    )
    .bind(q.msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    // DeleteCommentController.deleteComments: only a moderator may set
    // `bonus`/`delete_replys` - a plain author sees just the reason field.
    let mod_fields = if user.canmod {
        r#"<label>Штраф (0-20) <input type="number" name="bonus" min="0" max="20" value="0"></label>
  <label><input type="checkbox" name="delete_replys" value="true"> Удалить с ответами</label>"#
    } else {
        ""
    };
    Ok(Html(format!(
        r#"
<h1>Удалить комментарий #{}</h1>
<p>Тема #{} · {} · автор {}</p>
<form method="post" action="/delete_comment.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <label>Причина <input name="reason"></label>
  {mod_fields}
  <button type="submit">Удалить</button>
</form>
"#,
        q.msgid,
        row.0,
        html_escape::encode_text(&row.1),
        html_escape::encode_text(&row.2),
        q.msgid
    )))
}

pub async fn undelete_comment_form(
    Query(q): Query<JumpQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    Ok(Html(format!(
        r#"
<h1>Восстановить комментарий #{}</h1>
<form method="post" action="/undelete_comment">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Восстановить</button>
</form>
"#,
        q.msgid, q.msgid
    )))
}

#[derive(Deserialize)]
pub struct CommentAction {
    pub msgid: i32,
    pub reason: Option<String>,
    pub bonus: Option<i32>,
    pub delete_replys: Option<String>,
}

/// DeleteReasons.replyBonusAndReason: when the root comment's penalty was
/// more than a token amount (>2 points), decay the same penalty down the
/// reply tree - direct children lose 2, grandchildren 1, anything deeper 0.
/// Returned as a positive "points removed" count, matching this port's
/// `del_info.bonus` sign convention (Java stores the negative score delta
/// instead - see the `score_loss` query above for the same flip).
fn reply_bonus_and_reason(drop_score: bool, depth: i32) -> (i32, &'static str) {
    if !drop_score {
        return (0, "7.1 Ответ на некорректное сообщение (авто)");
    }
    match depth {
        0 => (2, "7.1 Ответ на некорректное сообщение (авто, уровень 0)"),
        1 => (1, "7.1 Ответ на некорректное сообщение (авто, уровень 1)"),
        _ => (0, "7.1 Ответ на некорректное сообщение (авто, уровень >1)"),
    }
}

async fn effective_delete_bonus(
    state: &AppState,
    author_id: i32,
    requested_bonus: i32,
) -> Result<i32> {
    if requested_bonus == 0 || author_id == ANONYMOUS_USER_ID {
        return Ok(requested_bonus);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(author_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    Ok(
        if frozen_until
            .map(|u| u > chrono::Utc::now())
            .unwrap_or(false)
        {
            0
        } else {
            requested_bonus
        },
    )
}

/// Matches TopicPermissionService.DeletePeriod: authors may delete their own
/// comment for 3 hours after posting (and only if nobody has replied yet).
/// Moderators bypass this window entirely.
const COMMENT_DELETE_WINDOW_HOURS: i64 = 3;

pub async fn delete_comment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<CommentAction>,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let row: (i32, i32, bool, chrono::DateTime<chrono::Utc>, bool) = sqlx::query_as(
        r#"SELECT c.topic, c.userid, c.deleted, c.postdate,
                  EXISTS(SELECT 1 FROM comments r WHERE r.replyto=c.id AND NOT r.deleted) AS has_replies
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
    let topic_deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1")
        .bind(topic_id)
        .fetch_one(&state.pool)
        .await?;

    // isCommentDeletableNow: moderators bypass the expired check entirely;
    // an author may only delete their own comment while the topic is
    // still "live".
    let deletable = user.canmod || {
        let within_window =
            chrono::Utc::now() <= postdate + chrono::Duration::hours(COMMENT_DELETE_WINDOW_HOURS);
        let topic_expired = is_topic_expired(&state, topic_id).await?;
        user.id == author_id && !has_replies && !topic_deleted && !topic_expired && within_window
    };
    if !deletable {
        return Err(AppError::Forbidden);
    }

    let requested_bonus = if user.canmod && user.id != author_id {
        form.bonus.unwrap_or(0).clamp(0, 20)
    } else {
        0
    };
    let bonus = effective_delete_bonus(&state, author_id, requested_bonus).await?;
    let reason = form.reason.clone().unwrap_or_default();

    // DeleteService.deleteCommentWithReplys: moderator-only cascade that
    // walks the still-live reply subtree, decaying the same penalty by
    // depth (see reply_bonus_and_reason), and skips reply notifications
    // when the topic has expired (matching notifyReplys = !topic.expired).
    let mut deleted_count = 1;
    if user.canmod && form.delete_replys.is_some() {
        let drop_score = bonus > 2;
        let topic_expired = is_topic_expired(&state, topic_id).await?;
        let replies: Vec<(i32, i32, i32)> = sqlx::query_as(
            r#"WITH RECURSIVE subtree AS (
                 SELECT id, userid, 0 AS depth FROM comments WHERE replyto=$1 AND NOT deleted
                 UNION ALL
                 SELECT c.id, c.userid, s.depth+1 FROM comments c JOIN subtree s ON c.replyto=s.id WHERE NOT c.deleted
               )
               SELECT id, userid, depth FROM subtree"#,
        )
        .bind(form.msgid)
        .fetch_all(&state.pool)
        .await?;
        for (reply_id, reply_author, depth) in &replies {
            let (reply_bonus, reply_reason) = reply_bonus_and_reason(drop_score, *depth);
            let reply_bonus = effective_delete_bonus(&state, *reply_author, reply_bonus).await?;
            sqlx::query("UPDATE comments SET deleted=true WHERE id=$1")
                .bind(reply_id)
                .execute(&state.pool)
                .await?;
            sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES($1,$2,$3,now(),$4) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now(), bonus=EXCLUDED.bonus")
                .bind(reply_id).bind(user.id).bind(reply_reason).bind(reply_bonus).execute(&state.pool).await?;
            if reply_bonus != 0 {
                sqlx::query("UPDATE users SET score=GREATEST(score-$2,0) WHERE id=$1")
                    .bind(reply_author)
                    .bind(reply_bonus)
                    .execute(&state.pool)
                    .await?;
            }
            if !topic_expired {
                notify_deleted(
                    &state,
                    *reply_author,
                    user.id,
                    Some(topic_id),
                    Some(*reply_id),
                    reply_reason,
                )
                .await?;
            }
            crate::search_index::index_comment(&state, *reply_id).await;
        }
        deleted_count += replies.len() as i32;
    }

    sqlx::query("UPDATE comments SET deleted=true WHERE id=$1")
        .bind(form.msgid)
        .execute(&state.pool)
        .await?;
    sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES($1,$2,$3,now(),$4) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now(), bonus=EXCLUDED.bonus")
        .bind(form.msgid).bind(user.id).bind(&reason).bind(bonus).execute(&state.pool).await?;
    if bonus != 0 {
        sqlx::query("UPDATE users SET score=GREATEST(score-$2,0) WHERE id=$1")
            .bind(author_id)
            .bind(bonus)
            .execute(&state.pool)
            .await?;
    }
    // CommentDao.deleteComment: unlike an insert, deletion has no DB
    // trigger - Java decrements topics.stat1 in app code and clamps stat3
    // so it never exceeds the (now smaller) live comment count.
    sqlx::query("UPDATE topics SET stat1=stat1-$2, lastmod=now() WHERE id=$1")
        .bind(topic_id)
        .bind(deleted_count)
        .execute(&state.pool)
        .await?;
    sqlx::query("UPDATE topics SET stat3=stat1 WHERE id=$1 AND stat3>stat1")
        .bind(topic_id)
        .execute(&state.pool)
        .await?;
    notify_deleted(
        &state,
        author_id,
        user.id,
        Some(topic_id),
        Some(form.msgid),
        &reason,
    )
    .await?;
    crate::search_index::index_comment(&state, form.msgid).await;
    Ok(Redirect::to(&comment_link(&state, form.msgid).await?))
}

/// UserEventService.insertTopicDeleteNotification/insertCommentDeleteNotification:
/// privately tell the author their content was deleted (with the reason),
/// unless they deleted it themselves, are the anonymous user (id=2), or are
/// currently frozen.
pub(crate) const ANONYMOUS_USER_ID: i32 = 2;

pub(crate) async fn notify_deleted(
    state: &AppState,
    author_id: i32,
    deleted_by: i32,
    topic_id: Option<i32>,
    comment_id: Option<i32>,
    reason: &str,
) -> Result<()> {
    if author_id == deleted_by || author_id == ANONYMOUS_USER_ID {
        return Ok(());
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(author_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    if frozen_until
        .map(|u| u > chrono::Utc::now())
        .unwrap_or(false)
    {
        return Ok(());
    }
    sqlx::query("INSERT INTO user_events(userid,type,private,message_id,comment_id,message) VALUES($1,'DEL',true,$2,$3,$4)")
        .bind(author_id)
        .bind(topic_id)
        .bind(comment_id)
        .bind(reason)
        .execute(&state.pool)
        .await?;
    sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=$1")
        .bind(author_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

pub async fn undelete_comment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<CommentAction>,
) -> Result<Redirect> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
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
    let topic_deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1")
        .bind(topic_id)
        .fetch_one(&state.pool)
        .await?;
    if topic_deleted {
        return Err(AppError::Forbidden);
    }
    // isUndeletable: unlike delete, the expired check here applies even to
    // moderators - once a topic has expired, its comments are frozen.
    if is_topic_expired(&state, topic_id).await? {
        return Err(AppError::Forbidden);
    }
    // Mirrors TopicPermissionService.isUndeletable: a comment cannot be
    // undeleted if its own author is the one who deleted it (self-moderation
    // is respected, only another moderator's deletion can be reversed).
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM comments WHERE id=$1")
        .bind(form.msgid)
        .fetch_one(&state.pool)
        .await?;
    let delby: Option<i32> = sqlx::query_scalar("SELECT delby FROM del_info WHERE msgid=$1")
        .bind(form.msgid)
        .fetch_optional(&state.pool)
        .await?;
    if delby == Some(author_id) {
        return Err(AppError::Forbidden);
    }

    sqlx::query("UPDATE comments SET deleted=false WHERE id=$1")
        .bind(form.msgid)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM del_info WHERE msgid=$1")
        .bind(form.msgid)
        .execute(&state.pool)
        .await?;
    crate::search_index::index_comment(&state, form.msgid).await;
    Ok(Redirect::to(&comment_link(&state, form.msgid).await?))
}

/// CommentCreateService.getCommentBody: 4096 chars for anonymous, 8192 for
/// registered users - this port has no anonymous-posting model, so only
/// the higher registered-user cap applies.
const COMMENT_MAX_LENGTH: usize = 8192;

async fn insert_comment(
    state: &AppState,
    user_id: i32,
    form: CommentForm,
    markup: &str,
) -> Result<i32> {
    if form.msg.trim().is_empty() {
        return Err(AppError::BadRequest(
            "комментарий не может быть пустым".into(),
        ));
    }
    if form.msg.chars().count() > COMMENT_MAX_LENGTH {
        return Err(AppError::BadRequest("Слишком большое сообщение".into()));
    }
    // FloodProtector.AddComment: minimum interval since the user's last
    // comment, not a count-per-window - Java keys this by IP with a
    // slow-mode-restricted tier this port doesn't model (no
    // SlowModeChecker), so only the score>=100 "trusted" vs. default
    // tiers apply, keyed by user instead of IP (no anonymous posting here
    // to guard against).
    let score: i32 = sqlx::query_scalar("SELECT COALESCE(score,0) FROM users WHERE id=$1")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await?;
    let threshold = if score >= 100 {
        chrono::Duration::seconds(3)
    } else {
        chrono::Duration::seconds(30)
    };
    let last_comment: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT max(postdate) FROM comments WHERE userid=$1")
            .bind(user_id)
            .fetch_one(&state.pool)
            .await?;
    if let Some(last) = last_comment
        && chrono::Utc::now() < last + threshold
    {
        return Err(AppError::BadRequest(format!(
            "Следующее сообщение может быть записано не менее чем через {} секунд после предыдущего",
            threshold.num_seconds()
        )));
    }

    // The original inline form uses replyto=0 for a top-level comment;
    // PostgreSQL expects NULL because comments.replyto is a foreign key.
    let replyto = form.replyto.filter(|id| *id > 0);
    let mut tx = state.pool.begin().await?;
    let id: i32 = sqlx::query_scalar("SELECT nextval('s_msgid')::int")
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO msgbase(id, message, markup) VALUES($1,$2,$3::markup_type)")
        .bind(id)
        .bind(&form.msg)
        .bind(markup)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO comments(id, topic, userid, title, postdate, replyto) VALUES($1,$2,$3,$4,now(),$5)",
    )
    .bind(id)
    .bind(form.topic)
    .bind(user_id)
    .bind(form.title.unwrap_or_else(|| "Комментарий".into()))
    .bind(replyto)
    .execute(&mut *tx)
    .await?;
    // topics.stat1/stat3 and groups.stat3 are now kept in sync by the
    // comins() trigger (see db/migrations/0013) - matches Java's DB-side
    // bookkeeping exactly, instead of a partial manual update here that
    // would double-count once the trigger exists.

    // Matches CommentCreateService.notifyReply / UserEventDao.insertCommentWatchNotification:
    // notify the parent comment's author (REPLY) and topic watchers (WATCH),
    // skipping the commenter themselves and anyone who has the commenter ignored.
    let mut notified: Vec<i32> = Vec::new();

    let mut parent_author: Option<i32> = None;
    if let Some(replyto) = replyto
        && let Some(parent_userid) =
            sqlx::query_scalar::<_, i32>("SELECT userid FROM comments WHERE id=$1")
                .bind(replyto)
                .fetch_optional(&mut *tx)
                .await?
    {
        parent_author = Some(parent_userid);
        if parent_userid != user_id {
            let ignored: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM ignore_list WHERE userid=$1 AND ignored=$2)",
            )
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

    // CommentCreateService.notifyMentions: notify each @nick referenced in
    // the raw comment text, skipping the commenter and anyone mentioned who
    // has the commenter on their ignore list.
    let mentioned_nicks = markup::extract_mentions(&form.msg);
    if !mentioned_nicks.is_empty() {
        let mentioned_ids: Vec<i32> = sqlx::query_scalar(
            r#"SELECT u.id FROM users u
               WHERE lower(u.nick) = ANY($1) AND u.id <> $2
                 AND NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=u.id AND il.ignored=$2)"#,
        )
        .bind(mentioned_nicks.iter().map(|n| n.to_lowercase()).collect::<Vec<_>>())
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        for mentioned_id in &mentioned_ids {
            sqlx::query("INSERT INTO user_events(userid,type,private,message_id,comment_id) VALUES($1,'REF',false,$2,$3)")
                .bind(mentioned_id)
                .bind(form.topic)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            notified.push(*mentioned_id);
        }
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
    // AddCommentController publishes only after the transaction succeeds and
    // preserves this order: topic subscribers first, notification owners
    // second.
    state.realtime.vNotifyNewComment(form.topic, id);
    state.realtime.vNotifyEvents(notified.iter().copied());
    crate::search_index::index_comment(state, id).await;
    Ok(id)
}

async fn locate_topic_or_comment(
    state: &AppState,
    msgid: i32,
) -> Result<Option<(String, String, i32, Option<i32>)>> {
    let row = sqlx::query_as::<_, (String, String, i32, Option<i32>)>(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section,
                  g.urlname, t.id, NULL::integer AS comment_id
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1
           UNION ALL
           SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section,
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

pub async fn deleted_comments_by_user(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    let comments = sqlx::query_as::<_, crate::models::CommentItem>(
        r#"SELECT c.id, c.topic, c.replyto, c.title, m.message, m.markup::text AS markup, c.postdate, u.id AS author_id, u.nick AS author, c.deleted
           FROM comments c
           JOIN msgbase m ON m.id=c.id
           JOIN users u ON u.id=c.userid
           WHERE lower(u.nick)=lower($1) AND c.deleted
           ORDER BY c.postdate DESC LIMIT 100"#,
    )
    .bind(&nick)
    .fetch_all(&state.pool)
    .await?;
    let mut html = format!(
        "<h1>Удалённые комментарии {}</h1>",
        html_escape::encode_text(&nick)
    );
    for c in comments {
        html.push_str(&format!(
            "<article id=\"comment-{}\"><h3>{}</h3><p>topic #{} · {}</p><div>{}</div></article>",
            c.id,
            html_escape::encode_text(&c.title),
            c.topic,
            c.postdate,
            markup::render_message_with_markup(&c.message, Some(&c.markup), None)
        ));
    }
    Ok(Html(html))
}
