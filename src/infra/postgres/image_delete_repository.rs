use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

use crate::{
    domain::{
        image::{
            StImageDeleteRestrictions, StImageDeleteTarget, StImageReference,
            TrImageDeleteRepository,
        },
        topic::posting::StIpBlockInfo,
    },
    error::{AppError, Result},
};

const S_TARGET_SQL: &str = r#"SELECT
    i.id AS i_image_id,
    i.topic AS i_topic_id,
    i.extension AS s_image_extension,
    t.userid AS i_author_id,
    t.title AS s_topic_title,
    t.deleted AS b_topic_deleted,
    t.draft AS b_draft,
    t.moderate AS b_committed,
    t.sticky AS b_sticky,
    (NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < CURRENT_TIMESTAMP-s.expire) AS b_expired,
    COALESCE(t.postscore,-9999) AS i_post_score,
    t.postdate AS dt_post_date,
    t.commitdate AS opt_commit_date,
    t.lastmod AS dt_last_mod,
    s.id AS i_section_id,
    s.moderate AS b_section_premoderated,
    s.imagepost AS b_section_image_post,
    CASE s.id
      WHEN 1 THEN 'news'
      WHEN 2 THEN 'forum'
      WHEN 3 THEN 'gallery'
      WHEN 5 THEN 'polls'
      WHEN 6 THEN 'articles'
      ELSE lower(s.name)
    END AS s_section_prefix,
    g.urlname AS s_group_url_name,
    m.markup::text AS s_markup,
    COALESCE((
      SELECT array_agg(ai.id ORDER BY ai.main DESC,ai.id)
      FROM images ai WHERE ai.topic=t.id AND NOT ai.deleted
    ),ARRAY[]::integer[]) AS vec_active_image_ids,
    COALESCE((
      SELECT array_agg(ai.extension ORDER BY ai.main DESC,ai.id)
      FROM images ai WHERE ai.topic=t.id AND NOT ai.deleted
    ),ARRAY[]::text[]) AS vec_active_image_extensions
  FROM images i
  JOIN topics t ON t.id=i.topic
  JOIN msgbase m ON m.id=t.id
  JOIN groups g ON g.id=t.groupid
  JOIN sections s ON s.id=g.section
  WHERE i.id=$1"#;

const S_RESTRICTIONS_SQL: &str = r#"SELECT
    COALESCE(u.frozen_until>CURRENT_TIMESTAMP,false),
    COALESCE((
      SELECT bi.ban_date IS NULL OR bi.ban_date>CURRENT_TIMESTAMP
      FROM b_ips bi WHERE bi.ip=$2::inet
    ),false),
    COALESCE((
      SELECT bi.allow_posting
      FROM b_ips bi WHERE bi.ip=$2::inet
    ),false)
  FROM users u WHERE u.id=$1"#;

const S_ACTIVE_IMAGES_FOR_UPDATE_SQL: &str =
    "SELECT id FROM images WHERE topic=$1 AND NOT deleted ORDER BY main DESC,id FOR UPDATE";
const S_DELETE_IMAGE_SQL: &str = "UPDATE images SET deleted=true WHERE id=$1 AND topic=$2";
const S_INSERT_HISTORY_SQL: &str = r#"INSERT INTO edit_info(
    msgid,editor,oldaddimages,object_type
  ) VALUES($1,$2,$3,'TOPIC'::edit_event_type)"#;
const S_UPDATE_LASTMOD_SQL: &str = "UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id=$1";

#[derive(Debug, FromRow)]
struct StTargetRow {
    i_image_id: i32,
    i_topic_id: i32,
    s_image_extension: String,
    i_author_id: i32,
    s_topic_title: String,
    b_topic_deleted: bool,
    b_draft: bool,
    b_committed: bool,
    b_sticky: bool,
    b_expired: bool,
    i_post_score: i32,
    dt_post_date: DateTime<Utc>,
    opt_commit_date: Option<DateTime<Utc>>,
    dt_last_mod: DateTime<Utc>,
    i_section_id: i32,
    b_section_premoderated: bool,
    b_section_image_post: bool,
    s_section_prefix: String,
    s_group_url_name: String,
    s_markup: String,
    vec_active_image_ids: Vec<i32>,
    vec_active_image_extensions: Vec<String>,
}

