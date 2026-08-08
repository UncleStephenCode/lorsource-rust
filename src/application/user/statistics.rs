use crate::{
    domain::user::statistics::{TrUserStatisticsRepository, TyUserYearStats},
    error::Result,
};

#[derive(Debug, Clone)]
pub struct CUserStatisticsService<R>
where
    R: TrUserStatisticsRepository,
{
    oRepository: R,
}

impl<R> CUserStatisticsService<R>
where
    R: TrUserStatisticsRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn mapYearStats(&self, sNick: &str, sTimezone: &str) -> Result<TyUserYearStats> {
        self.oRepository.mapYearStats(sNick, sTimezone).await
    }
}
