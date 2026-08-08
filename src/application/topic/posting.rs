use crate::{
    domain::topic::posting::{
        StAddTopicActor, StAddTopicPermission, StSlowModeInfo, StTopicLimitInfo,
        TrAddTopicRepository, bSlowModeRestricted, iTopicDailyLimit, stCheckAddTopic,
        stCheckTopicPublish,
    },
    error::Result,
};
use std::{collections::HashMap, time::Duration};
use tokio::{sync::Mutex, time::Instant};

#[derive(Debug, Clone)]
pub struct CAddTopicService<R>
where
    R: TrAddTopicRepository,
{
    oRepository: R,
}

#[derive(Debug)]
pub struct CTopicPublishService<R>
where
    R: TrAddTopicRepository,
{
    oRepository: R,
    bRateLimitEnabled: bool,
    mapPerformedActions: Mutex<HashMap<String, Instant>>,
}

impl<R> CTopicPublishService<R>
where
    R: TrAddTopicRepository,
{
    pub fn new(oRepository: R, sPublicUrl: &str) -> Self {
        // FloodProtector disables itself only when SiteConfig.mainURI.host is
        // exactly 127.0.0.1. `localhost` is intentionally not equivalent.
        let bRateLimitEnabled = reqwest::Url::parse(sPublicUrl)
            .ok()
            .and_then(|stUrl| stUrl.host_str().map(str::to_owned))
            .is_none_or(|sHost| sHost != "127.0.0.1");
        Self {
            oRepository,
            bRateLimitEnabled,
            mapPerformedActions: Mutex::new(HashMap::new()),
        }
    }

    pub async fn stTopicLimitInfo(
        &self,
        stActor: StAddTopicActor,
        iSectionId: i32,
    ) -> Result<StTopicLimitInfo> {
        if stActor.bAnonymous || stActor.bModerator || stActor.bCorrector {
            return Ok(StTopicLimitInfo {
                iLimit: 0,
                iCurrentCount: 0,
                bReached: false,
                bExempt: true,
            });
        }
        let iLimit = iTopicDailyLimit(stActor.iScore);
        let iCurrentCount = match stActor.optUserId {
            Some(iUserId) => {
                self.oRepository
                    .iCountRecentTopics(iUserId, iSectionId)
                    .await?
            }
            None => 0,
        };
        Ok(StTopicLimitInfo {
            iLimit,
            iCurrentCount,
            bReached: iCurrentCount >= iLimit,
            bExempt: false,
        })
    }

    pub fn stCheckPublish(
        &self,
        stAddPermission: StAddTopicPermission,
        stLimitInfo: StTopicLimitInfo,
    ) -> StAddTopicPermission {
        stCheckTopicPublish(stAddPermission, stLimitInfo)
    }

    /// `FloodProtector.AddTopic`: the cache is keyed by action+IP, records
    /// successful checks only, and does not extend the deadline on rejection.
    pub async fn optCheckAddTopicRate(
        &self,
        stActor: StAddTopicActor,
        sRemoteIp: &str,
    ) -> Result<Option<String>> {
        if !self.bRateLimitEnabled {
            return Ok(None);
        }
        let stSlowModeInfo = match stActor.optUserId {
            Some(iUserId) if !stActor.bAnonymous => {
                self.oRepository.stSlowModeInfo(iUserId).await?
            }
            _ => StSlowModeInfo::default(),
        };
        let iThresholdSeconds = iAddTopicThresholdSeconds(stActor, stSlowModeInfo);
        Ok(self
            .optRecordAddTopicAt(sRemoteIp, iThresholdSeconds, Instant::now())
            .await)
    }

    async fn optRecordAddTopicAt(
        &self,
        sRemoteIp: &str,
        iThresholdSeconds: u64,
        tmNow: Instant,
    ) -> Option<String> {
        let mut mapActions = self.mapPerformedActions.lock().await;
        mapActions.retain(|_, tmAction| {
            tmNow
                .checked_duration_since(*tmAction)
                .is_some_and(|stAge| stAge < Duration::from_secs(30 * 60))
        });
        let sKey = format!("AddTopic:{sRemoteIp}");
        if mapActions.get(&sKey).is_some_and(|tmAction| {
            tmAction
                .checked_add(Duration::from_secs(iThresholdSeconds))
                .is_some_and(|tmDeadline| tmDeadline > tmNow)
        }) {
            return Some(format!(
                "Следующее сообщение может быть записано не менее чем через {iThresholdSeconds} секунд после предыдущего"
            ));
        }
        mapActions.insert(sKey, tmNow);
        None
    }
}

fn iAddTopicThresholdSeconds(stActor: StAddTopicActor, stInfo: StSlowModeInfo) -> u64 {
    if stActor.bAnonymous || bSlowModeRestricted(stActor, stInfo) || stActor.iScore < 100 {
        10 * 60
    } else {
        30
    }
}

