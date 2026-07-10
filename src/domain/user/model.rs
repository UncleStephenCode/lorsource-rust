use chrono::NaiveDateTime;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StUserSummary {
    pub id: i32,
    pub nick: String,
    pub name: Option<String>,
    pub score: Option<i32>,
    pub max_score: Option<i32>,
    pub photo: Option<String>,
    pub town: Option<String>,
    pub regdate: Option<NaiveDateTime>,
    pub canmod: bool,
    pub blocked: Option<bool>,
    pub userinfo: Option<String>,
}
