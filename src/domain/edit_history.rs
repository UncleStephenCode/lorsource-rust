use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct StEditHistoryRow {
    pub iId: i32,
    pub sEditor: String,
    pub dtEdit: DateTime<Utc>,
    pub optOldMessage: Option<String>,
    pub optOldTitle: Option<String>,
    pub optOldTags: Option<String>,
    pub optOldLinkText: Option<String>,
    pub optOldUrl: Option<String>,
    pub optOldMinor: Option<bool>,
    pub optOldPoll: Option<serde_json::Value>,
    pub optOldAdditionalImages: Option<Vec<i32>>,
    pub optLegacyMainImage: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct StHistoryPoll {
    pub bMultiSelect: bool,
    pub vecVariants: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StTopicHistorySource {
    pub sAuthor: String,
    pub dtPost: DateTime<Utc>,
    pub sMessage: String,
    pub sMarkup: String,
    pub sTitle: String,
    pub optUrl: Option<String>,
    pub optLinkText: Option<String>,
    pub bMinor: bool,
    pub vecTags: Vec<String>,
    pub vecImageIds: Vec<i32>,
    pub optPoll: Option<StHistoryPoll>,
}

#[derive(Debug, Clone)]
pub struct StCommentHistorySource {
    pub iTopicId: i32,
    pub sAuthor: String,
    pub dtPost: DateTime<Utc>,
    pub sMessage: String,
    pub sMarkup: String,
    pub sTitle: String,
}

#[async_trait]
pub trait TrEditHistoryRepository: Send + Sync {
    async fn stTopicSource(&self, iTopicId: i32) -> Result<StTopicHistorySource>;
    async fn stCommentSource(&self, iCommentId: i32) -> Result<StCommentHistorySource>;
    async fn vecRows(&self, iMessageId: i32, sObjectType: &str) -> Result<Vec<StEditHistoryRow>>;
    async fn sRestorableTopicMessage(&self, iTopicId: i32, iRecordId: i32) -> Result<String>;
}