impl<R> CAddTopicService<R>
where
    R: TrAddTopicRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    /// Loads the same request-dependent inputs used by Java's AnySession and
    /// checks the maximum of group and section `restrict_topics`.
    pub async fn optCheckGroup(
        &self,
        iGroupId: i32,
        stActor: StAddTopicActor,
        sRemoteIp: &str,
    ) -> Result<Option<StAddTopicPermission>> {
        let Some(iRestriction) = self.oRepository.optGroupTopicRestriction(iGroupId).await? else {
            return Ok(None);
        };
        let bFrozen = match stActor.optUserId {
            Some(iUserId) => self.oRepository.bIsUserFrozen(iUserId).await?,
            None => false,
        };
        let stIpBlock = self.oRepository.stIpBlockInfo(sRemoteIp).await?;
        Ok(Some(stCheckAddTopic(
            stActor,
            bFrozen,
            stIpBlock,
            iRestriction,
        )))
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::domain::topic::posting::StIpBlockInfo;

    #[derive(Debug, Clone)]
    struct CTestRepository;

    #[async_trait]
    impl TrAddTopicRepository for CTestRepository {
        async fn optGroupTopicRestriction(&self, iGroupId: i32) -> Result<Option<i32>> {
            Ok((iGroupId == 42).then_some(200))
        }

        async fn bIsUserFrozen(&self, iUserId: i32) -> Result<bool> {
            Ok(iUserId == 99)
        }

        async fn stIpBlockInfo(&self, sIp: &str) -> Result<StIpBlockInfo> {
            Ok(StIpBlockInfo {
                bBlocked: sIp == "192.0.2.1",
                bAllowRegisteredPosting: false,
            })
        }

        async fn iCountRecentTopics(&self, _iUserId: i32, _iSectionId: i32) -> Result<i32> {
            Ok(2)
        }

        async fn stSlowModeInfo(&self, _iUserId: i32) -> Result<StSlowModeInfo> {
            Ok(StSlowModeInfo::default())
        }
    }

    fn stActor(iUserId: i32, iScore: i32) -> StAddTopicActor {
        StAddTopicActor {
            optUserId: Some(iUserId),
            bAnonymous: false,
            bModerator: false,
            bCorrector: false,
            bBlocked: false,
            iScore,
        }
    }

    #[tokio::test]
    async fn service_combines_group_user_and_request_ip_state() {
        let cService = CAddTopicService::new(CTestRepository);
        assert_eq!(
            cService
                .optCheckGroup(42, stActor(1, 199), "127.0.0.1")
                .await
                .unwrap()
                .unwrap()
                .sReason(),
            "только для зарегистрированных, минимум ★★"
        );
        assert_eq!(
            cService
                .optCheckGroup(42, stActor(99, 500), "127.0.0.1")
                .await
                .unwrap()
                .unwrap()
                .sReason(),
            "установлен режим только для чтения"
        );
        assert_eq!(
            cService
                .optCheckGroup(42, stActor(1, 500), "192.0.2.1")
                .await
                .unwrap()
                .unwrap()
                .sReason(),
            "постинг с этого IP адреса заблокирован"
        );
        assert!(
            cService
                .optCheckGroup(7, stActor(1, 500), "127.0.0.1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn daily_limit_and_ip_rate_state_match_java() {
        let cService = CTopicPublishService::new(CTestRepository, "https://linux.org.ru");
        let stLimit = cService.stTopicLimitInfo(stActor(1, 200), 1).await.unwrap();
        assert_eq!(stLimit.iLimit, 2);
        assert_eq!(stLimit.iCurrentCount, 2);
        assert!(stLimit.bReached);

        let tmStart = Instant::now();
        assert!(
            cService
                .optRecordAddTopicAt("192.0.2.1", 30, tmStart)
                .await
                .is_none()
        );
        assert_eq!(
            cService
                .optRecordAddTopicAt("192.0.2.1", 30, tmStart + Duration::from_secs(29))
                .await
                .as_deref(),
            Some(
                "Следующее сообщение может быть записано не менее чем через 30 секунд после предыдущего"
            )
        );
        assert!(
            cService
                .optRecordAddTopicAt("192.0.2.1", 30, tmStart + Duration::from_secs(30))
                .await
                .is_none()
        );
    }

    #[test]
    fn trusted_threshold_requires_score_and_no_slow_mode_reason() {
        assert_eq!(
            iAddTopicThresholdSeconds(stActor(1, 100), StSlowModeInfo::default()),
            30
        );
        assert_eq!(
            iAddTopicThresholdSeconds(
                stActor(1, 100),
                StSlowModeInfo {
                    bCurrentlyFrozen: false,
                    bFrozenWithinThreeDays: false,
                    iRecentScoreLoss: 30,
                }
            ),
            600
        );
        assert_eq!(
            iAddTopicThresholdSeconds(stActor(1, 99), StSlowModeInfo::default()),
            600
        );
    }
}
