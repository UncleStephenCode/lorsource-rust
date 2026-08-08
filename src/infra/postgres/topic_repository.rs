use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Transaction};

use crate::domain::comment::model::StCommentItem;
use crate::domain::topic::{
    model::{StTopicDetail, StTopicSummary},
    repository::{StEditTopic, StNewTopic, TrTopicRepository},
};
use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct CTopicPgRepository {
    oPool: PgPool,
}

impl CTopicPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrTopicRepository for CTopicPgRepository {
    async fn vecListTopics(
        &self,
        optSection: Option<&str>,
        optGroup: Option<&str>,
        iOffset: i64,
        iLimit: i64,
    ) -> Result<Vec<StTopicSummary>> {
        let vecRows = sqlx::query_as::<_, StTopicSummary>(S_LIST_TOPICS_SQL)
            .bind(optSection)
            .bind(optGroup)
            .bind(iOffset)
            .bind(iLimit)
            .fetch_all(&self.oPool)
            .await?;
        Ok(vecRows)
    }

    async fn stGetTopic(&self, iTopicId: i32) -> Result<StTopicDetail> {
        Ok(sqlx::query_as::<_, StTopicDetail>(S_GET_TOPIC_SQL)
            .bind(iTopicId)
            .fetch_one(&self.oPool)
            .await?)
    }

    async fn vecListComments(&self, iTopicId: i32) -> Result<Vec<StCommentItem>> {
        Ok(sqlx::query_as::<_, StCommentItem>(S_LIST_COMMENTS_SQL)
            .bind(iTopicId)
            .fetch_all(&self.oPool)
            .await?)
    }

    async fn iNextMessageId(&self, txPg: &mut Transaction<'_, Postgres>) -> Result<i32> {
        Ok(sqlx::query_scalar("SELECT nextval('s_msgid')::int")
            .fetch_one(&mut **txPg)
            .await?)
    }

    async fn vInsertTopicMessage(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        sMessage: &str,
        sMarkup: &str,
    ) -> Result<()> {
        sqlx::query("INSERT INTO msgbase(id, message, markup) VALUES ($1, $2, $3::markup_type)")
            .bind(iMsgId)
            .bind(sMessage)
            .bind(sMarkup)
            .execute(&mut **txPg)
            .await?;
        Ok(())
    }

