use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{
    domain::email_domain_block::{
        model::StEmailDomainBlock, repository::TrEmailDomainBlockRepository,
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct CEmailDomainBlockPgRepository {
    oPool: PgPool,
}

impl CEmailDomainBlockPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrEmailDomainBlockRepository for CEmailDomainBlockPgRepository {
    async fn optProfileSettings(&self, iUserId: i32) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                .bind(iUserId)
                .fetch_optional(&self.oPool)
                .await?,
        )
    }

    async fn iManualCount(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT count(*) FROM email_domains_block WHERE auto=false")
                .fetch_one(&self.oPool)
                .await?,
        )
    }

    async fn vecManualBlocks(&self, iOffset: i32, iLimit: i32) -> Result<Vec<StEmailDomainBlock>> {
        Ok(sqlx::query_as::<_, StEmailDomainBlock>(
            r#"SELECT b.domain AS "sDomain",
                      b.block_until AS "dtBlockUntil",
                      u.nick AS "optModeratorNick",
                      b.blocked_at AS "dtBlockedAt"
               FROM email_domains_block b
               LEFT JOIN users u ON u.id=b.moderator_id
               WHERE b.auto=false
               ORDER BY b.domain
               LIMIT $1 OFFSET $2"#,
        )
        .bind(iLimit)
        .bind(iOffset)
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn vBlockManual(
        &self,
        sDomain: &str,
        dtBlockUntil: DateTime<Utc>,
        iModeratorId: i32,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO email_domains_block(domain,block_until,auto,moderator_id,blocked_at)
               VALUES($1,$2,false,$3,CURRENT_TIMESTAMP)
               ON CONFLICT(domain) DO UPDATE SET
                 block_until=EXCLUDED.block_until,
                 auto=false,
                 moderator_id=EXCLUDED.moderator_id,
                 blocked_at=CURRENT_TIMESTAMP"#,
        )
        .bind(sDomain)
        .bind(dtBlockUntil)
        .bind(iModeratorId)
        .execute(&self.oPool)
        .await?;
        Ok(())
    }

    async fn vUnblock(&self, sDomain: &str) -> Result<()> {
        sqlx::query("DELETE FROM email_domains_block WHERE domain=$1")
            .bind(sDomain)
            .execute(&self.oPool)
            .await?;
        Ok(())
    }

    async fn bIsBlocked(&self, sDomain: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM email_domains_block WHERE domain=$1 AND block_until>CURRENT_TIMESTAMP)",
        )
        .bind(sDomain)
        .fetch_one(&self.oPool)
        .await?)
    }
}
