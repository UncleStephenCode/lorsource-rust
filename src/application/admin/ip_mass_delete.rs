use crate::{
    domain::{
        admin::ip_mass_delete::{
            StIpMassDeleteActor, StIpMassDeleteCommand, StIpMassDeleteResult,
            TrIpMassDeleteRepository,
        },
        comment::deletion::TrCommentReindexQueue,
        topic::options::TrTopicReindexQueue,
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone)]
pub struct CIpMassDeleteService<R, Q>
where
    R: TrIpMassDeleteRepository,
    Q: TrTopicReindexQueue + TrCommentReindexQueue,
{
    oRepository: R,
    oReindexQueue: Q,
}

impl<R, Q> CIpMassDeleteService<R, Q>
where
    R: TrIpMassDeleteRepository,
    Q: TrTopicReindexQueue + TrCommentReindexQueue,
{
    pub fn new(oRepository: R, oReindexQueue: Q) -> Self {
        Self {
            oRepository,
            oReindexQueue,
        }
    }

    pub async fn stExecute(
        &self,
        stActor: StIpMassDeleteActor,
        stCommand: StIpMassDeleteCommand,
    ) -> Result<StIpMassDeleteResult> {
        if !stActor.bModerator {
            return Err(AppError::Forbidden);
        }

        // DelIPController calls IpBlockDao before DeleteService. These must
        // remain two commits: a later delete/queue failure does not undo the
        // moderator's requested IP block.
        if let Some(stBan) = &stCommand.optBan {
            self.oRepository.vBlockIp(stActor.iUserId, stBan).await?;
        }

        let stResult = self
            .oRepository
            .stDeleteByIp(stActor.iUserId, &stCommand)
            .await?;

        // Java queues committed topic deletions one by one, then sends one
        // comment batch (including an empty batch). Queue errors are visible
        // after the database commit and stop later queue calls.
        for iTopicId in &stResult.vecDeletedTopicIds {
            self.oReindexQueue.vUpdateMessage(*iTopicId, true).await?;
        }
        self.oReindexQueue
            .vUpdateComments(&stResult.vecDeletedCommentIds)
            .await?;

        Ok(stResult)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::domain::admin::ip_mass_delete::{
        StIpBanCommand, StIpMassDeleteCommand, TrIpMassDeleteRepository,
    };

    #[derive(Clone)]
    struct CRepository {
        vecEvents: Arc<Mutex<Vec<String>>>,
        stResult: StIpMassDeleteResult,
        bDeleteFails: bool,
    }

    #[async_trait]
    impl TrIpMassDeleteRepository for CRepository {
        async fn vBlockIp(&self, _iModeratorId: i32, _stCommand: &StIpBanCommand) -> Result<()> {
            self.vecEvents.lock().unwrap().push("ban-commit".to_owned());
            Ok(())
        }

        async fn stDeleteByIp(
            &self,
            _iModeratorId: i32,
            _stCommand: &StIpMassDeleteCommand,
        ) -> Result<StIpMassDeleteResult> {
            if self.bDeleteFails {
                self.vecEvents
                    .lock()
                    .unwrap()
                    .push("delete-rollback".to_owned());
                return Err(AppError::Anyhow(anyhow::anyhow!("delete failed")));
            }
            self.vecEvents
                .lock()
                .unwrap()
                .push("delete-commit".to_owned());
            Ok(self.stResult.clone())
        }
    }

    #[derive(Clone)]
    struct CQueue {
        vecEvents: Arc<Mutex<Vec<String>>>,
        optFailTopic: Option<i32>,
    }

    #[async_trait]
    impl TrTopicReindexQueue for CQueue {
        async fn vUpdateMessage(&self, iTopicId: i32, bWithComments: bool) -> Result<()> {
            self.vecEvents
                .lock()
                .unwrap()
                .push(format!("topic:{iTopicId}:{bWithComments}"));
            if self.optFailTopic == Some(iTopicId) {
                return Err(AppError::Anyhow(anyhow::anyhow!("queue failed")));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl TrCommentReindexQueue for CQueue {
        async fn vUpdateComments(&self, vecCommentIds: &[i32]) -> Result<()> {
            self.vecEvents
                .lock()
                .unwrap()
                .push(format!("comments:{vecCommentIds:?}"));
            Ok(())
        }
    }

    fn stCommand(bBan: bool) -> StIpMassDeleteCommand {
        let dtCutoff = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        StIpMassDeleteCommand {
            sIp: "203.0.113.7".to_owned(),
            dtCutoff,
            sReason: "spam".to_owned(),
            optBan: bBan.then(|| StIpBanCommand {
                sIp: "203.0.113.7".to_owned(),
                sReason: "spam".to_owned(),
                optBanUntil: None,
                bAllowPosting: false,
                bCaptchaRequired: false,
            }),
        }
    }

    fn stService(
        stResult: StIpMassDeleteResult,
        bDeleteFails: bool,
        optFailTopic: Option<i32>,
    ) -> (
        CIpMassDeleteService<CRepository, CQueue>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let vecEvents = Arc::new(Mutex::new(Vec::new()));
        (
            CIpMassDeleteService::new(
                CRepository {
                    vecEvents: vecEvents.clone(),
                    stResult,
                    bDeleteFails,
                },
                CQueue {
                    vecEvents: vecEvents.clone(),
                    optFailTopic,
                },
            ),
            vecEvents,
        )
    }

    #[tokio::test]
    async fn policy_rejects_non_moderator_before_any_mutation() {
        let (cService, vecEvents) = stService(StIpMassDeleteResult::default(), false, None);
        let stError = cService
            .stExecute(
                StIpMassDeleteActor {
                    iUserId: 10,
                    bModerator: false,
                },
                stCommand(true),
            )
            .await
            .unwrap_err();

        assert!(matches!(stError, AppError::Forbidden));
        assert!(vecEvents.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ban_delete_commit_and_fallible_queue_order_match_controller() {
        let stResult = StIpMassDeleteResult {
            vecDeletedTopicIds: vec![41, 42],
            vecDeletedCommentIds: vec![51, 52],
            vecSkippedCommentIds: vec![50],
        };
        let (cService, vecEvents) = stService(stResult.clone(), false, None);
        let stActual = cService
            .stExecute(
                StIpMassDeleteActor {
                    iUserId: 9,
                    bModerator: true,
                },
                stCommand(true),
            )
            .await
            .unwrap();

        assert_eq!(stActual, stResult);
        assert_eq!(
            *vecEvents.lock().unwrap(),
            [
                "ban-commit",
                "delete-commit",
                "topic:41:true",
                "topic:42:true",
                "comments:[51, 52]",
            ]
        );
    }

    #[tokio::test]
    async fn delete_rollback_does_not_undo_the_separate_ban() {
        let (cService, vecEvents) = stService(StIpMassDeleteResult::default(), true, None);
        assert!(
            cService
                .stExecute(
                    StIpMassDeleteActor {
                        iUserId: 9,
                        bModerator: true,
                    },
                    stCommand(true),
                )
                .await
                .is_err()
        );
        assert_eq!(
            *vecEvents.lock().unwrap(),
            ["ban-commit", "delete-rollback"]
        );
    }

    #[tokio::test]
    async fn topic_queue_failure_is_post_commit_and_stops_the_comment_batch() {
        let stResult = StIpMassDeleteResult {
            vecDeletedTopicIds: vec![41, 42],
            vecDeletedCommentIds: vec![51],
            vecSkippedCommentIds: Vec::new(),
        };
        let (cService, vecEvents) = stService(stResult, false, Some(41));
        assert!(
            cService
                .stExecute(
                    StIpMassDeleteActor {
                        iUserId: 9,
                        bModerator: true,
                    },
                    stCommand(false),
                )
                .await
                .is_err()
        );
        assert_eq!(
            *vecEvents.lock().unwrap(),
            ["delete-commit", "topic:41:true"]
        );
    }

    #[tokio::test]
    async fn empty_result_still_sends_the_single_empty_comment_batch() {
        let (cService, vecEvents) = stService(StIpMassDeleteResult::default(), false, None);
        cService
            .stExecute(
                StIpMassDeleteActor {
                    iUserId: 9,
                    bModerator: true,
                },
                stCommand(false),
            )
            .await
            .unwrap();
        assert_eq!(*vecEvents.lock().unwrap(), ["delete-commit", "comments:[]"]);
    }
}
