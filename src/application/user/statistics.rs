use std::{collections::HashMap, time::Duration};

use crate::{
    domain::user::statistics::{
        StPreparedUserSectionStatistics, StPreparedUserStatistics, TrUserStatisticsLocalRepository,
        TrUserStatisticsRepository, TyUserYearStats,
    },
    error::{AppError, Result},
};

const D_SEARCH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct CUserStatisticsService<R>
where
    R: TrUserStatisticsRepository,
{
    oRepository: R,
}

impl<R> CUserStatisticsService<R>
where
    R: TrUserStatisticsRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn mapYearStats(&self, sNick: &str, sTimezone: &str) -> Result<TyUserYearStats> {
        self.oRepository.mapYearStats(sNick, sTimezone).await
    }
}

/// Java-compatible ordinary-profile statistics coordinator.
///
/// The year-histogram endpoint deliberately remains on
/// [`CUserStatisticsService`].  Keeping the ordinary profile coordinator
/// separate lets that established API stay small while this service combines
/// the OpenSearch and PostgreSQL halves of `UserStatisticsService.getStats`.
#[derive(Debug, Clone)]
pub struct CUserProfileStatisticsService<S, P>
where
    S: TrUserStatisticsRepository,
    P: TrUserStatisticsLocalRepository,
{
    oSearchRepository: S,
    oLocalRepository: P,
    dSearchTimeout: Duration,
}

