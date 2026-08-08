use async_trait::async_trait;
use sqlx::PgPool;

use crate::{domain::realtime::repository::TrRealtimeRepository, error::Result};

#[derive(Debug, Clone)]
pub struct CRealtimePgRepository {
    oPool: PgPool,
}

impl CRealtimePgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrRealtimeRepository for CRealtimePgRepository {
    async fn optMissedCommentIds(
        &self,
        iTopicId: i32,
        iLastSeenCommentId: i32,
    ) -> Result<Option<Vec<i32>>> {
        let optCommentsHidden: Option<bool> = sqlx::query_scalar(
            "SELECT postscore IS NOT DISTINCT FROM 10002 FROM topics WHERE id=$1",
        )
        .bind(iTopicId)
        .fetch_optional(&self.oPool)
        .await?;
        let Some(bCommentsHidden) = optCommentsHidden else {
            return Ok(None);
        };
        if bCommentsHidden {
            return Ok(Some(Vec::new()));
        }

        Ok(Some(
            sqlx::query_scalar(
                r#"SELECT id FROM comments
                   WHERE topic=$1 AND NOT deleted AND id>$2
                   ORDER BY id ASC"#,
            )
            .bind(iTopicId)
            .bind(iLastSeenCommentId)
            .fetch_all(&self.oPool)
            .await?,
        ))
    }

    async fn bIsCommentBranchIgnored(&self, iUserId: i32, iCommentId: i32) -> Result<bool> {
        Ok(sqlx::query_scalar(
            r#"SELECT EXISTS (
                   SELECT ignored FROM ignore_list WHERE userid=$1
                   INTERSECT
                   SELECT get_branch_authors($2)
               )"#,
        )
        .bind(iUserId)
        .bind(iCommentId)
        .fetch_one(&self.oPool)
        .await?)
    }
}
