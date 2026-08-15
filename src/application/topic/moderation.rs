use crate::{
    domain::topic::{
        moderation::{
            EnTopicModerationRestriction, EnTopicMoveForm, StMoveTopicCommand,
            StTopicModerationActor, StTopicModerationSnapshot, StTopicMoveGroup,
            TrTopicModerationRepository, bResolveValue, enMoveGroupScope, optMoveRestriction,
            optResolveRestriction, optUncommitRestriction,
        },
        options::TrTopicReindexQueue,
    },
    error::AppError,
};

#[derive(Debug, thiserror::Error)]
pub enum EnTopicModerationServiceError {
    #[error("topic not found")]
    NotFound,
    #[error("forbidden: {sReason}")]
    Forbidden { sReason: &'static str },
    #[error(transparent)]
    Infrastructure(#[from] AppError),
}

pub type TyTopicModerationResult<T> = std::result::Result<T, EnTopicModerationServiceError>;

impl EnTopicModerationServiceError {
    #[cfg(test)]
    pub fn optForbiddenReason(&self) -> Option<&'static str> {
        match self {
            Self::Forbidden { sReason } => Some(sReason),
            Self::NotFound | Self::Infrastructure(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StPreparedUncommit {
    pub stTopic: StTopicModerationSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StUncommitOutcome {
    pub sMessage: &'static str,
    pub sCanonicalUrl: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StPreparedMove {
    pub stTopic: StTopicModerationSnapshot,
    pub vecGroups: Vec<StTopicMoveGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StMoveOutcome {
    pub sRedirectUrl: String,
    pub bDatabaseMoveRequested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StResolveOutcome {
    pub sRedirectUrl: String,
    pub bResolved: bool,
}

#[derive(Debug, Clone)]
pub struct CTopicModerationService<R, Q>
where
    R: TrTopicModerationRepository,
    Q: TrTopicReindexQueue,
{
    oRepository: R,
    oReindexQueue: Q,
}

impl<R, Q> CTopicModerationService<R, Q>
where
    R: TrTopicModerationRepository,
    Q: TrTopicReindexQueue,
{
    pub fn new(oRepository: R, oReindexQueue: Q) -> Self {
        Self {
            oRepository,
            oReindexQueue,
        }
    }

    pub async fn stPrepareUncommit(
        &self,
        optActor: Option<StTopicModerationActor<'_>>,
        iTopicId: i32,
    ) -> TyTopicModerationResult<StPreparedUncommit> {
        stRequireModerator(optActor)?;
        let stTopic = self.stSnapshot(iTopicId).await?;
        vRestriction(optUncommitRestriction(&stTopic))?;
        Ok(StPreparedUncommit { stTopic })
    }

    pub async fn stUncommit(
        &self,
        optActor: Option<StTopicModerationActor<'_>>,
        iTopicId: i32,
    ) -> TyTopicModerationResult<StUncommitOutcome> {
        let stActor = stRequireModerator(optActor)?;
        let stTopic = self.stSnapshot(iTopicId).await?;
        vRestriction(optUncommitRestriction(&stTopic))?;

        // TopicDao.uncommit commits before SearchQueueSender.updateMessage.
        // A queue error must therefore be surfaced without rolling back the
        // already accepted PostgreSQL mutation.
        self.oRepository.vUncommit(iTopicId).await?;
        self.oReindexQueue.vUpdateMessage(iTopicId, true).await?;
        tracing::info!(
            topic_id = iTopicId,
            moderator = stActor.sNick,
            "topic confirmation cancelled"
        );

        Ok(StUncommitOutcome {
            sMessage: "Подтверждение отменено",
            sCanonicalUrl: stTopic.sCanonicalUrl(),
        })
    }

    pub async fn stPrepareMove(
        &self,
        optActor: Option<StTopicModerationActor<'_>>,
        iTopicId: i32,
        enForm: EnTopicMoveForm,
    ) -> TyTopicModerationResult<StPreparedMove> {
        stRequireModerator(optActor)?;
        let stTopic = self.stSnapshot(iTopicId).await?;
        // Unlike POST, both Java move forms deliberately display deleted
        // topics. The state check must not be shared with submission.
        let enScope = enMoveGroupScope(enForm, &stTopic);
        let vecGroups = self.oRepository.vecMoveGroups(enScope).await?;
        Ok(StPreparedMove { stTopic, vecGroups })
    }

    pub async fn stMove(
        &self,
        optActor: Option<StTopicModerationActor<'_>>,
        iTopicId: i32,
        iTargetGroupId: i32,
    ) -> TyTopicModerationResult<StMoveOutcome> {
        let stActor = stRequireModerator(optActor)?;
        let stTopic = self.stSnapshot(iTopicId).await?;
        vRestriction(optMoveRestriction(&stTopic))?;

        // GroupService.getGroup is called before the stale group comparison;
        // consequently even a same-group request must resolve an existing
        // target group.
        let stTarget = self
            .oRepository
            .optMoveGroup(iTargetGroupId)
            .await?
            .ok_or_else(|| {
                EnTopicModerationServiceError::Infrastructure(AppError::Anyhow(anyhow::anyhow!(
                    "move target group {iTargetGroupId} does not exist"
                )))
            })?;

        let bDatabaseMoveRequested = stTopic.iGroupId != stTarget.iId;
        if bDatabaseMoveRequested {
            self.oRepository
                .vMove(StMoveTopicCommand {
                    iTopicId,
                    iTargetGroupId,
                    bTargetLinksAllowed: stTarget.bLinksAllowed,
                    optOriginalUrl: stTopic.optUrl.clone(),
                    optOriginalLinkText: stTopic.optLinkText.clone(),
                    sOriginalGroupUrlName: stTopic.sGroupUrlName.clone(),
                    sModeratorNick: stActor.sNick.to_owned(),
                })
                .await?;
            tracing::info!(
                topic_id = iTopicId,
                moderator = stActor.sNick,
                old_group = stTopic.sGroupUrlName,
                new_group = stTarget.sTitle,
                "topic moved"
            );
        }

        // The Java controller sends this for both a real move and its
        // controller-level no-op branch, after the transaction when present.
        self.oReindexQueue.vUpdateMessage(iTopicId, true).await?;

        Ok(StMoveOutcome {
            sRedirectUrl: stTopic.sForceLastModUrl(),
            bDatabaseMoveRequested,
        })
    }

    pub async fn stResolve(
        &self,
        optActor: Option<StTopicModerationActor<'_>>,
        iTopicId: i32,
        sResolve: &str,
    ) -> TyTopicModerationResult<StResolveOutcome> {
        let stActor = stRequireAuthorized(optActor)?;
        let stTopic = self.stSnapshot(iTopicId).await?;
        vRestriction(optResolveRestriction(&stTopic, stActor))?;
        let bResolved = bResolveValue(sResolve);

        // No equality shortcut: TopicDao.resolveMessage always advances the
        // stored lastmod by exactly one second, even for the same value.
        self.oRepository.vResolve(iTopicId, bResolved).await?;

        Ok(StResolveOutcome {
            sRedirectUrl: stTopic.sForceLastModUrl(),
            bResolved,
        })
    }

    async fn stSnapshot(
        &self,
        iTopicId: i32,
    ) -> TyTopicModerationResult<StTopicModerationSnapshot> {
        self.oRepository
            .optSnapshot(iTopicId)
            .await?
            .ok_or(EnTopicModerationServiceError::NotFound)
    }
}

fn stRequireAuthorized(
    optActor: Option<StTopicModerationActor<'_>>,
) -> TyTopicModerationResult<StTopicModerationActor<'_>> {
    optActor.ok_or(EnTopicModerationServiceError::Forbidden {
        sReason: "Not authorized",
    })
}

fn stRequireModerator(
    optActor: Option<StTopicModerationActor<'_>>,
) -> TyTopicModerationResult<StTopicModerationActor<'_>> {
    let stActor = optActor.ok_or(EnTopicModerationServiceError::Forbidden {
        sReason: "Not moderator",
    })?;
    if !stActor.bModerator {
        return Err(EnTopicModerationServiceError::Forbidden {
            sReason: "Not moderator",
        });
    }
    Ok(stActor)
}

fn vRestriction(
    optRestriction: Option<EnTopicModerationRestriction>,
) -> TyTopicModerationResult<()> {
    match optRestriction {
        Some(enRestriction) => Err(EnTopicModerationServiceError::Forbidden {
            sReason: enRestriction.sReason(),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::{
        domain::topic::{
            moderation::{EnTopicMoveGroupScope, TrTopicModerationRepository},
            options::TrTopicReindexQueue,
        },
        error::Result,
    };

    #[derive(Clone)]
    struct CRepository {
        optTopic: Option<StTopicModerationSnapshot>,
        optTarget: Option<StTopicMoveGroup>,
        vecGroups: Vec<StTopicMoveGroup>,
        vecEvents: Arc<Mutex<Vec<String>>>,
        vecCommands: Arc<Mutex<Vec<StMoveTopicCommand>>>,
        vecScopes: Arc<Mutex<Vec<EnTopicMoveGroupScope>>>,
    }

    #[async_trait]
    impl TrTopicModerationRepository for CRepository {
        async fn optSnapshot(&self, iTopicId: i32) -> Result<Option<StTopicModerationSnapshot>> {
            self.vecEvents.lock().unwrap().push("snapshot".into());
            Ok(self
                .optTopic
                .as_ref()
                .filter(|stTopic| stTopic.iTopicId == iTopicId)
                .cloned())
        }

        async fn optMoveGroup(&self, iGroupId: i32) -> Result<Option<StTopicMoveGroup>> {
            self.vecEvents.lock().unwrap().push("target".into());
            Ok(self
                .optTarget
                .as_ref()
                .filter(|stGroup| stGroup.iId == iGroupId)
                .cloned())
        }

        async fn vecMoveGroups(
            &self,
            enScope: EnTopicMoveGroupScope,
        ) -> Result<Vec<StTopicMoveGroup>> {
            self.vecEvents.lock().unwrap().push("groups".into());
            self.vecScopes.lock().unwrap().push(enScope);
            Ok(self.vecGroups.clone())
        }

        async fn vUncommit(&self, _iTopicId: i32) -> Result<()> {
            self.vecEvents.lock().unwrap().push("uncommit".into());
            Ok(())
        }

        async fn vMove(&self, stCommand: StMoveTopicCommand) -> Result<()> {
            self.vecEvents.lock().unwrap().push("move".into());
            self.vecCommands.lock().unwrap().push(stCommand);
            Ok(())
        }

        async fn vResolve(&self, _iTopicId: i32, bResolved: bool) -> Result<()> {
            self.vecEvents
                .lock()
                .unwrap()
                .push(format!("resolve:{bResolved}"));
            Ok(())
        }
    }

    #[derive(Clone)]
    struct CQueue {
        vecEvents: Arc<Mutex<Vec<String>>>,
        bFail: bool,
    }

    #[async_trait]
    impl TrTopicReindexQueue for CQueue {
        async fn vUpdateMessage(&self, iTopicId: i32, bWithComments: bool) -> Result<()> {
            self.vecEvents
                .lock()
                .unwrap()
                .push(format!("queue:{iTopicId}:{bWithComments}"));
            if self.bFail {
                Err(anyhow::anyhow!("queue unavailable").into())
            } else {
                Ok(())
            }
        }
    }

    fn stTopic() -> StTopicModerationSnapshot {
        StTopicModerationSnapshot {
            iTopicId: 42,
            iAuthorId: 7,
            sAuthorNick: "author".into(),
            iAuthorScore: 300,
            bAuthorBlocked: false,
            sStoredTitle: "title".into(),
            sMessage: "body".into(),
            sMarkup: "MARKDOWN".into(),
            optUrl: Some("https://example.test".into()),
            optLinkText: Some("details".into()),
            iGroupId: 10,
            sGroupTitle: "Old".into(),
            sGroupUrlName: "old".into(),
            iSectionId: 1,
            sSectionPrefix: "news".into(),
            bSectionPremoderated: true,
            bSectionPollAllowed: false,
            bLinksAllowed: true,
            bGroupResolvable: true,
            bDeleted: false,
            bCommitted: true,
            bSticky: false,
            bExpired: false,
            dtLastMod: Utc.with_ymd_and_hms(2026, 8, 15, 10, 11, 12).unwrap(),
        }
    }

    fn stGroup(iId: i32, bLinksAllowed: bool) -> StTopicMoveGroup {
        StTopicMoveGroup {
            iId,
            sTitle: format!("group-{iId}"),
            iSectionId: 2,
            sSectionTitle: "Форум".into(),
            bLinksAllowed,
            bResolvable: true,
        }
    }

    fn stActor(iUserId: i32, bModerator: bool) -> StTopicModerationActor<'static> {
        StTopicModerationActor {
            iUserId,
            sNick: "moderator",
            bModerator,
        }
    }

    type TyFixture = (
        CTopicModerationService<CRepository, CQueue>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Vec<StMoveTopicCommand>>>,
        Arc<Mutex<Vec<EnTopicMoveGroupScope>>>,
    );

    fn stFixture(
        stTopic: StTopicModerationSnapshot,
        stTarget: StTopicMoveGroup,
        bQueueFail: bool,
    ) -> TyFixture {
        let vecEvents = Arc::new(Mutex::new(Vec::new()));
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let vecScopes = Arc::new(Mutex::new(Vec::new()));
        let oRepository = CRepository {
            optTopic: Some(stTopic),
            optTarget: Some(stTarget.clone()),
            vecGroups: vec![stTarget],
            vecEvents: vecEvents.clone(),
            vecCommands: vecCommands.clone(),
            vecScopes: vecScopes.clone(),
        };
        let oQueue = CQueue {
            vecEvents: vecEvents.clone(),
            bFail: bQueueFail,
        };
        (
            CTopicModerationService::new(oRepository, oQueue),
            vecEvents,
            vecCommands,
            vecScopes,
        )
    }

    #[tokio::test]
    async fn moderator_gate_runs_before_topic_lookup() {
        let (cService, vecEvents, _, _) = stFixture(stTopic(), stGroup(11, true), false);
        let stError = cService
            .stPrepareUncommit(Some(stActor(7, false)), 42)
            .await
            .unwrap_err();
        assert_eq!(stError.optForbiddenReason(), Some("Not moderator"));
        assert!(vecEvents.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn uncommit_commits_before_queue_and_queue_failure_stays_visible() {
        let (cService, vecEvents, _, _) = stFixture(stTopic(), stGroup(11, true), true);
        let stError = cService
            .stUncommit(Some(stActor(99, true)), 42)
            .await
            .unwrap_err();
        assert!(matches!(
            stError,
            EnTopicModerationServiceError::Infrastructure(_)
        ));
        assert_eq!(
            &*vecEvents.lock().unwrap(),
            &["snapshot", "uncommit", "queue:42:true"]
        );
    }

    #[tokio::test]
    async fn same_group_move_skips_database_but_always_reindexes() {
        let (cService, vecEvents, vecCommands, _) = stFixture(stTopic(), stGroup(10, false), false);
        let stOutcome = cService
            .stMove(Some(stActor(99, true)), 42, 10)
            .await
            .unwrap();
        assert!(!stOutcome.bDatabaseMoveRequested);
        assert!(vecCommands.lock().unwrap().is_empty());
        assert_eq!(
            &*vecEvents.lock().unwrap(),
            &["snapshot", "target", "queue:42:true"]
        );
        assert_eq!(stOutcome.sRedirectUrl, "/news/old/42?lastmod=1786788672000");
    }

    #[tokio::test]
    async fn disallowed_link_target_passes_stale_move_info_inputs_to_the_transaction() {
        let (cService, vecEvents, vecCommands, _) = stFixture(stTopic(), stGroup(11, false), false);
        cService
            .stMove(Some(stActor(99, true)), 42, 11)
            .await
            .unwrap();
        assert_eq!(
            &*vecEvents.lock().unwrap(),
            &["snapshot", "target", "move", "queue:42:true"]
        );
        let vecCommands = vecCommands.lock().unwrap();
        assert_eq!(vecCommands.len(), 1);
        assert!(!vecCommands[0].bTargetLinksAllowed);
        assert_eq!(
            vecCommands[0].optOriginalUrl.as_deref(),
            Some("https://example.test")
        );
        assert_eq!(
            vecCommands[0].optOriginalLinkText.as_deref(),
            Some("details")
        );
        assert_eq!(vecCommands[0].sOriginalGroupUrlName, "old");
        assert_eq!(vecCommands[0].sModeratorNick, "moderator");
    }

    #[tokio::test]
    async fn move_form_does_not_reject_deleted_topic_and_uses_mtn_scope() {
        let mut stDeleted = stTopic();
        stDeleted.bDeleted = true;
        let (cService, vecEvents, _, vecScopes) = stFixture(stDeleted, stGroup(11, true), false);
        let stPrepared = cService
            .stPrepareMove(
                Some(stActor(99, true)),
                42,
                EnTopicMoveForm::PremoderatedCompanion,
            )
            .await
            .unwrap();
        assert_eq!(stPrepared.vecGroups.len(), 1);
        assert_eq!(
            &*vecScopes.lock().unwrap(),
            &[EnTopicMoveGroupScope::PremoderatedNonPoll]
        );
        assert_eq!(&*vecEvents.lock().unwrap(), &["snapshot", "groups"]);
    }

    #[tokio::test]
    async fn resolve_writes_every_present_value_and_never_queues() {
        let (cService, vecEvents, _, _) = stFixture(stTopic(), stGroup(11, true), false);
        let stOutcome = cService
            .stResolve(Some(stActor(7, false)), 42, "arbitrary")
            .await
            .unwrap();
        assert!(!stOutcome.bResolved);
        assert_eq!(stOutcome.sRedirectUrl, "/news/old/42?lastmod=1786788672000");
        assert_eq!(&*vecEvents.lock().unwrap(), &["snapshot", "resolve:false"]);
    }
}
