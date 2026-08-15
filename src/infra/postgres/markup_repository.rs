use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    domain::markup::{
        model::{StMarkupSource, StMarkupUser},
        repository::TrMarkupUserRepository,
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct CMarkupUserPgRepository {
    oPool: PgPool,
}

impl CMarkupUserPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrMarkupUserRepository for CMarkupUserPgRepository {
    async fn vecFindByNicks(&self, vecNicks: &[String]) -> Result<Vec<StMarkupUser>> {
        if vecNicks.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as::<_, StMarkupUser>(
            r#"SELECT requested.nick AS "sInputNick",
                      users.nick AS "sCanonicalNick",
                      COALESCE(users.blocked,false) AS "bBlocked"
               FROM unnest($1::text[]) AS requested(nick)
               JOIN users ON users.nick=requested.nick"#,
        )
        .bind(vecNicks)
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn vecSourcesByMessageIds(&self, vecMessageIds: &[i32]) -> Result<Vec<StMarkupSource>> {
        if vecMessageIds.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as::<_, StMarkupSource>(
            r#"SELECT message AS "sMessage", markup::text AS "sMarkup"
               FROM msgbase
               WHERE id=ANY($1)"#,
        )
        .bind(vecMessageIds)
        .fetch_all(&self.oPool)
        .await?)
    }
}