    async fn vInsertTopic(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        stNewTopic: StNewTopic<'_>,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO topics(id, groupid, userid, title, url, postdate, linktext, stat1, stat3, lastmod, moderate, draft)
               VALUES ($1,$2,$3,$4,$5,now(),$6,0,0,now(),false,$7)"#,
        )
        .bind(stNewTopic.iMsgId)
        .bind(stNewTopic.iGroupId)
        .bind(stNewTopic.iUserId)
        .bind(stNewTopic.sTitle)
        .bind(stNewTopic.optUrl)
        .bind(stNewTopic.optLinkText)
        .bind(stNewTopic.bDraft)
        .execute(&mut **txPg)
        .await?;
        Ok(())
    }

    async fn vUpdateTopicMessage(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        sMessage: &str,
    ) -> Result<()> {
        sqlx::query("UPDATE msgbase SET message=$2 WHERE id=$1")
            .bind(iMsgId)
            .bind(sMessage)
            .execute(&mut **txPg)
            .await?;
        Ok(())
    }

    async fn vUpdateTopicHeader(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        stEditTopic: StEditTopic<'_>,
    ) -> Result<()> {
        sqlx::query("UPDATE topics SET title=$2, url=$3, linktext=$4, lastmod=now() WHERE id=$1")
            .bind(stEditTopic.iMsgId)
            .bind(stEditTopic.sTitle)
            .bind(stEditTopic.optUrl)
            .bind(stEditTopic.optLinkText)
            .execute(&mut **txPg)
            .await?;
        Ok(())
    }

    async fn vReplaceTags(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        optTags: Option<&str>,
    ) -> Result<()> {
        let tag_names = match optTags {
            Some(tags) => {
                crate::routes::tags::parse_and_validate_tags(tags).map_err(AppError::BadRequest)?
            }
            None => Vec::new(),
        };

        // TagService.getOrCreateTag resolves a synonym before considering a
        // same-named tag value. Without that step, entering a synonym created
        // a second canonical tag and split its topic counter.
        let mut desired_ids = Vec::with_capacity(tag_names.len());
        for tag in tag_names {
            let existing_id: Option<i32> = sqlx::query_scalar(
                r#"SELECT id FROM (
                     SELECT ts.tagid AS id, 0 AS priority
                     FROM tags_synonyms ts WHERE lower(ts.value)=lower($1)
                     UNION ALL
                     SELECT tv.id, 1 AS priority
                     FROM tags_values tv WHERE lower(tv.value)=lower($1)
                   ) found ORDER BY priority LIMIT 1"#,
            )
            .bind(&tag)
            .fetch_optional(&mut **txPg)
            .await?;
            let tag_id = match existing_id {
                Some(id) => id,
                None => {
                    sqlx::query_scalar(
                        r#"INSERT INTO tags_values(value,counter) VALUES($1,0)
                       ON CONFLICT(value) DO UPDATE SET value=EXCLUDED.value
                       RETURNING id"#,
                    )
                    .bind(&tag)
                    .fetch_one(&mut **txPg)
                    .await?
                }
            };
            if !desired_ids.contains(&tag_id) {
                desired_ids.push(tag_id);
            }
        }

        let old_ids: Vec<i32> = sqlx::query_scalar("SELECT tagid FROM tags WHERE msgid=$1")
            .bind(iMsgId)
            .fetch_all(&mut **txPg)
            .await?;
        sqlx::query("DELETE FROM tags WHERE msgid=$1")
            .bind(iMsgId)
            .execute(&mut **txPg)
            .await?;
        for tag_id in &desired_ids {
            sqlx::query("INSERT INTO tags(msgid,tagid) VALUES($1,$2) ON CONFLICT DO NOTHING")
                .bind(iMsgId)
                .bind(tag_id)
                .execute(&mut **txPg)
                .await?;
        }

        // Recalculate every affected value from the actual relation. The old
        // implementation incremented unchanged tags again on every edit and
        // never corrected removed tags, so counters quickly diverged.
        let mut affected_ids = old_ids;
        for tag_id in desired_ids {
            if !affected_ids.contains(&tag_id) {
                affected_ids.push(tag_id);
            }
        }
        if !affected_ids.is_empty() {
            sqlx::query(
                r#"UPDATE tags_values tv
                   SET counter=(SELECT count(*)::int FROM tags t WHERE t.tagid=tv.id)
                   WHERE tv.id=ANY($1)"#,
            )
            .bind(&affected_ids)
            .execute(&mut **txPg)
            .await?;
        }
        Ok(())
    }

    async fn vSetDeleted(&self, iTopicId: i32, bDeleted: bool) -> Result<()> {
        sqlx::query("UPDATE topics SET deleted=$2 WHERE id=$1")
            .bind(iTopicId)
            .bind(bDeleted)
            .execute(&self.oPool)
            .await?;
        Ok(())
    }

    async fn optResolveMeta(&self, iTopicId: i32) -> Result<Option<(i32, bool)>> {
        Ok(sqlx::query_as::<_, (i32, bool)>(
            "SELECT t.userid, g.resolvable FROM topics t JOIN groups g ON g.id=t.groupid WHERE t.id=$1",
        )
        .bind(iTopicId)
        .fetch_optional(&self.oPool)
        .await?)
    }

    async fn vSetResolved(&self, iTopicId: i32, optResolved: Option<bool>) -> Result<()> {
        if let Some(bResolved) = optResolved {
            sqlx::query("UPDATE topics SET resolved=$2, lastmod=now() WHERE id=$1")
                .bind(iTopicId)
                .bind(bResolved)
                .execute(&self.oPool)
                .await?;
        } else {
            sqlx::query("UPDATE topics SET resolved=COALESCE(NOT resolved, true), lastmod=now() WHERE id=$1")
                .bind(iTopicId)
                .execute(&self.oPool)
                .await?;
        }
        Ok(())
    }

    async fn vCommitTopic(&self, iTopicId: i32, iModeratorId: i32) -> Result<()> {
        sqlx::query("UPDATE topics SET moderate=true, commitby=$2, commitdate=now(), lastmod=now() WHERE id=$1")
            .bind(iTopicId)
            .bind(iModeratorId)
            .execute(&self.oPool)
            .await?;
        Ok(())
    }

    async fn vUncommitTopic(&self, iTopicId: i32) -> Result<()> {
        sqlx::query("UPDATE topics SET moderate=false, commitby=NULL, commitdate=NULL, lastmod=now() WHERE id=$1")
            .bind(iTopicId)
            .execute(&self.oPool)
            .await?;
        Ok(())
    }

    async fn vMoveTopic(&self, iTopicId: i32, iGroupId: i32) -> Result<()> {
        sqlx::query("UPDATE topics SET groupid=$2,lastmod=now() WHERE id=$1")
            .bind(iTopicId)
            .bind(iGroupId)
            .execute(&self.oPool)
            .await?;
        Ok(())
    }
}

const S_LIST_TOPICS_SQL: &str = r#"
SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
       g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
       s.id AS section_id, s.name AS section_name,
       CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
       t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
       (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
          FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
         WHERE tg.msgid=t.id) AS tags
FROM topics t
JOIN users u ON u.id=t.userid
JOIN groups g ON g.id=t.groupid
JOIN sections s ON s.id=g.section
WHERE ($1::text IS NULL OR CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END = $1)
  AND ($2::text IS NULL OR g.urlname=$2)
  AND NOT t.deleted
  AND NOT t.draft
  AND (t.moderate OR NOT s.moderate)
ORDER BY t.sticky DESC, COALESCE(t.lastmod,t.postdate) DESC
OFFSET $3 LIMIT $4
"#;

const S_GET_TOPIC_SQL: &str = r#"
SELECT t.id, t.title, m.message, m.markup::text AS markup, t.url, t.linktext, t.postdate, t.lastmod,
       u.id AS author_id, u.nick AS author,
       g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
       s.id AS section_id, s.name AS section_name,
       CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
       s.moderate AS section_premoderated,
       t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
       (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
          FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
         WHERE tg.msgid=t.id) AS tags,
       t.draft, t.moderate
FROM topics t
JOIN msgbase m ON m.id=t.id
JOIN users u ON u.id=t.userid
JOIN groups g ON g.id=t.groupid
JOIN sections s ON s.id=g.section
WHERE t.id=$1
"#;

// `comments.topic_deleted` was dropped from the real schema years ago (see
// db/migrations/0013) - a deleted topic's own visibility is gated
// separately (render_topic_view checks topics.deleted), so comments are
// listed here regardless of that flag, matching current Java behavior.
const S_LIST_COMMENTS_SQL: &str = r#"
SELECT c.id, c.topic, c.replyto, c.title, m.message, m.markup::text AS markup, c.postdate, u.id AS author_id, u.nick AS author, c.deleted
FROM comments c
JOIN msgbase m ON m.id=c.id
JOIN users u ON u.id=c.userid
WHERE c.topic=$1
ORDER BY c.postdate ASC
"#;
