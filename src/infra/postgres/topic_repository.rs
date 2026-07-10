use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Transaction};

use crate::domain::comment::model::StCommentItem;
use crate::domain::topic::{
    model::{StTopicDetail, StTopicSummary},
    repository::{StEditTopic, StNewTopic, TrTopicRepository},
};
use crate::error::Result;

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
    async fn vecListTopics(&self, optSection: Option<&str>, optGroup: Option<&str>, iOffset: i64, iLimit: i64) -> Result<Vec<StTopicSummary>> {
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
        Ok(sqlx::query_scalar("SELECT nextval('s_msgid')::int").fetch_one(&mut **txPg).await?)
    }

    async fn vInsertTopicMessage(&self, txPg: &mut Transaction<'_, Postgres>, iMsgId: i32, sMessage: &str) -> Result<()> {
        sqlx::query("INSERT INTO msgbase(id, message, markup) VALUES ($1, $2, 'BBCODE_TEX')")
            .bind(iMsgId)
            .bind(sMessage)
            .execute(&mut **txPg)
            .await?;
        Ok(())
    }

    async fn vInsertTopic(&self, txPg: &mut Transaction<'_, Postgres>, stNewTopic: StNewTopic<'_>) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO topics(id, groupid, userid, title, url, postdate, linktext, stat1, stat2, lastmod, moderate)
               VALUES ($1,$2,$3,$4,$5,now(),$6,0,0,now(),true)"#,
        )
        .bind(stNewTopic.iMsgId)
        .bind(stNewTopic.iGroupId)
        .bind(stNewTopic.iUserId)
        .bind(stNewTopic.sTitle)
        .bind(stNewTopic.optUrl)
        .bind(stNewTopic.optLinkText)
        .execute(&mut **txPg)
        .await?;
        Ok(())
    }

    async fn vUpdateTopicMessage(&self, txPg: &mut Transaction<'_, Postgres>, iMsgId: i32, sMessage: &str) -> Result<()> {
        sqlx::query("UPDATE msgbase SET message=$2 WHERE id=$1")
            .bind(iMsgId)
            .bind(sMessage)
            .execute(&mut **txPg)
            .await?;
        Ok(())
    }

    async fn vUpdateTopicHeader(&self, txPg: &mut Transaction<'_, Postgres>, stEditTopic: StEditTopic<'_>) -> Result<()> {
        sqlx::query("UPDATE topics SET title=$2, url=$3, linktext=$4, lastmod=now() WHERE id=$1")
            .bind(stEditTopic.iMsgId)
            .bind(stEditTopic.sTitle)
            .bind(stEditTopic.optUrl)
            .bind(stEditTopic.optLinkText)
            .execute(&mut **txPg)
            .await?;
        Ok(())
    }

    async fn vReplaceTags(&self, txPg: &mut Transaction<'_, Postgres>, iMsgId: i32, optTags: Option<&str>) -> Result<()> {
        sqlx::query("DELETE FROM tags WHERE msgid=$1").bind(iMsgId).execute(&mut **txPg).await?;
        if let Some(sTags) = optTags {
            for sTag in sTags.split(',').map(str::trim).filter(|sTag| !sTag.is_empty()).take(20) {
                let iTagId: i32 = sqlx::query_scalar(
                    r#"INSERT INTO tags_values(value,counter) VALUES ($1,1)
                       ON CONFLICT(value) DO UPDATE SET counter=tags_values.counter+1
                       RETURNING id"#,
                )
                .bind(sTag)
                .fetch_one(&mut **txPg)
                .await?;
                sqlx::query("INSERT INTO tags(msgid, tagid) VALUES ($1,$2) ON CONFLICT DO NOTHING")
                    .bind(iMsgId)
                    .bind(iTagId)
                    .execute(&mut **txPg)
                    .await?;
            }
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
        sqlx::query("UPDATE topics SET moderate=false, commitby=$2, commitdate=now(), lastmod=now() WHERE id=$1")
            .bind(iTopicId)
            .bind(iModeratorId)
            .execute(&self.oPool)
            .await?;
        Ok(())
    }

    async fn vUncommitTopic(&self, iTopicId: i32) -> Result<()> {
        sqlx::query("UPDATE topics SET moderate=true, commitby=NULL, commitdate=NULL, lastmod=now() WHERE id=$1")
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
       CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
       t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
       string_agg(tv.value, ',' ORDER BY tv.value) AS tags
FROM topics t
JOIN users u ON u.id=t.userid
JOIN groups g ON g.id=t.groupid
JOIN sections s ON s.id=g.section
LEFT JOIN tags tg ON tg.msgid=t.id
LEFT JOIN tags_values tv ON tv.id=tg.tagid
WHERE ($1::text IS NULL OR CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END = $1)
  AND ($2::text IS NULL OR g.urlname=$2)
  AND NOT t.deleted
GROUP BY t.id,u.id,g.id,s.id
ORDER BY t.sticky DESC, COALESCE(t.lastmod,t.postdate) DESC
OFFSET $3 LIMIT $4
"#;

const S_GET_TOPIC_SQL: &str = r#"
SELECT t.id, t.title, m.message, (m.markup::text <> 'PLAIN') AS bbcode, t.url, t.linktext, t.postdate, t.lastmod,
       u.id AS author_id, u.nick AS author,
       g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
       s.id AS section_id, s.name AS section_name,
       CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
       t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
       string_agg(tv.value, ',' ORDER BY tv.value) AS tags
FROM topics t
JOIN msgbase m ON m.id=t.id
JOIN users u ON u.id=t.userid
JOIN groups g ON g.id=t.groupid
JOIN sections s ON s.id=g.section
LEFT JOIN tags tg ON tg.msgid=t.id
LEFT JOIN tags_values tv ON tv.id=tg.tagid
WHERE t.id=$1
GROUP BY t.id,m.id,u.id,g.id,s.id
"#;

const S_LIST_COMMENTS_SQL: &str = r#"
SELECT c.id, c.topic, c.replyto, c.title, m.message, c.postdate, u.id AS author_id, u.nick AS author, c.deleted
FROM comments c
JOIN msgbase m ON m.id=c.id
JOIN users u ON u.id=c.userid
WHERE c.topic=$1 AND NOT c.topic_deleted
ORDER BY c.postdate ASC
"#;
