use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool, Postgres, Transaction};

use crate::{
    domain::topic::deletion::{
        I_ANONYMOUS_USER_ID, StDeleteTopicCommand, StTopicDeleteMutation, StTopicDeletionActor,
        StTopicDeletionSnapshot, TrTopicDeletionRepository,
    },
    error::{AppError, Result},
};

const S_SNAPSHOT_SQL: &str = r#"
SELECT t.id AS i_topic_id,
       t.userid AS i_author_id,
       u.nick AS s_author_nick,
       COALESCE(u.score,0) AS i_author_score,
       COALESCE(u.max_score,0) AS i_author_max_score,
       COALESCE(u.blocked,false) AS b_author_blocked,
       COALESCE(u.passwd,'')='' AS b_author_anonymous,
       COALESCE(u.frozen_until>CURRENT_TIMESTAMP,false) AS b_author_frozen,
       t.title AS s_stored_title,
       m.message AS s_message,
       m.markup::text AS s_markup,
       t.url AS opt_url,
       t.linktext AS opt_link_text,
       t.groupid AS i_group_id,
       g.title AS s_group_title,
       g.urlname AS s_group_url_name,
       s.id AS i_section_id,
       s.name AS s_section_title,
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
       s.imagepost AS b_section_image_post,
       s.imageallowed AS b_section_image_allowed,
       s.havelink AS b_links_allowed,
       t.deleted AS b_deleted,
       COALESCE(t.draft,false) AS b_draft,
       t.moderate AS b_committed,
       t.sticky AS b_sticky,
       COALESCE(t.resolved,false) AS b_resolved,
       NOT t.sticky
         AND COALESCE(t.commitdate,t.postdate)<CURRENT_TIMESTAMP-s.expire AS b_expired,
       COALESCE(t.stat1,0) AS i_comment_count,
       COALESCE(t.postscore,-9999) AS i_post_score,
       COALESCE(t.minor,false) AS b_minor,
       t.postdate AS dt_postdate,
       t.commitdate AS opt_commit_date,
       t.lastmod AS dt_last_mod,
       (SELECT di.deldate FROM del_info di WHERE di.msgid=t.id) AS opt_delete_date,
       COALESCE(host(t.postip),'') AS s_post_ip,
       COALESCE(t.ua_id,0) AS i_user_agent_id
  FROM topics t
  JOIN msgbase m ON m.id=t.id
  JOIN users u ON u.id=t.userid
  JOIN groups g ON g.id=t.groupid
  JOIN sections s ON s.id=g.section
 WHERE t.id=$1
"#;

const S_DELETE_TOPIC_SQL: &str =
    "UPDATE topics SET deleted=true,sticky=false WHERE id=$1 AND NOT deleted";
const S_INSERT_DELETE_INFO_SQL: &str = r#"
INSERT INTO del_info(msgid,delby,reason,deldate,bonus)
VALUES($1,$2,$3,CURRENT_TIMESTAMP,$4)
"#;
const S_UNDELETE_TOPIC_SQL: &str = "UPDATE topics SET deleted=false WHERE id=$1";
const S_DELETE_INFO_SQL: &str = "DELETE FROM del_info WHERE msgid=$1";

#[derive(Debug, FromRow)]
struct StSnapshotRow {
    i_topic_id: i32,
    i_author_id: i32,
    s_author_nick: String,
    i_author_score: i32,
    i_author_max_score: i32,
    b_author_blocked: bool,
    b_author_anonymous: bool,
    b_author_frozen: bool,
    s_stored_title: String,
    s_message: String,
    s_markup: String,
    opt_url: Option<String>,
    opt_link_text: Option<String>,
    i_group_id: i32,
    s_group_title: String,
    s_group_url_name: String,
    i_section_id: i32,
    s_section_title: String,
    s_section_prefix: String,
    b_section_premoderated: bool,
    b_section_poll_allowed: bool,
    b_section_image_post: bool,
    b_section_image_allowed: bool,
    b_links_allowed: bool,
    b_deleted: bool,
    b_draft: bool,
    b_committed: bool,
    b_sticky: bool,
    b_resolved: bool,
    b_expired: bool,
    i_comment_count: i32,
    i_post_score: i32,
    b_minor: bool,
    dt_postdate: DateTime<Utc>,
    opt_commit_date: Option<DateTime<Utc>>,
    dt_last_mod: DateTime<Utc>,
    opt_delete_date: Option<DateTime<Utc>>,
    s_post_ip: String,
    i_user_agent_id: i32,
}

