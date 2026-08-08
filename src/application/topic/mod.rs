pub mod posting;

use crate::domain::comment::model::StCommentItem;
use crate::domain::topic::{
    model::{StTopicDetail, StTopicSummary},
    repository::{StEditTopic, StNewTopic, TrTopicRepository},
};
use crate::error::{AppError, Result};
use sqlx::{Postgres, Transaction};

#[derive(Debug, Clone)]
pub struct CTopicService<R>
where
    R: TrTopicRepository,
{
    oRepository: R,
}

#[derive(Debug, Clone)]
pub struct StRssFeed {
    pub sTitle: String,
    pub vecTopics: Vec<StTopicSummary>,
}

impl<R> CTopicService<R>
where
    R: TrTopicRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn vecListTopics(
        &self,
        optSection: Option<&str>,
        optGroup: Option<&str>,
        iOffset: i64,
        iLimit: i64,
    ) -> Result<Vec<StTopicSummary>> {
        self.oRepository
            .vecListTopics(optSection, optGroup, iOffset, iLimit)
            .await
    }

    pub async fn stGetTopic(&self, iTopicId: i32) -> Result<StTopicDetail> {
        self.oRepository.stGetTopic(iTopicId).await
    }

    pub async fn stRssFeed(
        &self,
        iSectionId: i32,
        iGroupId: i32,
        optFilter: Option<&str>,
    ) -> Result<StRssFeed> {
        let (bNoTalks, bTech, optFilterTitle) = match optFilter {
            None => (false, false, None),
            Some("notalks") => (true, false, Some("без talks")),
            Some("tech") => (false, true, Some("тех. форум")),
            Some(_) => {
                return Err(AppError::BadRequest(
                    "Некорректное значение filter".to_string(),
                ));
            }
        };
        let stContext = self.oRepository.stRssContext(iSectionId, iGroupId).await?;
        let mut sTitle = stContext.sSectionName.clone();
        if let Some(sGroupTitle) = stContext.optGroupTitle.as_deref() {
            sTitle.push_str(" - ");
            sTitle.push_str(sGroupTitle);
        }
        if let Some(sFilterTitle) = optFilterTitle {
            sTitle.push_str(" (");
            sTitle.push_str(sFilterTitle);
            sTitle.push(')');
        }
        let vecTopics = self
            .oRepository
            .vecListRssTopics(iSectionId, iGroupId, bNoTalks, bTech)
            .await?;
        Ok(StRssFeed { sTitle, vecTopics })
    }

    pub async fn vecListComments(&self, iTopicId: i32) -> Result<Vec<StCommentItem>> {
        self.oRepository.vecListComments(iTopicId).await
    }

    pub async fn iNextMessageId(&self, txPg: &mut Transaction<'_, Postgres>) -> Result<i32> {
        self.oRepository.iNextMessageId(txPg).await
    }

    pub async fn vInsertTopicMessage(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        sMessage: &str,
        sMarkup: &str,
    ) -> Result<()> {
        self.oRepository
            .vInsertTopicMessage(txPg, iMsgId, sMessage, sMarkup)
            .await
    }

    pub async fn vInsertTopic(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        stNewTopic: StNewTopic<'_>,
    ) -> Result<()> {
        self.oRepository.vInsertTopic(txPg, stNewTopic).await
    }

    pub async fn vUpdateTopicMessage(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        sMessage: &str,
    ) -> Result<()> {
        self.oRepository
            .vUpdateTopicMessage(txPg, iMsgId, sMessage)
            .await
    }

    pub async fn vUpdateTopicHeader(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        stEditTopic: StEditTopic<'_>,
    ) -> Result<()> {
        self.oRepository.vUpdateTopicHeader(txPg, stEditTopic).await
    }

    pub async fn vReplaceTags(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        optTags: Option<&str>,
    ) -> Result<()> {
        self.oRepository.vReplaceTags(txPg, iMsgId, optTags).await
    }

    pub async fn optResolveMeta(&self, iTopicId: i32) -> Result<Option<(i32, bool)>> {
        self.oRepository.optResolveMeta(iTopicId).await
    }

    pub async fn vSetResolved(&self, iTopicId: i32, optResolved: Option<bool>) -> Result<()> {
        self.oRepository.vSetResolved(iTopicId, optResolved).await
    }

    pub async fn vCommitTopic(&self, iTopicId: i32, iModeratorId: i32) -> Result<()> {
        self.oRepository.vCommitTopic(iTopicId, iModeratorId).await
    }

    pub async fn vUncommitTopic(&self, iTopicId: i32) -> Result<()> {
        self.oRepository.vUncommitTopic(iTopicId).await
    }

    pub async fn vMoveTopic(&self, iTopicId: i32, iGroupId: i32) -> Result<()> {
        self.oRepository.vMoveTopic(iTopicId, iGroupId).await
    }
}
