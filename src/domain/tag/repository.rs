use async_trait::async_trait;
use sqlx::{Postgres, Transaction};

use crate::error::Result;

#[async_trait]
pub trait TrTagRepository: Send + Sync {
    async fn vReplaceTopicTags(&self, txPg: &mut Transaction<'_, Postgres>, iMsgId: i32, optTags: Option<&str>) -> Result<()>;
}
