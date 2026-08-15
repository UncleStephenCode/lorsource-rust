use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

use crate::{
    domain::topic::moderation::{
        EnTopicMarkup, EnTopicMoveGroupScope, StMoveTopicCommand, StTopicModerationSnapshot,
        StTopicMoveGroup, TrTopicModerationRepository, sMoveInfo,
    },
    error::{AppError, Result},
};

const S_TOPIC_SNAPSHOT_SQL: &str = r#"
SELECT t.id AS i_topic_id,
       t.userid AS i_author_id,
       u.nick AS s_author_nick,
       COALESCE(u.score,0) AS i_author_score,
       COALESCE(u.blocked,false) AS b_author_blocked,
       t.title AS s_stored_title,
       m.message AS s_message,
       m.markup::text AS s_markup,
       t.url AS opt_url,
       t.linktext AS opt_link_text,
       t.groupid AS i_group_id,
       g.title AS s_group_title,
       g.urlname AS s_group_url_name,
       s.id AS i_section_id,
       CASE s.id
         WHEN 1 THEN 'news'
         WHEN 2 THEN 'forum'
         WHEN 3 THEN 'gallery'
         WHEN 5 THEN 'polls'
         WHEN 6 THEN 'articles'
         ELSE lower(s.name)
       END AS s_section_prefix,
       s.moderate AS b_section_premoderated,
       COALESCE(s.vote,false) AS b_section_poll_allowed,
       s.havelink AS b_links_allowed,
       g.resolvable AS b_group_resolvable,
       t.deleted AS b_deleted,
       t.moderate AS b_committed,
       t.sticky AS b_sticky,
       (NOT t.sticky AND
        COALESCE(t.commitdate,t.postdate)<CURRENT_TIMESTAMP-s.expire) AS b_expired,
       t.lastmod AS dt_last_mod
  FROM topics t
  JOIN msgbase m ON m.id=t.id
  JOIN users u ON u.id=t.userid
  JOIN groups g ON g.id=t.groupid
  JOIN sections s ON s.id=g.section
 WHERE t.id=$1
"#;

const S_MOVE_GROUP_SQL: &str = r#"
SELECT g.id AS i_id,
       g.title AS s_title,
       s.id AS i_section_id,
       s.name AS s_section_title,
       s.havelink AS b_links_allowed,
       g.resolvable AS b_resolvable
  FROM groups g
  JOIN sections s ON s.id=g.section
 WHERE g.id=$1
"#;

const S_FORUM_ARTICLES_GROUPS_SQL: &str = r#"
SELECT g.id AS i_id,
       g.title AS s_title,
       s.id AS i_section_id,
       s.name AS s_section_title,
       s.havelink AS b_links_allowed,
       g.resolvable AS b_resolvable
  FROM sections s
  JOIN groups g ON g.section=s.id
 WHERE s.id IN (2,6)
 ORDER BY CASE s.id WHEN 2 THEN 0 WHEN 6 THEN 1 END,g.id
"#;

const S_CURRENT_SECTION_GROUPS_SQL: &str = r#"
SELECT g.id AS i_id,
       g.title AS s_title,
       s.id AS i_section_id,
       s.name AS s_section_title,
       s.havelink AS b_links_allowed,
       g.resolvable AS b_resolvable
  FROM sections s
  JOIN groups g ON g.section=s.id
 WHERE s.id=$1
 ORDER BY g.id
"#;

const S_PREMODERATED_GROUPS_SQL: &str = r#"
SELECT g.id AS i_id,
       g.title AS s_title,
       s.id AS i_section_id,
       s.name AS s_section_title,
       s.havelink AS b_links_allowed,
       g.resolvable AS b_resolvable
  FROM sections s
  JOIN groups g ON g.section=s.id
 WHERE s.moderate AND NOT COALESCE(s.vote,false)
 ORDER BY s.id,g.id
"#;

const S_UNCOMMIT_SQL: &str = r#"
UPDATE topics
   SET moderate=false,
       commitby=NULL,
       commitdate=NULL
 WHERE id=$1
"#;

