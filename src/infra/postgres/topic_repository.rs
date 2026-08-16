use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::HashMap;

use crate::domain::comment::model::{StCommentItem, StCommentPageMeta};
use crate::domain::topic::{
    model::{
        StLegacyTopicRedirect, StMainTopicSummary, StRssContext, StRssImage, StRssPoll,
        StRssPollVariant, StRssTag, StRssTopic, StTopicDetail, StTopicScrollerItem,
        StTopicScrollers, StTopicSummary,
    },
    repository::{StNewTopic, TrTopicRepository},
};
use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct CTopicPgRepository {
    oPool: PgPool,
}

#[derive(sqlx::FromRow)]
struct StRssTopicRow {
    iId: i32,
    sStoredTitle: String,
    dtPublished: DateTime<Utc>,
    dtLastModified: DateTime<Utc>,
    sAuthorNick: String,
    sGroupUrlName: String,
    sSectionPrefix: String,
    sMessage: String,
    sMarkup: String,
    bImagePost: bool,
    bImagesAllowed: bool,
    bPollPostAllowed: bool,
    iAuthorScore: i32,
    bAuthorBlocked: bool,
    bAuthorAnonymous: bool,
    bAuthorFrozen: bool,
    bCommitted: bool,
}

#[derive(sqlx::FromRow)]
struct StMainTopicRow {
    id: i32,
    title: String,
    url: Option<String>,
    postdate: DateTime<Utc>,
    lastmod: Option<DateTime<Utc>>,
    author_id: i32,
    author: String,
    group_id: i32,
    group_title: String,
    group_urlname: String,
    section_id: i32,
    section_name: String,
    section_prefix: String,
    comments: i32,
    deleted: bool,
    sticky: bool,
    resolved: Option<bool>,
    tags: Option<String>,
    minor: bool,
}

#[derive(sqlx::FromRow)]
struct StCommentPageMetaRow {
    i_comment_id: i32,
    opt_remark: Option<String>,
    i_edit_count: i32,
    opt_edit_date: Option<DateTime<Utc>>,
    opt_editor_nick: Option<String>,
    opt_post_ip: Option<String>,
    i_user_agent_id: i32,
    opt_user_agent: Option<String>,
    s_warnings_json: String,
}

impl From<StCommentPageMetaRow> for StCommentPageMeta {
    fn from(stRow: StCommentPageMetaRow) -> Self {
        Self {
            iCommentId: stRow.i_comment_id,
            optRemark: stRow.opt_remark,
            iEditCount: stRow.i_edit_count,
            optEditDate: stRow.opt_edit_date,
            optEditorNick: stRow.opt_editor_nick,
            optPostIp: stRow.opt_post_ip,
            iUserAgentId: stRow.i_user_agent_id,
            optUserAgent: stRow.opt_user_agent,
            sWarningsJson: stRow.s_warnings_json,
        }
    }
}

impl From<StMainTopicRow> for StMainTopicSummary {
    fn from(stRow: StMainTopicRow) -> Self {
        Self {
            stTopic: StTopicSummary {
                id: stRow.id,
                title: stRow.title,
                url: stRow.url,
                postdate: stRow.postdate,
                lastmod: stRow.lastmod,
                author_id: stRow.author_id,
                author: stRow.author,
                group_id: stRow.group_id,
                group_title: stRow.group_title,
                group_urlname: stRow.group_urlname,
                section_id: stRow.section_id,
                section_name: stRow.section_name,
                section_prefix: stRow.section_prefix,
                comments: stRow.comments,
                deleted: stRow.deleted,
                sticky: stRow.sticky,
                resolved: stRow.resolved,
                tags: stRow.tags,
            },
            bMinor: stRow.minor,
        }
    }
}

#[derive(sqlx::FromRow)]
struct StRssTagRow {
    iTopicId: i32,
    sName: String,
    iCounter: i32,
}

#[derive(sqlx::FromRow)]
struct StRssImageRow {
    iTopicId: i32,
    iId: i32,
    sExtension: String,
}

#[derive(sqlx::FromRow)]
struct StRssPollRow {
    iTopicId: i32,
    bMultiSelect: bool,
    iVoterCount: i64,
}

#[derive(sqlx::FromRow)]
struct StRssPollVariantRow {
    iTopicId: i32,
    sLabel: String,
    iVotes: i32,
}

