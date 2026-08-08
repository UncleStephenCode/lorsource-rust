use async_trait::async_trait;

use crate::domain::forum::model::StGroup;
use crate::error::Result;

#[async_trait]
pub trait TrForumRepository: Send + Sync {
    async fn vecListGroups(&self) -> Result<Vec<StGroup>>;
    async fn vecListGroupsBySection(&self, optSectionPrefix: Option<&str>) -> Result<Vec<StGroup>>;
    async fn stFindGroupByUrlName(&self, sUrlName: &str) -> Result<StGroup>;
}
