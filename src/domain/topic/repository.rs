use async_trait::async_trait;
use sqlx::{Postgres, Transaction};

use crate::domain::comment::model::StCommentItem;
use crate::domain::topic::model::{StRssContext, StTopicDetail, StTopicSummary};
use crate::error::Result;

#[async_trait]
pub trait TrTopicRepository: Send + Sync {
    async fn vecListTopics(
        &self,
        optSection: Option<&str>,
        optGroup: Option<&str>,
        iOffset: i64,
        iLimit: i64,
    ) -> Result<Vec<StTopicSummary>>;
    async fn stRssContext(&self, iSectionId: i32, iGroupId: i32) -> Result<StRssContext>;
    async fn vecListRssTopics(
        &self,
        iSectionId: i32,
        iGroupId: i32,
        bNoTalks: bool,
        bTech: bool,
    ) -> Result<Vec<StTopicSummary>>;
    async fn stGetTopic(&self, iTopicId: i32) -> Result<StTopicDetail>;
    async fn vecListComments(&self, iTopicId: i32) -> Result<Vec<StCommentItem>>;
    async fn iNextMessageId(&self, txPg: &mut Transaction<'_, Postgres>) -> Result<i32>;
    async fn vInsertTopicMessage(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        sMessage: &str,
        sMarkup: &str,
    ) -> Result<()>;
    async fn vInsertTopic(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        stNewTopic: StNewTopic<'_>,
    ) -> Result<()>;
    async fn vUpdateTopicMessage(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        sMessage: &str,
    ) -> Result<()>;
    async fn vUpdateTopicHeader(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        stEditTopic: StEditTopic<'_>,
    ) -> Result<()>;
    async fn vReplaceTags(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        optTags: Option<&str>,
    ) -> Result<()>;
    async fn optResolveMeta(&self, iTopicId: i32) -> Result<Option<(i32, bool)>>;
    async fn vSetResolved(&self, iTopicId: i32, optResolved: Option<bool>) -> Result<()>;
    async fn vCommitTopic(&self, iTopicId: i32, iModeratorId: i32) -> Result<()>;
    async fn vUncommitTopic(&self, iTopicId: i32) -> Result<()>;
    async fn vMoveTopic(&self, iTopicId: i32, iGroupId: i32) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct StNewTopic<'a> {
    pub iMsgId: i32,
    pub iGroupId: i32,
    pub iUserId: i32,
    pub sTitle: &'a str,
    pub optUrl: Option<&'a str>,
    pub optLinkText: Option<&'a str>,
    pub bDraft: bool,
    pub sPostIp: &'a str,
    pub optUserAgent: Option<&'a str>,
    pub bAllowAnonymous: bool,
}

#[derive(Debug, Clone)]
pub struct StEditTopic<'a> {
    pub iMsgId: i32,
    pub sTitle: &'a str,
    pub optUrl: Option<String>,
    pub optLinkText: Option<String>,
}
