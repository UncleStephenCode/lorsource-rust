use async_trait::async_trait;

use crate::domain::comment::model::StCommentItem;
use crate::error::Result;

#[async_trait]
pub trait TrCommentRepository: Send + Sync {
    async fn vecListByTopic(&self, iTopicId: i32) -> Result<Vec<StCommentItem>>;
}
