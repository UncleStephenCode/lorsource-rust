use async_trait::async_trait;
use chrono::SecondsFormat;
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    domain::user::{
        moderation::{
            EnUserModerationMutation, StMassDeleteResult, StModerationUser,
            StUserModerationMutationResult,
        },
        repository::TrUserModerationRepository,
    },
    error::Result,
};

const S_MASS_DELETE_REASON: &str = "Блокировка пользователя с удалением сообщений";

#[derive(Debug, Clone)]
pub struct CUserModerationPgRepository {
    oPool: PgPool,
}

impl CUserModerationPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrUserModerationRepository for CUserModerationPgRepository {
    async fn optUser(&self, iUserId: i32) -> Result<Option<StModerationUser>> {
        let optRow = sqlx::query_as::<_, (i32, String, bool, bool, bool, bool, bool, i32)>(
            r#"SELECT id, nick,
                      COALESCE(canmod,false),
                      COALESCE(candel,false),
                      COALESCE(passwd,'')='',
                      COALESCE(corrector,false),
                      COALESCE(blocked,false),
                      COALESCE(score,0)
               FROM users WHERE id=$1"#,
        )
        .bind(iUserId)
        .fetch_optional(&self.oPool)
        .await?;
        Ok(optRow.map(
            |(iId, sNick, bModerator, bAdministrator, bAnonymous, bCorrector, bBlocked, iScore)| {
                StModerationUser {
                    iId,
                    sNick,
                    bModerator,
                    bAdministrator,
                    bAnonymous,
                    bCorrector,
                    bBlocked,
                    iScore,
                }
            },
        ))
    }

