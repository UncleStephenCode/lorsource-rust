use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    domain::topic::posting::{StIpBlockInfo, StSlowModeInfo, TrAddTopicRepository},
    error::Result,
};

const GROUP_TOPIC_RESTRICTION_SQL: &str = r#"SELECT GREATEST(
        COALESCE(g.restrict_topics, -9999),
        COALESCE(s.restrict_topics, -9999)
    )
    FROM groups g
    JOIN sections s ON s.id=g.section
    WHERE g.id=$1"#;

const IP_BLOCK_INFO_SQL: &str = r#"SELECT
        (ban_date IS NULL OR ban_date > CURRENT_TIMESTAMP) AS blocked,
        COALESCE(allow_posting, false) AS allow_posting
    FROM b_ips
    WHERE ip=$1::inet"#;

const RECENT_TOPIC_COUNT_SQL: &str = r#"SELECT COUNT(*)
    FROM topics t
    LEFT JOIN del_info di ON di.msgid=t.id
    WHERE t.userid=$1
      AND t.postdate >= (CURRENT_TIMESTAMP - '24 hours'::interval)
      AND NOT t.draft
      AND NOT (t.deleted AND (di.msgid IS NULL OR di.delby=t.userid))
      AND EXISTS (
          SELECT 1 FROM groups g
          WHERE g.id=t.groupid AND g.section=$2
      )"#;

const SLOW_MODE_INFO_SQL: &str = r#"SELECT
    COALESCE(u.frozen_until > CURRENT_TIMESTAMP, false),
    COALESCE(u.frozen_until > CURRENT_TIMESTAMP - '3 days'::interval, false),
    COALESCE(ABS((
        SELECT SUM(di.bonus)::bigint
        FROM del_info di
        WHERE di.deldate > CURRENT_TIMESTAMP - '3 days'::interval
          AND di.msgid IN (
              SELECT c.id FROM comments c WHERE c.userid=$1
              UNION ALL
              SELECT t.id FROM topics t WHERE t.userid=$1
          )
    )), 0)
    FROM users u
    WHERE u.id=$1"#;

#[derive(Debug, Clone)]
pub struct CAddTopicPgRepository {
    oPool: PgPool,
}

impl CAddTopicPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrAddTopicRepository for CAddTopicPgRepository {
    async fn optGroupTopicRestriction(&self, iGroupId: i32) -> Result<Option<i32>> {
        Ok(sqlx::query_scalar(GROUP_TOPIC_RESTRICTION_SQL)
            .bind(iGroupId)
            .fetch_optional(&self.oPool)
            .await?)
    }

    async fn bIsUserFrozen(&self, iUserId: i32) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(frozen_until > CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
        )
        .bind(iUserId)
        .fetch_one(&self.oPool)
        .await?)
    }

    async fn stIpBlockInfo(&self, sIp: &str) -> Result<StIpBlockInfo> {
        let optRow: Option<(bool, bool)> = sqlx::query_as(IP_BLOCK_INFO_SQL)
            .bind(sIp)
            .fetch_optional(&self.oPool)
            .await?;
        Ok(match optRow {
            Some((bBlocked, bAllowPosting)) => StIpBlockInfo {
                bBlocked,
                bAllowRegisteredPosting: !bBlocked || bAllowPosting,
            },
            None => StIpBlockInfo::default(),
        })
    }

    async fn iCountRecentTopics(&self, iUserId: i32, iSectionId: i32) -> Result<i32> {
        let iCount: i64 = sqlx::query_scalar(RECENT_TOPIC_COUNT_SQL)
            .bind(iUserId)
            .bind(iSectionId)
            .fetch_one(&self.oPool)
            .await?;
        Ok(iCount.min(i64::from(i32::MAX)) as i32)
    }

    async fn stSlowModeInfo(&self, iUserId: i32) -> Result<StSlowModeInfo> {
        let (bCurrentlyFrozen, bFrozenWithinThreeDays, iRecentScoreLoss): (bool, bool, i64) =
            sqlx::query_as(SLOW_MODE_INFO_SQL)
                .bind(iUserId)
                .fetch_one(&self.oPool)
                .await?;
        Ok(StSlowModeInfo {
            bCurrentlyFrozen,
            bFrozenWithinThreeDays,
            iRecentScoreLoss: iRecentScoreLoss.min(i64::from(i32::MAX)) as i32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_query_uses_java_maximum_of_group_and_section_restrictions() {
        assert!(GROUP_TOPIC_RESTRICTION_SQL.contains("GREATEST("));
        assert!(GROUP_TOPIC_RESTRICTION_SQL.contains("g.restrict_topics"));
        assert!(GROUP_TOPIC_RESTRICTION_SQL.contains("s.restrict_topics"));
    }

    #[test]
    fn ip_query_uses_canonical_b_ips_columns_and_expiry_semantics() {
        assert!(IP_BLOCK_INFO_SQL.contains("FROM b_ips"));
        assert!(IP_BLOCK_INFO_SQL.contains("ban_date IS NULL"));
        assert!(IP_BLOCK_INFO_SQL.contains("ban_date > CURRENT_TIMESTAMP"));
        assert!(IP_BLOCK_INFO_SQL.contains("allow_posting"));
        assert!(IP_BLOCK_INFO_SQL.contains("ip=$1::inet"));
    }

    #[test]
    fn daily_count_matches_java_draft_deleted_section_and_window_rules() {
        assert!(RECENT_TOPIC_COUNT_SQL.contains("'24 hours'::interval"));
        assert!(RECENT_TOPIC_COUNT_SQL.contains("AND NOT t.draft"));
        assert!(
            RECENT_TOPIC_COUNT_SQL
                .contains("NOT (t.deleted AND (di.msgid IS NULL OR di.delby=t.userid))")
        );
        assert!(RECENT_TOPIC_COUNT_SQL.contains("g.section=$2"));
        // Premoderated, not-yet-committed topics count too: Java has no
        // topics.moderate predicate in TopicDao.countRecentTopics.
        assert!(!RECENT_TOPIC_COUNT_SQL.contains("moderate"));
    }

    #[test]
    fn slow_mode_query_uses_java_three_day_windows_and_score_loss_sources() {
        assert!(SLOW_MODE_INFO_SQL.contains("frozen_until"));
        assert!(SLOW_MODE_INFO_SQL.contains("frozen_until > CURRENT_TIMESTAMP,"));
        assert_eq!(SLOW_MODE_INFO_SQL.matches("'3 days'::interval").count(), 2);
        assert!(SLOW_MODE_INFO_SQL.contains("SELECT c.id FROM comments"));
        assert!(SLOW_MODE_INFO_SQL.contains("SELECT t.id FROM topics"));
        assert!(SLOW_MODE_INFO_SQL.contains("SUM(di.bonus)"));
    }
}
