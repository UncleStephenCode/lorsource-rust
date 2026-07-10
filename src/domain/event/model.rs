use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StEventItem {
    pub id: i32,
    pub event_date: DateTime<Utc>,
    pub event_type: String,
    pub message: String,
    pub topic_id: Option<i32>,
}
