use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard},
};

use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    domain::realtime::{
        model::{EnRealtimeDelivery, StTopicSubscriptionRequest},
        repository::TrRealtimeRepository,
    },
    error::{AppError, Result},
};

pub type TyRealtimeSessionId = Uuid;

pub struct StRealtimeSessionRegistration {
    pub uuidSessionId: TyRealtimeSessionId,
    pub rxDelivery: mpsc::UnboundedReceiver<EnRealtimeDelivery>,
}

struct StRealtimeSession {
    optUserId: Option<i32>,
    txDelivery: mpsc::UnboundedSender<EnRealtimeDelivery>,
    setTopics: HashSet<i32>,
}

#[derive(Default)]
struct StRealtimeHubData {
    mapSessions: HashMap<TyRealtimeSessionId, StRealtimeSession>,
    mapTopicSessions: HashMap<i32, HashSet<TyRealtimeSessionId>>,
    mapUserSessions: HashMap<i32, HashSet<TyRealtimeSessionId>>,
}

/// In-process equivalent of the original typed Pekko hub. Session mailboxes
/// are unbounded, just like Pekko's default mailbox, and subscriptions are
/// sets so repeated requests for the same topic do not duplicate delivery.
#[derive(Clone, Default)]
struct CRealtimeHub {
    oData: Arc<Mutex<StRealtimeHubData>>,
}

impl CRealtimeHub {
    fn stLock(&self) -> MutexGuard<'_, StRealtimeHubData> {
        self.oData
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn stRegister(&self, optUserId: Option<i32>) -> StRealtimeSessionRegistration {
        let (txDelivery, rxDelivery) = mpsc::unbounded_channel();
        let uuidSessionId = Uuid::new_v4();
        let mut stData = self.stLock();
        stData.mapSessions.insert(
            uuidSessionId,
            StRealtimeSession {
                optUserId,
                txDelivery,
                setTopics: HashSet::new(),
            },
        );
        if let Some(iUserId) = optUserId {
            stData
                .mapUserSessions
                .entry(iUserId)
                .or_default()
                .insert(uuidSessionId);
        }
        StRealtimeSessionRegistration {
            uuidSessionId,
            rxDelivery,
        }
    }

    fn bSubscribe(
        &self,
        uuidSessionId: TyRealtimeSessionId,
        iTopicId: i32,
        vecMissedCommentIds: &[i32],
    ) -> bool {
        let mut stData = self.stLock();
        let Some(stSession) = stData.mapSessions.get_mut(&uuidSessionId) else {
            return false;
        };
        stSession.setTopics.insert(iTopicId);
        let txDelivery = stSession.txDelivery.clone();
        stData
            .mapTopicSessions
            .entry(iTopicId)
            .or_default()
            .insert(uuidSessionId);

        for &iCommentId in vecMissedCommentIds {
            if txDelivery
                .send(EnRealtimeDelivery::Comment(iCommentId))
                .is_err()
            {
                return false;
            }
        }
        true
    }

    fn vUnregister(&self, uuidSessionId: TyRealtimeSessionId) {
        let mut stData = self.stLock();
        let Some(stSession) = stData.mapSessions.remove(&uuidSessionId) else {
            return;
        };

        for iTopicId in stSession.setTopics {
            if let Some(setSessions) = stData.mapTopicSessions.get_mut(&iTopicId) {
                setSessions.remove(&uuidSessionId);
                if setSessions.is_empty() {
                    stData.mapTopicSessions.remove(&iTopicId);
                }
            }
        }
        if let Some(iUserId) = stSession.optUserId
            && let Some(setSessions) = stData.mapUserSessions.get_mut(&iUserId)
        {
            setSessions.remove(&uuidSessionId);
            if setSessions.is_empty() {
                stData.mapUserSessions.remove(&iUserId);
            }
        }
    }

    fn vecTopicSenders(
        &self,
        iTopicId: i32,
    ) -> Vec<(
        TyRealtimeSessionId,
        mpsc::UnboundedSender<EnRealtimeDelivery>,
    )> {
        let stData = self.stLock();
        stData
            .mapTopicSessions
            .get(&iTopicId)
            .into_iter()
            .flatten()
            .filter_map(|uuidSessionId| {
                stData
                    .mapSessions
                    .get(uuidSessionId)
                    .map(|stSession| (*uuidSessionId, stSession.txDelivery.clone()))
            })
            .collect()
    }

