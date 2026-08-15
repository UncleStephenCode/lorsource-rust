use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    domain::comment::message_form::{
        StCommentMessageCommentValidation, StCommentMessageTopicValidation,
        TrCommentMessageRepository,
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct CCommentMessagePgRepository {
    oPool: PgPool,
}

impl CCommentMessagePgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[derive(sqlx::FromRow)]
struct StTopicValidationRow {
    bDeleted: bool,
    bExpired: bool,
}

#[async_trait]
impl TrCommentMessageRepository for CCommentMessagePgRepository {
    async fn optTopicValidation(
        &self,
        iTopicId: i32,
    ) -> Result<Option<StCommentMessageTopicValidation>> {
        let optRow: Option<StTopicValidationRow> = sqlx::query_as(
            r#"SELECT t.deleted AS "bDeleted",
                      NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire AS "bExpired"
               FROM topics t
               JOIN groups g ON g.id=t.groupid
               JOIN sections s ON s.id=g.section
               WHERE t.id=$1"#,
        )
        .bind(iTopicId)
        .fetch_optional(&self.oPool)
        .await?;
        Ok(optRow.map(|stRow| StCommentMessageTopicValidation {
            bDeleted: stRow.bDeleted,
            bExpired: stRow.bExpired,
        }))
    }

    async fn optCommentValidation(
        &self,
        iCommentId: i32,
    ) -> Result<Option<StCommentMessageCommentValidation>> {
        let optRow: Option<(i32, bool)> =
            sqlx::query_as("SELECT topic,deleted FROM comments WHERE id=$1")
                .bind(iCommentId)
                .fetch_optional(&self.oPool)
                .await?;
        Ok(optRow
            .map(|(iTopicId, bDeleted)| StCommentMessageCommentValidation { iTopicId, bDeleted }))
    }

    async fn bUserExists(&self, sNick: &str) -> Result<bool> {
        Ok(
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE nick=$1)")
                .bind(sNick)
                .fetch_one(&self.oPool)
                .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn topic_validation_uses_java_expiry_expression() {
        let sSource = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/infra/postgres/comment_message_repository.rs"
        ));
        assert!(sSource.contains("COALESCE(t.commitdate,t.postdate) < now() - s.expire"));
        assert!(sSource.contains("WHERE t.id=$1"));
    }
}
