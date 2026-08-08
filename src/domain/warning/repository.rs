use async_trait::async_trait;

use crate::{
    domain::warning::model::{
        StClearWarningMutation, StCreateWarningMutation, StWarningRecord, StWarningTopic,
    },
    error::Result,
};

#[async_trait]
pub trait TrWarningRepository: Send + Sync {
    async fn optTopic(&self, iTopicId: i32) -> Result<Option<StWarningTopic>>;
    async fn optCommentDeleted(&self, iTopicId: i32, iCommentId: i32) -> Result<Option<bool>>;
    async fn bUserFrozen(&self, iUserId: i32) -> Result<bool>;
    async fn iRecentWarnings(&self, iUserId: i32) -> Result<i64>;
    async fn iCreate(&self, stMutation: StCreateWarningMutation) -> Result<i32>;
    async fn optWarning(&self, iWarningId: i32) -> Result<Option<StWarningRecord>>;
    async fn vClear(&self, stMutation: StClearWarningMutation) -> Result<()>;
}
