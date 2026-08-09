use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    domain::edit_history::{
        StCommentHistorySource, StEditHistoryRow, StHistoryPoll, StTopicHistorySource,
        TrEditHistoryRepository,
    },
    error::{AppError, Result},
};

#[derive(Clone)]
pub struct CEditHistoryPgRepository {
    pool: PgPool,
}

impl CEditHistoryPgRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TrEditHistoryRepository for CEditHistoryPgRepository {
    async fn stTopicSource(&self, iTopicId: i32) -> Result<StTopicHistorySource> {
        type TyTopicRow = (
            String,
            chrono::DateTime<chrono::Utc>,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            bool,
            Vec<String>,
            Vec<i32>,
        );
        let stRow: TyTopicRow = sqlx::query_as(
            r#"SELECT u.nick,t.postdate,t.title,m.message,m.markup::text,
                      t.url,t.linktext,t.minor,
                      COALESCE((SELECT array_agg(tv.value ORDER BY tv.value)
                                  FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                                 WHERE tg.msgid=t.id), ARRAY[]::text[]),
                      COALESCE((SELECT array_agg(i.id ORDER BY i.main DESC,i.id)
                                  FROM images i WHERE i.topic=t.id AND NOT i.deleted), ARRAY[]::int[])
                 FROM topics t JOIN msgbase m ON m.id=t.id JOIN users u ON u.id=t.userid
                WHERE t.id=$1"#,
        )
        .bind(iTopicId)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;

        let optPollRow: Option<(i32, bool)> =
            sqlx::query_as("SELECT id,multiselect FROM polls WHERE topic=$1")
                .bind(iTopicId)
                .fetch_optional(&self.pool)
                .await?;
        let optPoll = if let Some((iPollId, bMultiSelect)) = optPollRow {
            let vecVariants: Vec<String> =
                sqlx::query_scalar("SELECT label FROM polls_variants WHERE vote=$1 ORDER BY id")
                    .bind(iPollId)
                    .fetch_all(&self.pool)
                    .await?;
            Some(StHistoryPoll {
                bMultiSelect,
                vecVariants,
            })
        } else {
            None
        };

        Ok(StTopicHistorySource {
            sAuthor: stRow.0,
            dtPost: stRow.1,
            sTitle: stRow.2,
            sMessage: stRow.3,
            sMarkup: stRow.4,
            optUrl: stRow.5,
            optLinkText: stRow.6,
            bMinor: stRow.7,
            vecTags: stRow.8,
            vecImageIds: stRow.9,
            optPoll,
        })
    }

    async fn stCommentSource(&self, iCommentId: i32) -> Result<StCommentHistorySource> {
        type TyCommentRow = (
            i32,
            String,
            chrono::DateTime<chrono::Utc>,
            String,
            String,
            String,
        );
        let stRow: TyCommentRow = sqlx::query_as(
            r#"SELECT c.topic,u.nick,c.postdate,c.title,m.message,m.markup::text
                 FROM comments c JOIN msgbase m ON m.id=c.id JOIN users u ON u.id=c.userid
                WHERE c.id=$1"#,
        )
        .bind(iCommentId)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        Ok(StCommentHistorySource {
            iTopicId: stRow.0,
            sAuthor: stRow.1,
            dtPost: stRow.2,
            sTitle: stRow.3,
            sMessage: stRow.4,
            sMarkup: stRow.5,
        })
    }

    async fn vecRows(&self, iMessageId: i32, sObjectType: &str) -> Result<Vec<StEditHistoryRow>> {
        type TyEditRow = (
            i32,
            String,
            chrono::DateTime<chrono::Utc>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<bool>,
            Option<serde_json::Value>,
            Option<Vec<i32>>,
            Option<i32>,
        );
        let vecRows: Vec<TyEditRow> = sqlx::query_as(
            r#"SELECT e.id,u.nick,e.editdate,e.oldmessage,e.oldtitle,e.oldtags,
                      e.oldlinktext,e.oldurl,e.oldminor,e.oldpoll,e.oldaddimages,e.oldimage
                 FROM edit_info e JOIN users u ON u.id=e.editor
                WHERE e.msgid=$1 AND e.object_type=$2::edit_event_type
                ORDER BY e.id DESC"#,
        )
        .bind(iMessageId)
        .bind(sObjectType)
        .fetch_all(&self.pool)
        .await?;
        Ok(vecRows
            .into_iter()
            .map(|stRow| StEditHistoryRow {
                iId: stRow.0,
                sEditor: stRow.1,
                dtEdit: stRow.2,
                optOldMessage: stRow.3,
                optOldTitle: stRow.4,
                optOldTags: stRow.5,
                optOldLinkText: stRow.6,
                optOldUrl: stRow.7,
                optOldMinor: stRow.8,
                optOldPoll: stRow.9,
                optOldAdditionalImages: stRow.10,
                optLegacyMainImage: stRow.11,
            })
            .collect())
    }

    async fn sRestorableTopicMessage(&self, iTopicId: i32, iRecordId: i32) -> Result<String> {
        let optRow: Option<(Option<String>,)> = sqlx::query_as(
            r#"SELECT oldmessage FROM edit_info
                WHERE msgid=$1 AND id=$2 AND object_type='TOPIC'::edit_event_type"#,
        )
        .bind(iTopicId)
        .bind(iRecordId)
        .fetch_optional(&self.pool)
        .await?;
        optRow
            .map(|stRow| stRow.0.unwrap_or_default())
            .ok_or(AppError::NotFound)
    }
}
