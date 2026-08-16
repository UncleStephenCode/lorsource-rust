use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    domain::{
        tag::{
            model::{
                StTagForumTopic, StTagInfo, StTagSection, StTagViewerProfile, StTagViewerState,
            },
            repository::TrTagTopicListRepository,
        },
        topic::model::StTopicSummary,
    },
    error::Result,
};

#[derive(Debug, Clone)]
pub struct CTagTopicListPgRepository {
    oPool: PgPool,
}

impl CTagTopicListPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

const S_SECTION_SELECT: &str = r#"
SELECT s.id AS "iId", s.name AS "sName",
       CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery'
            WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS "sUrlName",
       s.moderate AS "bPremoderated",
       COALESCE(s.restrict_topics,-9999) AS "iTopicsRestriction"
  FROM sections s
"#;

#[async_trait]
impl TrTagTopicListRepository for CTagTopicListPgRepository {
    async fn optSection(&self, iSectionId: i32) -> Result<Option<StTagSection>> {
        Ok(
            sqlx::query_as::<_, StTagSection>(sqlx::AssertSqlSafe(format!(
                "{S_SECTION_SELECT} WHERE s.id=$1"
            )))
            .bind(iSectionId)
            .fetch_optional(&self.oPool)
            .await?,
        )
    }

    async fn optTagInfo(&self, sTag: &str) -> Result<Option<StTagInfo>> {
        // TagDao.getTagId(tag, skipZero=true) is deliberately exact-case and
        // hides a zero-counter tag from moderators too on this controller.
        Ok(sqlx::query_as::<_, StTagInfo>(
            r#"SELECT id AS "iId",value AS "sName"
                 FROM tags_values WHERE value=$1 AND counter>0"#,
        )
        .bind(sTag)
        .fetch_optional(&self.oPool)
        .await?)
    }

