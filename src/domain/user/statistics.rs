use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::error::Result;

pub type TyUserYearStats = BTreeMap<i64, i64>;

#[async_trait]
pub trait TrUserStatisticsRepository: Send + Sync {
    async fn mapYearStats(&self, sNick: &str, sTimezone: &str) -> Result<TyUserYearStats>;
}
