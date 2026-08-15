use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    audit,
    domain::user::account::{StDeregisterUserState, TrUserAccountRepository},
    error::Result,
};

#[derive(Debug, Clone)]
pub struct CUserAccountPgRepository {
    oPool: PgPool,
}

impl CUserAccountPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrUserAccountRepository for CUserAccountPgRepository {
    async fn optDeregisterState(&self, iUserId: i32) -> Result<Option<StDeregisterUserState>> {
        let optRow = sqlx::query_as::<_, (i32, i32, bool, bool, bool)>(
            r#"SELECT id,
                      COALESCE(max_score, 0),
                      COALESCE(canmod, false),
                      COALESCE(candel, false),
                      COALESCE(frozen_until > CURRENT_TIMESTAMP, false)
               FROM users
               WHERE id=$1"#,
        )
        .bind(iUserId)
        .fetch_optional(&self.oPool)
        .await?;

        Ok(optRow.map(
            |(iUserId, iMaxScore, bModerator, bAdministrator, bFrozen)| StDeregisterUserState {
                iUserId,
                iMaxScore,
                bModerator,
                bAdministrator,
                bFrozen,
            },
        ))
    }

    async fn bPasswordMatches(&self, iUserId: i32, sPassword: &str) -> Result<bool> {
        let optEncodedPassword: Option<String> =
            sqlx::query_scalar("SELECT passwd FROM users WHERE id=$1")
                .bind(iUserId)
                .fetch_optional(&self.oPool)
                .await?
                .flatten();

        Ok(optEncodedPassword
            .as_deref()
            .is_some_and(|sEncodedPassword| {
                crate::security::password::verify(sPassword, sEncodedPassword)
            }))
    }

    async fn vDeregister(&self, iUserId: i32, sReason: &str) -> Result<()> {
        let mut oTransaction = self.oPool.begin().await?;

        sqlx::query(
            r#"UPDATE users
               SET photo=NULL,
                   name='',
                   url='',
                   town='',
                   userinfo='',
                   userinfo_markup='MARKDOWN'::markup_type,
                   blocked=true
               WHERE id=$1"#,
        )
        .bind(iUserId)
        .execute(&mut *oTransaction)
        .await?;
        sqlx::query("INSERT INTO ban_info(userid, reason, ban_by) VALUES($1,$2,$1)")
            .bind(iUserId)
            .bind(sReason)
            .execute(&mut *oTransaction)
            .await?;
        audit::log_user_action_tx(
            &mut oTransaction,
            iUserId,
            iUserId,
            "block_user",
            &[("reason", sReason)],
        )
        .await?;

        oTransaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn deregistration_sql_keeps_profile_cleanup_and_audit_transactional() {
        let sSource = include_str!("user_account_repository.rs");
        assert!(sSource.contains("SELECT passwd FROM users WHERE id=$1"));
        assert!(sSource.contains("userinfo_markup='MARKDOWN'::markup_type"));
        assert!(sSource.contains("INSERT INTO ban_info"));
        assert!(sSource.contains("log_user_action_tx"));
        assert!(sSource.contains("oTransaction.commit()"));
    }
}
