use async_trait::async_trait;

use crate::domain::user::model::StUserSummary;
use crate::error::Result;

#[async_trait]
pub trait TrUserRepository: Send + Sync {
    async fn optFindSummaryById(&self, iUserId: i32) -> Result<Option<StUserSummary>>;
    async fn optFindSummaryByNick(&self, sNick: &str) -> Result<Option<StUserSummary>>;
}