impl CTopicPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

fn stTopicScrollerItem(
    (iId, sStoredTitle, sGroupUrlName, sSectionPrefix): (i32, String, String, String),
) -> StTopicScrollerItem {
    StTopicScrollerItem {
        iId,
        sStoredTitle,
        sGroupUrlName,
        sSectionPrefix,
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
        bNoTalks: bool,
        bTech: bool,
    ) -> Result<Vec<StTopicSummary>> {
        let vecRows = sqlx::query_as::<_, StTopicSummary>(S_LIST_TOPICS_SQL)
            .bind(optSection)
            .bind(optGroup)
            .bind(iOffset)
            .bind(iLimit)
            .bind(bNoTalks)
            .bind(bTech)
            .fetch_all(&self.oPool)
            .await?;
        Ok(vecRows)
    }

    async fn vecListMainTopics(
        &self,
        bShowGalleryOnMain: bool,
        optViewerId: Option<i32>,
        iLimit: i64,
    ) -> Result<Vec<StMainTopicSummary>> {
        let vecRows = sqlx::query_as::<_, StMainTopicRow>(S_LIST_MAIN_TOPICS_SQL)
            .bind(bShowGalleryOnMain)
            .bind(optViewerId)
            .bind(iLimit)
            .fetch_all(&self.oPool)
            .await?;
        Ok(vecRows.into_iter().map(Into::into).collect())
    }

    async fn stRssContext(&self, iSectionId: i32, iGroupId: i32) -> Result<StRssContext> {
        let sSectionName: String = sqlx::query_scalar("SELECT name FROM sections WHERE id=$1")
            .bind(iSectionId)
            .fetch_optional(&self.oPool)
            .await?
            .ok_or(AppError::NotFound)?;

        let optGroupTitle = if iGroupId == 0 {
            None
        } else {
            let (sGroupTitle, iGroupSection): (String, i32) =
                sqlx::query_as("SELECT title, section FROM groups WHERE id=$1")
                    .bind(iGroupId)
                    .fetch_optional(&self.oPool)
                    .await?
                    .ok_or(AppError::NotFound)?;
            if iGroupSection != iSectionId {
                return Err(AppError::Anyhow(anyhow::anyhow!(
                    "group #{iGroupId} does not belong to section #{iSectionId}"
                )));
            }
            Some(sGroupTitle)
        };

        Ok(StRssContext {
            sSectionName,
            optGroupTitle,
        })
    }

    async fn vecListRssTopics(
        &self,
        iSectionId: i32,
        iGroupId: i32,
        bNoTalks: bool,
        bTech: bool,
        optViewerId: Option<i32>,
        dtFrom: DateTime<Utc>,
    ) -> Result<Vec<StRssTopic>> {
        let vecRows = sqlx::query_as::<_, StRssTopicRow>(S_LIST_RSS_TOPICS_SQL)
            .bind(iSectionId)
            .bind(iGroupId)
            .bind(bNoTalks)
            .bind(bTech)
            .bind(optViewerId)
            .bind(dtFrom)
            .fetch_all(&self.oPool)
            .await?;
        if vecRows.is_empty() {
            return Ok(Vec::new());
        }

        let vecTopicIds = vecRows.iter().map(|stRow| stRow.iId).collect::<Vec<_>>();
        let mut mapTags = HashMap::<i32, Vec<StRssTag>>::new();
        for stRow in sqlx::query_as::<_, StRssTagRow>(
            r#"SELECT tg.msgid AS "iTopicId", tv.value AS "sName",
                      COALESCE(tv.counter,0) AS "iCounter"
                 FROM tags tg
                 JOIN tags_values tv ON tv.id=tg.tagid
                WHERE tg.msgid=ANY($1)
                ORDER BY tv.value"#,
        )
        .bind(&vecTopicIds)
        .fetch_all(&self.oPool)
        .await?
        {
            mapTags.entry(stRow.iTopicId).or_default().push(StRssTag {
                sName: stRow.sName,
                iCounter: stRow.iCounter,
            });
        }

        let mut mapImages = HashMap::<i32, Vec<StRssImage>>::new();
        for stRow in sqlx::query_as::<_, StRssImageRow>(
            r#"SELECT topic AS "iTopicId", id AS "iId", extension AS "sExtension"
                 FROM images
                WHERE topic=ANY($1) AND NOT deleted
                ORDER BY topic, main DESC, id"#,
        )
        .bind(&vecTopicIds)
        .fetch_all(&self.oPool)
        .await?
        {
            mapImages
                .entry(stRow.iTopicId)
                .or_default()
                .push(StRssImage {
                    iId: stRow.iId,
                    sExtension: stRow.sExtension,
                });
        }

        let mut mapPolls = HashMap::<i32, StRssPoll>::new();
        for stRow in sqlx::query_as::<_, StRssPollRow>(
            r#"SELECT p.topic AS "iTopicId", p.multiselect AS "bMultiSelect",
                      (SELECT count(DISTINCT vu.userid)
                         FROM vote_users vu WHERE vu.vote=p.id) AS "iVoterCount"
                 FROM polls p
                WHERE p.topic=ANY($1)"#,
        )
        .bind(&vecTopicIds)
        .fetch_all(&self.oPool)
        .await?
        {
            mapPolls.insert(
                stRow.iTopicId,
                StRssPoll {
                    bMultiSelect: stRow.bMultiSelect,
                    iVoterCount: stRow.iVoterCount,
                    vecVariants: Vec::new(),
                },
            );
        }
        for stRow in sqlx::query_as::<_, StRssPollVariantRow>(
            r#"SELECT p.topic AS "iTopicId", pv.label AS "sLabel", pv.votes AS "iVotes"
                 FROM polls p
                 JOIN polls_variants pv ON pv.vote=p.id
                WHERE p.topic=ANY($1)
                ORDER BY p.topic, pv.votes DESC, pv.id"#,
        )
        .bind(&vecTopicIds)
        .fetch_all(&self.oPool)
        .await?
        {
            if let Some(stPoll) = mapPolls.get_mut(&stRow.iTopicId) {
                stPoll.vecVariants.push(StRssPollVariant {
                    sLabel: stRow.sLabel,
                    iVotes: stRow.iVotes,
                });
            }
        }

        Ok(vecRows
            .into_iter()
            .map(|stRow| {
                let iTopicId = stRow.iId;
                let bNofollow = !crate::domain::topic::link_policy::StAuthorLinkState {
                    iScore: stRow.iAuthorScore,
                    bBlocked: stRow.bAuthorBlocked,
                    bAnonymous: stRow.bAuthorAnonymous,
                    bFrozen: stRow.bAuthorFrozen,
                }
                .bFollowInTopic(stRow.bCommitted);
                StRssTopic {
                    iId: iTopicId,
                    sStoredTitle: stRow.sStoredTitle,
                    dtPublished: stRow.dtPublished,
                    dtLastModified: stRow.dtLastModified,
                    sAuthorNick: stRow.sAuthorNick,
                    sGroupUrlName: stRow.sGroupUrlName,
                    sSectionPrefix: stRow.sSectionPrefix,
                    sMessage: stRow.sMessage,
                    sMarkup: stRow.sMarkup,
                    bImagePost: stRow.bImagePost,
                    bImagesAllowed: stRow.bImagesAllowed,
                    bPollPostAllowed: stRow.bPollPostAllowed,
                    bNofollow,
                    vecTags: mapTags.remove(&iTopicId).unwrap_or_default(),
                    vecImages: mapImages.remove(&iTopicId).unwrap_or_default(),
                    optPoll: mapPolls.remove(&iTopicId),
                }
            })
            .collect())
    }

    async fn stGetTopic(&self, iTopicId: i32) -> Result<StTopicDetail> {
        Ok(sqlx::query_as::<_, StTopicDetail>(S_GET_TOPIC_SQL)
            .bind(iTopicId)
            .fetch_one(&self.oPool)
            .await?)
    }

    async fn stLegacyTopicRedirect(&self, iTopicId: i32) -> Result<StLegacyTopicRedirect> {
        sqlx::query_as::<_, StLegacyTopicRedirect>(
            r#"SELECT t.id AS "iTopicId", g.urlname AS "sGroupUrlName",
                      CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum'
                           WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls'
                           WHEN 6 THEN 'articles' ELSE lower(s.name)
                      END AS "sSectionPrefix",
                      t.lastmod AS "dtLastModified",
                      (NOT t.sticky
                       AND COALESCE(t.commitdate,t.postdate)
                           < CURRENT_TIMESTAMP-s.expire) AS "bExpired"
                 FROM topics t
                 JOIN groups g ON g.id=t.groupid
                 JOIN sections s ON s.id=g.section
                WHERE t.id=$1"#,
        )
        .bind(iTopicId)
        .fetch_optional(&self.oPool)
        .await?
        .ok_or(AppError::NotFound)
    }

    async fn stTopicScrollers(
        &self,
        iTopicId: i32,
        optViewerIdForIgnoreList: Option<i32>,
    ) -> Result<StTopicScrollers> {
        type TyContext = (bool, String, DateTime<Utc>, Option<DateTime<Utc>>, i32, i32);
        type TyItem = (i32, String, String, String);

        let Some((bSticky, sScrollMode, dtPostDate, optCommitDate, iGroupId, iSectionId)) =
            sqlx::query_as::<_, TyContext>(
                r#"SELECT t.sticky, s.scroll_mode, t.postdate, t.commitdate,
                          t.groupid, s.id
                     FROM topics t JOIN groups g ON g.id=t.groupid
                     JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
            )
            .bind(iTopicId)
            .fetch_optional(&self.oPool)
            .await?
        else {
            return Err(AppError::NotFound);
        };
        let bEnabled = sScrollMode != "NO_SCROLL";
        if !bEnabled || bSticky {
            return Ok(StTopicScrollers {
                bEnabled,
                optPrevious: None,
                optNext: None,
            });
        }

        let (optPrevious, optNext): (Option<TyItem>, Option<TyItem>) = match sScrollMode.as_str() {
            "SECTION" => {
                let Some(dtCommitDate) = optCommitDate else {
                    return Ok(StTopicScrollers {
                        bEnabled: true,
                        optPrevious: None,
                        optNext: None,
                    });
                };
                let optPrevious = sqlx::query_as(
                    r#"SELECT t.id, t.title, g.urlname,
                              CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum'
                                   WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls'
                                   WHEN 6 THEN 'articles' ELSE lower(s.name) END
                         FROM topics t
                         JOIN groups g ON g.id=t.groupid
                         JOIN sections s ON s.id=g.section
                        WHERE NOT t.draft AND t.commitdate<$1 AND g.section=$2
                          AND (t.moderate OR NOT s.moderate)
                          AND NOT t.deleted AND NOT t.sticky
                        ORDER BY t.commitdate DESC LIMIT 1"#,
                )
                .bind(dtCommitDate)
                .bind(iSectionId)
                .fetch_optional(&self.oPool)
                .await?;
                let optNext = sqlx::query_as(
                    r#"SELECT t.id, t.title, g.urlname,
                              CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum'
                                   WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls'
                                   WHEN 6 THEN 'articles' ELSE lower(s.name) END
                         FROM topics t
                         JOIN groups g ON g.id=t.groupid
                         JOIN sections s ON s.id=g.section
                        WHERE NOT t.draft AND t.commitdate>$1 AND g.section=$2
                          AND (t.moderate OR NOT s.moderate)
                          AND NOT t.deleted AND NOT t.sticky
                        ORDER BY t.commitdate ASC LIMIT 1"#,
                )
                .bind(dtCommitDate)
                .bind(iSectionId)
                .fetch_optional(&self.oPool)
                .await?;
                (optPrevious, optNext)
            }
            "GROUP" => {
                let optPrevious = sqlx::query_as(
                    r#"SELECT t.id, t.title, g.urlname,
                              CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum'
                                   WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls'
                                   WHEN 6 THEN 'articles' ELSE lower(s.name) END
                         FROM topics t
                         JOIN groups g ON g.id=t.groupid
                         JOIN sections s ON s.id=g.section
                        WHERE NOT t.draft AND t.postdate<$1 AND t.groupid=$2
                          AND NOT t.deleted AND NOT t.sticky
                          AND ($3::integer IS NULL OR t.userid NOT IN
                              (SELECT ignored FROM ignore_list WHERE userid=$3))
                        ORDER BY t.postdate DESC LIMIT 1"#,
                )
                .bind(dtPostDate)
                .bind(iGroupId)
                .bind(optViewerIdForIgnoreList)
                .fetch_optional(&self.oPool)
                .await?;
                let optNext = sqlx::query_as(
                    r#"SELECT t.id, t.title, g.urlname,
                              CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum'
                                   WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls'
                                   WHEN 6 THEN 'articles' ELSE lower(s.name) END
                         FROM topics t
                         JOIN groups g ON g.id=t.groupid
                         JOIN sections s ON s.id=g.section
                        WHERE NOT t.draft AND t.postdate>$1 AND t.groupid=$2
                          AND NOT t.deleted AND NOT t.sticky
                          AND ($3::integer IS NULL OR t.userid NOT IN
                              (SELECT ignored FROM ignore_list WHERE userid=$3))
                        ORDER BY t.postdate ASC LIMIT 1"#,
                )
                .bind(dtPostDate)
                .bind(iGroupId)
                .bind(optViewerIdForIgnoreList)
                .fetch_optional(&self.oPool)
                .await?;
                (optPrevious, optNext)
            }
            // Only the three current Java enum values are valid. Treat a
            // stale port-only value as disabled instead of inventing order.
            _ => {
                return Ok(StTopicScrollers {
                    bEnabled: false,
                    optPrevious: None,
                    optNext: None,
                });
            }
        };
        Ok(StTopicScrollers {
            bEnabled: true,
            optPrevious: optPrevious.map(stTopicScrollerItem),
            optNext: optNext.map(stTopicScrollerItem),
        })
    }

    async fn vecListComments(&self, iTopicId: i32) -> Result<Vec<StCommentItem>> {
        Ok(sqlx::query_as::<_, StCommentItem>(S_LIST_COMMENTS_SQL)
            .bind(iTopicId)
            .fetch_all(&self.oPool)
            .await?)
    }

    async fn vecCommentPageMeta(
        &self,
        vecCommentIds: &[i32],
        optViewerId: Option<i32>,
        bModeratorSession: bool,
        bLoadWarnings: bool,
    ) -> Result<Vec<StCommentPageMeta>> {
        if vecCommentIds.is_empty() {
            return Ok(Vec::new());
        }
        let vecRows = sqlx::query_as::<_, StCommentPageMetaRow>(S_COMMENT_PAGE_META_SQL)
            .bind(vecCommentIds)
            .bind(optViewerId)
            .bind(bModeratorSession)
            .bind(bLoadWarnings)
            .fetch_all(&self.oPool)
            .await?;
        Ok(vecRows.into_iter().map(Into::into).collect())
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
            r#"INSERT INTO topics(id, groupid, userid, title, url, postdate, linktext,
                                  stat1, stat3, lastmod, moderate, draft, postip, ua_id,
                                  allow_anonymous)
               VALUES ($1,$2,$3,$4,$5,now(),$6,0,0,now(),false,$7,$8::inet,
                       create_user_agent($9),$10)"#,
        )
        .bind(stNewTopic.iMsgId)
        .bind(stNewTopic.iGroupId)
        .bind(stNewTopic.iUserId)
        .bind(stNewTopic.sTitle)
        .bind(stNewTopic.optUrl)
        .bind(stNewTopic.optLinkText)
        .bind(stNewTopic.bDraft)
        .bind(stNewTopic.sPostIp)
        .bind(stNewTopic.optUserAgent.map(|sValue| {
            let mut iEnd = sValue.len().min(511);
            while !sValue.is_char_boundary(iEnd) {
                iEnd -= 1;
            }
            &sValue[..iEnd]
        }))
        .bind(stNewTopic.bAllowAnonymous)
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
  AND ((s.moderate AND t.commitdate IS NOT NULL) OR NOT s.moderate)
  AND ($1::text IS DISTINCT FROM 'forum' OR $2::text IS NOT NULL OR t.postdate > CURRENT_TIMESTAMP - interval '6 months')
  AND (NOT $5::boolean OR t.groupid<>8404)
  AND (NOT $6::boolean OR t.groupid NOT IN (8404,4068,9326,19405))
