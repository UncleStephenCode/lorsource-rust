use crate::domain::comment::model::StCommentItem;
use crate::domain::topic::{model::{StTopicDetail, StTopicSummary}, repository::{StEditTopic, StNewTopic, TrTopicRepository}};
use crate::error::Result;
use sqlx::{Postgres, Transaction};

#[derive(Debug, Clone)]
pub struct CTopicService<R>
where
    R: TrTopicRepository,
{
    oRepository: R,
}

impl<R> CTopicService<R>
where
    R: TrTopicRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn vecListTopics(&self, optSection: Option<&str>, optGroup: Option<&str>, iOffset: i64, iLimit: i64) -> Result<Vec<StTopicSummary>> {
        self.oRepository.vecListTopics(optSection, optGroup, iOffset, iLimit).await
    }

    pub async fn stGetTopic(&self, iTopicId: i32) -> Result<StTopicDetail> {
        self.oRepository.stGetTopic(iTopicId).await
    }

    pub async fn vecListComments(&self, iTopicId: i32) -> Result<Vec<StCommentItem>> {
        self.oRepository.vecListComments(iTopicId).await
    }

    pub async fn iNextMessageId(&self, txPg: &mut Transaction<'_, Postgres>) -> Result<i32> {
        self.oRepository.iNextMessageId(txPg).await
    }

    pub async fn vInsertTopicMessage(&self, txPg: &mut Transaction<'_, Postgres>, iMsgId: i32, sMessage: &str) -> Result<()> {
        self.oRepository.vInsertTopicMessage(txPg, iMsgId, sMessage).await
    }

    pub async fn vInsertTopic(&self, txPg: &mut Transaction<'_, Postgres>, stNewTopic: StNewTopic<'_>) -> Result<()> {
        self.oRepository.vInsertTopic(txPg, stNewTopic).await
    }

    pub async fn vUpdateTopicMessage(&self, txPg: &mut Transaction<'_, Postgres>, iMsgId: i32, sMessage: &str) -> Result<()> {
        self.oRepository.vUpdateTopicMessage(txPg, iMsgId, sMessage).await
    }

    pub async fn vUpdateTopicHeader(&self, txPg: &mut Transaction<'_, Postgres>, stEditTopic: StEditTopic<'_>) -> Result<()> {
        self.oRepository.vUpdateTopicHeader(txPg, stEditTopic).await
    }

    pub async fn vReplaceTags(&self, txPg: &mut Transaction<'_, Postgres>, iMsgId: i32, optTags: Option<&str>) -> Result<()> {
        self.oRepository.vReplaceTags(txPg, iMsgId, optTags).await
    }

    pub async fn vSetDeleted(&self, iTopicId: i32, bDeleted: bool) -> Result<()> {
        self.oRepository.vSetDeleted(iTopicId, bDeleted).await
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