impl<S, P> CUserProfileStatisticsService<S, P>
where
    S: TrUserStatisticsRepository,
    P: TrUserStatisticsLocalRepository,
{
    pub fn new(oSearchRepository: S, oLocalRepository: P) -> Self {
        Self {
            oSearchRepository,
            oLocalRepository,
            dSearchTimeout: D_SEARCH_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn stWithSearchTimeout(
        oSearchRepository: S,
        oLocalRepository: P,
        dSearchTimeout: Duration,
    ) -> Self {
        Self {
            oSearchRepository,
            oLocalRepository,
            dSearchTimeout,
        }
    }

    pub async fn stGetStats(&self, iUserId: i32, sNick: &str) -> Result<StPreparedUserStatistics> {
        // Both OpenSearch operations receive the same absolute deadline, not
        // independent five-second budgets.  `join!` polls them concurrently
        // together with the synchronous-data equivalent from PostgreSQL.
        let dtDeadline = tokio::time::Instant::now() + self.dSearchTimeout;
        let fCommentCount =
            tokio::time::timeout_at(dtDeadline, self.oSearchRepository.iCommentCount(sNick));
        let fTopicStatistics =
            tokio::time::timeout_at(dtDeadline, self.oSearchRepository.stTopicStatistics(sNick));
        let fLocalData = self.oLocalRepository.stLocalData(iUserId);

        let (stLocalData, stCommentCount, stTopicStatistics) =
            tokio::join!(fLocalData, fCommentCount, fTopicStatistics);
        let stLocalData = stLocalData?;
        let optCommentCount = optRecoverSearchResult(stCommentCount, "comments");
        let optTopicStatistics = optRecoverSearchResult(stTopicStatistics, "topics");
        let bIncomplete = optCommentCount.is_none() || optTopicStatistics.is_none();

        let mapSections = stLocalData
            .vecSections
            .iter()
            .map(|stSection| (stSection.sUrlName.as_str(), stSection))
            .collect::<HashMap<_, _>>();
        let mut vecTopicsBySection = Vec::new();
        if let Some(stTopicStatistics) = optTopicStatistics.as_ref() {
            for stCount in &stTopicStatistics.vecSectionCounts {
                // SectionService.getSectionByName throws for an unknown
                // OpenSearch key.  Do not silently discard index/schema drift.
                let stSection = mapSections
                    .get(stCount.sSectionUrlName.as_str())
                    .ok_or_else(|| {
                        AppError::Anyhow(anyhow::anyhow!(
                            "OpenSearch returned unknown section {:?}",
                            stCount.sSectionUrlName
                        ))
                    })?;
                vecTopicsBySection.push(StPreparedUserSectionStatistics {
                    iId: stSection.iId,
                    sName: stSection.sName.clone(),
                    iCount: stCount.iCount,
                });
            }
            vecTopicsBySection.sort_by_key(|stSection| stSection.iId);
        }

        Ok(StPreparedUserStatistics {
            iIgnoreCount: stLocalData.iIgnoreCount,
            iCommentCount: optCommentCount.unwrap_or(0),
            bIncomplete,
            optFirstComment: stLocalData.optFirstComment,
            optLastComment: stLocalData.optLastComment,
            optFirstTopic: optTopicStatistics
                .as_ref()
                .and_then(|stStatistics| stStatistics.optFirstTopic),
            optLastTopic: optTopicStatistics
                .as_ref()
                .and_then(|stStatistics| stStatistics.optLastTopic),
            vecTopicsBySection,
        })
    }
}

fn optRecoverSearchResult<T>(
    stResult: std::result::Result<Result<T>, tokio::time::error::Elapsed>,
    sKind: &'static str,
) -> Option<T> {
    match stResult {
        Ok(Ok(stValue)) => Some(stValue),
        Ok(Err(_)) => {
            tracing::warn!(statistics = sKind, "unable to load user statistics");
            None
        }
        Err(_) => {
            tracing::warn!(
                statistics = sKind,
                "user statistics request exceeded deadline"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::domain::user::statistics::{
        StUserSectionCount, StUserStatisticsLocalData, StUserStatisticsSection,
        StUserTopicStatistics,
    };

    #[derive(Debug, Clone)]
    struct StSearchFake {
        bFailComments: bool,
        bFailTopics: bool,
        dCommentDelay: Duration,
        dTopicDelay: Duration,
        iCommentCount: i64,
        stTopics: StUserTopicStatistics,
    }

    #[async_trait]
    impl TrUserStatisticsRepository for StSearchFake {
        async fn mapYearStats(&self, _sNick: &str, _sTimezone: &str) -> Result<TyUserYearStats> {
            Ok(BTreeMap::new())
        }

        async fn iCommentCount(&self, _sNick: &str) -> Result<i64> {
            tokio::time::sleep(self.dCommentDelay).await;
            if self.bFailComments {
                Err(AppError::Anyhow(anyhow::anyhow!("comment failure")))
            } else {
                Ok(self.iCommentCount)
            }
        }

        async fn stTopicStatistics(&self, _sNick: &str) -> Result<StUserTopicStatistics> {
            tokio::time::sleep(self.dTopicDelay).await;
            if self.bFailTopics {
                Err(AppError::Anyhow(anyhow::anyhow!("topic failure")))
            } else {
                Ok(self.stTopics.clone())
            }
        }
    }

    #[derive(Debug, Clone)]
    struct StLocalFake(StUserStatisticsLocalData);

    #[async_trait]
    impl TrUserStatisticsLocalRepository for StLocalFake {
        async fn stLocalData(&self, _iUserId: i32) -> Result<StUserStatisticsLocalData> {
            Ok(self.0.clone())
        }
    }

    fn stFixture() -> (StSearchFake, StLocalFake) {
        let dtFirstComment = Utc.with_ymd_and_hms(2020, 1, 2, 3, 4, 5).unwrap();
        let dtLastComment = Utc.with_ymd_and_hms(2024, 5, 6, 7, 8, 9).unwrap();
        let dtFirstTopic = Utc.with_ymd_and_hms(2021, 2, 3, 4, 5, 6).unwrap();
        let dtLastTopic = Utc.with_ymd_and_hms(2025, 6, 7, 8, 9, 10).unwrap();
        (
            StSearchFake {
                bFailComments: false,
                bFailTopics: false,
                dCommentDelay: Duration::ZERO,
                dTopicDelay: Duration::ZERO,
                iCommentCount: 73,
                stTopics: StUserTopicStatistics {
                    optFirstTopic: Some(dtFirstTopic),
                    optLastTopic: Some(dtLastTopic),
                    // OpenSearch terms order is count-based, not section-ID
                    // based; the application service must reorder it.
                    vecSectionCounts: vec![
                        StUserSectionCount {
                            sSectionUrlName: "articles".to_owned(),
                            iCount: 2,
                        },
                        StUserSectionCount {
                            sSectionUrlName: "forum".to_owned(),
                            iCount: 15,
                        },
                    ],
                },
            },
            StLocalFake(StUserStatisticsLocalData {
                iIgnoreCount: 4,
                optFirstComment: Some(dtFirstComment),
                optLastComment: Some(dtLastComment),
                vecSections: vec![
                    StUserStatisticsSection {
                        iId: 2,
                        sName: "Форум".to_owned(),
                        sUrlName: "forum".to_owned(),
                    },
                    StUserStatisticsSection {
                        iId: 6,
                        sName: "Статьи".to_owned(),
                        sUrlName: "articles".to_owned(),
                    },
                ],
            }),
        )
    }

    #[tokio::test]
    async fn combines_java_sources_and_sorts_sections_by_id() {
        let (stSearch, stLocal) = stFixture();
        let stActual = CUserProfileStatisticsService::new(stSearch, stLocal)
            .stGetStats(42, "tester")
            .await
            .unwrap();

        assert!(!stActual.bIncomplete);
        assert_eq!(stActual.iIgnoreCount, 4);
        assert_eq!(stActual.iCommentCount, 73);
        assert_eq!(
            stActual
                .vecTopicsBySection
                .iter()
                .map(|stSection| (stSection.iId, stSection.iCount))
                .collect::<Vec<_>>(),
            vec![(2, 15), (6, 2)]
        );
    }

    #[tokio::test]
    async fn comment_failure_is_independent_and_keeps_topic_statistics() {
        let (mut stSearch, stLocal) = stFixture();
        stSearch.bFailComments = true;
        let stActual = CUserProfileStatisticsService::new(stSearch, stLocal)
            .stGetStats(42, "tester")
            .await
            .unwrap();

        assert!(stActual.bIncomplete);
        assert_eq!(stActual.iCommentCount, 0);
        assert!(stActual.optFirstTopic.is_some());
        assert_eq!(stActual.vecTopicsBySection.len(), 2);
    }

    #[tokio::test]
    async fn topic_failure_is_independent_and_keeps_comment_statistics() {
        let (mut stSearch, stLocal) = stFixture();
        stSearch.bFailTopics = true;
        let stActual = CUserProfileStatisticsService::new(stSearch, stLocal)
            .stGetStats(42, "tester")
            .await
            .unwrap();

        assert!(stActual.bIncomplete);
        assert_eq!(stActual.iCommentCount, 73);
        assert!(stActual.optFirstTopic.is_none());
        assert!(stActual.vecTopicsBySection.is_empty());
        assert!(stActual.optFirstComment.is_some());
    }

    #[tokio::test]
    async fn search_operations_are_polled_concurrently() {
        let (mut stSearch, stLocal) = stFixture();
        stSearch.dCommentDelay = Duration::from_millis(100);
        stSearch.dTopicDelay = Duration::from_millis(100);
        let stActual = CUserProfileStatisticsService::stWithSearchTimeout(
            stSearch,
            stLocal,
            Duration::from_millis(180),
        )
        .stGetStats(42, "tester")
        .await
        .unwrap();

        // Sequential polling would consume roughly 200 ms and lose the
        // second result to the absolute 180 ms deadline.
        assert!(!stActual.bIncomplete);
        assert_eq!(stActual.iCommentCount, 73);
        assert_eq!(stActual.vecTopicsBySection.len(), 2);
    }

    #[tokio::test]
    async fn operations_share_one_absolute_deadline() {
        let (mut stSearch, stLocal) = stFixture();
        stSearch.dCommentDelay = Duration::from_millis(60);
        stSearch.dTopicDelay = Duration::from_millis(60);
        let stActual = CUserProfileStatisticsService::stWithSearchTimeout(
            stSearch,
            stLocal,
            Duration::from_millis(10),
        )
        .stGetStats(42, "tester")
        .await
        .unwrap();

        assert!(stActual.bIncomplete);
        assert_eq!(stActual.iCommentCount, 0);
        assert!(stActual.vecTopicsBySection.is_empty());
    }

    #[tokio::test]
    async fn successful_empty_index_is_complete() {
        let (mut stSearch, stLocal) = stFixture();
        stSearch.iCommentCount = 0;
        stSearch.stTopics = StUserTopicStatistics {
            optFirstTopic: None,
            optLastTopic: None,
            vecSectionCounts: Vec::new(),
        };
        let stActual = CUserProfileStatisticsService::new(stSearch, stLocal)
            .stGetStats(42, "tester")
            .await
            .unwrap();

        assert!(!stActual.bIncomplete);
        assert_eq!(stActual.iCommentCount, 0);
        assert!(stActual.vecTopicsBySection.is_empty());
    }

    #[tokio::test]
    async fn unknown_index_section_is_not_silently_dropped() {
        let (mut stSearch, stLocal) = stFixture();
        stSearch.stTopics.vecSectionCounts = vec![StUserSectionCount {
            sSectionUrlName: "unknown".to_owned(),
            iCount: 1,
        }];
        let stResult = CUserProfileStatisticsService::new(stSearch, stLocal)
            .stGetStats(42, "tester")
            .await;

        assert!(matches!(stResult, Err(AppError::Anyhow(_))));
    }
}