const S_LOCK_TOPIC_GROUP_SQL: &str = "SELECT groupid FROM topics WHERE id=$1 FOR UPDATE";
const S_MOVE_TOPIC_SQL: &str = "UPDATE topics SET groupid=$2,lastmod=CURRENT_TIMESTAMP WHERE id=$1";
const S_CLEAR_TOPIC_LINK_SQL: &str = "UPDATE topics SET linktext=NULL,url=NULL WHERE id=$1";
const S_LOAD_MOVE_MARKUP_SQL: &str = "SELECT markup::text FROM msgbase WHERE id=$1";
const S_APPEND_MOVE_INFO_SQL: &str = "UPDATE msgbase SET message=message||$2 WHERE id=$1";
const S_RESOLVE_SQL: &str = r#"
UPDATE topics
   SET resolved=$2,
       lastmod=lastmod+'1 second'::interval
 WHERE id=$1
"#;

#[derive(Debug, FromRow)]
struct StTopicSnapshotRow {
    i_topic_id: i32,
    i_author_id: i32,
    s_author_nick: String,
    i_author_score: i32,
    b_author_blocked: bool,
    s_stored_title: String,
    s_message: String,
    s_markup: String,
    opt_url: Option<String>,
    opt_link_text: Option<String>,
    i_group_id: i32,
    s_group_title: String,
    s_group_url_name: String,
    i_section_id: i32,
    s_section_prefix: String,
    b_section_premoderated: bool,
    b_section_poll_allowed: bool,
    b_links_allowed: bool,
    b_group_resolvable: bool,
    b_deleted: bool,
    b_committed: bool,
    b_sticky: bool,
    b_expired: bool,
    dt_last_mod: DateTime<Utc>,
}

impl From<StTopicSnapshotRow> for StTopicModerationSnapshot {
    fn from(stRow: StTopicSnapshotRow) -> Self {
        Self {
            iTopicId: stRow.i_topic_id,
            iAuthorId: stRow.i_author_id,
            sAuthorNick: stRow.s_author_nick,
            iAuthorScore: stRow.i_author_score,
            bAuthorBlocked: stRow.b_author_blocked,
            sStoredTitle: stRow.s_stored_title,
            sMessage: stRow.s_message,
            sMarkup: stRow.s_markup,
            optUrl: stRow.opt_url,
            optLinkText: stRow.opt_link_text,
            iGroupId: stRow.i_group_id,
            sGroupTitle: stRow.s_group_title,
            sGroupUrlName: stRow.s_group_url_name,
            iSectionId: stRow.i_section_id,
            sSectionPrefix: stRow.s_section_prefix,
            bSectionPremoderated: stRow.b_section_premoderated,
            bSectionPollAllowed: stRow.b_section_poll_allowed,
            bLinksAllowed: stRow.b_links_allowed,
            bGroupResolvable: stRow.b_group_resolvable,
            bDeleted: stRow.b_deleted,
            bCommitted: stRow.b_committed,
            bSticky: stRow.b_sticky,
            bExpired: stRow.b_expired,
            dtLastMod: stRow.dt_last_mod,
        }
    }
}

#[derive(Debug, FromRow)]
struct StMoveGroupRow {
    i_id: i32,
    s_title: String,
    i_section_id: i32,
    s_section_title: String,
    b_links_allowed: bool,
    b_resolvable: bool,
}

