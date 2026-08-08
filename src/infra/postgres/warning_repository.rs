use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    domain::warning::{
        model::{
            EnWarningType, StClearWarningMutation, StCreateWarningMutation, StWarningRecord,
            StWarningTopic,
        },
        repository::TrWarningRepository,
    },
    error::Result,
};

type TyWarningTopicRow = (i32, bool, bool, i32, bool, bool, bool, String, String);

#[derive(Debug, Clone)]
pub struct CWarningPgRepository {
    oPool: PgPool,
}

impl CWarningPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrWarningRepository for CWarningPgRepository {
    async fn optTopic(&self, iTopicId: i32) -> Result<Option<StWarningTopic>> {
        let optRow: Option<TyWarningTopicRow> = sqlx::query_as(
            r#"SELECT t.userid,t.deleted,t.draft,COALESCE(t.postscore,-9999),
                      (NOT t.sticky AND COALESCE(t.commitdate,t.postdate)<CURRENT_TIMESTAMP-s.expire),
                      COALESCE(s.moderate,false),COALESCE(t.moderate,false),g.urlname,
                      CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery'
                        WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END
               FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
               WHERE t.id=$1"#,
        )
        .bind(iTopicId)
        .fetch_optional(&self.oPool)
        .await?;
        Ok(optRow.map(
            |(
                iAuthorId,
                bDeleted,
                bDraft,
                iPostScore,
                bExpired,
                bPremoderated,
                bCommitted,
                sGroupUrl,
                sSectionPrefix,
            )| StWarningTopic {
                iId: iTopicId,
                iAuthorId,
                bDeleted,
                bDraft,
                iPostScore,
                bExpired,
                bPremoderated,
                bCommitted,
                sGroupUrl,
                sSectionPrefix,
            },
        ))
    }

    async fn optCommentDeleted(&self, iTopicId: i32, iCommentId: i32) -> Result<Option<bool>> {
        Ok(
            sqlx::query_scalar("SELECT deleted FROM comments WHERE id=$1 AND topic=$2")
                .bind(iCommentId)
                .bind(iTopicId)
                .fetch_optional(&self.oPool)
                .await?,
        )
    }

    async fn bUserFrozen(&self, iUserId: i32) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(frozen_until>CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
        )
        .bind(iUserId)
        .fetch_one(&self.oPool)
        .await?)
    }

    async fn iRecentWarnings(&self, iUserId: i32) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT count(*) FROM message_warnings WHERE postdate>CURRENT_TIMESTAMP-interval '1 hour' AND author=$1",
        )
        .bind(iUserId)
        .fetch_one(&self.oPool)
        .await?)
    }

    async fn iCreate(&self, stMutation: StCreateWarningMutation) -> Result<i32> {
        let mut stTransaction = self.oPool.begin().await?;
        let iWarningId: i32 = sqlx::query_scalar(
            "INSERT INTO message_warnings(topic,comment,author,message,warning_type) VALUES($1,$2,$3,$4,$5::warning_type) RETURNING id",
        )
        .bind(stMutation.iTopicId)
        .bind(stMutation.optCommentId)
        .bind(stMutation.iAuthorId)
        .bind(&stMutation.sMessage)
        .bind(stMutation.enWarningType.sId())
        .fetch_one(&mut *stTransaction)
        .await?;

        let bNotifyCorrectors = matches!(
            stMutation.enWarningType,
            EnWarningType::Tag | EnWarningType::Spelling
        );
        let vecRecipients: Vec<i32> = sqlx::query_scalar(
            r#"SELECT id FROM users
               WHERE (canmod OR ($1 AND corrector))
                 AND lastlogin>CURRENT_TIMESTAMP-interval '30 days'
               ORDER BY id"#,
        )
        .bind(bNotifyCorrectors)
        .fetch_all(&mut *stTransaction)
        .await?;
        let sEventMessage = format!(
            "[{}] {}",
            stMutation.enWarningType.sName(),
            stMutation.sMessage
        );
        for iRecipientId in &vecRecipients {
            sqlx::query(
                r#"INSERT INTO user_events(userid,type,private,message_id,comment_id,message,origin_user,warning_id)
                   VALUES($1,'WARNING',true,$2,$3,$4,$5,$6)"#,
            )
            .bind(iRecipientId)
            .bind(stMutation.iTopicId)
            .bind(stMutation.optCommentId)
            .bind(&sEventMessage)
            .bind(stMutation.iAuthorId)
            .bind(iWarningId)
            .execute(&mut *stTransaction)
            .await?;
        }
        if !vecRecipients.is_empty() {
            sqlx::query(
                "UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=ANY($1)",
            )
            .bind(&vecRecipients)
            .execute(&mut *stTransaction)
            .await?;
        }
        if stMutation.optCommentId.is_none() {
            vUpdateTopicWarnings(&mut stTransaction, stMutation.iTopicId).await?;
        }
        stTransaction.commit().await?;
        Ok(iWarningId)
    }

    async fn optWarning(&self, iWarningId: i32) -> Result<Option<StWarningRecord>> {
        let optRow: Option<(i32, Option<i32>)> =
            sqlx::query_as("SELECT topic,comment FROM message_warnings WHERE id=$1")
                .bind(iWarningId)
                .fetch_optional(&self.oPool)
                .await?;
        Ok(optRow.map(|(iTopicId, optCommentId)| StWarningRecord {
            iTopicId,
            optCommentId,
        }))
    }

    async fn vClear(&self, stMutation: StClearWarningMutation) -> Result<()> {
        let mut stTransaction = self.oPool.begin().await?;
        sqlx::query(
            "UPDATE message_warnings SET closed_by=$2, closed_when=now() WHERE id=$1 AND closed_by IS NULL",
        )
        .bind(stMutation.iWarningId)
        .bind(stMutation.iActorId)
        .execute(&mut *stTransaction)
        .await?;
        if stMutation.optCommentId.is_none() {
            vUpdateTopicWarnings(&mut stTransaction, stMutation.iTopicId).await?;
        }
        stTransaction.commit().await?;
        Ok(())
    }
}

async fn vUpdateTopicWarnings(
    stTransaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    iTopicId: i32,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE topics SET lastmod=CURRENT_TIMESTAMP,open_warnings=(
            SELECT count(DISTINCT mw.author) FROM message_warnings mw
            WHERE mw.topic=topics.id AND mw.comment IS NULL AND mw.closed_by IS NULL AND mw.warning_type='rule'
        ) WHERE id=$1"#,
    )
    .bind(iTopicId)
    .execute(&mut **stTransaction)
    .await?;
    Ok(())
}