ORDER BY CASE WHEN s.moderate THEN t.commitdate ELSE t.postdate END DESC
OFFSET $3 LIMIT $4
"#;

// TopicListService.getMainPage uses a single CommittedOnly request.  Its DAO
// therefore filters/sorts on commitdate (not lastmod, postdate or sticky)
// across all selected sections before applying the 30-row limit.
const S_LIST_MAIN_TOPICS_SQL: &str = r#"
SELECT t.id, t.title, t.url, t.postdate, t.lastmod,
       u.id AS author_id, u.nick AS author,
       g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
       s.id AS section_id, s.name AS section_name,
       CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery'
            WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
       t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
       (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
          FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
         WHERE tg.msgid=t.id) AS tags,
       t.minor
FROM topics t
JOIN users u ON u.id=t.userid
JOIN groups g ON g.id=t.groupid
JOIN sections s ON s.id=g.section
WHERE NOT t.deleted
  AND NOT t.draft
  AND ($2::integer IS NOT NULL OR t.open_warnings<=2)
  AND s.moderate
  AND t.commitdate IS NOT NULL
  AND (($1::boolean AND s.id IN (1,3,5,6)) OR (NOT $1::boolean AND s.id=1))
  AND t.commitdate>=CURRENT_TIMESTAMP-interval '3 months'
