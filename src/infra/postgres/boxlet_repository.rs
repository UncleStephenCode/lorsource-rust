use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    domain::boxlet::{
        model::{StGalleryBoxletRow, StTagCloudRow},
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
}