    fn vecUserSenders(
        &self,
        setUserIds: &HashSet<i32>,
    ) -> Vec<(
        TyRealtimeSessionId,
        mpsc::UnboundedSender<EnRealtimeDelivery>,
    )> {
        let stData = self.stLock();
        let mut setSeenSessions = HashSet::new();
        setUserIds
            .iter()
            .filter_map(|iUserId| stData.mapUserSessions.get(iUserId))
            .flatten()
            .filter(|uuidSessionId| setSeenSessions.insert(**uuidSessionId))
            .filter_map(|uuidSessionId| {
                stData
                    .mapSessions
                    .get(uuidSessionId)
                    .map(|stSession| (*uuidSessionId, stSession.txDelivery.clone()))
            })
            .collect()
    }

    fn vSend(
        &self,
        vecTargets: Vec<(
            TyRealtimeSessionId,
            mpsc::UnboundedSender<EnRealtimeDelivery>,
        )>,
        enDelivery: EnRealtimeDelivery,
    ) {
        for (uuidSessionId, txDelivery) in vecTargets {
            if txDelivery.send(enDelivery).is_err() {
                self.vUnregister(uuidSessionId);
            }
        }
    }
}

#[derive(Clone)]
pub struct CRealtimeService<R>
where
    R: TrRealtimeRepository,
{
    oRepository: R,
    cHub: CRealtimeHub,
}

