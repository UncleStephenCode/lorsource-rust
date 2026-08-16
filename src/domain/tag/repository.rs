use async_trait::async_trait;

use crate::{
    domain::{
        tag::model::{
            StTagForumTopic, StTagInfo, StTagSection, StTagViewerProfile, StTagViewerState,
        },
        topic::model::StTopicSummary,
    },
    error::Result,
};

#[async_trait]
pub trait TrTagTopicListRepository: Send + Sync {
    async fn optSection(&self, iSectionId: i32) -> Result<Option<StTagSection>>;
    async fn optTagInfo(&self, sTag: &str) -> Result<Option<StTagInfo>>;
    async fn optSynonymTarget(&self, sTag: &str) -> Result<Option<String>>;
    async fn vecTagSections(&self, iTagId: i32) -> Result<Vec<StTagSection>>;
    async fn stViewerProfile(&self, optViewerId: Option<i32>) -> Result<StTagViewerProfile>;
    async fn stViewerState(
        &self,
        iTagId: i32,
        optViewerId: Option<i32>,
    ) -> Result<StTagViewerState>;
    async fn vecFeedTopics(
        &self,
        stSection: &StTagSection,
        iTagId: i32,
        optViewerId: Option<i32>,
        iOffset: i32,
        iLimit: i32,
    ) -> Result<Vec<StTopicSummary>>;
    async fn vecForumTopics(
        &self,
        stSection: &StTagSection,
        iTagId: i32,
        optViewerId: Option<i32>,
        iOffset: i32,
        iLimit: i32,
    ) -> Result<Vec<StTagForumTopic>>;
}

#[async_trait]
pub trait TrTagTopicCountRepository: Send + Sync {
    async fn iCountTagTopics(&self, sTag: &str, sSectionUrlName: &str) -> Result<i64>;
}
