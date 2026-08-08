use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StCommentItem {
    pub id: i32,
    pub topic: i32,
    pub replyto: Option<i32>,
    pub title: String,
    pub message: String,
    pub markup: String,
    pub postdate: DateTime<Utc>,
    pub author_id: i32,
    pub author: String,
    pub deleted: bool,
}
