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
    /// Java's "administrator" tier (a strict superset of moderator
    /// privileges - e.g. only administrators may rename a group or change
    /// its urlName, see GroupModificationController).
    pub candel: bool,
    /// Java's `corrector` role: may commit/uncommit news topics (except
    /// their own) alongside moderators, see EditTopicChecker.checkCommit.
    pub corrector: bool,
    pub blocked: Option<bool>,
    pub userinfo: Option<String>,
}
