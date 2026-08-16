use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    domain::admin::ip_mass_delete::{
        StIpBanCommand, StIpMassDeleteCommand, StIpMassDeleteResult, TrIpMassDeleteRepository,
    },
    error::Result,
};

const I_ANONYMOUS_USER_ID: i32 = 2;

const S_SELECT_TOPIC_IDS: &str = r#"
SELECT id
  FROM topics
 WHERE postip=$1::inet
   AND NOT deleted
   AND postdate>$2
 FOR UPDATE
"#;

const S_SELECT_COMMENT_IDS: &str = r#"
SELECT id
  FROM comments
 WHERE postip=$1::inet
   AND NOT deleted
   AND postdate>$2
 ORDER BY id DESC
 FOR UPDATE
"#;

const S_DELETE_TOPIC: &str =
    "UPDATE topics SET deleted=true,sticky=false WHERE id=$1 AND NOT deleted";
const S_DELETE_COMMENT: &str = "UPDATE comments SET deleted=true WHERE id=$1 AND NOT deleted";
const S_INSERT_DELETE_INFO: &str = r#"
INSERT INTO del_info(msgid,delby,reason,deldate,bonus)
VALUES($1,$2,$3,CURRENT_TIMESTAMP,0)
"#;

#[derive(Debug, Clone)]
pub struct CIpMassDeletePgRepository {
    oPool: PgPool,
}

impl CIpMassDeletePgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrIpMassDeleteRepository for CIpMassDeletePgRepository {
    async fn vBlockIp(&self, iModeratorId: i32, stCommand: &StIpBanCommand) -> Result<()> {
        // This intentionally runs on the pool rather than in the deletion
        // transaction. DelIPController invokes IpBlockDao first and does not
        // roll the block back when DeleteService later fails.
        sqlx::query(
            r#"INSERT INTO b_ips(ip,mod_id,date,reason,ban_date,allow_posting,captcha_required)
               VALUES($1::inet,$2,CURRENT_TIMESTAMP,$3,$4,$5,$6)
               ON CONFLICT(ip) DO UPDATE SET
                 mod_id=EXCLUDED.mod_id,
                 date=CURRENT_TIMESTAMP,
                 reason=EXCLUDED.reason,
                 ban_date=EXCLUDED.ban_date,
                 allow_posting=EXCLUDED.allow_posting,
                 captcha_required=EXCLUDED.captcha_required"#,
        )
        .bind(&stCommand.sIp)
        .bind(iModeratorId)
        .bind(&stCommand.sReason)
        .bind(stCommand.optBanUntil)
        .bind(stCommand.bAllowPosting)
        .bind(stCommand.bCaptchaRequired)
        .execute(&self.oPool)
        .await?;
        Ok(())
    }

