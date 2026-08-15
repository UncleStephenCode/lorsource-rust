use chrono::Utc;

use crate::{
    domain::topic::{
        deletion::{
            EnTopicDeletionRestriction, StDeleteTopicCommand, StTopicDeleteMutation,
            StTopicDeletionActor, StTopicDeletionSnapshot, TrTopicDeletionRepository,
            optDeleteRestriction, optUndeleteRestriction,
        },
        options::TrTopicReindexQueue,
    },
    error::AppError,
};

#[derive(Debug, thiserror::Error)]
pub enum EnTopicDeletionServiceError {
    #[error("topic not found")]
    NotFound,
    #[error("not authorized")]
    NotAuthorized,
    #[error("{0}")]
    Restricted(EnTopicDeletionRestriction),
    #[error("неправильный размер штрафа")]
    InvalidPenalty,
    #[error(transparent)]
    Infrastructure(#[from] AppError),
}

pub type TyTopicDeletionResult<T> = std::result::Result<T, EnTopicDeletionServiceError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StDeleteTopicFormData {
    pub stTopic: StTopicDeletionSnapshot,
    /// Exact `bonus` model attribute.  The JSP combines this with the global
    /// moderator-session flag before displaying the penalty control.
    pub bBonusEligible: bool,
    pub bModeratorSession: bool,
    pub bUncommitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StUndeleteTopicFormData {
    /// The route must pass this through the shared PreparedTopic pipeline and
    /// render the full menu-free topic card; the Java form is not an ID-only
    /// confirmation page.
    pub stTopic: StTopicDeletionSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicDeletionOutcome {
    pub sMessage: &'static str,
    pub optLink: Option<String>,
    pub optMutation: Option<StTopicDeleteMutation>,
}

#[derive(Debug, Clone)]
pub struct CTopicDeletionService<R, Q>
where
    R: TrTopicDeletionRepository,
    Q: TrTopicReindexQueue,
{
    oRepository: R,
    oReindexQueue: Q,
}

impl<R, Q> CTopicDeletionService<R, Q>
where
    R: TrTopicDeletionRepository,
    Q: TrTopicReindexQueue,
{
    pub fn new(oRepository: R, oReindexQueue: Q) -> Self {
        Self {
            oRepository,
            oReindexQueue,
        }
    }

    pub async fn stPrepareDelete(
        &self,
        optActor: Option<StTopicDeletionActor<'_>>,
        iTopicId: i32,
    ) -> TyTopicDeletionResult<StDeleteTopicFormData> {
        let stActor = stRequireAuthorized(optActor)?;
        let stTopic = self.stSnapshot(iTopicId).await?;
        vRestriction(optDeleteRestriction(stActor, &stTopic, Utc::now()))?;
        Ok(StDeleteTopicFormData {
            bBonusEligible: stTopic.bDeleteBonusEligible(),
            bModeratorSession: stActor.bModerator,
            bUncommitted: stTopic.bUncommitted(),
            stTopic,
        })
    }

    pub async fn stDelete(
        &self,
        optActor: Option<StTopicDeletionActor<'_>>,
        mut stCommand: StDeleteTopicCommand,
    ) -> TyTopicDeletionResult<StTopicDeletionOutcome> {
        // Binding is performed by the route before this call, but the range
        // check lives inside AuthorizedOnly in Java and therefore follows the
        // authorization check for a successfully bound request.
        let stActor = stRequireAuthorized(optActor)?;
        if !(0..=20).contains(&stCommand.iPenalty) {
            return Err(EnTopicDeletionServiceError::InvalidPenalty);
        }
        let stTopic = self.stSnapshot(stCommand.iTopicId).await?;
        vRestriction(optDeleteRestriction(stActor, &stTopic, Utc::now()))?;

        if !stActor.bModerator || stActor.iUserId == stTopic.iAuthorId || stTopic.bDraft {
            stCommand.iPenalty = 0;
        }

        let stMutation = self
            .oRepository
            .stDelete(stActor, &stTopic, &stCommand)
            .await?;

        // DeleteTopicController sends the update after DeleteService.localTx
        // returns.  It does so even when TopicDao.delete lost a race and no DB
        // side effect occurred; a queue failure cannot roll the transaction
        // back and is still surfaced to the request.
        self.oReindexQueue
            .vUpdateMessage(stCommand.iTopicId, true)
            .await?;

        Ok(StTopicDeletionOutcome {
            sMessage: "Сообщение удалено",
            optLink: None,
            optMutation: Some(stMutation),
        })
    }

    pub async fn stPrepareUndelete(
        &self,
        optActor: Option<StTopicDeletionActor<'_>>,
        iTopicId: i32,
    ) -> TyTopicDeletionResult<StUndeleteTopicFormData> {
        let stActor = stRequireAuthorized(optActor)?;
        let stTopic = self.stSnapshot(iTopicId).await?;
        vRestriction(optUndeleteRestriction(stActor, &stTopic, Utc::now()))?;
        Ok(StUndeleteTopicFormData { stTopic })
    }

    pub async fn stUndelete(
        &self,
        optActor: Option<StTopicDeletionActor<'_>>,
        iTopicId: i32,
    ) -> TyTopicDeletionResult<StTopicDeletionOutcome> {
        let stActor = stRequireAuthorized(optActor)?;
        let stTopic = self.stSnapshot(iTopicId).await?;
        vRestriction(optUndeleteRestriction(stActor, &stTopic, Utc::now()))?;

        self.oRepository.vUndelete(&stTopic).await?;
        self.oReindexQueue.vUpdateMessage(iTopicId, true).await?;

        Ok(StTopicDeletionOutcome {
            sMessage: "Сообщение восстановлено",
            optLink: Some(stTopic.sCanonicalUrl()),
            optMutation: None,
        })
    }

    async fn stSnapshot(&self, iTopicId: i32) -> TyTopicDeletionResult<StTopicDeletionSnapshot> {
        self.oRepository
            .optSnapshot(iTopicId)
            .await?
            .ok_or(EnTopicDeletionServiceError::NotFound)
    }
}

fn stRequireAuthorized(
    optActor: Option<StTopicDeletionActor<'_>>,
) -> TyTopicDeletionResult<StTopicDeletionActor<'_>> {
    optActor.ok_or(EnTopicDeletionServiceError::NotAuthorized)
}

fn vRestriction(optRestriction: Option<EnTopicDeletionRestriction>) -> TyTopicDeletionResult<()> {
    match optRestriction {
        Some(enRestriction) => Err(EnTopicDeletionServiceError::Restricted(enRestriction)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::error::Result;

    #[derive(Clone)]
    struct CRepository {
        stTopic: StTopicDeletionSnapshot,
        stMutation: StTopicDeleteMutation,
        vecCommands: Arc<Mutex<Vec<StDeleteTopicCommand>>>,
        vecEvents: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl TrTopicDeletionRepository for CRepository {
        async fn optSnapshot(&self, _iTopicId: i32) -> Result<Option<StTopicDeletionSnapshot>> {
            Ok(Some(self.stTopic.clone()))
        }

        async fn stDelete(
            &self,
            _stActor: StTopicDeletionActor<'_>,
            _stTopic: &StTopicDeletionSnapshot,
            stCommand: &StDeleteTopicCommand,
        ) -> Result<StTopicDeleteMutation> {
            self.vecCommands.lock().unwrap().push(stCommand.clone());
            self.vecEvents.lock().unwrap().push("commit");
            Ok(self.stMutation)
        }

        async fn vUndelete(&self, _stTopic: &StTopicDeletionSnapshot) -> Result<()> {
            self.vecEvents.lock().unwrap().push("commit");
            Ok(())
        }
    }

    #[derive(Clone)]
    struct CQueue {
        vecEvents: Arc<Mutex<Vec<&'static str>>>,
        vecCalls: Arc<Mutex<Vec<(i32, bool)>>>,
        bFail: bool,
    }

    type TyCommandLog = Arc<Mutex<Vec<StDeleteTopicCommand>>>;
    type TyEventLog = Arc<Mutex<Vec<&'static str>>>;
    type TyQueueCallLog = Arc<Mutex<Vec<(i32, bool)>>>;
    type TyServiceFixture = (
        CTopicDeletionService<CRepository, CQueue>,
        TyCommandLog,
        TyEventLog,
        TyQueueCallLog,
    );

    #[async_trait]
    impl TrTopicReindexQueue for CQueue {
        async fn vUpdateMessage(&self, iTopicId: i32, bWithComments: bool) -> Result<()> {
            self.vecEvents.lock().unwrap().push("queue");
            self.vecCalls
                .lock()
                .unwrap()
                .push((iTopicId, bWithComments));
            if self.bFail {
                return Err(AppError::Anyhow(anyhow::anyhow!("queue unavailable")));
            }
            Ok(())
        }
    }

    fn stTopic() -> StTopicDeletionSnapshot {
        StTopicDeletionSnapshot {
            iTopicId: 42,
            iAuthorId: 7,
            sAuthorNick: "author".into(),
            iAuthorScore: 10,
            iAuthorMaxScore: 20,
            bAuthorBlocked: false,
            bAuthorAnonymous: false,
            bAuthorFrozen: false,
            sStoredTitle: "title".into(),
            sMessage: "body".into(),
            sMarkup: "MARKDOWN".into(),
            optUrl: None,
            optLinkText: None,
            iGroupId: 10,
            sGroupTitle: "General".into(),
            sGroupUrlName: "general".into(),
            iSectionId: 2,
            sSectionTitle: "Форум".into(),
            sSectionPrefix: "forum".into(),
            bSectionPremoderated: false,
            bSectionPollAllowed: false,
            bSectionImagePost: false,
            bSectionImageAllowed: false,
            bLinksAllowed: true,
            bDeleted: false,
            bDraft: false,
            bCommitted: true,
            bSticky: false,
            bResolved: false,
            bExpired: false,
            iCommentCount: 0,
            iPostScore: -9999,
            bMinor: false,
            dtPostdate: Utc::now() - Duration::hours(1),
            optCommitDate: None,
            dtLastMod: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            optDeleteDate: None,
            sPostIp: "127.0.0.1".into(),
            iUserAgentId: 1,
        }
    }

    fn stActor(iUserId: i32, bModerator: bool) -> StTopicDeletionActor<'static> {
        StTopicDeletionActor {
            iUserId,
            sNick: "actor",
            bModerator,
            bAdministrator: false,
        }
    }

    fn stService(
        stTopic: StTopicDeletionSnapshot,
        stMutation: StTopicDeleteMutation,
        bQueueFail: bool,
    ) -> TyServiceFixture {
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let vecEvents = Arc::new(Mutex::new(Vec::new()));
        let vecCalls = Arc::new(Mutex::new(Vec::new()));
        let oRepository = CRepository {
            stTopic,
            stMutation,
            vecCommands: vecCommands.clone(),
            vecEvents: vecEvents.clone(),
        };
        let oQueue = CQueue {
            vecEvents: vecEvents.clone(),
            vecCalls: vecCalls.clone(),
            bFail: bQueueFail,
        };
        (
            CTopicDeletionService::new(oRepository, oQueue),
            vecCommands,
            vecEvents,
            vecCalls,
        )
    }

    #[tokio::test]
    async fn delete_race_still_queues_after_the_committed_repository_result() {
        let (oService, _, vecEvents, vecCalls) = stService(
            stTopic(),
            StTopicDeleteMutation {
                bDeleted: false,
                iAppliedScoreDelta: 0,
            },
            false,
        );
        let stOutcome = oService
            .stDelete(
                Some(stActor(8, true)),
                StDeleteTopicCommand {
                    iTopicId: 42,
                    sReason: "spam".into(),
                    iPenalty: 7,
                },
            )
            .await
            .unwrap();
        assert_eq!(*vecEvents.lock().unwrap(), ["commit", "queue"]);
        assert_eq!(*vecCalls.lock().unwrap(), [(42, true)]);
        assert_eq!(stOutcome.sMessage, "Сообщение удалено");
        assert_eq!(stOutcome.optLink, None);
        assert!(!stOutcome.optMutation.unwrap().bDeleted);
    }

    #[tokio::test]
    async fn only_a_moderator_deleting_another_users_non_draft_can_apply_penalty() {
        for (stActor, bDraft, iExpected) in [
            (stActor(8, true), false, 9),
            (stActor(7, true), false, 0),
            (stActor(7, false), false, 0),
            (stActor(8, true), true, 0),
        ] {
            let mut stTopic = stTopic();
            stTopic.bDraft = bDraft;
            let (oService, vecCommands, _, _) = stService(
                stTopic,
                StTopicDeleteMutation {
                    bDeleted: true,
                    iAppliedScoreDelta: -iExpected,
                },
                false,
            );
            oService
                .stDelete(
                    Some(stActor),
                    StDeleteTopicCommand {
                        iTopicId: 42,
                        sReason: "reason".into(),
                        iPenalty: 9,
                    },
                )
                .await
                .unwrap();
            assert_eq!(vecCommands.lock().unwrap()[0].iPenalty, iExpected);
        }
    }

    #[tokio::test]
    async fn queue_failure_is_after_commit_and_remains_visible() {
        let (oService, _, vecEvents, _) = stService(
            stTopic(),
            StTopicDeleteMutation {
                bDeleted: true,
                iAppliedScoreDelta: -7,
            },
            true,
        );
        let stError = oService
            .stDelete(
                Some(stActor(8, true)),
                StDeleteTopicCommand {
                    iTopicId: 42,
                    sReason: "reason".into(),
                    iPenalty: 7,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            stError,
            EnTopicDeletionServiceError::Infrastructure(_)
        ));
        assert_eq!(*vecEvents.lock().unwrap(), ["commit", "queue"]);
    }

    #[tokio::test]
    async fn undelete_returns_the_stale_canonical_link_and_queues_after_commit() {
        let mut stTopic = stTopic();
        stTopic.bDeleted = true;
        stTopic.optDeleteDate = Some(Utc::now());
        let (oService, _, vecEvents, vecCalls) = stService(
            stTopic,
            StTopicDeleteMutation {
                bDeleted: false,
                iAppliedScoreDelta: 0,
            },
            false,
        );
        let stOutcome = oService
            .stUndelete(Some(stActor(8, true)), 42)
            .await
            .unwrap();
        assert_eq!(*vecEvents.lock().unwrap(), ["commit", "queue"]);
        assert_eq!(*vecCalls.lock().unwrap(), [(42, true)]);
        assert_eq!(stOutcome.sMessage, "Сообщение восстановлено");
        assert_eq!(stOutcome.optLink.as_deref(), Some("/forum/general/42"));
    }

    #[tokio::test]
    async fn authorization_precedes_the_in_method_penalty_range_check() {
        let (oService, vecCommands, _, _) = stService(
            stTopic(),
            StTopicDeleteMutation {
                bDeleted: true,
                iAppliedScoreDelta: 0,
            },
            false,
        );
        let stError = oService
            .stDelete(
                None,
                StDeleteTopicCommand {
                    iTopicId: 42,
                    sReason: "reason".into(),
                    iPenalty: -1,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            stError,
            EnTopicDeletionServiceError::NotAuthorized
        ));
        assert!(vecCommands.lock().unwrap().is_empty());
    }
}
