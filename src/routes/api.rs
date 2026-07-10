use crate::{auth::CurrentUser, error::Result, state::AppState};
use axum::{extract::State, response::Html, Json};
use serde_json::json;

pub async fn notifications(CurrentUser(user): CurrentUser) -> Json<serde_json::Value> {
    Json(json!({"user": user.map(|u| u.nick), "events": []}))
}

pub async fn notifications_count(CurrentUser(user): CurrentUser) -> Json<serde_json::Value> {
    Json(json!({"user": user.map(|u| u.nick), "count": 0}))
}

pub async fn notifications_reset() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

pub async fn tracker(State(state): State<AppState>) -> Result<Json<serde_json::Value>> {
    let topics = crate::routes::topics::list_topics(&state, None, None, 0, 20).await?;
    Ok(Json(json!({"items": topics})))
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

pub async fn reactions_get(State(state): State<AppState>, axum::extract::Query(q): axum::extract::Query<ReactionQuery>) -> Result<Json<serde_json::Value>> {
    let (topic_id, comment_id) = resolve_reaction_target(&state.pool, q.topic, q.comment, q.msgid).await?;
    let rows = sqlx::query_as::<_, (String, i64)>(
        r#"SELECT reaction, count(*)
           FROM reactions_log
           WHERE topic_id=$1 AND (($2::int IS NULL AND comment_id IS NULL) OR comment_id=$2)
           GROUP BY reaction ORDER BY reaction"#,
    )
    .bind(topic_id)
    .bind(comment_id)
    .fetch_all(&state.pool)
    .await?;
    let counts: serde_json::Map<String, serde_json::Value> = rows.into_iter().map(|(k, v)| (k, json!(v))).collect();
    Ok(Json(json!({"topic": topic_id, "comment": comment_id, "reactions": counts})))
}

pub async fn reactions_post(State(state): State<AppState>, CurrentUser(user): CurrentUser, axum::Form(form): axum::Form<ReactionForm>) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(crate::error::AppError::Forbidden); };
    let (topic_id, comment_id) = resolve_reaction_target(&state.pool, form.topic, form.comment, form.msgid).await?;
    let (reaction, set) = parse_reaction_action(form.reaction, form.value);

    if set {
        if let Some(comment_id) = comment_id {
            sqlx::query("UPDATE comments SET reactions = reactions || jsonb_build_object($2::text, $3::text) WHERE id=$1")
                .bind(comment_id).bind(user.id).bind(&reaction).execute(&state.pool).await?;
        } else {
            sqlx::query("UPDATE topics SET reactions = reactions || jsonb_build_object($2::text, $3::text) WHERE id=$1")
                .bind(topic_id).bind(user.id).bind(&reaction).execute(&state.pool).await?;
        }
        sqlx::query(
            r#"INSERT INTO reactions_log(origin_user,topic_id,comment_id,reaction,set_date)
               VALUES($1,$2,$3,$4,now())
               ON CONFLICT (origin_user, topic_id, comment_id)
               DO UPDATE SET set_date=now(), reaction=EXCLUDED.reaction"#,
        )
        .bind(user.id).bind(topic_id).bind(comment_id).bind(&reaction).execute(&state.pool).await?;
    } else {
        if let Some(comment_id) = comment_id {
            sqlx::query("UPDATE comments SET reactions = reactions - $2::text WHERE id=$1")
                .bind(comment_id).bind(user.id.to_string()).execute(&state.pool).await?;
        } else {
            sqlx::query("UPDATE topics SET reactions = reactions - $2::text WHERE id=$1")
                .bind(topic_id).bind(user.id.to_string()).execute(&state.pool).await?;
        }
        sqlx::query(
            r#"DELETE FROM reactions_log
               WHERE origin_user=$1 AND topic_id=$2 AND (($3::int IS NULL AND comment_id IS NULL) OR comment_id=$3)"#,
        )
        .bind(user.id).bind(topic_id).bind(comment_id).execute(&state.pool).await?;
    }

    let count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM reactions_log
           WHERE topic_id=$1 AND (($2::int IS NULL AND comment_id IS NULL) OR comment_id=$2) AND reaction=$3"#,
    )
    .bind(topic_id).bind(comment_id).bind(&reaction).fetch_one(&state.pool).await?;

    Ok(Json(json!({"count": count, "topic": topic_id, "comment": comment_id, "reaction": reaction, "set": set})))
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

    let Some((topic_id, multiselect, section_prefix, group_urlname)) = sqlx::query_as::<_, (i32, bool, String, String)>(
        r#"SELECT p.topic, p.multiselect,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                  g.urlname
           FROM polls p
           JOIN topics t ON t.id=p.topic
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE p.id=$1 AND t.moderate AND NOT t.deleted"#,
    )
    .bind(form.voteid)
    .fetch_optional(&state.pool)
    .await? else {
        return Err(crate::error::AppError::BadRequest("опрос не найден или ещё не подтверждён".into()));
    };

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
