use crate::domain::forum::{model::StGroup, repository::TrForumRepository};
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct CForumService<R>
where
    R: TrForumRepository,
{
    oRepository: R,
}

impl<R> CForumService<R>
where
    R: TrForumRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn vecListForumGroups(&self) -> Result<Vec<StGroup>> {
        self.oRepository.vecListGroupsBySection(Some("forum")).await
    }

    pub async fn vecListGroupsBySection(
        &self,
        optSectionPrefix: Option<&str>,
    ) -> Result<Vec<StGroup>> {
        self.oRepository
            .vecListGroupsBySection(optSectionPrefix)
            .await
    }

    pub async fn stGroupBySectionAndUrlName(
        &self,
        sSectionPrefix: &str,
        sGroupUrlName: &str,
    ) -> Result<StGroup> {
        self.oRepository
            .stFindGroupBySectionAndUrlName(sSectionPrefix, sGroupUrlName)
            .await
    }
}