impl<R> CRealtimeService<R>
where
    R: TrRealtimeRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self {
            oRepository,
            cHub: CRealtimeHub::default(),
        }
    }

    pub fn stRegisterSession(&self, optUserId: Option<i32>) -> StRealtimeSessionRegistration {
        self.cHub.stRegister(optUserId)
    }

    pub fn vUnregisterSession(&self, uuidSessionId: TyRealtimeSessionId) {
        self.cHub.vUnregister(uuidSessionId);
    }

    pub async fn vSubscribeTopic(
        &self,
        uuidSessionId: TyRealtimeSessionId,
        stRequest: StTopicSubscriptionRequest,
    ) -> Result<()> {
        let Some(vecMissedCommentIds) = self
            .oRepository
            .optMissedCommentIds(stRequest.iTopicId, stRequest.iLastSeenCommentId)
            .await?
        else {
            return Err(AppError::NotFound);
        };

        if !self
            .cHub
            .bSubscribe(uuidSessionId, stRequest.iTopicId, &vecMissedCommentIds)
        {
            return Err(AppError::Anyhow(anyhow::anyhow!(
                "realtime session is already closed"
            )));
        }
        Ok(())
    }

    pub async fn bShouldDeliverComment(
        &self,
        optUserId: Option<i32>,
        iCommentId: i32,
    ) -> Result<bool> {
        let Some(iUserId) = optUserId else {
            return Ok(true);
        };
        Ok(!self
            .oRepository
            .bIsCommentBranchIgnored(iUserId, iCommentId)
            .await?)
    }

    pub fn vNotifyNewComment(&self, iTopicId: i32, iCommentId: i32) {
        self.cHub.vSend(
            self.cHub.vecTopicSenders(iTopicId),
            EnRealtimeDelivery::Comment(iCommentId),
        );
    }

    pub fn vNotifyEvents<I>(&self, iterUserIds: I)
    where
        I: IntoIterator<Item = i32>,
    {
        let setUserIds: HashSet<i32> = iterUserIds.into_iter().collect();
        self.cHub.vSend(
            self.cHub.vecUserSenders(&setUserIds),
            EnRealtimeDelivery::EventsRefresh,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use async_trait::async_trait;

    use super::*;

    #[derive(Clone, Default)]
    struct CMockRealtimeRepository {
        setTopics: Arc<HashSet<i32>>,
        setIgnoredComments: Arc<HashSet<(i32, i32)>>,
        vecComments: Arc<Vec<i32>>,
    }

    #[async_trait]
    impl TrRealtimeRepository for CMockRealtimeRepository {
        async fn optMissedCommentIds(
            &self,
            iTopicId: i32,
            iLastSeenCommentId: i32,
        ) -> Result<Option<Vec<i32>>> {
            Ok(self.setTopics.contains(&iTopicId).then(|| {
                self.vecComments
                    .iter()
                    .copied()
                    .filter(|iCommentId| *iCommentId > iLastSeenCommentId)
                    .collect()
            }))
        }

        async fn bIsCommentBranchIgnored(&self, iUserId: i32, iCommentId: i32) -> Result<bool> {
            Ok(self.setIgnoredComments.contains(&(iUserId, iCommentId)))
        }
    }

    fn cService() -> CRealtimeService<CMockRealtimeRepository> {
        CRealtimeService::new(CMockRealtimeRepository {
            setTopics: Arc::new(HashSet::from([42])),
            setIgnoredComments: Arc::new(HashSet::from([(7, 102)])),
            vecComments: Arc::new(vec![100, 101, 102]),
        })
    }

    #[tokio::test]
    async fn subscription_delivers_missed_then_live_comments_once() {
        let cService = cService();
        let mut stRegistration = cService.stRegisterSession(None);
        cService
            .vSubscribeTopic(
                stRegistration.uuidSessionId,
                StTopicSubscriptionRequest {
                    iTopicId: 42,
                    iLastSeenCommentId: 100,
                },
            )
            .await
            .unwrap();
        cService
            .vSubscribeTopic(
                stRegistration.uuidSessionId,
                StTopicSubscriptionRequest {
                    iTopicId: 42,
                    iLastSeenCommentId: 102,
                },
            )
            .await
            .unwrap();
        cService.vNotifyNewComment(42, 103);

        assert_eq!(
            stRegistration.rxDelivery.recv().await,
            Some(EnRealtimeDelivery::Comment(101))
        );
        assert_eq!(
            stRegistration.rxDelivery.recv().await,
            Some(EnRealtimeDelivery::Comment(102))
        );
        assert_eq!(
            stRegistration.rxDelivery.recv().await,
            Some(EnRealtimeDelivery::Comment(103))
        );
        assert!(stRegistration.rxDelivery.try_recv().is_err());
    }

    #[tokio::test]
    async fn refresh_is_delivered_only_to_authenticated_matching_sessions() {
        let cService = cService();
        let mut stUser7 = cService.stRegisterSession(Some(7));
        let mut stUser8 = cService.stRegisterSession(Some(8));
        let mut stAnonymous = cService.stRegisterSession(None);

        cService.vNotifyEvents([7, 7]);

        assert_eq!(
            stUser7.rxDelivery.recv().await,
            Some(EnRealtimeDelivery::EventsRefresh)
        );
        assert!(stUser7.rxDelivery.try_recv().is_err());
        assert!(stUser8.rxDelivery.try_recv().is_err());
        assert!(stAnonymous.rxDelivery.try_recv().is_err());
    }

    #[tokio::test]
    async fn unregister_removes_topic_and_user_subscriptions() {
        let cService = cService();
        let mut stRegistration = cService.stRegisterSession(Some(7));
        cService
            .vSubscribeTopic(
                stRegistration.uuidSessionId,
                StTopicSubscriptionRequest {
                    iTopicId: 42,
                    iLastSeenCommentId: 102,
                },
            )
            .await
            .unwrap();
        cService.vUnregisterSession(stRegistration.uuidSessionId);
        cService.vNotifyNewComment(42, 103);
        cService.vNotifyEvents([7]);

        assert_eq!(stRegistration.rxDelivery.recv().await, None);
    }

    #[tokio::test]
    async fn ignored_branch_is_filtered_for_authenticated_user_only() {
        let cService = cService();
        assert!(!cService.bShouldDeliverComment(Some(7), 102).await.unwrap());
        assert!(cService.bShouldDeliverComment(Some(8), 102).await.unwrap());
        assert!(cService.bShouldDeliverComment(None, 102).await.unwrap());
    }

    #[tokio::test]
    async fn missing_topic_is_not_a_subscription() {
        let cService = cService();
        let stRegistration = cService.stRegisterSession(None);
        let stError = cService
            .vSubscribeTopic(
                stRegistration.uuidSessionId,
                StTopicSubscriptionRequest {
                    iTopicId: 404,
                    iLastSeenCommentId: 0,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(stError, AppError::NotFound));
    }
}
