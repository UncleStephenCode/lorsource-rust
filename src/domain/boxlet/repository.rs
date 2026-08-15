use async_trait::async_trait;

use crate::{
    domain::boxlet::model::{
        StGalleryBoxletRow, StPollBoxletRow, StPollVariantResult, StTagCloudRow, StTopicBoxletRow,
    },
    error::Result,
};

#[async_trait]
pub trait TrBoxletRepository: Send + Sync {
    async fn vecTopTags(&self, iLimit: i32) -> Result<Vec<StTagCloudRow>>;
    async fn vecGalleryItems(&self, iLimit: i32) -> Result<Vec<StGalleryBoxletRow>>;
    async fn sUserNick(&self, iUserId: i32) -> Result<String>;
    async fn vecTopTopics(&self) -> Result<Vec<StTopicBoxletRow>>;
    async fn vecArticles(&self) -> Result<Vec<StTopicBoxletRow>>;
    async fn optUserSettings(&self, iUserId: i32) -> Result<Option<String>>;
    async fn vecMostRecentPolls(&self) -> Result<Vec<StPollBoxletRow>>;
    async fn vecPollResults(&self, iPollId: i32, iUserId: i32) -> Result<Vec<StPollVariantResult>>;
    async fn iPollVotes(&self, iPollId: i32) -> Result<i32>;
    async fn iPollUsers(&self, iPollId: i32) -> Result<i32>;
}
