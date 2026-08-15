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
    #[serde(skip)]
    pub author_score: i32,
    #[serde(skip)]
    pub author_blocked: bool,
    #[serde(skip)]
    pub author_anonymous: bool,
    #[serde(skip)]
    pub author_frozen: bool,
    pub deleted: bool,
}

impl StCommentItem {
    pub fn bNofollowAuthorLinks(&self) -> bool {
        !crate::domain::topic::link_policy::StAuthorLinkState {
            iScore: self.author_score,
            bBlocked: self.author_blocked,
            bAnonymous: self.author_anonymous,
            bFrozen: self.author_frozen,
        }
        .bFollowAuthorLinks()
    }

    /// Java `PreparedComment.title` plus `TitleTag`, represented as plain DOM
    /// text for Askama's normal contextual escaping. The raw field remains
    /// available to persistence and edit-history code.
    pub fn optTitlePlain(&self) -> Option<String> {
        crate::domain::title::optCommentTitlePlainForDisplay(&self.title)
    }
}