    async fn optSynonymTarget(&self, sTag: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            r#"SELECT tv.value
                 FROM tags_synonyms ts JOIN tags_values tv ON tv.id=ts.tagid
                WHERE ts.value=$1"#,
        )
        .bind(sTag)
        .fetch_optional(&self.oPool)
        .await?)
    }

    async fn vecTagSections(&self, iTagId: i32) -> Result<Vec<StTagSection>> {
        // TopicTagDao.getTagSections intentionally does not filter an
        // uncommitted topic; only deleted/draft topics are excluded.
        Ok(
            sqlx::query_as::<_, StTagSection>(sqlx::AssertSqlSafe(format!(
                r#"{S_SECTION_SELECT}
                WHERE s.id IN (
                    SELECT DISTINCT g.section
                      FROM groups g
                      JOIN topics t ON t.groupid=g.id
                      JOIN tags tg ON tg.msgid=t.id
                     WHERE tg.tagid=$1 AND NOT t.deleted AND NOT t.draft
                )
                ORDER BY s.id"#
            )))
            .bind(iTagId)
            .fetch_all(&self.oPool)
            .await?,
        )
    }

    async fn stViewerProfile(&self, optViewerId: Option<i32>) -> Result<StTagViewerProfile> {
        let optSettings = match optViewerId {
            Some(iViewerId) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT settings::text FROM user_settings WHERE id=$1",
                )
                .bind(iViewerId)
                .fetch_optional(&self.oPool)
                .await?
            }
            None => None,
        };
        let stProfile = crate::profile::ProfileSettings::from_hstore_text(optSettings);
        Ok(StTagViewerProfile {
            iTopics: stProfile.topics,
            iMessages: stProfile.messages,
            bOldTracker: stProfile.old_tracker,
        })
    }

    async fn stViewerState(
        &self,
        iTagId: i32,
        optViewerId: Option<i32>,
    ) -> Result<StTagViewerState> {
        let (bFavorite, bIgnored, iFavoritesCount, iIgnoreCount): (bool, bool, i64, i64) =
            sqlx::query_as(
                r#"SELECT
                     ($2::int IS NOT NULL AND EXISTS(
                         SELECT 1 FROM user_tags
                          WHERE user_id=$2 AND tag_id=$1 AND is_favorite
                     )),
                     ($2::int IS NOT NULL AND EXISTS(
                         SELECT 1 FROM user_tags
                          WHERE user_id=$2 AND tag_id=$1 AND NOT is_favorite
                     )),
                     (SELECT count(*) FROM user_tags WHERE tag_id=$1 AND is_favorite),
                     (SELECT count(*) FROM user_tags WHERE tag_id=$1 AND NOT is_favorite)"#,
            )
            .bind(iTagId)
            .bind(optViewerId)
            .fetch_one(&self.oPool)
            .await?;
        Ok(StTagViewerState {
            bFavorite,
            bIgnored,
            iFavoritesCount,
            iIgnoreCount,
        })
    }

    async fn vecFeedTopics(
        &self,
        stSection: &StTagSection,
        iTagId: i32,
        optViewerId: Option<i32>,
        iOffset: i32,
        iLimit: i32,
    ) -> Result<Vec<StTopicSummary>> {
        Ok(sqlx::query_as::<_, StTopicSummary>(
            r#"SELECT t.id,t.title,t.url,t.postdate,t.lastmod,
                      u.id AS author_id,u.nick AS author,
                      g.id AS group_id,g.title AS group_title,g.urlname AS group_urlname,
                      s.id AS section_id,s.name AS section_name,
                      CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery'
                           WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
                      t.stat1 AS comments,t.deleted,t.sticky,t.resolved,
                      (SELECT string_agg(tv.value,',' ORDER BY tv.value)
                         FROM tags all_tags JOIN tags_values tv ON tv.id=all_tags.tagid
                        WHERE all_tags.msgid=t.id) AS tags
                 FROM topics t
                 JOIN users u ON u.id=t.userid
                 JOIN groups g ON g.id=t.groupid
                 JOIN sections s ON s.id=g.section
                WHERE s.id=$1 AND t.id IN (SELECT msgid FROM tags WHERE tagid=$2)
                  AND NOT t.deleted AND NOT t.draft
                  AND (($3 AND s.moderate AND t.commitdate IS NOT NULL)
                       OR (NOT $3 AND NOT s.moderate))
                  AND ($4::int IS NOT NULL OR t.open_warnings<=2)
                  AND ($4::int IS NULL OR $3 OR t.userid NOT IN (
                      SELECT ignored FROM ignore_list WHERE userid=$4
                  ))
                ORDER BY CASE WHEN $3 THEN t.commitdate ELSE t.postdate END DESC
                LIMIT $5 OFFSET $6"#,
        )
        .bind(stSection.iId)
        .bind(iTagId)
        .bind(stSection.bPremoderated)
        .bind(optViewerId)
        .bind(iLimit)
        .bind(iOffset)
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn vecForumTopics(
        &self,
        stSection: &StTagSection,
        iTagId: i32,
        optViewerId: Option<i32>,
        iOffset: i32,
        iLimit: i32,
    ) -> Result<Vec<StTagForumTopic>> {
        Ok(sqlx::query_as::<_, StTagForumTopic>(
            r#"WITH selected_topics AS (
                SELECT t.id,t.title,t.userid,t.stat1,t.resolved,t.deleted,t.postscore,
                       t.postdate AS topic_postdate,t.moderate,
                       u.nick AS topic_author,COALESCE(u.blocked,false) AS topic_author_blocked,
                       g.title AS group_title,g.urlname AS group_urlname,
                       s.name AS section_name,s.id AS section_id,s.moderate AS section_moderate
                  FROM topics t
                  JOIN users u ON u.id=t.userid
                  JOIN groups g ON g.id=t.groupid
                  JOIN sections s ON s.id=g.section
                 WHERE s.id=$1 AND t.id IN (SELECT msgid FROM tags WHERE tagid=$2)
                   AND NOT t.deleted AND NOT t.draft
                   AND (t.moderate OR NOT s.moderate)
                   AND ($3::int IS NOT NULL OR t.open_warnings<=2)
                   AND ($3::int IS NULL OR t.userid NOT IN (
                       SELECT ignored FROM ignore_list WHERE userid=$3
                   ))
                   AND ($3::int IS NULL OR NOT (
                       EXISTS (
                           SELECT 1 FROM tags topic_tag
                           JOIN user_tags ignored_tag ON ignored_tag.tag_id=topic_tag.tagid
                            WHERE topic_tag.msgid=t.id AND ignored_tag.user_id=$3
                              AND NOT ignored_tag.is_favorite
                       ) AND NOT EXISTS (
                           SELECT 1 FROM tags topic_tag
                           JOIN user_tags favorite_tag ON favorite_tag.tag_id=topic_tag.tagid
                            WHERE topic_tag.msgid=t.id AND favorite_tag.user_id=$3
                              AND favorite_tag.is_favorite
                       )
                   ))
                 ORDER BY t.postdate DESC
                 LIMIT $4 OFFSET $5
            )
            SELECT st.id AS "iTopicId",st.title AS "sStoredTitle",
                   st.topic_author AS "sTopicAuthor",st.topic_author_blocked AS "bTopicAuthorBlocked",
                   COALESCE(lu.nick,st.topic_author) AS "sLastAuthor",
                   COALESCE(lu.blocked,st.topic_author_blocked) AS "bLastAuthorBlocked",
                   st.group_title AS "sGroupTitle",st.group_urlname AS "sGroupUrlName",
                   CASE st.section_id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery'
                        WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(st.section_name) END AS "sSectionUrlName",
                   COALESCE(lc.postdate,st.topic_postdate) AS "dtLastPost",
                   st.stat1 AS "iCommentCount",lc.id AS "optLastCommentId",
                   COALESCE(st.resolved,false) AS "bResolved",
                   (st.section_moderate AND NOT st.moderate) AS "bUncommitted",
                   st.deleted AS "bDeleted",COALESCE(st.postscore,-9999) AS "iTopicPostscore",
                   (SELECT string_agg(tag_names.value,',' ORDER BY tag_names.value)
                      FROM (
                          SELECT tv.value FROM tags all_tags
                          JOIN tags_values tv ON tv.id=all_tags.tagid
                           WHERE all_tags.msgid=st.id ORDER BY tv.value LIMIT 3
                      ) tag_names) AS "optTags"
              FROM selected_topics st
              LEFT JOIN LATERAL (
                  SELECT c.id,c.userid,c.postdate
                    FROM comments c
                   WHERE c.topic=st.id AND NOT c.deleted
                     AND ($3::int IS NULL OR NOT EXISTS (
                         SELECT ignored FROM ignore_list WHERE userid=$3
                         INTERSECT SELECT get_branch_authors(c.id)
                     ))
                   ORDER BY c.postdate DESC
                   LIMIT 1
              ) lc ON st.postscore IS DISTINCT FROM 10002
              LEFT JOIN users lu ON lu.id=lc.userid
             ORDER BY st.topic_postdate DESC"#,
        )
        .bind(stSection.iId)
        .bind(iTagId)
        .bind(optViewerId)
        .bind(iLimit)
        .bind(iOffset)
        .fetch_all(&self.oPool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_contract_keeps_java_tag_section_visibility_rules() {
        let sSource = include_str!("tag_topic_list_repository.rs");
        assert!(sSource.contains("WHERE value=$1 AND counter>0"));
        assert!(sSource.contains("NOT t.deleted AND NOT t.draft"));
        assert!(sSource.contains("t.commitdate IS NOT NULL"));
        assert!(sSource.contains("INTERSECT SELECT get_branch_authors(c.id)"));
        assert!(sSource.contains("ORDER BY st.topic_postdate DESC"));
        assert!(sSource.contains("LIMIT 3"));
    }

    #[tokio::test]
    #[ignore = "requires an explicitly selected disposable Java/Liquibase PostgreSQL database"]
    async fn canonical_database_serves_a_real_tag_section_page() {
        assert_eq!(
            std::env::var("LOR_TAG_SECTION_INTEGRATION_CONFIRM").as_deref(),
            Ok("read-disposable-tag-fixture"),
            "set LOR_TAG_SECTION_INTEGRATION_CONFIRM=read-disposable-tag-fixture"
        );
        let sDatabaseUrl = std::env::var("LOR_TAG_SECTION_INTEGRATION_DATABASE_URL")
            .expect("set LOR_TAG_SECTION_INTEGRATION_DATABASE_URL to a disposable canonical DB");
        let oPool = PgPool::connect(&sDatabaseUrl)
            .await
            .expect("disposable canonical database must be reachable");
        let (sTag, iSectionId): (String, i32) = sqlx::query_as(
            r#"SELECT tv.value,g.section
                 FROM tags_values tv
                 JOIN tags tg ON tg.tagid=tv.id
                 JOIN topics t ON t.id=tg.msgid
                 JOIN groups g ON g.id=t.groupid
                 JOIN sections s ON s.id=g.section
                WHERE tv.counter>0 AND NOT t.deleted AND NOT t.draft
                  AND ((s.moderate AND t.commitdate IS NOT NULL) OR NOT s.moderate)
                  AND t.open_warnings<=2
                ORDER BY tv.value,g.section,t.postdate DESC LIMIT 1"#,
        )
        .fetch_one(&oPool)
        .await
        .expect("fixture needs one visible tagged topic");
        let oRepository = CTagTopicListPgRepository::new(oPool);
        let stSection = oRepository
            .optSection(iSectionId)
            .await
            .expect("section lookup")
            .expect("known section");
        let stTag = oRepository
            .optTagInfo(&sTag)
            .await
            .expect("tag lookup")
            .expect("positive tag");
        let vecSections = oRepository
            .vecTagSections(stTag.iId)
            .await
            .expect("tag sections");
        assert!(
            vecSections
                .iter()
                .any(|stCandidate| stCandidate.iId == iSectionId)
        );
        let iLoaded = if iSectionId == 2 {
            oRepository
                .vecForumTopics(&stSection, stTag.iId, None, 0, 30)
                .await
                .expect("forum tag topics")
                .len()
        } else {
            oRepository
                .vecFeedTopics(&stSection, stTag.iId, None, 0, 20)
                .await
                .expect("feed tag topics")
                .len()
        };
        assert!(iLoaded > 0);
    }
}
