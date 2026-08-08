use std::sync::Arc;

use crate::{
    application::{realtime::CRealtimeService, topic::posting::CTopicPublishService},
    config::Config,
    infra::postgres::{
        add_topic_repository::CAddTopicPgRepository, realtime_repository::CRealtimePgRepository,
    },
};
use sqlx::PgPool;

#[derive(Clone)]
pub struct StAppState {
    pub config: Config,
    pub pool: PgPool,
    pub http: reqwest::Client,
    pub realtime: Arc<CRealtimeService<CRealtimePgRepository>>,
    pub topic_publish: Arc<CTopicPublishService<CAddTopicPgRepository>>,
}

pub type AppState = StAppState;

impl StAppState {
    pub fn stNew(stConfig: Config, oPool: PgPool) -> Self {
        let oRealtimeRepository = CRealtimePgRepository::new(oPool.clone());
        let oAddTopicRepository = CAddTopicPgRepository::new(oPool.clone());
        let oTopicPublishService =
            CTopicPublishService::new(oAddTopicRepository, &stConfig.public_url);
        Self {
            config: stConfig,
            pool: oPool,
            http: reqwest::Client::new(),
            realtime: Arc::new(CRealtimeService::new(oRealtimeRepository)),
            topic_publish: Arc::new(oTopicPublishService),
        }
    }

    pub fn new(stConfig: Config, oPool: PgPool) -> Self {
        Self::stNew(stConfig, oPool)
    }
}