ORDER BY t.commitdate DESC
LIMIT $3
"#;

const S_LIST_RSS_TOPICS_SQL: &str = r#"
SELECT t.id AS "iId", t.title AS "sStoredTitle",
       CASE WHEN s.moderate THEN t.commitdate ELSE t.postdate END AS "dtPublished",
       t.lastmod AS "dtLastModified", u.nick AS "sAuthorNick",
       g.urlname AS "sGroupUrlName",
       CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery'
            WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS "sSectionPrefix",
       mb.message AS "sMessage", mb.markup::text AS "sMarkup",
       s.imagepost AS "bImagePost", s.imageallowed AS "bImagesAllowed",
       COALESCE(s.vote,false) AS "bPollPostAllowed",
       COALESCE(u.score,0) AS "iAuthorScore",
       COALESCE(u.blocked,false) AS "bAuthorBlocked",
       COALESCE(u.passwd,'')='' AS "bAuthorAnonymous",
       COALESCE(u.frozen_until > CURRENT_TIMESTAMP,false) AS "bAuthorFrozen",
       t.moderate AS "bCommitted"
FROM topics t
JOIN msgbase mb ON mb.id=t.id
JOIN users u ON u.id=t.userid
JOIN groups g ON g.id=t.groupid
JOIN sections s ON s.id=g.section
WHERE s.id=$1
  AND ($2=0 OR g.id=$2)
  AND NOT t.deleted
  AND NOT t.draft
  AND ($5::integer IS NOT NULL OR t.open_warnings <= 2)
  AND ((s.moderate AND t.commitdate IS NOT NULL) OR NOT s.moderate)
  AND CASE WHEN s.moderate THEN t.commitdate ELSE t.postdate END >= $6
  AND (s.moderate OR $5::integer IS NULL OR t.userid NOT IN (
        SELECT ignored FROM ignore_list WHERE userid=$5
      ))
  AND (NOT $3 OR t.groupid<>8404)
  AND (NOT $4 OR t.groupid NOT IN (8404,4068,9326,19405))