    async fn stApply(
        &self,
        enMutation: EnUserModerationMutation,
    ) -> Result<StUserModerationMutationResult> {
        let mut oTransaction = self.oPool.begin().await?;
        let mut stResult = StUserModerationMutationResult::default();

        match enMutation {
            EnUserModerationMutation::Block {
                iTargetUserId,
                iModeratorId,
                sReason,
            } => {
                vBlock(&mut oTransaction, iTargetUserId, iModeratorId, &sReason).await?;
            }
            EnUserModerationMutation::Unblock {
                iTargetUserId,
                iModeratorId,
            } => {
                sqlx::query("UPDATE users SET blocked=false WHERE id=$1")
                    .bind(iTargetUserId)
                    .execute(&mut *oTransaction)
                    .await?;
                sqlx::query("DELETE FROM ban_info WHERE userid=$1")
                    .bind(iTargetUserId)
                    .execute(&mut *oTransaction)
                    .await?;
                vLog(
                    &mut oTransaction,
                    iTargetUserId,
                    iModeratorId,
                    "unblock_user",
                    Vec::new(),
                )
                .await?;
            }
            EnUserModerationMutation::Score50 {
                iTargetUserId,
                iModeratorId,
            } => {
                let stUpdated = sqlx::query(
                    r#"UPDATE users
                       SET score=GREATEST(score,50), max_score=GREATEST(max_score,50)
                       WHERE id=$1 AND score<50"#,
                )
                .bind(iTargetUserId)
                .execute(&mut *oTransaction)
                .await?;
                if stUpdated.rows_affected() > 0 {
                    vLog(
                        &mut oTransaction,
                        iTargetUserId,
                        iModeratorId,
                        "score50",
                        Vec::new(),
                    )
                    .await?;
                }
            }
            EnUserModerationMutation::SetCorrector {
                iTargetUserId,
                iModeratorId,
                bCorrector,
            } => {
                sqlx::query("UPDATE users SET corrector=$2 WHERE id=$1")
                    .bind(iTargetUserId)
                    .bind(bCorrector)
                    .execute(&mut *oTransaction)
                    .await?;
                vLog(
                    &mut oTransaction,
                    iTargetUserId,
                    iModeratorId,
                    if bCorrector {
                        "set_corrector"
                    } else {
                        "unset_corrector"
                    },
                    Vec::new(),
                )
                .await?;
            }
            EnUserModerationMutation::ResetPassword {
                iTargetUserId,
                iModeratorId,
                sPasswordHash,
            } => {
                sqlx::query("UPDATE users SET passwd=$2, lostpwd='epoch'::timestamptz WHERE id=$1")
                    .bind(iTargetUserId)
                    .bind(sPasswordHash)
                    .execute(&mut *oTransaction)
                    .await?;
                vLog(
                    &mut oTransaction,
                    iTargetUserId,
                    iModeratorId,
                    "reset_password",
                    Vec::new(),
                )
                .await?;
            }
            EnUserModerationMutation::ResetUserpic {
                iTargetUserId,
                iActorUserId,
                bScorePenalty,
            } => {
                let optOldUserpic: Option<String> =
                    sqlx::query_scalar("SELECT photo FROM users WHERE id=$1 FOR UPDATE")
                        .bind(iTargetUserId)
                        .fetch_one(&mut *oTransaction)
                        .await?;
                if let Some(sOldUserpic) = optOldUserpic {
                    if bScorePenalty {
                        sqlx::query("UPDATE users SET photo=NULL, score=score-10 WHERE id=$1")
                            .bind(iTargetUserId)
                            .execute(&mut *oTransaction)
                            .await?;
                    } else {
                        sqlx::query("UPDATE users SET photo=NULL WHERE id=$1")
                            .bind(iTargetUserId)
                            .execute(&mut *oTransaction)
                            .await?;
                    }
                    let mut vecInfo = vec![("old_userpic".to_owned(), sOldUserpic)];
                    if bScorePenalty {
                        vecInfo.push(("bonus".to_owned(), "-10".to_owned()));
                    }
                    vLog(
                        &mut oTransaction,
                        iTargetUserId,
                        iActorUserId,
                        "reset_userpic",
                        vecInfo,
                    )
                    .await?;
                }
            }
            EnUserModerationMutation::RemoveUserInfo {
                iTargetUserId,
                iModeratorId,
            } => {
                let optOldValue: Option<String> =
                    sqlx::query_scalar("SELECT userinfo FROM users WHERE id=$1 FOR UPDATE")
                        .bind(iTargetUserId)
                        .fetch_one(&mut *oTransaction)
                        .await?;
                if let Some(sOldValue) = optOldValue.filter(|sValue| !sValue.trim().is_empty()) {
                    sqlx::query("UPDATE users SET userinfo=NULL, score=score-10 WHERE id=$1")
                        .bind(iTargetUserId)
                        .execute(&mut *oTransaction)
                        .await?;
                    vLog(
                        &mut oTransaction,
                        iTargetUserId,
                        iModeratorId,
                        "reset_info",
                        vec![
                            ("old_info".to_owned(), sOldValue),
                            ("bonus".to_owned(), "-10".to_owned()),
                        ],
                    )
                    .await?;
                }
            }
            EnUserModerationMutation::RemoveTown {
                iTargetUserId,
                iModeratorId,
            } => {
                let optOldValue: Option<String> =
                    sqlx::query_scalar("SELECT town FROM users WHERE id=$1 FOR UPDATE")
                        .bind(iTargetUserId)
                        .fetch_one(&mut *oTransaction)
                        .await?;
                if let Some(sOldValue) = optOldValue.filter(|sValue| !sValue.trim().is_empty()) {
                    sqlx::query("UPDATE users SET town=NULL, score=score-10 WHERE id=$1")
                        .bind(iTargetUserId)
                        .execute(&mut *oTransaction)
                        .await?;
                    vLog(
                        &mut oTransaction,
                        iTargetUserId,
                        iModeratorId,
                        "reset_town",
                        vec![
                            ("old_town".to_owned(), sOldValue),
                            ("bonus".to_owned(), "-10".to_owned()),
                        ],
                    )
                    .await?;
                }
            }
            EnUserModerationMutation::RemoveUrl {
                iTargetUserId,
                iModeratorId,
            } => {
                let optOldValue: Option<String> =
                    sqlx::query_scalar("SELECT url FROM users WHERE id=$1 FOR UPDATE")
                        .bind(iTargetUserId)
                        .fetch_one(&mut *oTransaction)
                        .await?;
                if let Some(sOldValue) = optOldValue {
                    sqlx::query("UPDATE users SET url=NULL, score=score+0 WHERE id=$1")
                        .bind(iTargetUserId)
                        .execute(&mut *oTransaction)
                        .await?;
                    vLog(
                        &mut oTransaction,
                        iTargetUserId,
                        iModeratorId,
                        "reset_url",
                        vec![
                            ("old_url".to_owned(), sOldValue),
                            ("bonus".to_owned(), "0".to_owned()),
                        ],
                    )
                    .await?;
                }
            }
            EnUserModerationMutation::Freeze {
                iTargetUserId,
                iModeratorId,
                sReason,
                dtUntil,
                bDefrost,
            } => {
                sqlx::query(
                    r#"UPDATE users
                       SET frozen_until=$2, frozen_by=$3, freezing_reason=$4
                       WHERE id=$1"#,
                )
                .bind(iTargetUserId)
                .bind(dtUntil)
                .bind(iModeratorId)
                .bind(&sReason)
                .execute(&mut *oTransaction)
                .await?;
                let mut vecInfo = vec![("reason".to_owned(), sReason)];
                if !bDefrost {
                    vecInfo.push((
                        "until".to_owned(),
                        dtUntil.to_rfc3339_opts(SecondsFormat::AutoSi, true),
                    ));
                }
                vLog(
                    &mut oTransaction,
                    iTargetUserId,
                    iModeratorId,
                    if bDefrost { "defrosted" } else { "frozen" },
                    vecInfo,
                )
                .await?;
            }
            EnUserModerationMutation::BlockAndDelete {
                iTargetUserId,
                iModeratorId,
                sReason,
            } => {
                vBlock(&mut oTransaction, iTargetUserId, iModeratorId, &sReason).await?;
                stResult.optMassDelete =
                    Some(stMassDelete(&mut oTransaction, iTargetUserId, iModeratorId).await?);
            }
        }

        oTransaction.commit().await?;
        Ok(stResult)
    }
}

