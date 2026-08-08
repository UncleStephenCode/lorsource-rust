use async_trait::async_trait;

use crate::error::Result;

#[async_trait]
pub trait TrRealtimeRepository: Send + Sync {
    /// Returns `None` when the topic does not exist. A topic whose comments
    /// are hidden exists, but deliberately returns an empty list.
    async fn optMissedCommentIds(
        &self,
        iTopicId: i32,
        iLastSeenCommentId: i32,
    ) -> Result<Option<Vec<i32>>>;

    /// Java's `IgnoreListDao.isIgnored` hides a notification when any author
    /// in the comment's parent chain is ignored by the connected user.
    async fn bIsCommentBranchIgnored(&self, iUserId: i32, iCommentId: i32) -> Result<bool>;
}
