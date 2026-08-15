use async_trait::async_trait;

use crate::{
    domain::markup::model::{StMarkupSource, StMarkupUser},
    error::Result,
};

#[async_trait]
pub trait TrMarkupUserRepository: Send + Sync {
    async fn vecFindByNicks(&self, vecNicks: &[String]) -> Result<Vec<StMarkupUser>>;
    async fn vecSourcesByMessageIds(&self, vecMessageIds: &[i32]) -> Result<Vec<StMarkupSource>>;
}
