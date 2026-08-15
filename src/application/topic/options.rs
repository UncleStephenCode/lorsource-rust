use crate::{
    domain::{
        topic::options::{
            StSetTopicOptions, StTopicOptions, TrTopicOptionsRepository, TrTopicReindexQueue,
            bValidPostScore, sPostScoreInfoFull,
        },
        user::model::StUserSummary,
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StSetPostScoreOutcome {
    pub sBigMessage: String,
    pub sCanonicalUrl: String,
    pub bPostScoreChanged: bool,
}

#[derive(Debug, Clone)]
pub struct CTopicOptionsService<R, Q>
where
    R: TrTopicOptionsRepository,
    Q: TrTopicReindexQueue,
{
    oRepository: R,
    oReindexQueue: Q,
}

impl<R, Q> CTopicOptionsService<R, Q>
where
    R: TrTopicOptionsRepository,
    Q: TrTopicReindexQueue,
{
    pub fn new(oRepository: R, oReindexQueue: Q) -> Self {
        Self {
            oRepository,
            oReindexQueue,
        }
    }

    pub async fn stForm(
        &self,
        optUser: Option<&StUserSummary>,
        iTopicId: i32,
    ) -> Result<StTopicOptions> {
        vRequireModerator(optUser)?;
        self.oRepository
            .optFind(iTopicId)
            .await?
            .ok_or(AppError::NotFound)
    }

    pub async fn stSet(
        &self,
        optUser: Option<&StUserSummary>,
        stOptions: StSetTopicOptions,
    ) -> Result<StSetPostScoreOutcome> {
        let stModerator = vRequireModerator(optUser)?;
        if !bValidPostScore(stOptions.iPostScore) {
            return Err(AppError::BadRequest(format!(
                "invalid postscore {}",
                stOptions.iPostScore
            )));
        }

        // TopicModificationController loads this comparison snapshot before
        // opening SpringDB.localTx; preserve that race behavior exactly.
        let stBefore = self
            .oRepository
            .optFind(stOptions.iTopicId)
            .await?
            .ok_or(AppError::NotFound)?;
        self.oRepository.vSet(stOptions).await?;
        let bPostScoreChanged = stBefore.iPostScore != stOptions.iPostScore;
        let mut sBigMessage = String::new();
        if bPostScoreChanged {
            sBigMessage.push_str("Установлен новый уровень записи: ");
            sBigMessage.push_str(&sPostScoreInfoFull(stOptions.iPostScore));
            sBigMessage.push_str("<br>");
            tracing::info!(
                topic_id = stOptions.iTopicId,
                postscore = stOptions.iPostScore,
                moderator = %stModerator.nick,
                "topic postscore changed"
            );
        }
        if stBefore.bSticky != stOptions.bSticky {
            sBigMessage.push_str(&format!("Новое значение sticky: {}<br>", stOptions.bSticky));
            tracing::info!(sticky = stOptions.bSticky, "topic sticky changed");
        }
        if stBefore.bNoTop != stOptions.bNoTop {
            sBigMessage.push_str(&format!("Новое значение notop: {}<br>", stOptions.bNoTop));
            tracing::info!(notop = stOptions.bNoTop, "topic notop changed");
        }

        // Java sends this after the DB transaction and only for a postscore
        // delta. A queue failure therefore leaves the committed DB update in
        // place and surfaces an HTTP error to the caller.
        if bPostScoreChanged {
            self.oReindexQueue
                .vUpdateMessage(stOptions.iTopicId, true)
                .await?;
        }

        Ok(StSetPostScoreOutcome {
            sBigMessage,
            sCanonicalUrl: stBefore.sCanonicalUrl,
            bPostScoreChanged,
        })
    }
}

fn vRequireModerator(optUser: Option<&StUserSummary>) -> Result<&StUserSummary> {
    optUser
        .filter(|stUser| stUser.canmod)
        .ok_or(AppError::Forbidden)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;

    #[derive(Clone)]
    struct CRepository {
        stBefore: StTopicOptions,
        vecWrites: Arc<Mutex<Vec<StSetTopicOptions>>>,
    }

    #[async_trait]
    impl TrTopicOptionsRepository for CRepository {
        async fn optFind(&self, iTopicId: i32) -> Result<Option<StTopicOptions>> {
            Ok((self.stBefore.iTopicId == iTopicId).then(|| self.stBefore.clone()))
        }

        async fn vSet(&self, stOptions: StSetTopicOptions) -> Result<()> {
            self.vecWrites.lock().unwrap().push(stOptions);
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct CQueue(Arc<Mutex<Vec<(i32, bool)>>>);

    #[async_trait]
    impl TrTopicReindexQueue for CQueue {
        async fn vUpdateMessage(&self, iTopicId: i32, bWithComments: bool) -> Result<()> {
            self.0.lock().unwrap().push((iTopicId, bWithComments));
            Ok(())
        }
    }

    #[derive(Clone)]
    struct CFailingQueue;

    #[async_trait]
    impl TrTopicReindexQueue for CFailingQueue {
        async fn vUpdateMessage(&self, _iTopicId: i32, _bWithComments: bool) -> Result<()> {
            Err(anyhow::anyhow!("queue unavailable").into())
        }
    }

    fn stModerator() -> StUserSummary {
        StUserSummary {
            id: 7,
            nick: "moderator".into(),
            name: None,
            score: Some(100),
            max_score: Some(100),
            photo: None,
            town: None,
            regdate: None,
            canmod: true,
            candel: false,
            corrector: false,
            blocked: Some(false),
            userinfo: None,
        }
    }

    fn stBefore() -> StTopicOptions {
        StTopicOptions {
            iTopicId: 42,
            iPostScore: -9999,
            bSticky: false,
            bNoTop: false,
            bPremoderated: false,
            sCanonicalUrl: "/forum/group/42".into(),
        }
    }

    #[tokio::test]
    async fn only_a_postscore_delta_queues_topic_and_comments_after_the_write() {
        let vecWrites = Arc::new(Mutex::new(Vec::new()));
        let oQueue = CQueue::default();
        let cService = CTopicOptionsService::new(
            CRepository {
                stBefore: stBefore(),
                vecWrites: vecWrites.clone(),
            },
            oQueue.clone(),
        );
        let stOutcome = cService
            .stSet(
                Some(&stModerator()),
                StSetTopicOptions {
                    iTopicId: 42,
                    iPostScore: 1234,
                    bSticky: true,
                    bNoTop: true,
                },
            )
            .await
            .unwrap();

        assert!(stOutcome.bPostScoreChanged);
        assert_eq!(&*oQueue.0.lock().unwrap(), &[(42, true)]);
        assert_eq!(vecWrites.lock().unwrap().len(), 1);
        assert_eq!(stOutcome.sCanonicalUrl, "/forum/group/42");
        assert!(stOutcome.sBigMessage.contains("score>=1234"));
        assert!(stOutcome.sBigMessage.contains("sticky: true<br>"));
        assert!(stOutcome.sBigMessage.contains("notop: true<br>"));
    }

    #[tokio::test]
    async fn sticky_notop_and_noop_updates_do_not_queue_reindex() {
        let oQueue = CQueue::default();
        let cService = CTopicOptionsService::new(
            CRepository {
                stBefore: stBefore(),
                vecWrites: Arc::new(Mutex::new(Vec::new())),
            },
            oQueue.clone(),
        );
        let stOutcome = cService
            .stSet(
                Some(&stModerator()),
                StSetTopicOptions {
                    iTopicId: 42,
                    iPostScore: -9999,
                    bSticky: true,
                    bNoTop: true,
                },
            )
            .await
            .unwrap();
        assert!(!stOutcome.bPostScoreChanged);
        assert!(oQueue.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn queue_failure_is_reported_after_the_database_write() {
        let vecWrites = Arc::new(Mutex::new(Vec::new()));
        let cService = CTopicOptionsService::new(
            CRepository {
                stBefore: stBefore(),
                vecWrites: vecWrites.clone(),
            },
            CFailingQueue,
        );
        let stResult = cService
            .stSet(
                Some(&stModerator()),
                StSetTopicOptions {
                    iTopicId: 42,
                    iPostScore: 50,
                    bSticky: false,
                    bNoTop: false,
                },
            )
            .await;

        assert!(matches!(stResult, Err(AppError::Anyhow(_))));
        assert_eq!(vecWrites.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn auth_and_range_fail_before_any_database_write() {
        let vecWrites = Arc::new(Mutex::new(Vec::new()));
        let cService = CTopicOptionsService::new(
            CRepository {
                stBefore: stBefore(),
                vecWrites: vecWrites.clone(),
            },
            CQueue::default(),
        );
        let stCommand = StSetTopicOptions {
            iTopicId: 42,
            iPostScore: -9998,
            bSticky: false,
            bNoTop: false,
        };
        assert!(matches!(
            cService.stSet(None, stCommand).await,
            Err(AppError::Forbidden)
        ));
        assert!(matches!(
            cService.stSet(Some(&stModerator()), stCommand).await,
            Err(AppError::BadRequest(_))
        ));
        assert!(vecWrites.lock().unwrap().is_empty());
    }
}