impl From<StSnapshotRow> for StTopicDeletionSnapshot {
    fn from(stRow: StSnapshotRow) -> Self {
        Self {
            iTopicId: stRow.i_topic_id,
            iAuthorId: stRow.i_author_id,
            sAuthorNick: stRow.s_author_nick,
            iAuthorScore: stRow.i_author_score,
            iAuthorMaxScore: stRow.i_author_max_score,
            bAuthorBlocked: stRow.b_author_blocked,
            bAuthorAnonymous: stRow.b_author_anonymous,
            bAuthorFrozen: stRow.b_author_frozen,
            sStoredTitle: stRow.s_stored_title,
            sMessage: stRow.s_message,
            sMarkup: stRow.s_markup,
            optUrl: stRow.opt_url,
            optLinkText: stRow.opt_link_text,
            iGroupId: stRow.i_group_id,
            sGroupTitle: stRow.s_group_title,
            sGroupUrlName: stRow.s_group_url_name,
            iSectionId: stRow.i_section_id,
            sSectionTitle: stRow.s_section_title,
            sSectionPrefix: stRow.s_section_prefix,
            bSectionPremoderated: stRow.b_section_premoderated,
            bSectionPollAllowed: stRow.b_section_poll_allowed,
            bSectionImagePost: stRow.b_section_image_post,
            bSectionImageAllowed: stRow.b_section_image_allowed,
            bLinksAllowed: stRow.b_links_allowed,
            bDeleted: stRow.b_deleted,
            bDraft: stRow.b_draft,
            bCommitted: stRow.b_committed,
            bSticky: stRow.b_sticky,
            bResolved: stRow.b_resolved,
            bExpired: stRow.b_expired,
            iCommentCount: stRow.i_comment_count,
            iPostScore: stRow.i_post_score,
            bMinor: stRow.b_minor,
            dtPostdate: stRow.dt_postdate,
            optCommitDate: stRow.opt_commit_date,
            dtLastMod: stRow.dt_last_mod,
            optDeleteDate: stRow.opt_delete_date,
            sPostIp: stRow.s_post_ip,
            iUserAgentId: stRow.i_user_agent_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CTopicDeletionPgRepository {
    oPool: PgPool,
}

impl CTopicDeletionPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrTopicDeletionRepository for CTopicDeletionPgRepository {
    async fn optSnapshot(&self, iTopicId: i32) -> Result<Option<StTopicDeletionSnapshot>> {
        Ok(sqlx::query_as::<_, StSnapshotRow>(S_SNAPSHOT_SQL)
            .bind(iTopicId)
            .fetch_optional(&self.oPool)
            .await?
            .map(Into::into))
    }

    async fn stDelete(
        &self,
        stActor: StTopicDeletionActor<'_>,
        stTopic: &StTopicDeletionSnapshot,
        stCommand: &StDeleteTopicCommand,
    ) -> Result<StTopicDeleteMutation> {
        let mut oTransaction = self.oPool.begin().await?;
        let stUpdate = sqlx::query(S_DELETE_TOPIC_SQL)
            .bind(stTopic.iTopicId)
            .execute(&mut *oTransaction)
            .await?;
        if stUpdate.rows_affected() == 0 {
            // `doDeleteTopic` returns None.  No score, del_info, event or
            // notification side effect is allowed, but the controller still
            // enqueues this stale snapshot after this transaction returns.
            oTransaction.commit().await?;
            return Ok(StTopicDeleteMutation {
                bDeleted: false,
                iAppliedScoreDelta: 0,
            });
        }

        let iRequestedScoreDelta = -stCommand.iPenalty;
        let iEffectiveScoreDelta = if iRequestedScoreDelta != 0
            && stTopic.iAuthorId != I_ANONYMOUS_USER_ID
            && stTopic.bAuthorFrozen
        {
            0
        } else {
            iRequestedScoreDelta
        };
        if iEffectiveScoreDelta != 0 {
            let stScoreUpdate = sqlx::query("UPDATE users SET score=score+$2 WHERE id=$1")
                .bind(stTopic.iAuthorId)
                .bind(iEffectiveScoreDelta)
                .execute(&mut *oTransaction)
                .await?;
            vRequireUserUpdate(stScoreUpdate.rows_affected(), stTopic.iAuthorId)?;
        }

        // DeleteInfoDao.insert is intentionally a plain INSERT.  A stale
        // conflicting row aborts the whole localTx; an UPSERT would silently
        // change both race behavior and trigger side effects.
        sqlx::query(S_INSERT_DELETE_INFO_SQL)
            .bind(stTopic.iTopicId)
            .bind(stActor.iUserId)
            .bind(&stCommand.sReason)
            .bind(iEffectiveScoreDelta)
            .execute(&mut *oTransaction)
            .await?;

        vDeleteTopicEvents(&mut oTransaction, stTopic.iTopicId).await?;
        vNotifyTopicDeleted(
            &mut oTransaction,
            stTopic,
            stActor.iUserId,
            &stCommand.sReason,
        )
        .await?;
        oTransaction.commit().await?;

        Ok(StTopicDeleteMutation {
            bDeleted: true,
            iAppliedScoreDelta: iEffectiveScoreDelta,
        })
    }

    async fn vUndelete(&self, stTopic: &StTopicDeletionSnapshot) -> Result<()> {
        let mut oTransaction = self.oPool.begin().await?;
        let optBonus: Option<i32> =
            sqlx::query_scalar("SELECT bonus FROM del_info WHERE msgid=$1 FOR UPDATE")
                .bind(stTopic.iTopicId)
                .fetch_optional(&mut *oTransaction)
                .await?
                .flatten();
        if let Some(iBonus) = optBonus.filter(|iValue| *iValue != 0) {
            let stScoreUpdate = sqlx::query("UPDATE users SET score=score-$2 WHERE id=$1")
                .bind(stTopic.iAuthorId)
                .bind(iBonus)
                .execute(&mut *oTransaction)
                .await?;
            vRequireUserUpdate(stScoreUpdate.rows_affected(), stTopic.iAuthorId)?;
        }

        // TopicDao.undelete is unconditional and does not inspect affected
        // rows.  `msgundel_t`, fired by the following del_info DELETE, owns the
        // canonical lastmod side effect; do not duplicate it here.
        sqlx::query(S_UNDELETE_TOPIC_SQL)
            .bind(stTopic.iTopicId)
            .execute(&mut *oTransaction)
            .await?;
        sqlx::query(S_DELETE_INFO_SQL)
            .bind(stTopic.iTopicId)
            .execute(&mut *oTransaction)
            .await?;
        oTransaction.commit().await?;
        Ok(())
    }
}

fn vRequireUserUpdate(iRowsAffected: u64, iUserId: i32) -> Result<()> {
    if iRowsAffected == 0 {
        return Err(AppError::Anyhow(anyhow::anyhow!(
            "topic deletion author {iUserId} does not exist"
        )));
    }
    Ok(())
}

async fn vDeleteTopicEvents(
    oTransaction: &mut Transaction<'_, Postgres>,
    iTopicId: i32,
) -> Result<()> {
    let vecAffectedUsers: Vec<i32> = sqlx::query_scalar(
        r#"SELECT DISTINCT userid FROM user_events
             WHERE message_id=$1
               AND type IN ('TAG','REF','REPLY','WATCH','REACTION','WARNING')"#,
    )
    .bind(iTopicId)
    .fetch_all(&mut **oTransaction)
    .await?;
    sqlx::query(
        r#"DELETE FROM user_events
             WHERE message_id=$1
               AND type IN ('TAG','REF','REPLY','WATCH','REACTION','WARNING')"#,
    )
    .bind(iTopicId)
    .execute(&mut **oTransaction)
    .await?;
    if !vecAffectedUsers.is_empty() {
        sqlx::query(
            r#"UPDATE users SET unread_events=(
                   SELECT count(*) FROM user_events e
                    WHERE e.unread AND e.userid=users.id
               ) WHERE id=ANY($1)"#,
        )
        .bind(&vecAffectedUsers)
        .execute(&mut **oTransaction)
        .await?;
    }
    Ok(())
}

async fn vNotifyTopicDeleted(
    oTransaction: &mut Transaction<'_, Postgres>,
    stTopic: &StTopicDeletionSnapshot,
    iDeletedBy: i32,
    sReason: &str,
) -> Result<()> {
    if iDeletedBy == stTopic.iAuthorId
        || stTopic.iAuthorId == I_ANONYMOUS_USER_ID
        || stTopic.bAuthorFrozen
    {
        return Ok(());
    }
    // The canonical `new_event_t` trigger increments users.unread_events.
    // UserEventDao.addEvent deliberately does not recalculate it itself.
    sqlx::query(
        r#"INSERT INTO user_events(userid,type,private,message_id,message)
           VALUES($1,'DEL',true,$2,$3)"#,
    )
    .bind(stTopic.iAuthorId)
    .bind(stTopic.iTopicId)
    .bind(sReason)
    .execute(&mut **oTransaction)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_sql_is_additive_conditional_and_never_upserts() {
        let sProductionSource = include_str!("topic_deletion_repository.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(sProductionSource.contains("SET score=score+$2"));
        assert!(sProductionSource.contains("SET score=score-$2"));
        assert!(!sProductionSource.contains("GREATEST(score"));
        assert!(!sProductionSource.contains("ON CONFLICT"));
        assert!(sProductionSource.contains("if stUpdate.rows_affected() == 0"));
        assert!(sProductionSource.contains("INSERT INTO del_info"));
    }

    #[test]
    fn lastmod_is_left_to_the_canonical_delete_info_triggers() {
        assert!(!S_DELETE_TOPIC_SQL.contains("lastmod"));
        assert!(!S_UNDELETE_TOPIC_SQL.contains("lastmod"));
        assert!(!S_DELETE_INFO_SQL.contains("lastmod"));
    }

    #[test]
    fn expiration_snapshot_matches_topic_from_result_set_sticky_exception() {
        assert!(
            S_SNAPSHOT_SQL.contains(
                "NOT t.sticky\n         AND COALESCE(t.commitdate,t.postdate)<CURRENT_TIMESTAMP-s.expire AS b_expired"
            )
        );
    }
}
