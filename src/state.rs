use crate::config::Config;
use sqlx::PgPool;

#[derive(Clone)]
pub struct StAppState {
    pub config: Config,
    pub pool: PgPool,
    pub http: reqwest::Client,
}

pub type AppState = StAppState;

impl StAppState {
    pub fn stNew(stConfig: Config, oPool: PgPool) -> Self {
        Self { config: stConfig, pool: oPool, http: reqwest::Client::new() }
    }

    pub fn new(stConfig: Config, oPool: PgPool) -> Self {
        Self::stNew(stConfig, oPool)
    }
}
