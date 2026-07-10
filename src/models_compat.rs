//! Database model inventory for the original LOR schema.
//!
//! These structs are intentionally close to the table names/columns used by the
//! Scala code and demo dump. They are not all wired into handlers yet; the goal
//! is to keep the Rust port honest while services are moved one by one.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OriginalUser {
    pub id: i32,
    pub name: Option<String>,
    pub nick: String,
    pub passwd: Option<String>,
    pub url: Option<String>,
    pub email: Option<String>,
    pub canmod: bool,
    pub photo: Option<String>,
    pub town: Option<String>,
    pub candel: bool,
    pub blocked: Option<bool>,
    pub score: Option<i32>,
    pub max_score: Option<i32>,
    pub lastlogin: Option<NaiveDateTime>,
    pub regdate: Option<NaiveDateTime>,
    pub activated: bool,
    pub corrector: bool,
    pub userinfo: Option<String>,
    pub unread_events: i32,
    pub new_email: Option<String>,
    pub style: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OriginalSection {
    pub id: i32,
    pub name: String,
    pub moderate: bool,
    pub imagepost: bool,
    pub preformat: bool,
    pub linktext: Option<String>,
    pub havelink: bool,
    pub vote: Option<bool>,
    pub add_info: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OriginalGroup {
    pub id: i32,
    pub title: String,
    pub image: Option<String>,
    pub section: i32,
    pub stat1: i32,
    pub stat2: i32,
    pub stat3: i32,
    pub stat4: i32,
    pub restrict_topics: Option<i32>,
    pub info: Option<String>,
    pub restrict_comments: i32,
    pub longinfo: Option<String>,
    pub resolvable: bool,
    pub urlname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OriginalTopic {
    pub id: i32,
    pub groupid: i32,
    pub userid: i32,
    pub title: String,
    pub url: Option<String>,
    pub moderate: bool,
    pub postdate: DateTime<Utc>,
    pub linktext: Option<String>,
    pub deleted: bool,
    pub stat1: i32,
    pub stat2: i32,
    pub stat3: i32,
    pub stat4: i32,
    pub lastmod: Option<DateTime<Utc>>,
    pub commitby: Option<i32>,
    pub notop: Option<bool>,
    pub commitdate: Option<NaiveDateTime>,
    pub postscore: Option<i32>,
    pub sticky: bool,
    pub ua_id: Option<i32>,
    pub resolved: Option<bool>,
    pub minor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OriginalComment {
    pub id: i32,
    pub topic: i32,
    pub userid: i32,
    pub title: String,
    pub postdate: DateTime<Utc>,
    pub replyto: Option<i32>,
    pub deleted: bool,
    pub ua_id: Option<i32>,
    pub topic_deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MessageTextRow {
    pub id: i64,
    pub message: String,
    pub bbcode: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TagValueRow {
    pub id: i32,
    pub counter: Option<i32>,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TopicTagRow {
    pub msgid: Option<i32>,
    pub tagid: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PollRow {
    pub id: i32,
    pub topic: i32,
    pub multiselect: bool,
}

/// Pre-2011 demo-dump alias for `PollRow`; current Java code uses `polls`.
pub type VoteNameRow = PollRow;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PollVariantRow {
    pub id: i32,
    pub vote: i32,
    pub label: String,
    pub votes: i32,
}

/// Pre-2011 demo-dump alias for `PollVariantRow`; current Java code uses `polls_variants`.
pub type VoteVariantRow = PollVariantRow;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct VoteUserRow {
    /// Current Java semantics: poll id.
    pub vote: i32,
    pub userid: i32,
    /// Selected poll variant id, introduced by the 2011 poll Liquibase update.
    pub variant_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EditInfoRow {
    pub id: i32,
    pub msgid: i32,
    pub editor: i32,
    pub oldmessage: Option<String>,
    pub editdate: NaiveDateTime,
    pub oldtitle: Option<String>,
    pub oldtags: Option<String>,
    pub oldlinktext: Option<String>,
    pub oldurl: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DeleteInfoRow {
    pub msgid: i32,
    pub delby: i32,
    pub reason: Option<String>,
    pub deldate: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MemoryRow {
    pub id: i32,
    pub userid: i32,
    pub topic: i32,
    pub add_date: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IgnoreListRow {
    pub userid: i32,
    pub ignored: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserAgentRow {
    pub id: i32,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BanInfoRow {
    pub userid: i32,
    pub bandate: NaiveDateTime,
    pub reason: String,
    pub ban_by: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserEventRow {
    pub id: i32,
    pub userid: i32,
    pub event_date: NaiveDateTime,
    pub message_id: Option<i32>,
    pub comment_id: Option<i32>,
    pub message: Option<String>,
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ImageRow {
    pub id: i32,
    pub userid: i32,
    pub topic: Option<i32>,
    pub original: Option<String>,
    pub medium: Option<String>,
    pub thumbnail: Option<String>,
    pub deleted: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ReactionLogRow {
    pub id: i32,
    pub userid: i32,
    pub msgid: i32,
    pub reaction: String,
    pub action_date: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WarningRow {
    pub id: i32,
    pub userid: i32,
    pub topic_id: Option<i32>,
    pub comment_id: Option<i32>,
    pub reason: String,
    pub postdate: DateTime<Utc>,
}


#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserSettingsRow {
    pub id: i32,
    /// Stored as PostgreSQL hstore in the Java schema; represented as text when
    /// queried through generic compatibility code.
    pub settings: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserLogRow {
    pub id: i32,
    pub userid: i32,
    pub action_userid: i32,
    pub action_date: DateTime<Utc>,
    pub action: String,
    /// Stored as PostgreSQL hstore in the Java schema; represented as text when
    /// queried through generic compatibility code.
    pub info: String,
}