ORDER BY CASE WHEN s.moderate THEN t.commitdate ELSE t.postdate END DESC
LIMIT 30
"#;

const S_GET_TOPIC_SQL: &str = r#"
SELECT t.id, t.title, m.message, m.markup::text AS markup, t.url, t.linktext, t.postdate, t.lastmod,
       u.id AS author_id, u.nick AS author,
       COALESCE(u.score,0) AS author_score,
       COALESCE(u.blocked,false) AS author_blocked,
       COALESCE(u.passwd,'')='' AS author_anonymous,
       COALESCE(u.frozen_until > CURRENT_TIMESTAMP,false) AS author_frozen,
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
SELECT c.id, c.topic, c.replyto, c.title, m.message, m.markup::text AS markup,
       c.postdate, u.id AS author_id, u.nick AS author,
       COALESCE(u.score,0) AS author_score,
       COALESCE(u.blocked,false) AS author_blocked,
       COALESCE(u.passwd,'')='' AS author_anonymous,
       COALESCE(u.frozen_until > CURRENT_TIMESTAMP,false) AS author_frozen,
       c.deleted
FROM comments c
JOIN msgbase m ON m.id=c.id
JOIN users u ON u.id=c.userid
WHERE c.topic=$1
ORDER BY c.id ASC
"#;