async fn vBlock(
    oTransaction: &mut Transaction<'_, Postgres>,
    iTargetUserId: i32,
    iModeratorId: i32,
    sReason: &str,
) -> Result<()> {
    sqlx::query("UPDATE users SET blocked=true WHERE id=$1")
        .bind(iTargetUserId)
        .execute(&mut **oTransaction)
        .await?;
    sqlx::query("INSERT INTO ban_info(userid, reason, ban_by) VALUES($1,$2,$3)")
        .bind(iTargetUserId)
        .bind(sReason)
        .bind(iModeratorId)
        .execute(&mut **oTransaction)
        .await?;
    vLog(
        oTransaction,
        iTargetUserId,
        iModeratorId,
        "block_user",
        vec![("reason".to_owned(), sReason.to_owned())],
    )
    .await
}

async fn stMassDelete(
    oTransaction: &mut Transaction<'_, Postgres>,
    iTargetUserId: i32,
    iModeratorId: i32,
) -> Result<StMassDeleteResult> {
    let vecCandidateTopicIds: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM topics WHERE userid=$1 AND NOT deleted FOR UPDATE")
            .bind(iTargetUserId)
            .fetch_all(&mut **oTransaction)
            .await?;
    let mut vecTopicIds = Vec::with_capacity(vecCandidateTopicIds.len());
    for iTopicId in vecCandidateTopicIds {
        let stUpdated =
            sqlx::query("UPDATE topics SET deleted=true, sticky=false WHERE id=$1 AND NOT deleted")
                .bind(iTopicId)
                .execute(&mut **oTransaction)
                .await?;
        if stUpdated.rows_affected() > 0 {
            vecTopicIds.push(iTopicId);
        }
    }
    vDeleteTopicEvents(oTransaction, &vecTopicIds).await?;

    // Java intentionally iterates newest-to-oldest and rechecks replies after
    // each deletion. Consequently a user's reply chain can be removed from
    // the leaves upward, while a comment with any surviving reply is skipped.
    let vecCandidateCommentIds: Vec<i32> = sqlx::query_scalar(
        r#"SELECT id FROM comments
           WHERE userid=$1 AND NOT deleted ORDER BY id DESC FOR UPDATE"#,
    )
    .bind(iTargetUserId)
    .fetch_all(&mut **oTransaction)
    .await?;
    let mut vecCommentIds = Vec::new();
    let mut vecSkippedCommentIds = Vec::new();
    for iCommentId in vecCandidateCommentIds {
        let bHasReplies: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM comments WHERE replyto=$1 AND NOT deleted)",
        )
        .bind(iCommentId)
        .fetch_one(&mut **oTransaction)
        .await?;
        if bHasReplies {
            vecSkippedCommentIds.push(iCommentId);
            continue;
        }

        let stUpdated = sqlx::query("UPDATE comments SET deleted=true WHERE id=$1 AND NOT deleted")
            .bind(iCommentId)
            .execute(&mut **oTransaction)
            .await?;
        if stUpdated.rows_affected() > 0 {
            sqlx::query(
                r#"UPDATE topics SET stat1=stat1-1, lastmod=CURRENT_TIMESTAMP
                   WHERE id=(SELECT topic FROM comments WHERE id=$1)"#,
            )
            .bind(iCommentId)
            .execute(&mut **oTransaction)
            .await?;
            sqlx::query(
                r#"UPDATE topics SET stat3=stat1
                   WHERE id=(SELECT topic FROM comments WHERE id=$1) AND stat3>stat1"#,
            )
            .bind(iCommentId)
            .execute(&mut **oTransaction)
            .await?;
            vecCommentIds.push(iCommentId);
        }
    }
    vDeleteCommentEvents(oTransaction, &vecCommentIds).await?;

    for iMessageId in vecTopicIds.iter().chain(&vecCommentIds) {
        sqlx::query(
            r#"INSERT INTO del_info(msgid,reason,bonus,delby,deldate)
               VALUES($1,$2,0,$3,CURRENT_TIMESTAMP)"#,
        )
        .bind(iMessageId)
        .bind(S_MASS_DELETE_REASON)
        .bind(iModeratorId)
        .execute(&mut **oTransaction)
        .await?;
    }

    Ok(StMassDeleteResult {
        vecTopicIds,
        vecCommentIds,
        vecSkippedCommentIds,
    })
}

