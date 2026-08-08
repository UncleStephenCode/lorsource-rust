use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{domain::email_domain_block::model::StEmailDomainBlock, error::Result};

#[async_trait]
pub trait TrEmailDomainBlockRepository: Send + Sync {
    async fn optProfileSettings(&self, iUserId: i32) -> Result<Option<String>>;
    async fn iManualCount(&self) -> Result<i64>;
    async fn vecManualBlocks(&self, iOffset: i32, iLimit: i32) -> Result<Vec<StEmailDomainBlock>>;
    async fn vBlockManual(
        &self,
        sDomain: &str,
        dtBlockUntil: DateTime<Utc>,
        iModeratorId: i32,
    ) -> Result<()>;
    async fn vUnblock(&self, sDomain: &str) -> Result<()>;
    async fn bIsBlocked(&self, sDomain: &str) -> Result<bool>;
}
