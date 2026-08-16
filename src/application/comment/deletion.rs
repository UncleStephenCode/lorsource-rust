use chrono::Utc;

use crate::{
    domain::comment::deletion::{
        StCommentDeleteActor, StCommentDeleteMutation, StCommentDeletePreview,
        StCommentDeleteTarget, StDeleteCommentCommand, TrCommentDeletionRepository,
        TrCommentReindexQueue,
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnCommentDeletionRestriction {
    AlreadyDeleted,
    TopicDeleted,
    CannotDelete,
    CannotUndelete,
    HasReplies,
    InvalidPenalty,
}

impl EnCommentDeletionRestriction {
    pub const fn sMessage(self) -> &'static str {
        match self {
            Self::AlreadyDeleted => "комментарий уже удален",
            Self::TopicDeleted => "тема удалена",
            Self::CannotDelete => "комментарий нельзя удалить",
            Self::CannotUndelete => "этот комментарий нельзя восстановить",
            Self::HasReplies => "Нельзя удалить комментарий с ответами",
            Self::InvalidPenalty => "Неправильный формат параметра ``неправильный размер штрафа''",
        }
    }
}

#[derive(Debug)]
pub enum EnCommentDeletionError {
    Restricted(EnCommentDeletionRestriction),
    Application(AppError),
}

impl From<AppError> for EnCommentDeletionError {
    fn from(stError: AppError) -> Self {
        Self::Application(stError)
    }
}

type TyCommentDeletionResult<T> = std::result::Result<T, EnCommentDeletionError>;

#[derive(Debug, Clone)]
pub struct StCommentDeleteFormData {
    pub stTarget: StCommentDeleteTarget,
    pub vecPreview: Vec<StCommentDeletePreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StCommentDeleteOutcome {
    pub stTarget: StCommentDeleteTarget,
    pub vecDeletedIds: Vec<i32>,
    pub optNextCommentId: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct CCommentDeletionService<R, Q>
where
    R: TrCommentDeletionRepository,
    Q: TrCommentReindexQueue,
{
    oRepository: R,
    oReindexQueue: Q,
}

impl<R, Q> CCommentDeletionService<R, Q>
where
    R: TrCommentDeletionRepository,
    Q: TrCommentReindexQueue,
{
    pub fn new(oRepository: R, oReindexQueue: Q) -> Self {
        Self {
            oRepository,
            oReindexQueue,
        }
    }

    /// Read-only `CommentPrepareService.prepareCommentOnly` source model.
    ///
    /// The existing preview repository already owns the complete comment
    /// projection (delete/edit metadata, profile-dependent remark source,
    /// moderator IP/UA and userpic inputs).  Exposing one root comment here
    /// lets browser routes render that model without duplicating its SQL or
    /// coupling HTTP handlers directly to a PostgreSQL implementation.
    pub async fn optPrepareCommentOnly(
        &self,
        iCommentId: i32,
        iViewerId: i32,
    ) -> Result<Option<(StCommentDeleteTarget, StCommentDeletePreview)>> {
        let Some(stTarget) = self.oRepository.optFindTarget(iCommentId).await? else {
            return Ok(None);
        };
        let optComment = self
            .oRepository
            .vecDeletePreview(iCommentId, iViewerId)
            .await?
            .into_iter()
            .find(|stComment| stComment.iCommentId == iCommentId);
        Ok(optComment.map(|stComment| (stTarget, stComment)))
    }

    pub async fn stDeleteForm(
        &self,
        stActor: StCommentDeleteActor,
        iCommentId: i32,
    ) -> TyCommentDeletionResult<StCommentDeleteFormData> {
        let stTarget = self.stTarget(iCommentId).await?;
        if stTarget.bDeleted {
            return Err(EnCommentDeletionError::Restricted(
                EnCommentDeletionRestriction::AlreadyDeleted,
            ));
        }
        if stTarget.bTopicDeleted {
            return Err(EnCommentDeletionError::Restricted(
                EnCommentDeletionRestriction::TopicDeleted,
            ));
        }
        if !stTarget.bCanDelete(stActor, Utc::now()) {
            return Err(EnCommentDeletionError::Restricted(
                EnCommentDeletionRestriction::CannotDelete,
            ));
        }
        let vecPreview = self
            .oRepository
            .vecDeletePreview(iCommentId, stActor.iUserId)
            .await?;
        Ok(StCommentDeleteFormData {
            stTarget,
            vecPreview,
        })
    }

    pub async fn stDelete(
        &self,
        stActor: StCommentDeleteActor,
        mut stCommand: StDeleteCommentCommand,
    ) -> TyCommentDeletionResult<StCommentDeleteOutcome> {
        if !(0..=20).contains(&stCommand.iPenalty) {
            return Err(EnCommentDeletionError::Restricted(
                EnCommentDeletionRestriction::InvalidPenalty,
            ));
        }
        let stTarget = self.stTarget(stCommand.iCommentId).await?;
        if stTarget.bDeleted {
            return Err(EnCommentDeletionError::Restricted(
                EnCommentDeletionRestriction::AlreadyDeleted,
            ));
        }
        if !stTarget.bCanDelete(stActor, Utc::now()) {
            return Err(EnCommentDeletionError::Restricted(
                EnCommentDeletionRestriction::CannotDelete,
            ));
        }
        if !stActor.bModerator {
            stCommand.bDeleteReplies = false;
            stCommand.iPenalty = 0;
        } else if stActor.iUserId == stTarget.iAuthorId {
            stCommand.iPenalty = 0;
        }
        let StCommentDeleteMutation {
            vecDeletedIds,
            optNextCommentId,
        } = match self
            .oRepository
            .stDelete(stActor, &stTarget, &stCommand)
            .await
        {
            Err(AppError::Forbidden) if !stActor.bModerator => {
                return Err(EnCommentDeletionError::Restricted(
                    EnCommentDeletionRestriction::HasReplies,
                ));
            }
            Err(stError) => return Err(stError.into()),
            Ok(stMutation) => stMutation,
        };

        // DeleteCommentController invokes SearchQueueSender after localTx,
        // even when a concurrent delete made the resulting list empty.
        self.oReindexQueue.vUpdateComments(&vecDeletedIds).await?;
        Ok(StCommentDeleteOutcome {
            stTarget,
            vecDeletedIds,
            optNextCommentId,
        })
    }

    pub async fn stUndeleteForm(
        &self,
        stActor: StCommentDeleteActor,
        iCommentId: i32,
    ) -> TyCommentDeletionResult<StCommentDeleteFormData> {
        let stTarget = self.stTarget(iCommentId).await?;
        if !stTarget.bCanUndelete(stActor) {
            return Err(EnCommentDeletionError::Restricted(
                EnCommentDeletionRestriction::CannotUndelete,
            ));
        }
        let vecPreview = self
            .oRepository
            .vecUndeletePreview(iCommentId, stActor.iUserId)
            .await?;
        Ok(StCommentDeleteFormData {
            stTarget,
            vecPreview,
        })
    }

    pub async fn vUndelete(
        &self,
        stActor: StCommentDeleteActor,
        iCommentId: i32,
    ) -> TyCommentDeletionResult<StCommentDeleteTarget> {
        let stTarget = self.stTarget(iCommentId).await?;
        if !stTarget.bCanUndelete(stActor) {
            return Err(EnCommentDeletionError::Restricted(
                EnCommentDeletionRestriction::CannotUndelete,
            ));
        }
        self.oRepository.vUndelete(&stTarget).await?;
        self.oReindexQueue.vUpdateComments(&[iCommentId]).await?;
        Ok(stTarget)
    }

    async fn stTarget(&self, iCommentId: i32) -> TyCommentDeletionResult<StCommentDeleteTarget> {
        Ok(self
            .oRepository
            .optFindTarget(iCommentId)
            .await?
            .ok_or(AppError::NotFound)?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Duration;

    use super::*;

    #[derive(Clone)]
    struct CTestRepository {
        stTarget: StCommentDeleteTarget,
        stMutation: StCommentDeleteMutation,
        vecCommands: Arc<Mutex<Vec<StDeleteCommentCommand>>>,
        bReplyRace: bool,
    }

    #[async_trait]
    impl TrCommentDeletionRepository for CTestRepository {
        async fn optFindTarget(
            &self,
            _iCommentId: i32,
        ) -> crate::error::Result<Option<StCommentDeleteTarget>> {
            Ok(Some(self.stTarget.clone()))
        }

        async fn vecDeletePreview(
            &self,
            _iCommentId: i32,
            _iViewerId: i32,
        ) -> crate::error::Result<Vec<StCommentDeletePreview>> {
            Ok(Vec::new())
        }

        async fn vecUndeletePreview(
            &self,
            _iCommentId: i32,
            _iViewerId: i32,
        ) -> crate::error::Result<Vec<StCommentDeletePreview>> {
            Ok(Vec::new())
        }

        async fn stDelete(
            &self,
            _stActor: StCommentDeleteActor,
            _stTarget: &StCommentDeleteTarget,
            stCommand: &StDeleteCommentCommand,
        ) -> crate::error::Result<StCommentDeleteMutation> {
            self.vecCommands
                .lock()
                .expect("command lock")
                .push(stCommand.clone());
            if self.bReplyRace {
                return Err(AppError::Forbidden);
            }
            Ok(self.stMutation.clone())
        }

        async fn vUndelete(&self, _stTarget: &StCommentDeleteTarget) -> crate::error::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct CTestQueue {
        vecBatches: Arc<Mutex<Vec<Vec<i32>>>>,
        bFail: bool,
    }

    #[async_trait]
    impl TrCommentReindexQueue for CTestQueue {
        async fn vUpdateComments(&self, vecCommentIds: &[i32]) -> crate::error::Result<()> {
            self.vecBatches
                .lock()
                .expect("batch lock")
                .push(vecCommentIds.to_vec());
            if self.bFail {
                Err(AppError::Anyhow(anyhow::anyhow!("queue failed")))
            } else {
                Ok(())
            }
        }
    }

    fn stTarget() -> StCommentDeleteTarget {
        StCommentDeleteTarget {
            iCommentId: 20,
            iTopicId: 10,
            iAuthorId: 7,
            sAuthorNick: "author".into(),
            iAuthorScore: 100,
            bDeleted: false,
            bTopicDeleted: false,
            bTopicExpired: false,
            bTopicDraft: false,
            bCommentsHidden: false,
            bHasReplies: false,
            dtPostdate: Utc::now() - Duration::minutes(10),
            optDeletedBy: None,
            sPostIp: "127.0.0.1".into(),
            iUserAgentId: 1,
            sCanonicalTopicUrl: "/forum/general/10".into(),
        }
    }

    fn stRepository(
        stTarget: StCommentDeleteTarget,
        vecCommands: Arc<Mutex<Vec<StDeleteCommentCommand>>>,
    ) -> CTestRepository {
        CTestRepository {
            stTarget,
            stMutation: StCommentDeleteMutation {
                vecDeletedIds: vec![20],
                optNextCommentId: Some(21),
            },
            vecCommands,
            bReplyRace: false,
        }
    }

    #[test]
    fn prepare_comment_only_keeps_the_complete_repository_projection_layered() {
        let sSource = include_str!("deletion.rs");
        let sMethod = sSource
            .split_once("pub async fn optPrepareCommentOnly(")
            .expect("prepare-comment method")
            .1
            .split_once("pub async fn stDeleteForm(")
            .expect("end of prepare-comment method")
            .0;
        assert!(sMethod.contains("self.oRepository.optFindTarget(iCommentId)"));
        assert!(sMethod.contains(".vecDeletePreview(iCommentId, iViewerId)"));
        assert!(sMethod.contains("stComment.iCommentId == iCommentId"));
    }

    #[tokio::test]
    async fn plain_author_cannot_smuggle_penalty_or_cascade_fields() {
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let cService = CCommentDeletionService::new(
            stRepository(stTarget(), Arc::clone(&vecCommands)),
            CTestQueue::default(),
        );
        cService
            .stDelete(
                StCommentDeleteActor {
                    iUserId: 7,
                    bModerator: false,
                },
                StDeleteCommentCommand {
                    iCommentId: 20,
                    sReason: "self-delete".into(),
                    iPenalty: 20,
                    bDeleteReplies: true,
                },
            )
            .await
            .expect("delete");
        let vecCommands = vecCommands.lock().expect("command lock");
        assert_eq!(vecCommands[0].iPenalty, 0);
        assert!(!vecCommands[0].bDeleteReplies);
    }

    #[tokio::test]
    async fn concurrent_noop_still_sends_one_empty_batch() {
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let vecBatches = Arc::new(Mutex::new(Vec::new()));
        let mut cRepository = stRepository(stTarget(), vecCommands);
        cRepository.stMutation = StCommentDeleteMutation {
            vecDeletedIds: Vec::new(),
            optNextCommentId: Some(21),
        };
        let cService = CCommentDeletionService::new(
            cRepository,
            CTestQueue {
                vecBatches: Arc::clone(&vecBatches),
                bFail: false,
            },
        );
        let stOutcome = cService
            .stDelete(
                StCommentDeleteActor {
                    iUserId: 9,
                    bModerator: true,
                },
                StDeleteCommentCommand {
                    iCommentId: 20,
                    sReason: "race".into(),
                    iPenalty: 0,
                    bDeleteReplies: false,
                },
            )
            .await
            .expect("race response");
        assert!(stOutcome.vecDeletedIds.is_empty());
        assert_eq!(
            *vecBatches.lock().expect("batch lock"),
            vec![Vec::<i32>::new()]
        );
    }

    #[tokio::test]
    async fn author_reply_race_has_the_controller_specific_restriction() {
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let mut cRepository = stRepository(stTarget(), vecCommands);
        cRepository.bReplyRace = true;
        let stError = CCommentDeletionService::new(cRepository, CTestQueue::default())
            .stDelete(
                StCommentDeleteActor {
                    iUserId: 7,
                    bModerator: false,
                },
                StDeleteCommentCommand {
                    iCommentId: 20,
                    sReason: "self-delete".into(),
                    iPenalty: 0,
                    bDeleteReplies: false,
                },
            )
            .await
            .expect_err("reply race");
        assert!(matches!(
            stError,
            EnCommentDeletionError::Restricted(EnCommentDeletionRestriction::HasReplies)
        ));
    }

    #[test]
    fn local_controller_restrictions_keep_exact_java_headers() {
        assert_eq!(
            EnCommentDeletionRestriction::AlreadyDeleted.sMessage(),
            "комментарий уже удален"
        );
        assert_eq!(
            EnCommentDeletionRestriction::TopicDeleted.sMessage(),
            "тема удалена"
        );
        assert_eq!(
            EnCommentDeletionRestriction::CannotUndelete.sMessage(),
            "этот комментарий нельзя восстановить"
        );
    }
}