async fn vDeleteTopicEvents(
    oTransaction: &mut Transaction<'_, Postgres>,
    vecTopicIds: &[i32],
) -> Result<()> {
    if vecTopicIds.is_empty() {
        return Ok(());
    }
    let vecUserIds: Vec<i32> = sqlx::query_scalar(
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
    vRecalculateUnread(oTransaction, &vecUserIds).await
}

async fn vDeleteCommentEvents(
    oTransaction: &mut Transaction<'_, Postgres>,
    vecCommentIds: &[i32],
) -> Result<()> {
    if vecCommentIds.is_empty() {
        return Ok(());
    }
    let vecUserIds: Vec<i32> = sqlx::query_scalar(
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
    vRecalculateUnread(oTransaction, &vecUserIds).await
}

async fn vRecalculateUnread(
    oTransaction: &mut Transaction<'_, Postgres>,
    vecUserIds: &[i32],
) -> Result<()> {
    if !vecUserIds.is_empty() {
        sqlx::query(
            r#"UPDATE users
               SET unread_events=(SELECT count(*) FROM user_events
                                  WHERE unread AND userid=users.id)
               WHERE id=ANY($1)"#,
        )
        .bind(vecUserIds)
        .execute(&mut **oTransaction)
        .await?;
    }
    Ok(())
}

async fn vLog(
    oTransaction: &mut Transaction<'_, Postgres>,
    iTargetUserId: i32,
    iModeratorId: i32,
    sAction: &str,
    vecInfo: Vec<(String, String)>,
) -> Result<()> {
    let (vecKeys, vecValues): (Vec<String>, Vec<String>) = vecInfo.into_iter().unzip();
    sqlx::query(
        r#"INSERT INTO user_log(userid,action_userid,action_date,action,info)
           VALUES($1,$2,CURRENT_TIMESTAMP,$3::user_log_action,hstore($4::text[],$5::text[]))"#,
    )
    .bind(iTargetUserId)
    .bind(iModeratorId)
    .bind(sAction)
    .bind(vecKeys)
    .bind(vecValues)
    .execute(&mut **oTransaction)
    .await?;
    Ok(())
}
