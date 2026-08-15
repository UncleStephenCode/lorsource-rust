use async_trait::async_trait;

use crate::domain::forum::model::StGroup;
use crate::error::Result;

#[async_trait]
pub trait TrForumRepository: Send + Sync {
    async fn vecListGroupsBySection(&self, optSectionPrefix: Option<&str>) -> Result<Vec<StGroup>>;
    async fn stFindGroupBySectionAndUrlName(
        &self,
        sSectionPrefix: &str,
        sUrlName: &str,
    ) -> Result<StGroup>;
}