impl From<StMoveGroupRow> for StTopicMoveGroup {
    fn from(stRow: StMoveGroupRow) -> Self {
        Self {
            iId: stRow.i_id,
            sTitle: stRow.s_title,
            iSectionId: stRow.i_section_id,
            sSectionTitle: stRow.s_section_title,
            bLinksAllowed: stRow.b_links_allowed,
            bResolvable: stRow.b_resolvable,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CTopicModerationPgRepository {
    oPool: PgPool,
}

impl CTopicModerationPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrTopicModerationRepository for CTopicModerationPgRepository {
    async fn optSnapshot(&self, iTopicId: i32) -> Result<Option<StTopicModerationSnapshot>> {
        Ok(
            sqlx::query_as::<_, StTopicSnapshotRow>(S_TOPIC_SNAPSHOT_SQL)
                .bind(iTopicId)
                .fetch_optional(&self.oPool)
                .await?
                .map(Into::into),
        )
    }

    async fn optMoveGroup(&self, iGroupId: i32) -> Result<Option<StTopicMoveGroup>> {
        Ok(sqlx::query_as::<_, StMoveGroupRow>(S_MOVE_GROUP_SQL)
            .bind(iGroupId)
            .fetch_optional(&self.oPool)
            .await?
            .map(Into::into))
    }

    async fn vecMoveGroups(&self, enScope: EnTopicMoveGroupScope) -> Result<Vec<StTopicMoveGroup>> {
        let vecRows = match enScope {
            EnTopicMoveGroupScope::ForumAndArticles => {
                sqlx::query_as::<_, StMoveGroupRow>(S_FORUM_ARTICLES_GROUPS_SQL)
                    .fetch_all(&self.oPool)
                    .await?
            }
            EnTopicMoveGroupScope::CurrentSection(iSectionId) => {
                sqlx::query_as::<_, StMoveGroupRow>(S_CURRENT_SECTION_GROUPS_SQL)
                    .bind(iSectionId)
                    .fetch_all(&self.oPool)
                    .await?
            }
            EnTopicMoveGroupScope::PremoderatedNonPoll => {
                sqlx::query_as::<_, StMoveGroupRow>(S_PREMODERATED_GROUPS_SQL)
                    .fetch_all(&self.oPool)
                    .await?
            }
        };
        Ok(vecRows.into_iter().map(Into::into).collect())
    }

    async fn vUncommit(&self, iTopicId: i32) -> Result<()> {
        let mut oTransaction = self.oPool.begin().await?;
        sqlx::query(S_UNCOMMIT_SQL)
            .bind(iTopicId)
            .execute(&mut *oTransaction)
            .await?;
        oTransaction.commit().await?;
        Ok(())
    }

    async fn vMove(&self, stCommand: StMoveTopicCommand) -> Result<()> {
        let mut oTransaction = self.oPool.begin().await?;
        let iCurrentGroupId = sqlx::query_scalar::<_, i32>(S_LOCK_TOPIC_GROUP_SQL)
            .bind(stCommand.iTopicId)
            .fetch_optional(&mut *oTransaction)
            .await?
            .ok_or_else(|| {
                AppError::Anyhow(anyhow::anyhow!(
                    "topic {} disappeared during move",
                    stCommand.iTopicId
                ))
            })?;

        if iCurrentGroupId != stCommand.iTargetGroupId {
            sqlx::query(S_MOVE_TOPIC_SQL)
                .bind(stCommand.iTopicId)
                .bind(stCommand.iTargetGroupId)
                .execute(&mut *oTransaction)
                .await?;
            if !stCommand.bTargetLinksAllowed {
                sqlx::query(S_CLEAR_TOPIC_LINK_SQL)
                    .bind(stCommand.iTopicId)
                    .execute(&mut *oTransaction)
                    .await?;
            }
        }

        // TopicService loads the current msgbase markup after TopicDao's
        // locked move, but formats with the stale URL/linktext/group values
        // supplied by the controller. It appends whenever it entered
        // moveTopic for a link-disabled target, even if the row lock observes
        // that another request has already moved the row to that target.
        if !stCommand.bTargetLinksAllowed {
            let sMarkup = sqlx::query_scalar::<_, String>(S_LOAD_MOVE_MARKUP_SQL)
                .bind(stCommand.iTopicId)
                .fetch_optional(&mut *oTransaction)
                .await?
                .ok_or_else(|| {
                    AppError::Anyhow(anyhow::anyhow!(
                        "msgbase row {} disappeared during move",
                        stCommand.iTopicId
                    ))
                })?;
            let enMarkup = EnTopicMarkup::try_from(sMarkup.as_str())
                .map_err(|stError| AppError::Anyhow(stError.into()))?;
            let sAppendedInfo = sMoveInfo(
                enMarkup,
                stCommand.optOriginalUrl.as_deref(),
                stCommand.optOriginalLinkText.as_deref(),
                &stCommand.sModeratorNick,
                &stCommand.sOriginalGroupUrlName,
            );
            sqlx::query(S_APPEND_MOVE_INFO_SQL)
                .bind(stCommand.iTopicId)
                .bind(sAppendedInfo)
                .execute(&mut *oTransaction)
                .await?;
        }

        oTransaction.commit().await?;
        Ok(())
    }

    async fn vResolve(&self, iTopicId: i32, bResolved: bool) -> Result<()> {
        let mut oTransaction = self.oPool.begin().await?;
        sqlx::query(S_RESOLVE_SQL)
            .bind(iTopicId)
            .bind(bResolved)
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
    fn snapshot_reproduces_sticky_expiration_and_group_model_fields() {
        assert!(S_TOPIC_SNAPSHOT_SQL.contains("NOT t.sticky"));
        assert!(S_TOPIC_SNAPSHOT_SQL.contains("CURRENT_TIMESTAMP-s.expire"));
        assert!(S_TOPIC_SNAPSHOT_SQL.contains("s.havelink AS b_links_allowed"));
        assert!(S_TOPIC_SNAPSHOT_SQL.contains("g.resolvable AS b_group_resolvable"));
        assert!(!S_TOPIC_SNAPSHOT_SQL.contains("WHERE t.id=$1 AND"));
    }

    #[test]
    fn mt_group_choices_are_forum_then_articles_and_mtn_has_its_own_scope() {
        assert!(S_FORUM_ARTICLES_GROUPS_SQL.contains("s.id IN (2,6)"));
        assert!(
            S_FORUM_ARTICLES_GROUPS_SQL.contains("CASE s.id WHEN 2 THEN 0 WHEN 6 THEN 1 END,g.id")
        );
        assert!(S_CURRENT_SECTION_GROUPS_SQL.contains("WHERE s.id=$1"));
        assert!(S_PREMODERATED_GROUPS_SQL.contains("s.moderate AND NOT COALESCE(s.vote,false)"));
        assert!(S_PREMODERATED_GROUPS_SQL.contains("ORDER BY s.id,g.id"));
    }

    #[test]
    fn uncommit_does_not_touch_lastmod_or_unrelated_side_effect_tables() {
        for sFragment in ["moderate=false", "commitby=NULL", "commitdate=NULL"] {
            assert!(S_UNCOMMIT_SQL.contains(sFragment), "{sFragment}");
        }
        for sForbidden in ["lastmod", "score", "edit_info", "user_events"] {
            assert!(!S_UNCOMMIT_SQL.contains(sForbidden), "{sForbidden}");
        }
    }

    #[test]
    fn move_transaction_has_the_source_lock_update_clear_append_contract() {
        assert_eq!(
            S_LOCK_TOPIC_GROUP_SQL,
            "SELECT groupid FROM topics WHERE id=$1 FOR UPDATE"
        );
        assert!(S_MOVE_TOPIC_SQL.contains("lastmod=CURRENT_TIMESTAMP"));
        assert_eq!(
            S_CLEAR_TOPIC_LINK_SQL,
            "UPDATE topics SET linktext=NULL,url=NULL WHERE id=$1"
        );
        assert_eq!(
            S_LOAD_MOVE_MARKUP_SQL,
            "SELECT markup::text FROM msgbase WHERE id=$1"
        );
        assert_eq!(
            S_APPEND_MOVE_INFO_SQL,
            "UPDATE msgbase SET message=message||$2 WHERE id=$1"
        );
        for sForbidden in ["edit_info", "user_events", "telegram", "score"] {
            assert!(!S_MOVE_TOPIC_SQL.contains(sForbidden));
            assert!(!S_APPEND_MOVE_INFO_SQL.contains(sForbidden));
        }
    }

    #[test]
    fn resolve_is_unconditional_and_advances_lastmod_by_exactly_one_second() {
        assert!(S_RESOLVE_SQL.contains("resolved=$2"));
        assert!(S_RESOLVE_SQL.contains("lastmod=lastmod+'1 second'::interval"));
        assert!(!S_RESOLVE_SQL.contains("IS DISTINCT FROM"));
        assert!(!S_RESOLVE_SQL.contains("CURRENT_TIMESTAMP"));
    }
}
