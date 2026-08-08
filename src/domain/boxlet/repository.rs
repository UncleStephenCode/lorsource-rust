use async_trait::async_trait;

use crate::{
    domain::boxlet::model::{StGalleryBoxletRow, StTagCloudRow},
    error::Result,
};

#[async_trait]
pub trait TrBoxletRepository: Send + Sync {
    async fn vecTopTags(&self, iLimit: i32) -> Result<Vec<StTagCloudRow>>;
    async fn vecGalleryItems(&self, iLimit: i32) -> Result<Vec<StGalleryBoxletRow>>;
    async fn sUserNick(&self, iUserId: i32) -> Result<String>;
}