// Viewer-specific parts of Java `CommentPrepareService.prepareCommentList`.
// The route supplies only IDs from the already selected page, and moderator
// fields are suppressed in SQL as well as in the template-facing model.
const S_COMMENT_PAGE_META_SQL: &str = r#"
SELECT c.id AS i_comment_id,
       remark.remark_text AS opt_remark,
       COALESCE(c.edit_count,0) AS i_edit_count,
       (c.edit_date AT TIME ZONE 'UTC') AS opt_edit_date,
       editor.nick AS opt_editor_nick,
       CASE WHEN $3 THEN host(c.postip) END AS opt_post_ip,
       CASE WHEN $3 THEN COALESCE(c.ua_id,0) ELSE 0 END AS i_user_agent_id,
       CASE WHEN $3 THEN user_agent.name END AS opt_user_agent,
       CASE WHEN $4 THEN COALESCE((
         SELECT jsonb_agg(jsonb_build_object(
           'id',warning.id,'postdate',warning.postdate,'message',warning.message,
           'warning_type',warning.warning_type::text,
           'author',warning_author.nick,
           'author_blocked',COALESCE(warning_author.blocked,false),
           'closed_by',closed_by.nick,
           'closed_by_blocked',COALESCE(closed_by.blocked,false)
         ) ORDER BY warning.postdate)
           FROM message_warnings warning
           JOIN users warning_author ON warning_author.id=warning.author
           LEFT JOIN users closed_by ON closed_by.id=warning.closed_by
          WHERE warning.comment=c.id
       ),'[]'::jsonb) ELSE '[]'::jsonb END::text AS s_warnings_json
  FROM comments c
  LEFT JOIN users editor ON editor.id=c.editor_id
  LEFT JOIN user_agents user_agent ON user_agent.id=c.ua_id
  LEFT JOIN user_remarks remark ON remark.user_id=$2 AND remark.ref_user_id=c.userid
 WHERE c.id=ANY($1)
