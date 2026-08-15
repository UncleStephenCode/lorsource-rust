use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    domain::boxlet::{
        model::{
            StGalleryBoxletRow, StPollBoxletRow, StPollVariantResult, StTagCloudRow,
            StTopicBoxletRow,
        },
        repository::TrBoxletRepository,
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct CBoxletPgRepository {
    oPool: PgPool,
}

impl CBoxletPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrBoxletRepository for CBoxletPgRepository {
    async fn vecTopTags(&self, iLimit: i32) -> Result<Vec<StTagCloudRow>> {
        Ok(sqlx::query_as::<_, StTagCloudRow>(
            r#"SELECT value AS "sValue", counter AS "iCounter"
               FROM tags_values
               WHERE counter >= 10
               ORDER BY counter DESC
               LIMIT $1"#,
        )
        .bind(iLimit)
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn vecGalleryItems(&self, iLimit: i32) -> Result<Vec<StGalleryBoxletRow>> {
        Ok(sqlx::query_as::<_, StGalleryBoxletRow>(
            r#"SELECT g.msgid AS "iMsgId",
                      g.userid AS "iUserId",
                      g.title AS "sTitle",
                      g.stat1 AS "iStat",
                      g.urlname AS "sGroupUrlName",
                      g.imageid AS "iImageId",
                      g.extension AS "sExtension"
               FROM (
                   SELECT DISTINCT ON (t.msgid)
                          t.msgid, t.stat1, t.title, t.userid, t.urlname,
                          images.extension, images.id AS imageid, t.commitdate
                   FROM (
                       SELECT topics.id AS msgid, topics.stat1, topics.title,
                              topics.userid, groups.urlname, topics.commitdate
                       FROM topics
                       JOIN groups ON topics.groupid = groups.id
                       WHERE topics.moderate
                         AND groups.section = 3
                         AND NOT topics.deleted
                         AND topics.commitdate IS NOT NULL
                       ORDER BY topics.commitdate DESC
                       LIMIT $1
                   ) AS t
                   JOIN images ON t.msgid = images.topic
                   WHERE NOT images.deleted
                   ORDER BY t.msgid, images.main DESC, images.id, t.commitdate DESC
               ) AS g
               ORDER BY g.commitdate DESC"#,
        )
        .bind(iLimit)
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn sUserNick(&self, iUserId: i32) -> Result<String> {
        Ok(sqlx::query_scalar("SELECT nick FROM users WHERE id=$1")
            .bind(iUserId)
            .fetch_one(&self.oPool)
            .await?)
    }

    async fn vecTopTopics(&self) -> Result<Vec<StTopicBoxletRow>> {
        Ok(sqlx::query_as::<_, StTopicBoxletRow>(
            r#"SELECT topics.id AS "iMsgId",
                      groups.urlname AS "sGroupUrlName",
                      groups.section AS "iSectionId",
                      topics.title AS "sTitle",
                      topics.lastmod AS "dtLastModified",
                      topics.stat1 AS "iCommentCount"
               FROM topics
               JOIN groups ON groups.id=topics.groupid
               WHERE topics.postdate > (CURRENT_TIMESTAMP - '1 month 1 day'::interval)
                 AND NOT topics.deleted
                 AND NOT topics.notop
                 AND topics.open_warnings <= 2
                 AND topics.postscore IS DISTINCT FROM 10002
               ORDER BY topics.stat1 DESC, topics.id
               LIMIT 10"#,
        )
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn vecArticles(&self) -> Result<Vec<StTopicBoxletRow>> {
        Ok(sqlx::query_as::<_, StTopicBoxletRow>(
            r#"SELECT topics.id AS "iMsgId",
                      groups.urlname AS "sGroupUrlName",
                      groups.section AS "iSectionId",
                      topics.title AS "sTitle",
                      topics.lastmod AS "dtLastModified",
                      topics.stat1 AS "iCommentCount"
               FROM topics
               JOIN groups ON groups.id=topics.groupid
               WHERE NOT topics.deleted
                 AND NOT topics.notop
                 AND topics.moderate
                 AND topics.commitdate IS NOT NULL
                 AND topics.postscore IS DISTINCT FROM 10002
                 AND groups.section=6
               ORDER BY topics.commitdate DESC, topics.id
               LIMIT 10"#,
        )
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn optUserSettings(&self, iUserId: i32) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                .bind(iUserId)
                .fetch_optional(&self.oPool)
                .await?,
        )
    }

    async fn vecMostRecentPolls(&self) -> Result<Vec<StPollBoxletRow>> {
        Ok(sqlx::query_as::<_, StPollBoxletRow>(
            r#"SELECT polls.id AS "iPollId",
                      polls.topic AS "iTopicId",
                      polls.multiselect AS "bMultiSelect",
                      topics.title AS "sTitle"
               FROM polls,topics
               WHERE topics.id=polls.topic
                 AND topics.moderate='t'
                 AND topics.deleted='f'
                 AND topics.commitdate=(
                     SELECT max(commitdate)
                     FROM topics
                     WHERE groupid=19387 AND moderate AND NOT deleted
                 )"#,
        )
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn vecPollResults(&self, iPollId: i32, iUserId: i32) -> Result<Vec<StPollVariantResult>> {
        Ok(sqlx::query_as::<_, StPollVariantResult>(
            r#"SELECT v.id AS "iId",
                      v.label AS "sLabel",
                      v.votes AS "iVotes",
                      EXISTS(
                          SELECT 1
                          FROM vote_users u
                          WHERE u.vote=v.vote
                            AND u.variant_id=v.id
                            AND u.userid>0
                            AND u.userid=$2
                          LIMIT 1
                      ) AS "bUserVoted"
               FROM polls_variants v
               WHERE v.vote=$1
               ORDER BY v.id"#,
        )
        .bind(iPollId)
        .bind(iUserId)
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn iPollVotes(&self, iPollId: i32) -> Result<i32> {
        Ok(sqlx::query_scalar::<_, Option<i64>>(
            "SELECT sum(votes)::bigint FROM polls_variants WHERE vote=$1",
        )
        .bind(iPollId)
        .fetch_optional(&self.oPool)
        .await?
        .flatten()
        .unwrap_or(0) as i32)
    }

    async fn iPollUsers(&self, iPollId: i32) -> Result<i32> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT count(DISTINCT userid) FROM vote_users WHERE vote=$1",
        )
        .bind(iPollId)
        .fetch_optional(&self.oPool)
        .await?
        .unwrap_or(0) as i32)
    }
}