    async fn stDeleteByIp(
        &self,
        iModeratorId: i32,
        stCommand: &StIpMassDeleteCommand,
    ) -> Result<StIpMassDeleteResult> {
        let mut oTransaction = self.oPool.begin().await?;

        let vecCandidateTopicIds: Vec<i32> = sqlx::query_scalar(S_SELECT_TOPIC_IDS)
            .bind(&stCommand.sIp)
            .bind(stCommand.dtCutoff)
            .fetch_all(&mut *oTransaction)
            .await?;
        // deleteByIPAddress resolves and locks both candidate collections
        // before massDelete mutates either one.
        let vecCandidateCommentIds: Vec<i32> = sqlx::query_scalar(S_SELECT_COMMENT_IDS)
            .bind(&stCommand.sIp)
            .bind(stCommand.dtCutoff)
            .fetch_all(&mut *oTransaction)
            .await?;

        let mut vecDeletedTopicIds = Vec::with_capacity(vecCandidateTopicIds.len());
        for iTopicId in vecCandidateTopicIds {
            let stUpdate = sqlx::query(S_DELETE_TOPIC)
                .bind(iTopicId)
                .execute(&mut *oTransaction)
                .await?;
            if stUpdate.rows_affected() != 0 {
                vecDeletedTopicIds.push(iTopicId);
            }
        }
        vDeleteTopicEvents(&mut oTransaction, &vecDeletedTopicIds).await?;

        // Newest-to-oldest is observable: deleting an eligible leaf can make
        // its parent eligible later in the same pass. A reply which survives
        // this pass keeps its parent in the skipped result.
        let mut vecDeletedCommentIds = Vec::with_capacity(vecCandidateCommentIds.len());
        let mut vecSkippedCommentIds = Vec::new();
        for iCommentId in vecCandidateCommentIds {
            let iReplyCount: i64 = sqlx::query_scalar(
                "SELECT count(id) FROM comments WHERE replyto=$1 AND NOT deleted",
            )
            .bind(iCommentId)
            .fetch_one(&mut *oTransaction)
            .await?;
            if iReplyCount != 0 {
                vecSkippedCommentIds.push(iCommentId);
                continue;
            }

            let stUpdate = sqlx::query(S_DELETE_COMMENT)
                .bind(iCommentId)
                .execute(&mut *oTransaction)
                .await?;
            if stUpdate.rows_affected() == 0 {
                continue;
            }

            // CommentDao.updateStatsAfterDelete performs these as two
            // statements for every successful delete.
            sqlx::query(
                r#"UPDATE topics SET stat1=stat1-1,lastmod=CURRENT_TIMESTAMP
                    WHERE id=(SELECT topic FROM comments WHERE id=$1)"#,
            )
            .bind(iCommentId)
            .execute(&mut *oTransaction)
            .await?;
            sqlx::query(
                r#"UPDATE topics SET stat3=stat1
                    WHERE id=(SELECT topic FROM comments WHERE id=$1) AND stat3>stat1"#,
            )
            .bind(iCommentId)
            .execute(&mut *oTransaction)
            .await?;
            vecDeletedCommentIds.push(iCommentId);
        }
        vDeleteCommentEvents(&mut oTransaction, &vecDeletedCommentIds).await?;

        // DeleteInfoDao uses plain INSERTs. Comments precede topics in
        // DeleteService.massDelete; stale/conflicting rows abort the whole
        // transaction instead of being silently overwritten.
        for iMessageId in vecDeletedCommentIds.iter().chain(&vecDeletedTopicIds) {
            sqlx::query(S_INSERT_DELETE_INFO)
                .bind(iMessageId)
                .bind(iModeratorId)
                .bind(&stCommand.sReason)
                .execute(&mut *oTransaction)
                .await?;
        }

        vNotifyDeletedTopics(
            &mut oTransaction,
            &vecDeletedTopicIds,
            iModeratorId,
            &stCommand.sReason,
        )
        .await?;
        vNotifyDeletedComments(
            &mut oTransaction,
            &vecDeletedCommentIds,
            iModeratorId,
            &stCommand.sReason,
        )
        .await?;

        oTransaction.commit().await?;
        Ok(StIpMassDeleteResult {
            vecDeletedTopicIds,
            vecDeletedCommentIds,
            vecSkippedCommentIds,
        })
    }
}

async fn vDeleteTopicEvents(
    oTransaction: &mut Transaction<'_, Postgres>,
    vecTopicIds: &[i32],
) -> Result<()> {
    if vecTopicIds.is_empty() {
        return Ok(());
    }
    let vecAffectedUserIds: Vec<i32> = sqlx::query_scalar(
        r#"SELECT DISTINCT userid FROM user_events
            WHERE message_id=ANY($1)
              AND type IN ('TAG','REF','REPLY','WATCH','REACTION','WARNING')"#,
    )
    .bind(vecTopicIds)
    .fetch_all(&mut **oTransaction)
    .await?;
    sqlx::query(
        r#"DELETE FROM user_events
            WHERE message_id=ANY($1)
              AND type IN ('TAG','REF','REPLY','WATCH','REACTION','WARNING')"#,
    )
    .bind(vecTopicIds)
    .execute(&mut **oTransaction)
    .await?;
    vRecalculateUnread(oTransaction, &vecAffectedUserIds).await
}

async fn vDeleteCommentEvents(
    oTransaction: &mut Transaction<'_, Postgres>,
    vecCommentIds: &[i32],
) -> Result<()> {
    if vecCommentIds.is_empty() {
        return Ok(());
    }
    let vecAffectedUserIds: Vec<i32> = sqlx::query_scalar(
        r#"SELECT DISTINCT userid FROM user_events
            WHERE comment_id=ANY($1)
              AND type IN ('REPLY','WATCH','REF','REACTION','WARNING')"#,
    )
    .bind(vecCommentIds)
    .fetch_all(&mut **oTransaction)
    .await?;
    sqlx::query(
        r#"DELETE FROM user_events
            WHERE comment_id=ANY($1)
              AND type IN ('REPLY','WATCH','REF','REACTION','WARNING')"#,
    )
    .bind(vecCommentIds)
    .execute(&mut **oTransaction)
    .await?;
    vRecalculateUnread(oTransaction, &vecAffectedUserIds).await
}

async fn vRecalculateUnread(
    oTransaction: &mut Transaction<'_, Postgres>,
    vecUserIds: &[i32],
) -> Result<()> {
    if vecUserIds.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r#"UPDATE users SET unread_events=(
               SELECT count(*) FROM user_events
                WHERE unread AND userid=users.id)
            WHERE id=ANY($1)"#,
    )
    .bind(vecUserIds)
    .execute(&mut **oTransaction)
    .await?;
    Ok(())
}

