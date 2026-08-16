use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;

pub type TyUserYearStats = BTreeMap<i64, i64>;

/// One OpenSearch `sections` terms bucket.  The key is the canonical section
/// URL name (`forum`, `news`, ...), exactly as stored by the Java message
/// indexer; presentation names and numeric IDs remain PostgreSQL data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StUserSectionCount {
    pub sSectionUrlName: String,
    pub iCount: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StUserTopicStatistics {
    pub optFirstTopic: Option<DateTime<Utc>>,
    pub optLastTopic: Option<DateTime<Utc>>,
    pub vecSectionCounts: Vec<StUserSectionCount>,
}

/// The database-backed half of Java's `UserStatisticsService.getStats`.
///
/// `firstComment`/`lastComment` intentionally include deleted comments.  The
/// comment *count* is not present here because the original obtains it from
/// the `messages` OpenSearch index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StUserStatisticsLocalData {
    pub iIgnoreCount: i64,
    pub optFirstComment: Option<DateTime<Utc>>,
    pub optLastComment: Option<DateTime<Utc>>,
    pub vecSections: Vec<StUserStatisticsSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StUserStatisticsSection {
    pub iId: i32,
    pub sName: String,
    pub sUrlName: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StPreparedUserSectionStatistics {
    pub iId: i32,
    pub sName: String,
    pub iCount: i64,
}

/// Complete view model produced by the application service.  `bIncomplete`
/// is true only when at least one of the two independent OpenSearch requests
/// failed or exceeded their shared deadline, matching the Java service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StPreparedUserStatistics {
    pub iIgnoreCount: i64,
    pub iCommentCount: i64,
    pub bIncomplete: bool,
    pub optFirstComment: Option<DateTime<Utc>>,
    pub optLastComment: Option<DateTime<Utc>>,
    pub optFirstTopic: Option<DateTime<Utc>>,
    pub optLastTopic: Option<DateTime<Utc>>,
    pub vecTopicsBySection: Vec<StPreparedUserSectionStatistics>,
}

#[async_trait]
pub trait TrUserStatisticsRepository: Send + Sync {
    async fn mapYearStats(&self, sNick: &str, sTimezone: &str) -> Result<TyUserYearStats>;

    async fn iCommentCount(&self, sNick: &str) -> Result<i64>;

    async fn stTopicStatistics(&self, sNick: &str) -> Result<StUserTopicStatistics>;
}

#[async_trait]
pub trait TrUserStatisticsLocalRepository: Send + Sync {
    async fn stLocalData(&self, iUserId: i32) -> Result<StUserStatisticsLocalData>;
}