"#;

#[cfg(test)]
mod listing_contract_tests {
    use super::{
        S_COMMENT_PAGE_META_SQL, S_LIST_COMMENTS_SQL, S_LIST_MAIN_TOPICS_SQL, S_LIST_TOPICS_SQL,
    };

    #[test]
    fn comment_list_uses_java_msgid_order_for_paging_and_jumps() {
        assert!(S_LIST_COMMENTS_SQL.contains("ORDER BY c.id ASC"));
        assert!(!S_LIST_COMMENTS_SQL.contains("ORDER BY c.postdate"));
    }

    #[test]
    fn java_timestamp_without_timezone_edit_date_is_decoded_as_utc() {
        assert!(S_COMMENT_PAGE_META_SQL.contains("c.edit_date AT TIME ZONE 'UTC'"));
    }

    #[test]
    fn comment_page_meta_covers_prepared_comment_viewer_fields() {
        for sToken in [
            "remark.remark_text",
            "c.edit_count",
            "c.edit_date",
            "editor.nick",
            "host(c.postip)",
            "c.ua_id",
            "user_agent.name",
            "message_warnings",
            "warning.comment=c.id",
        ] {
            assert!(S_COMMENT_PAGE_META_SQL.contains(sToken), "missing {sToken}");
        }
        assert!(S_COMMENT_PAGE_META_SQL.contains("CASE WHEN $3"));
        assert!(S_COMMENT_PAGE_META_SQL.contains("CASE WHEN $4"));
    }

    #[test]
    fn forum_feed_sql_keeps_java_filters_and_six_month_window() {
        assert!(S_LIST_TOPICS_SQL.contains("t.groupid<>8404"));
        assert!(S_LIST_TOPICS_SQL.contains("t.groupid NOT IN (8404,4068,9326,19405)"));
        assert!(S_LIST_TOPICS_SQL.contains(
            "$1::text IS DISTINCT FROM 'forum' OR $2::text IS NOT NULL OR t.postdate > CURRENT_TIMESTAMP - interval '6 months'"
        ));
    }

    #[test]
    fn main_feed_is_one_java_committed_only_query_ordered_by_commitdate() {
        let sSql = S_LIST_MAIN_TOPICS_SQL
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(sSql.contains("s.moderate AND t.commitdate IS NOT NULL"));
        assert!(sSql.contains("s.id IN (1,3,5,6)"));
        assert!(sSql.contains("t.commitdate>=CURRENT_TIMESTAMP-interval '3 months'"));
        assert!(sSql.contains("ORDER BY t.commitdate DESC LIMIT $3"));
        assert!(!sSql.contains("ORDER BY t.lastmod"));
        assert!(!sSql.contains("ORDER BY t.sticky"));
    }
}