async fn vNotifyDeletedTopics(
    oTransaction: &mut Transaction<'_, Postgres>,
    vecTopicIds: &[i32],
    iModeratorId: i32,
    sReason: &str,
) -> Result<()> {
    if vecTopicIds.is_empty() {
        return Ok(());
    }
    // The canonical `new_event_t` trigger increments unread_events for each
    // inserted notification. Do not duplicate that counter update here.
    sqlx::query(
        r#"INSERT INTO user_events(userid,type,private,message_id,message)
           SELECT topics.userid,'DEL',true,topics.id,$2
             FROM topics
             JOIN users ON topics.userid=users.id
            WHERE topics.id=ANY($1)
              AND topics.userid<>$3
              AND topics.userid<>$4
              AND (users.frozen_until IS NULL OR users.frozen_until<CURRENT_TIMESTAMP)"#,
    )
    .bind(vecTopicIds)
    .bind(sReason)
    .bind(iModeratorId)
    .bind(I_ANONYMOUS_USER_ID)
    .execute(&mut **oTransaction)
    .await?;
    Ok(())
}

async fn vNotifyDeletedComments(
    oTransaction: &mut Transaction<'_, Postgres>,
    vecCommentIds: &[i32],
    iModeratorId: i32,
    sReason: &str,
) -> Result<()> {
    if vecCommentIds.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r#"INSERT INTO user_events(userid,type,private,message_id,comment_id,message)
           SELECT comments.userid,'DEL',true,comments.topic,comments.id,$2
             FROM comments
             JOIN users ON comments.userid=users.id
            WHERE comments.id=ANY($1)
              AND comments.userid<>$3
              AND comments.userid<>$4
              AND (users.frozen_until IS NULL OR users.frozen_until<CURRENT_TIMESTAMP)"#,
    )
    .bind(vecCommentIds)
    .bind(sReason)
    .bind(iModeratorId)
    .bind(I_ANONYMOUS_USER_ID)
    .execute(&mut **oTransaction)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    const I_FIXTURE_MODERATOR: i32 = 2_100_010_001;
    const I_FIXTURE_AUTHOR: i32 = 2_100_010_002;
    const I_FIXTURE_FROZEN_AUTHOR: i32 = 2_100_010_003;
    const ARR_FIXTURE_USERS: [i32; 3] = [
        I_FIXTURE_MODERATOR,
        I_FIXTURE_AUTHOR,
        I_FIXTURE_FROZEN_AUTHOR,
    ];

    const I_CANDIDATE_TOPIC: i32 = 2_100_100_001;
    const I_HOST_TOPIC: i32 = 2_100_100_002;
    const I_BOUNDARY_TOPIC: i32 = 2_100_100_003;
    const ARR_FIXTURE_TOPICS: [i32; 3] = [I_CANDIDATE_TOPIC, I_HOST_TOPIC, I_BOUNDARY_TOPIC];

    const I_CHAIN_PARENT: i32 = 2_100_200_001;
    const I_CHAIN_CHILD: i32 = 2_100_200_002;
    const I_SKIPPED_PARENT: i32 = 2_100_200_003;
    const I_SURVIVING_REPLY: i32 = 2_100_200_004;
    const I_MODERATOR_COMMENT: i32 = 2_100_200_005;
    const I_ANONYMOUS_COMMENT: i32 = 2_100_200_006;
    const I_FROZEN_COMMENT: i32 = 2_100_200_007;
    const I_BOUNDARY_COMMENT: i32 = 2_100_200_008;
    const ARR_FIXTURE_COMMENTS: [i32; 8] = [
        I_CHAIN_PARENT,
        I_CHAIN_CHILD,
        I_SKIPPED_PARENT,
        I_SURVIVING_REPLY,
        I_MODERATOR_COMMENT,
        I_ANONYMOUS_COMMENT,
        I_FROZEN_COMMENT,
        I_BOUNDARY_COMMENT,
    ];
    const S_FIXTURE_IP: &str = "203.0.113.241";

    #[test]
    fn candidates_use_java_strict_cutoff_and_row_locks() {
        assert!(S_SELECT_TOPIC_IDS.contains("postdate>$2"));
        assert!(!S_SELECT_TOPIC_IDS.contains("postdate>=$2"));
        assert!(S_SELECT_TOPIC_IDS.contains("FOR UPDATE"));
        assert!(S_SELECT_COMMENT_IDS.contains("postdate>$2"));
        assert!(S_SELECT_COMMENT_IDS.contains("ORDER BY id DESC"));
        assert!(S_SELECT_COMMENT_IDS.contains("FOR UPDATE"));
        let sProduction = include_str!("ip_mass_delete_repository.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(
            sProduction
                .find("sqlx::query_scalar(S_SELECT_COMMENT_IDS)")
                .unwrap()
                < sProduction.find("sqlx::query(S_DELETE_TOPIC)").unwrap()
        );
    }

    #[test]
    fn mutations_are_conditional_and_delete_info_is_never_upserted() {
        assert!(S_DELETE_TOPIC.contains("sticky=false"));
        assert!(S_DELETE_TOPIC.contains("AND NOT deleted"));
        assert!(S_DELETE_COMMENT.contains("AND NOT deleted"));
        assert!(S_INSERT_DELETE_INFO.contains("bonus"));
        assert!(S_INSERT_DELETE_INFO.contains(",0)"));
        assert!(!S_INSERT_DELETE_INFO.contains("ON CONFLICT"));
    }

    #[test]
    fn production_path_keeps_stats_events_notifications_and_one_commit() {
        let sProduction = include_str!("ip_mass_delete_repository.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        for sFragment in [
            "stat1=stat1-1",
            "stat3=stat1",
            "lastmod=CURRENT_TIMESTAMP",
            "vDeleteTopicEvents",
            "vDeleteCommentEvents",
            "vNotifyDeletedTopics",
            "vNotifyDeletedComments",
            "frozen_until IS NULL",
            "comments.userid<>$3",
            "topics.userid<>$3",
        ] {
            assert!(sProduction.contains(sFragment), "{sFragment}");
        }
        assert_eq!(
            sProduction.matches("oTransaction.commit().await?").count(),
            1
        );
    }

    #[test]
    fn notification_counter_remains_owned_by_the_java_trigger() {
        let sProduction = include_str!("ip_mass_delete_repository.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let sNotifySource = sProduction
            .split("async fn vNotifyDeletedTopics")
            .nth(1)
            .unwrap();
        assert!(sNotifySource.contains("INSERT INTO user_events"));
        assert!(!sNotifySource.contains("SET unread_events=unread_events+1"));
    }

    async fn vCleanupIntegrationFixtures(oPool: &PgPool) -> Result<()> {
        let vecMessageIds = ARR_FIXTURE_TOPICS
            .iter()
            .chain(&ARR_FIXTURE_COMMENTS)
            .copied()
            .collect::<Vec<_>>();
        sqlx::query(
            r#"DELETE FROM user_events
                WHERE userid=ANY($1) OR message_id=ANY($2) OR comment_id=ANY($3)"#,
        )
        .bind(&ARR_FIXTURE_USERS[..])
        .bind(&ARR_FIXTURE_TOPICS[..])
        .bind(&ARR_FIXTURE_COMMENTS[..])
        .execute(oPool)
        .await?;
        // A broken notification exclusion would have let new_event_t bump the
        // real anonymous account. Recalculate it even on failure cleanup so
        // the guarded fixture cannot leave that shared canonical row changed.
        sqlx::query(
            r#"UPDATE users SET unread_events=(
                   SELECT count(*) FROM user_events WHERE userid=users.id AND unread)
                WHERE id=$1"#,
        )
        .bind(I_ANONYMOUS_USER_ID)
        .execute(oPool)
        .await?;
        sqlx::query("DELETE FROM memories WHERE topic=ANY($1) OR userid=ANY($2)")
            .bind(&ARR_FIXTURE_TOPICS[..])
            .bind(&ARR_FIXTURE_USERS[..])
            .execute(oPool)
            .await?;
        sqlx::query("DELETE FROM del_info WHERE msgid=ANY($1)")
            .bind(&vecMessageIds)
            .execute(oPool)
            .await?;
        sqlx::query("DELETE FROM comments WHERE id=ANY($1)")
            .bind(&ARR_FIXTURE_COMMENTS[..])
            .execute(oPool)
            .await?;
        sqlx::query("DELETE FROM topics WHERE id=ANY($1)")
            .bind(&ARR_FIXTURE_TOPICS[..])
            .execute(oPool)
            .await?;
        sqlx::query("DELETE FROM msgbase WHERE id=ANY($1)")
            .bind(&vecMessageIds)
            .execute(oPool)
            .await?;
        sqlx::query("DELETE FROM b_ips WHERE ip=$1::inet")
            .bind(S_FIXTURE_IP)
            .execute(oPool)
            .await?;
        sqlx::query("DELETE FROM users WHERE id=ANY($1)")
            .bind(&ARR_FIXTURE_USERS[..])
            .execute(oPool)
            .await?;
        Ok(())
    }

    async fn vInsertIntegrationFixtures(
        oPool: &PgPool,
        dtCutoff: chrono::DateTime<Utc>,
    ) -> Result<()> {
        let mut oTransaction = oPool.begin().await?;
        let (iGroupId, iGroupStatBefore): (i32, i32) =
            sqlx::query_as("SELECT id,stat3 FROM groups ORDER BY id LIMIT 1 FOR UPDATE")
                .fetch_one(&mut *oTransaction)
                .await?;
        let bAnonymousExists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=$1)")
                .bind(I_ANONYMOUS_USER_ID)
                .fetch_one(&mut *oTransaction)
                .await?;
        if !bAnonymousExists {
            return Err(crate::error::AppError::Anyhow(anyhow::anyhow!(
                "canonical anonymous user id 2 is absent"
            )));
        }

        for (iUserId, sNick, bModerator, optFrozenUntil) in [
            (I_FIXTURE_MODERATOR, "delip_fixture_moderator", true, None),
            (I_FIXTURE_AUTHOR, "delip_fixture_author", false, None),
            (
                I_FIXTURE_FROZEN_AUTHOR,
                "delip_fixture_frozen",
                false,
                Some(Utc::now() + Duration::days(1)),
            ),
        ] {
            sqlx::query(
                r#"INSERT INTO users(
                     id,nick,passwd,canmod,candel,blocked,score,max_score,
                     activated,corrector,unread_events,userinfo_markup,frozen_until)
                   VALUES($1,$2,'fixture-password',$3,false,false,100,100,
                          true,false,0,'MARKDOWN',$4)"#,
            )
            .bind(iUserId)
            .bind(sNick)
            .bind(bModerator)
            .bind(optFrozenUntil)
            .execute(&mut *oTransaction)
            .await?;
        }

        for iMessageId in ARR_FIXTURE_TOPICS.iter().chain(&ARR_FIXTURE_COMMENTS) {
            sqlx::query(
                "INSERT INTO msgbase(id,message,markup) VALUES($1,'delip integration fixture','MARKDOWN')",
            )
            .bind(iMessageId)
            .execute(&mut *oTransaction)
            .await?;
        }

        let dtOldLastMod = dtCutoff - Duration::days(1);
        for (iTopicId, iAuthorId, sIp, dtPostdate, bSticky) in [
            (
                I_CANDIDATE_TOPIC,
                I_FIXTURE_AUTHOR,
                S_FIXTURE_IP,
                dtCutoff + Duration::minutes(1),
                true,
            ),
            (
                I_HOST_TOPIC,
                I_FIXTURE_MODERATOR,
                "198.51.100.241",
                dtCutoff - Duration::days(2),
                false,
            ),
            (
                I_BOUNDARY_TOPIC,
                I_FIXTURE_AUTHOR,
                S_FIXTURE_IP,
                dtCutoff,
                true,
            ),
        ] {
            sqlx::query(
                r#"INSERT INTO topics(
                     id,groupid,userid,title,url,moderate,postdate,linktext,deleted,
                     stat1,stat3,lastmod,commitby,notop,commitdate,postscore,postip,
                     sticky,resolved,minor,draft,allow_anonymous,reactions,open_warnings)
                   VALUES($1,$2,$3,'delip integration topic',NULL,false,$4,NULL,false,
                          0,0,$5,NULL,false,NULL,-9999,$6::inet,$7,false,false,
                          false,true,'{}',0)"#,
            )
            .bind(iTopicId)
            .bind(iGroupId)
            .bind(iAuthorId)
            .bind(dtPostdate)
            .bind(dtOldLastMod)
            .bind(sIp)
            .bind(bSticky)
            .execute(&mut *oTransaction)
            .await?;
        }
        // topins_t subscribes each non-anonymous author. The mass-delete
        // contract itself does not need those fixtures, so remove them before
        // inserting comments and their event data.
        sqlx::query("DELETE FROM memories WHERE topic=ANY($1)")
            .bind(&ARR_FIXTURE_TOPICS[..])
            .execute(&mut *oTransaction)
            .await?;

        for (iCommentId, iAuthorId, optReplyTo, sIp, dtPostdate) in [
            (
                I_CHAIN_PARENT,
                I_FIXTURE_AUTHOR,
                None,
                S_FIXTURE_IP,
                dtCutoff + Duration::minutes(10),
            ),
            (
                I_CHAIN_CHILD,
                I_FIXTURE_AUTHOR,
                Some(I_CHAIN_PARENT),
                S_FIXTURE_IP,
                dtCutoff + Duration::minutes(11),
            ),
            (
                I_SKIPPED_PARENT,
                I_FIXTURE_AUTHOR,
                None,
                S_FIXTURE_IP,
                dtCutoff + Duration::minutes(12),
            ),
            (
                I_SURVIVING_REPLY,
                I_FIXTURE_MODERATOR,
                Some(I_SKIPPED_PARENT),
                "198.51.100.242",
                dtCutoff + Duration::minutes(13),
            ),
            (
                I_MODERATOR_COMMENT,
                I_FIXTURE_MODERATOR,
                None,
                S_FIXTURE_IP,
                dtCutoff + Duration::minutes(14),
            ),
            (
                I_ANONYMOUS_COMMENT,
                I_ANONYMOUS_USER_ID,
                None,
                S_FIXTURE_IP,
                dtCutoff + Duration::minutes(15),
            ),
            (
                I_FROZEN_COMMENT,
                I_FIXTURE_FROZEN_AUTHOR,
                None,
                S_FIXTURE_IP,
                dtCutoff + Duration::minutes(16),
            ),
            (
                I_BOUNDARY_COMMENT,
                I_FIXTURE_AUTHOR,
                None,
                S_FIXTURE_IP,
                dtCutoff,
            ),
        ] {
            sqlx::query(
                r#"INSERT INTO comments(
                     id,topic,userid,title,postdate,replyto,deleted,postip,
                     editor_id,edit_date,edit_count,reactions)
                   VALUES($1,$2,$3,'delip integration comment',$4,$5,false,
                          $6::inet,NULL,NULL,0,'{}')"#,
            )
            .bind(iCommentId)
            .bind(I_HOST_TOPIC)
            .bind(iAuthorId)
            .bind(dtPostdate)
            .bind(optReplyTo)
            .bind(sIp)
            .execute(&mut *oTransaction)
            .await?;
        }
        // comins_t produced the same count. Reset lastmod so the per-success
        // update can be asserted independently from fixture insertion.
        sqlx::query("UPDATE topics SET stat1=8,stat3=8,lastmod=$2 WHERE id=$1")
            .bind(I_HOST_TOPIC)
            .bind(dtOldLastMod)
            .execute(&mut *oTransaction)
            .await?;

        for (sType, optCommentId) in [
            ("REF", None),
            ("REPLY", Some(I_CHAIN_CHILD)),
            // DEL is deliberately outside the relevant-event deletion set.
            ("DEL", Some(I_CHAIN_CHILD)),
        ] {
            sqlx::query(
                r#"INSERT INTO user_events(userid,type,private,message_id,comment_id)
                   VALUES($1,$2::event_type,false,$3,$4)"#,
            )
            .bind(I_FIXTURE_MODERATOR)
            .bind(sType)
            .bind(if optCommentId.is_some() {
                I_HOST_TOPIC
            } else {
                I_CANDIDATE_TOPIC
            })
            .bind(optCommentId)
            .execute(&mut *oTransaction)
            .await?;
        }

        // topins_t/comins_t own live counters, but the disposable fixture must
        // not perturb the selected catalog group after setup or cleanup.
        sqlx::query("UPDATE groups SET stat3=$2 WHERE id=$1")
            .bind(iGroupId)
            .bind(iGroupStatBefore)
            .execute(&mut *oTransaction)
            .await?;
        oTransaction.commit().await?;
        Ok(())
    }

    fn stIntegrationCommand(dtCutoff: chrono::DateTime<Utc>) -> StIpMassDeleteCommand {
        StIpMassDeleteCommand {
            sIp: S_FIXTURE_IP.to_owned(),
            dtCutoff,
            sReason: "delip integration reason".to_owned(),
            optBan: None,
        }
    }

    async fn vAssertSuccessfulMutation(
        oPool: &PgPool,
        stResult: &StIpMassDeleteResult,
        dtCutoff: chrono::DateTime<Utc>,
    ) -> Result<()> {
        assert_eq!(stResult.vecDeletedTopicIds, [I_CANDIDATE_TOPIC]);
        assert_eq!(
            stResult.vecDeletedCommentIds,
            [
                I_FROZEN_COMMENT,
                I_ANONYMOUS_COMMENT,
                I_MODERATOR_COMMENT,
                I_CHAIN_CHILD,
                I_CHAIN_PARENT,
            ]
        );
        assert_eq!(stResult.vecSkippedCommentIds, [I_SKIPPED_PARENT]);

        let (bDeleted, bSticky): (bool, bool) =
            sqlx::query_as("SELECT deleted,sticky FROM topics WHERE id=$1")
                .bind(I_CANDIDATE_TOPIC)
                .fetch_one(oPool)
                .await?;
        assert!(bDeleted);
        assert!(!bSticky);
        let (bBoundaryDeleted, bBoundarySticky): (bool, bool) =
            sqlx::query_as("SELECT deleted,sticky FROM topics WHERE id=$1")
                .bind(I_BOUNDARY_TOPIC)
                .fetch_one(oPool)
                .await?;
        assert!(!bBoundaryDeleted);
        assert!(bBoundarySticky);

        let (iStat1, iStat3, dtLastMod): (i32, i32, chrono::DateTime<Utc>) =
            sqlx::query_as("SELECT stat1,stat3,lastmod FROM topics WHERE id=$1")
                .bind(I_HOST_TOPIC)
                .fetch_one(oPool)
                .await?;
        assert_eq!((iStat1, iStat3), (3, 3));
        assert!(dtLastMod > dtCutoff);
        let vecLiveCommentIds: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM comments WHERE id=ANY($1) AND NOT deleted ORDER BY id",
        )
        .bind(&ARR_FIXTURE_COMMENTS[..])
        .fetch_all(oPool)
        .await?;
        assert_eq!(
            vecLiveCommentIds,
            [I_SKIPPED_PARENT, I_SURVIVING_REPLY, I_BOUNDARY_COMMENT]
        );

        let (iDeleteInfoCount, iZeroBonusCount): (i64, i64) = sqlx::query_as(
            r#"SELECT count(*),count(*) FILTER(WHERE bonus=0)
                 FROM del_info WHERE msgid=ANY($1) AND reason=$2"#,
        )
        .bind(
            ARR_FIXTURE_TOPICS
                .iter()
                .chain(&ARR_FIXTURE_COMMENTS)
                .copied()
                .collect::<Vec<_>>(),
        )
        .bind("delip integration reason")
        .fetch_one(oPool)
        .await?;
        assert_eq!((iDeleteInfoCount, iZeroBonusCount), (6, 6));

        let iRelevantModeratorEvents: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM user_events WHERE userid=$1
                AND ((message_id=$2 AND type='REF') OR
                     (comment_id=$3 AND type='REPLY'))"#,
        )
        .bind(I_FIXTURE_MODERATOR)
        .bind(I_CANDIDATE_TOPIC)
        .bind(I_CHAIN_CHILD)
        .fetch_one(oPool)
        .await?;
        assert_eq!(iRelevantModeratorEvents, 0);
        let (iRetainedDeleteEvents, iModeratorUnread): (i64, i32) = sqlx::query_as(
            r#"SELECT count(*) FILTER(WHERE e.type='DEL'),max(u.unread_events)
                 FROM users u LEFT JOIN user_events e ON e.userid=u.id
                WHERE u.id=$1"#,
        )
        .bind(I_FIXTURE_MODERATOR)
        .fetch_one(oPool)
        .await?;
        assert_eq!((iRetainedDeleteEvents, iModeratorUnread), (1, 1));

        let (iNormalNotifications, iNormalUnread): (i64, i32) = sqlx::query_as(
            r#"SELECT count(e.id),max(u.unread_events)
                 FROM users u LEFT JOIN user_events e ON e.userid=u.id
                  AND e.type='DEL' AND (e.message_id=ANY($2) OR e.comment_id=ANY($3))
                WHERE u.id=$1"#,
        )
        .bind(I_FIXTURE_AUTHOR)
        .bind(&ARR_FIXTURE_TOPICS[..])
        .bind(&ARR_FIXTURE_COMMENTS[..])
        .fetch_one(oPool)
        .await?;
        assert_eq!((iNormalNotifications, iNormalUnread), (3, 3));
        for iExcludedUserId in [
            I_FIXTURE_MODERATOR,
            I_FIXTURE_FROZEN_AUTHOR,
            I_ANONYMOUS_USER_ID,
        ] {
            let iNotifications: i64 = sqlx::query_scalar(
                r#"SELECT count(*) FROM user_events WHERE userid=$1 AND type='DEL'
                    AND (message_id=ANY($2) OR comment_id=ANY($3))"#,
            )
            .bind(iExcludedUserId)
            .bind(&ARR_FIXTURE_TOPICS[..])
            .bind(&ARR_FIXTURE_COMMENTS[..])
            .fetch_one(oPool)
            .await?;
            // The moderator's deliberately seeded DEL is not a mass-delete
            // notification and is tied to a deleted comment, so exclude it.
            let iExpected = if iExcludedUserId == I_FIXTURE_MODERATOR {
                1
            } else {
                0
            };
            assert_eq!(iNotifications, iExpected);
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an explicitly selected disposable Java/Liquibase PostgreSQL database"]
    async fn transaction_reply_events_notifications_and_rollback_match_java() {
        assert_eq!(
            std::env::var("LOR_DELIP_INTEGRATION_CONFIRM").as_deref(),
            Ok("mutate-disposable-delip-fixture"),
            "set LOR_DELIP_INTEGRATION_CONFIRM=mutate-disposable-delip-fixture"
        );
        let sDatabaseUrl = std::env::var("LOR_DELIP_INTEGRATION_DATABASE_URL")
            .expect("set LOR_DELIP_INTEGRATION_DATABASE_URL to a disposable canonical database");
        let oPool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&sDatabaseUrl)
            .await
            .expect("disposable canonical database must be reachable");
        vCleanupIntegrationFixtures(&oPool)
            .await
            .expect("clean stale delip integration fixtures");

        let stRun: Result<()> = async {
            let dtCutoff = chrono::DateTime::<Utc>::from_timestamp_millis(
                (Utc::now() - Duration::hours(1)).timestamp_millis(),
            )
            .expect("fixture cutoff");
            vInsertIntegrationFixtures(&oPool, dtCutoff).await?;
            let cRepository = CIpMassDeletePgRepository::new(oPool.clone());
            cRepository
                .vBlockIp(
                    I_FIXTURE_MODERATOR,
                    &StIpBanCommand {
                        sIp: S_FIXTURE_IP.to_owned(),
                        sReason: "success ban".to_owned(),
                        optBanUntil: None,
                        bAllowPosting: true,
                        bCaptchaRequired: true,
                    },
                )
                .await?;
            let stResult = cRepository
                .stDeleteByIp(I_FIXTURE_MODERATOR, &stIntegrationCommand(dtCutoff))
                .await?;
            vAssertSuccessfulMutation(&oPool, &stResult, dtCutoff).await?;

            vCleanupIntegrationFixtures(&oPool).await?;
            vInsertIntegrationFixtures(&oPool, dtCutoff).await?;
            // A stale delete-info row fails late, after row selection, topic
            // deletion, comment deletion, stats and event cleanup. Every one
            // of those changes must roll back together.
            sqlx::query(
                r#"INSERT INTO del_info(msgid,delby,reason,deldate,bonus)
                   VALUES($1,$2,'preexisting',CURRENT_TIMESTAMP,0)"#,
            )
            .bind(I_FROZEN_COMMENT)
            .bind(I_FIXTURE_MODERATOR)
            .execute(&oPool)
            .await?;
            cRepository
                .vBlockIp(
                    I_FIXTURE_MODERATOR,
                    &StIpBanCommand {
                        sIp: S_FIXTURE_IP.to_owned(),
                        sReason: "rollback ban persists".to_owned(),
                        optBanUntil: Some(Utc::now() + Duration::hours(1)),
                        bAllowPosting: false,
                        bCaptchaRequired: false,
                    },
                )
                .await?;
            assert!(
                cRepository
                    .stDeleteByIp(I_FIXTURE_MODERATOR, &stIntegrationCommand(dtCutoff))
                    .await
                    .is_err()
            );
            let (bTopicDeleted, bTopicSticky): (bool, bool) =
                sqlx::query_as("SELECT deleted,sticky FROM topics WHERE id=$1")
                    .bind(I_CANDIDATE_TOPIC)
                    .fetch_one(&oPool)
                    .await?;
            assert!(!bTopicDeleted);
            assert!(bTopicSticky);
            let iDeletedComments: i64 =
                sqlx::query_scalar("SELECT count(*) FROM comments WHERE id=ANY($1) AND deleted")
                    .bind(&ARR_FIXTURE_COMMENTS[..])
                    .fetch_one(&oPool)
                    .await?;
            assert_eq!(iDeletedComments, 0);
            let (iStat1, iStat3): (i32, i32) =
                sqlx::query_as("SELECT stat1,stat3 FROM topics WHERE id=$1")
                    .bind(I_HOST_TOPIC)
                    .fetch_one(&oPool)
                    .await?;
            assert_eq!((iStat1, iStat3), (8, 8));
            let (iSeededEvents, iUnread): (i64, i32) = sqlx::query_as(
                r#"SELECT count(e.id),max(u.unread_events)
                     FROM users u LEFT JOIN user_events e ON e.userid=u.id
                    WHERE u.id=$1"#,
            )
            .bind(I_FIXTURE_MODERATOR)
            .fetch_one(&oPool)
            .await?;
            assert_eq!((iSeededEvents, iUnread), (3, 3));
            let sPersistedBanReason: String =
                sqlx::query_scalar("SELECT reason FROM b_ips WHERE ip=$1::inet")
                    .bind(S_FIXTURE_IP)
                    .fetch_one(&oPool)
                    .await?;
            assert_eq!(sPersistedBanReason, "rollback ban persists");
            Ok(())
        }
        .await;

        let stCleanup = vCleanupIntegrationFixtures(&oPool).await;
        oPool.close().await;
        stRun.expect("delip integration assertions");
        stCleanup.expect("clean delip integration fixtures");
    }
}
