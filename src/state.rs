use std::sync::Arc;

use crate::{
    application::{
        auth::{CCommentFloodCache, CLoginAttemptCache},
        exception_reporting::CExceptionReporter,
        realtime::CRealtimeService,
        topic::posting::CTopicPublishService,
    },
    config::Config,
    infra::postgres::{
        add_topic_repository::CAddTopicPgRepository, realtime_repository::CRealtimePgRepository,
    },
    infra::smtp::CSmtpEmailSender,
};
use sqlx::PgPool;

#[derive(Clone)]
pub struct StAppState {
    pub config: Config,
    pub pool: PgPool,
    pub http: reqwest::Client,
    pub proxy_http: Option<reqwest::Client>,
    pub realtime: Arc<CRealtimeService<CRealtimePgRepository>>,
    pub topic_publish: Arc<CTopicPublishService<CAddTopicPgRepository>>,
    pub login_attempts: Arc<CLoginAttemptCache>,
    pub comment_flood: Arc<CCommentFloodCache>,
    pub exception_reporter: CExceptionReporter,
}

pub type AppState = StAppState;

impl StAppState {
    pub fn stNew(stConfig: Config, oPool: PgPool) -> Self {
        let oRealtimeRepository = CRealtimePgRepository::new(oPool.clone());
        let oAddTopicRepository = CAddTopicPgRepository::new(oPool.clone());
        let oTopicPublishService =
            CTopicPublishService::new(oAddTopicRepository, &stConfig.public_url);
        let oCommentFlood = CCommentFloodCache::new(&stConfig.public_url);
        let cSmtpSender = CSmtpEmailSender::new(
            stConfig.smtp_host.clone(),
            stConfig.smtp_port,
            stConfig.smtp_helo_name.clone(),
        );
        let cExceptionReporter =
            CExceptionReporter::stNew(stConfig.admin_email.clone(), cSmtpSender);
        let optProxyHttp = stConfig.fallback_proxy_url.as_deref().map(|sProxyUrl| {
            reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(sProxyUrl).expect("valid FALLBACK_PROXY_URL"))
                .connect_timeout(std::time::Duration::from_secs(3))
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("static fallback HTTP client configuration")
        });
        Self {
            config: stConfig,
            pool: oPool,
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(3))
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("static HTTP client configuration"),
            proxy_http: optProxyHttp,
            realtime: Arc::new(CRealtimeService::new(oRealtimeRepository)),
            topic_publish: Arc::new(oTopicPublishService),
            login_attempts: Arc::new(CLoginAttemptCache::default()),
            comment_flood: Arc::new(oCommentFlood),
            exception_reporter: cExceptionReporter,
        }
    }

    pub fn new(stConfig: Config, oPool: PgPool) -> Self {
        Self::stNew(stConfig, oPool)
    }
}
