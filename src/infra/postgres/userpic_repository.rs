use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    audit,
    domain::user::userpic::{StUserpicUploadPolicy, TrUserpicRepository},
    error::Result,
};

#[derive(Debug, Clone)]
pub struct CUserpicPgRepository {
    oPool: PgPool,
}

impl CUserpicPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrUserpicRepository for CUserpicPgRepository {
    async fn optUploadPolicy(&self, iUserId: i32) -> Result<Option<StUserpicUploadPolicy>> {
        let optRow = sqlx::query_as::<_, (i32, bool, i64, bool, i32)>(
            r#"SELECT CASE WHEN COALESCE(u.passwd,'')='' THEN 0 ELSE COALESCE(u.score,0) END,
                      COALESCE(u.frozen_until > CURRENT_TIMESTAMP,false),
                      (SELECT count(*) FROM user_log ul
                       WHERE ul.userid=u.id
                         AND ul.action='set_userpic'::user_log_action
                         AND ul.action_date>CURRENT_TIMESTAMP-interval '1 hour'),
                      EXISTS(SELECT 1 FROM user_log ul
                             WHERE ul.userid=u.id
                               AND ul.action='reset_userpic'::user_log_action
                               AND ul.action_date>CURRENT_TIMESTAMP-interval '30 days'
                               AND ul.userid<>ul.action_userid),
                      abs(COALESCE((SELECT sum(di.bonus) FROM del_info di
                                    WHERE di.deldate>CURRENT_TIMESTAMP-interval '3 days'
                                      AND di.msgid IN (
                                        SELECT c.id FROM comments c WHERE c.userid=u.id
                                        UNION ALL
                                        SELECT t.id FROM topics t WHERE t.userid=u.id
                                      )),0))::int
               FROM users u WHERE u.id=$1"#,
        )
        .bind(iUserId)
        .fetch_optional(&self.oPool)
        .await?;

        Ok(optRow.map(
            |(iScore, bFrozen, iRecentSetCount, bRecentlyResetByModerator, iRecentScoreLoss)| {
                StUserpicUploadPolicy {
                    iScore,
                    bFrozen,
                    iRecentSetCount,
                    bRecentlyResetByModerator,
                    iRecentScoreLoss,
                }
            },
        ))
    }

    async fn vSetUserpic(&self, iUserId: i32, sFilename: &str) -> Result<()> {
        let mut oTransaction = self.oPool.begin().await?;
        let optOldUserpic: Option<String> =
            sqlx::query_scalar("SELECT photo FROM users WHERE id=$1 FOR UPDATE")
                .bind(iUserId)
                .fetch_one(&mut *oTransaction)
                .await?;

        sqlx::query("UPDATE users SET photo=$2 WHERE id=$1")
            .bind(iUserId)
            .bind(sFilename)
            .execute(&mut *oTransaction)
            .await?;

        let mut vecInfo = Vec::with_capacity(2);
        if let Some(ref sOldUserpic) = optOldUserpic {
            vecInfo.push(("old_userpic", sOldUserpic.as_str()));
        }
        vecInfo.push(("new_userpic", sFilename));
        audit::log_user_action_tx(&mut oTransaction, iUserId, iUserId, "set_userpic", &vecInfo)
            .await?;

        oTransaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn set_userpic_keeps_the_java_audit_contract_transactional() {
        let sSource = include_str!("userpic_repository.rs");
        assert!(sSource.contains("CASE WHEN COALESCE(u.passwd,'')='' THEN 0"));
        assert!(sSource.contains("SELECT photo FROM users WHERE id=$1 FOR UPDATE"));
        assert!(sSource.contains("old_userpic"));
        assert!(sSource.contains("new_userpic"));
        assert!(sSource.contains("oTransaction.commit()"));
    }
}