impl From<StTargetRow> for StImageDeleteTarget {
    fn from(stRow: StTargetRow) -> Self {
        let vecActiveImages = stRow
            .vec_active_image_ids
            .into_iter()
            .zip(stRow.vec_active_image_extensions)
            .map(|(iId, sExtension)| StImageReference { iId, sExtension })
            .collect();
        Self {
            iImageId: stRow.i_image_id,
            iTopicId: stRow.i_topic_id,
            sImageExtension: stRow.s_image_extension,
            iAuthorId: stRow.i_author_id,
            sTopicTitle: stRow.s_topic_title,
            bTopicDeleted: stRow.b_topic_deleted,
            bDraft: stRow.b_draft,
            bCommitted: stRow.b_committed,
            bSticky: stRow.b_sticky,
            bExpired: stRow.b_expired,
            iPostScore: stRow.i_post_score,
            dtPostDate: stRow.dt_post_date,
            optCommitDate: stRow.opt_commit_date,
            dtLastMod: stRow.dt_last_mod,
            iSectionId: stRow.i_section_id,
            bSectionPremoderated: stRow.b_section_premoderated,
            bSectionImagePost: stRow.b_section_image_post,
            sSectionPrefix: stRow.s_section_prefix,
            sGroupUrlName: stRow.s_group_url_name,
            sMarkup: stRow.s_markup,
            vecActiveImages,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CImageDeletePgRepository {
    oPool: PgPool,
}

impl CImageDeletePgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrImageDeleteRepository for CImageDeletePgRepository {
    async fn optTarget(&self, iImageId: i32) -> Result<Option<StImageDeleteTarget>> {
        Ok(sqlx::query_as::<_, StTargetRow>(S_TARGET_SQL)
            .bind(iImageId)
            .fetch_optional(&self.oPool)
            .await?
            .map(Into::into))
    }

    async fn stRestrictions(
        &self,
        iUserId: i32,
        sRemoteIp: &str,
    ) -> Result<StImageDeleteRestrictions> {
        let (bFrozen, bIpBlocked, bAllowRegisteredPosting): (bool, bool, bool) =
            sqlx::query_as(S_RESTRICTIONS_SQL)
                .bind(iUserId)
                .bind(sRemoteIp)
                .fetch_optional(&self.oPool)
                .await?
                .ok_or(AppError::NotFound)?;
        Ok(StImageDeleteRestrictions {
            bFrozen,
            stIpBlock: StIpBlockInfo {
                bBlocked: bIpBlocked,
                bAllowRegisteredPosting: !bIpBlocked || bAllowRegisteredPosting,
            },
        })
    }

    async fn vDelete(&self, iImageId: i32, iTopicId: i32, iEditorId: i32) -> Result<()> {
        let mut oTransaction = self.oPool.begin().await?;
        // `ImageService.deleteImage`: snapshot every active attachment before
        // changing the target, then persist history and lastmod atomically.
        let vecOldImageIds: Vec<i32> = sqlx::query_scalar(S_ACTIVE_IMAGES_FOR_UPDATE_SQL)
            .bind(iTopicId)
            .fetch_all(&mut *oTransaction)
            .await?;
        let stResult = sqlx::query(S_DELETE_IMAGE_SQL)
            .bind(iImageId)
            .bind(iTopicId)
            .execute(&mut *oTransaction)
            .await?;
        if stResult.rows_affected() != 1 {
            return Err(AppError::NotFound);
        }
        sqlx::query(S_INSERT_HISTORY_SQL)
            .bind(iTopicId)
            .bind(iEditorId)
            .bind(&vecOldImageIds)
            .execute(&mut *oTransaction)
            .await?;
        sqlx::query(S_UPDATE_LASTMOD_SQL)
            .bind(iTopicId)
            .execute(&mut *oTransaction)
            .await?;
        oTransaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_query_loads_the_complete_edit_checker_context_and_canonical_section() {
        for sRequired in [
            "t.deleted AS b_topic_deleted",
            "t.draft AS b_draft",
            "t.moderate AS b_committed",
            "b_expired",
            "s.moderate AS b_section_premoderated",
            "s.imagepost AS b_section_image_post",
            "m.markup::text AS s_markup",
            "WHEN 3 THEN 'gallery'",
            "WHEN 5 THEN 'polls'",
            "WHEN 6 THEN 'articles'",
        ] {
            assert!(S_TARGET_SQL.contains(sRequired), "missing {sRequired}");
        }
        assert!(S_TARGET_SQL.contains("ORDER BY ai.main DESC,ai.id"));
        assert!(!S_TARGET_SQL.contains("WHERE i.id=$1 AND NOT i.deleted"));
    }

    #[test]
    fn mutation_is_soft_delete_with_full_image_snapshot_history_and_lastmod() {
        assert!(S_ACTIVE_IMAGES_FOR_UPDATE_SQL.contains("NOT deleted"));
        assert!(S_ACTIVE_IMAGES_FOR_UPDATE_SQL.contains("ORDER BY main DESC,id"));
        assert!(S_ACTIVE_IMAGES_FOR_UPDATE_SQL.contains("FOR UPDATE"));
        assert_eq!(
            S_DELETE_IMAGE_SQL,
            "UPDATE images SET deleted=true WHERE id=$1 AND topic=$2"
        );
        assert!(S_INSERT_HISTORY_SQL.contains("oldaddimages"));
        assert!(S_INSERT_HISTORY_SQL.contains("'TOPIC'::edit_event_type"));
        assert!(S_UPDATE_LASTMOD_SQL.contains("lastmod=CURRENT_TIMESTAMP"));
    }

    #[test]
    fn restrictions_use_current_frozen_and_java_ip_expiry_columns() {
        assert!(S_RESTRICTIONS_SQL.contains("frozen_until>CURRENT_TIMESTAMP"));
        assert!(S_RESTRICTIONS_SQL.contains("ban_date IS NULL"));
        assert!(S_RESTRICTIONS_SQL.contains("ban_date>CURRENT_TIMESTAMP"));
        assert!(S_RESTRICTIONS_SQL.contains("allow_posting"));
        assert!(S_RESTRICTIONS_SQL.contains("ip=$2::inet"));
    }
}
