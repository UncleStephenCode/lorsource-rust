use std::collections::HashMap;

use async_trait::async_trait;
use sqlx::{FromRow, PgPool, Postgres, Transaction};

use crate::{
    domain::comment::deletion::{
        StCommentDeleteActor, StCommentDeleteMutation, StCommentDeletePreview,
        StCommentDeleteTarget, StDeleteCommentCommand, TrCommentDeletionRepository,
    },
    error::{AppError, Result},
};

const I_ANONYMOUS_USER_ID: i32 = 2;

const S_TARGET_SQL: &str = r#"
SELECT c.id AS i_comment_id,
       c.topic AS i_topic_id,
       c.userid AS i_author_id,
       u.nick AS s_author_nick,
       COALESCE(u.score,0) AS i_author_score,
       c.deleted AS b_deleted,
       t.deleted AS b_topic_deleted,
       (NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < CURRENT_TIMESTAMP-s.expire) AS b_topic_expired,
       COALESCE(t.draft,false) AS b_topic_draft,
       COALESCE(t.postscore,-9999)=10002 AS b_comments_hidden,
       EXISTS(SELECT 1 FROM comments r WHERE r.replyto=c.id AND NOT r.deleted) AS b_has_replies,
       c.postdate AS dt_postdate,
       di.delby AS opt_deleted_by,
       COALESCE(host(c.postip),'') AS s_post_ip,
       COALESCE(c.ua_id,0) AS i_user_agent_id,
       '/' || CASE s.id
         WHEN 1 THEN 'news'
         WHEN 2 THEN 'forum'
         WHEN 3 THEN 'gallery'
         WHEN 5 THEN 'polls'
         WHEN 6 THEN 'articles'
         ELSE lower(s.name)
       END || '/' || g.urlname || '/' || t.id AS s_canonical_topic_url
  FROM comments c
  JOIN users u ON u.id=c.userid
  JOIN topics t ON t.id=c.topic
  JOIN groups g ON g.id=t.groupid
  JOIN sections s ON s.id=g.section
  LEFT JOIN del_info di ON di.msgid=c.id
 WHERE c.id=$1
"#;

const S_DELETE_PREVIEW_SQL: &str = r#"
WITH RECURSIVE subtree AS (
  SELECT c.id,c.replyto,0::int AS depth,ARRAY[c.id]::int[] AS path
    FROM comments c WHERE c.id=$2
  UNION ALL
  SELECT c.id,c.replyto,st.depth+1,st.path||c.id
    FROM comments c JOIN subtree st ON c.replyto=st.id
   WHERE c.topic=(SELECT topic FROM comments WHERE id=$2)
     AND (NOT c.deleted OR EXISTS(
       SELECT 1 FROM users viewer WHERE viewer.id=$3 AND viewer.canmod
     ))
)
SELECT c.id AS i_comment_id,c.userid AS i_author_id,t.userid AS i_topic_author_id,
       c.deleted AS b_deleted,delete_info.delby AS opt_deleted_by_id,
       deleted_by.nick AS opt_deleted_by_nick,delete_info.reason AS opt_delete_reason,
       c.replyto AS opt_reply_to,
       (c.replyto IS NOT NULL AND (parent.id IS NULL OR
         (parent.deleted AND NOT EXISTS(
           SELECT 1 FROM users viewer WHERE viewer.id=$3 AND viewer.canmod
         )))) AS b_reply_deleted,
       CASE WHEN parent.id IS NOT NULL AND (NOT parent.deleted OR EXISTS(
         SELECT 1 FROM users viewer WHERE viewer.id=$3 AND viewer.canmod
       )) THEN parent.title END AS opt_reply_title,
       CASE WHEN parent.id IS NOT NULL AND (NOT parent.deleted OR EXISTS(
         SELECT 1 FROM users viewer WHERE viewer.id=$3 AND viewer.canmod
       )) THEN parent_user.nick END AS opt_reply_author,
       CASE WHEN parent.id IS NOT NULL AND (NOT parent.deleted OR EXISTS(
         SELECT 1 FROM users viewer WHERE viewer.id=$3 AND viewer.canmod
       )) THEN parent.postdate END AS opt_reply_postdate,
       st.depth AS i_depth,
       c.title AS s_title,m.message AS s_message,m.markup::text AS s_markup,
       c.postdate AS dt_postdate,u.nick AS s_author_nick,
       COALESCE(u.score,0) AS i_author_score,
       COALESCE(u.max_score,0) AS i_author_max_score,
       COALESCE(u.blocked,false) AS b_author_blocked,
       COALESCE(u.passwd,'')='' AS b_author_anonymous,
       COALESCE(u.frozen_until>CURRENT_TIMESTAMP,false) AS b_author_frozen,
       u.photo AS opt_photo,u.email AS opt_email,remark.remark_text AS opt_remark,
       COALESCE(c.edit_count,0) AS i_edit_count,
       (c.edit_date AT TIME ZONE $1::text) AS opt_edit_date,
       editor.nick AS opt_editor_nick,COALESCE(host(c.postip),'') AS s_post_ip,
       COALESCE(c.ua_id,0) AS i_user_agent_id,
       user_agent.name AS opt_user_agent,COALESCE(c.reactions,'{}'::jsonb)::text AS s_reactions_json,
       COALESCE((
         SELECT jsonb_agg(jsonb_build_object(
           'id',warning.id,'postdate',warning.postdate,'message',warning.message,
           'warning_type',warning.warning_type::text,
           'author',warning_author.nick,'author_blocked',COALESCE(warning_author.blocked,false),
           'closed_by',closed_by.nick
         ) ORDER BY warning.id)
           FROM message_warnings warning
           JOIN users warning_author ON warning_author.id=warning.author
           LEFT JOIN users closed_by ON closed_by.id=warning.closed_by
          WHERE warning.comment=c.id
       ),'[]'::jsonb)::text AS s_warnings_json
  FROM subtree st
  JOIN comments c ON c.id=st.id
  JOIN msgbase m ON m.id=c.id
  JOIN users u ON u.id=c.userid
  JOIN topics t ON t.id=c.topic
  LEFT JOIN comments parent ON parent.id=c.replyto AND parent.topic=c.topic
  LEFT JOIN users parent_user ON parent_user.id=parent.userid
  LEFT JOIN del_info delete_info ON delete_info.msgid=c.id
  LEFT JOIN users deleted_by ON deleted_by.id=delete_info.delby
  LEFT JOIN users editor ON editor.id=c.editor_id
  LEFT JOIN user_agents user_agent ON user_agent.id=c.ua_id
  LEFT JOIN user_remarks remark ON remark.user_id=$3 AND remark.ref_user_id=c.userid
 ORDER BY st.path
"#;

const S_UNDELETE_PREVIEW_SQL: &str = r#"
SELECT c.id AS i_comment_id,c.userid AS i_author_id,t.userid AS i_topic_author_id,
       c.deleted AS b_deleted,delete_info.delby AS opt_deleted_by_id,
       deleted_by.nick AS opt_deleted_by_nick,delete_info.reason AS opt_delete_reason,
       NULL::integer AS opt_reply_to,false AS b_reply_deleted,
       NULL::varchar AS opt_reply_title,NULL::varchar AS opt_reply_author,
       NULL::timestamptz AS opt_reply_postdate,0::int AS i_depth,
       c.title AS s_title,m.message AS s_message,m.markup::text AS s_markup,
       c.postdate AS dt_postdate,u.nick AS s_author_nick,
       COALESCE(u.score,0) AS i_author_score,
       COALESCE(u.max_score,0) AS i_author_max_score,
       COALESCE(u.blocked,false) AS b_author_blocked,
       COALESCE(u.passwd,'')='' AS b_author_anonymous,
       COALESCE(u.frozen_until>CURRENT_TIMESTAMP,false) AS b_author_frozen,
       u.photo AS opt_photo,u.email AS opt_email,NULL::text AS opt_remark,
       COALESCE(c.edit_count,0) AS i_edit_count,
       (c.edit_date AT TIME ZONE $1::text) AS opt_edit_date,
       editor.nick AS opt_editor_nick,COALESCE(host(c.postip),'') AS s_post_ip,
       COALESCE(c.ua_id,0) AS i_user_agent_id,
       user_agent.name AS opt_user_agent,COALESCE(c.reactions,'{}'::jsonb)::text AS s_reactions_json,
       '[]'::text AS s_warnings_json
  FROM comments c JOIN msgbase m ON m.id=c.id JOIN users u ON u.id=c.userid
  JOIN topics t ON t.id=c.topic
  LEFT JOIN del_info delete_info ON delete_info.msgid=c.id
  LEFT JOIN users deleted_by ON deleted_by.id=delete_info.delby
  LEFT JOIN users editor ON editor.id=c.editor_id
  LEFT JOIN user_agents user_agent ON user_agent.id=c.ua_id
 WHERE c.id=$2
"#;

#[derive(Debug, FromRow)]
struct StTargetRow {
    i_comment_id: i32,
    i_topic_id: i32,
    i_author_id: i32,
    s_author_nick: String,
    i_author_score: i32,
    b_deleted: bool,
    b_topic_deleted: bool,
    b_topic_expired: bool,
    b_topic_draft: bool,
    b_comments_hidden: bool,
    b_has_replies: bool,
    dt_postdate: chrono::DateTime<chrono::Utc>,
    opt_deleted_by: Option<i32>,
    s_post_ip: String,
    i_user_agent_id: i32,
    s_canonical_topic_url: String,
}

impl From<StTargetRow> for StCommentDeleteTarget {
    fn from(stRow: StTargetRow) -> Self {
        Self {
            iCommentId: stRow.i_comment_id,
            iTopicId: stRow.i_topic_id,
            iAuthorId: stRow.i_author_id,
            sAuthorNick: stRow.s_author_nick,
            iAuthorScore: stRow.i_author_score,
            bDeleted: stRow.b_deleted,
            bTopicDeleted: stRow.b_topic_deleted,
            bTopicExpired: stRow.b_topic_expired,
            bTopicDraft: stRow.b_topic_draft,
            bCommentsHidden: stRow.b_comments_hidden,
            bHasReplies: stRow.b_has_replies,
            dtPostdate: stRow.dt_postdate,
            optDeletedBy: stRow.opt_deleted_by,
            sPostIp: stRow.s_post_ip,
            iUserAgentId: stRow.i_user_agent_id,
            sCanonicalTopicUrl: stRow.s_canonical_topic_url,
        }
    }
}

#[derive(Debug, FromRow)]
struct StPreviewRow {
    i_comment_id: i32,
    i_author_id: i32,
    i_topic_author_id: i32,
    b_deleted: bool,
    opt_deleted_by_id: Option<i32>,
    opt_deleted_by_nick: Option<String>,
    opt_delete_reason: Option<String>,
    opt_reply_to: Option<i32>,
    b_reply_deleted: bool,
    opt_reply_title: Option<String>,
    opt_reply_author: Option<String>,
    opt_reply_postdate: Option<chrono::DateTime<chrono::Utc>>,
    i_depth: i32,
    s_title: String,
    s_message: String,
    s_markup: String,
    dt_postdate: chrono::DateTime<chrono::Utc>,
    s_author_nick: String,
    i_author_score: i32,
    i_author_max_score: i32,
    b_author_blocked: bool,
    b_author_anonymous: bool,
    b_author_frozen: bool,
    opt_photo: Option<String>,
    opt_email: Option<String>,
    opt_remark: Option<String>,
    i_edit_count: i32,
    opt_edit_date: Option<chrono::DateTime<chrono::Utc>>,
    opt_editor_nick: Option<String>,
    s_post_ip: String,
    i_user_agent_id: i32,
    opt_user_agent: Option<String>,
    s_reactions_json: String,
    s_warnings_json: String,
}

impl From<StPreviewRow> for StCommentDeletePreview {
    fn from(stRow: StPreviewRow) -> Self {
        Self {
            iCommentId: stRow.i_comment_id,
            iAuthorId: stRow.i_author_id,
            iTopicAuthorId: stRow.i_topic_author_id,
            bDeleted: stRow.b_deleted,
            optDeletedById: stRow.opt_deleted_by_id,
            optDeletedByNick: stRow.opt_deleted_by_nick,
            optDeleteReason: stRow.opt_delete_reason,
            optReplyTo: stRow.opt_reply_to,
            bReplyDeleted: stRow.b_reply_deleted,
            optReplyTitle: stRow.opt_reply_title,
            optReplyAuthor: stRow.opt_reply_author,
            optReplyPostdate: stRow.opt_reply_postdate,
            iDepth: stRow.i_depth,
            sTitle: stRow.s_title,
            sMessage: stRow.s_message,
            sMarkup: stRow.s_markup,
            dtPostdate: stRow.dt_postdate,
            sAuthorNick: stRow.s_author_nick,
            iAuthorScore: stRow.i_author_score,
            iAuthorMaxScore: stRow.i_author_max_score,
            bAuthorBlocked: stRow.b_author_blocked,
            bAuthorAnonymous: stRow.b_author_anonymous,
            bAuthorFrozen: stRow.b_author_frozen,
            optPhoto: stRow.opt_photo,
            optEmail: stRow.opt_email,
            optRemark: stRow.opt_remark,
            iEditCount: stRow.i_edit_count,
            optEditDate: stRow.opt_edit_date,
            optEditorNick: stRow.opt_editor_nick,
            sPostIp: stRow.s_post_ip,
            iUserAgentId: stRow.i_user_agent_id,
            optUserAgent: stRow.opt_user_agent,
            sReactionsJson: stRow.s_reactions_json,
            sWarningsJson: stRow.s_warnings_json,
        }
    }
}

#[derive(Debug, Clone, Copy, FromRow)]
struct StReplyNode {
    i_id: i32,
    i_author_id: i32,
    opt_reply_to: Option<i32>,
    i_depth: i32,
}

#[derive(Debug, Clone)]
pub struct CCommentDeletionPgRepository {
    oPool: PgPool,
    stLegacyJdbcTimezone: chrono_tz::Tz,
}

impl CCommentDeletionPgRepository {
    pub fn new(oPool: PgPool, stLegacyJdbcTimezone: chrono_tz::Tz) -> Self {
        Self {
            oPool,
            stLegacyJdbcTimezone,
        }
    }
}

#[async_trait]
impl TrCommentDeletionRepository for CCommentDeletionPgRepository {
    async fn optFindTarget(&self, iCommentId: i32) -> Result<Option<StCommentDeleteTarget>> {
        Ok(sqlx::query_as::<_, StTargetRow>(S_TARGET_SQL)
            .bind(iCommentId)
            .fetch_optional(&self.oPool)
            .await?
            .map(Into::into))
    }

    async fn vecDeletePreview(
        &self,
        iCommentId: i32,
        iViewerId: i32,
    ) -> Result<Vec<StCommentDeletePreview>> {
        Ok(sqlx::query_as::<_, StPreviewRow>(S_DELETE_PREVIEW_SQL)
            .bind(
                crate::infra::postgres::legacy_timestamp::sLegacyJdbcTimezone(
                    self.stLegacyJdbcTimezone,
                ),
            )
            .bind(iCommentId)
            .bind(iViewerId)
            .fetch_all(&self.oPool)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    async fn vecUndeletePreview(
        &self,
        iCommentId: i32,
        _iViewerId: i32,
    ) -> Result<Vec<StCommentDeletePreview>> {
        Ok(sqlx::query_as::<_, StPreviewRow>(S_UNDELETE_PREVIEW_SQL)
            .bind(
                crate::infra::postgres::legacy_timestamp::sLegacyJdbcTimezone(
                    self.stLegacyJdbcTimezone,
                ),
            )
            .bind(iCommentId)
            .fetch_all(&self.oPool)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    async fn stDelete(
        &self,
        stActor: StCommentDeleteActor,
        stTarget: &StCommentDeleteTarget,
        stCommand: &StDeleteCommentCommand,
    ) -> Result<StCommentDeleteMutation> {
        let mut oTransaction = self.oPool.begin().await?;

        if !stActor.bModerator {
            let iReplyCount: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM comments WHERE replyto=$1 AND NOT deleted",
            )
            .bind(stTarget.iCommentId)
            .fetch_one(&mut *oTransaction)
            .await?;
            if iReplyCount != 0 {
                return Err(AppError::Forbidden);
            }
        }

        let iRootBonus = -stCommand.iPenalty;
        let mut vecDeletedIds = Vec::new();
        if stActor.bModerator && stCommand.bDeleteReplies {
            let vecNodes = vecReplyNodesPostorder(&mut oTransaction, stTarget.iCommentId).await?;
            let bDropScore = iRootBonus < -2;
            for stNode in vecNodes {
                let (iBonus, sReason) = stReplyBonusAndReason(bDropScore, stNode.i_depth);
                if bDeleteOne(
                    &mut oTransaction,
                    stNode.i_id,
                    stNode.i_author_id,
                    stActor.iUserId,
                    sReason,
                    iBonus,
                    stTarget.iTopicId,
                    !stTarget.bTopicExpired,
                )
                .await?
                {
                    vecDeletedIds.push(stNode.i_id);
                }
            }
        }

        if bDeleteOne(
            &mut oTransaction,
            stTarget.iCommentId,
            stTarget.iAuthorId,
            stActor.iUserId,
            &stCommand.sReason,
            iRootBonus,
            stTarget.iTopicId,
            true,
        )
        .await?
        {
            vecDeletedIds.push(stTarget.iCommentId);
        }

        if !vecDeletedIds.is_empty() {
            sqlx::query("UPDATE topics SET stat1=stat1-$2,lastmod=CURRENT_TIMESTAMP WHERE id=$1")
                .bind(stTarget.iTopicId)
                .bind(vecDeletedIds.len() as i32)
                .execute(&mut *oTransaction)
                .await?;
            sqlx::query("UPDATE topics SET stat3=stat1 WHERE id=$1 AND stat3>stat1")
                .bind(stTarget.iTopicId)
                .execute(&mut *oTransaction)
                .await?;
        }
        vDeleteEvents(&mut oTransaction, &vecDeletedIds).await?;
        oTransaction.commit().await?;

        // DeleteCommentController.findNextComment runs after DeleteService's
        // localTx completes and before the search-queue send.
        let optNextCommentId: Option<i32> = sqlx::query_scalar(
            "SELECT min(id) FROM comments WHERE topic=$1 AND NOT deleted AND id >= $2",
        )
        .bind(stTarget.iTopicId)
        .bind(stTarget.iCommentId)
        .fetch_one(&self.oPool)
        .await?;
        Ok(StCommentDeleteMutation {
            vecDeletedIds,
            optNextCommentId,
        })
    }

    async fn vUndelete(&self, stTarget: &StCommentDeleteTarget) -> Result<()> {
        let mut oTransaction = self.oPool.begin().await?;
        let optBonus: Option<i32> =
            sqlx::query_scalar("SELECT bonus FROM del_info WHERE msgid=$1 FOR UPDATE")
                .bind(stTarget.iCommentId)
                .fetch_optional(&mut *oTransaction)
                .await?
                .flatten();
        if let Some(iBonus) = optBonus.filter(|iValue| *iValue != 0) {
            sqlx::query("UPDATE users SET score=score-$2 WHERE id=$1")
                .bind(stTarget.iAuthorId)
                .bind(iBonus)
                .execute(&mut *oTransaction)
                .await?;
        }
        sqlx::query("UPDATE comments SET deleted=false WHERE id=$1")
            .bind(stTarget.iCommentId)
            .execute(&mut *oTransaction)
            .await?;
        sqlx::query("DELETE FROM del_info WHERE msgid=$1")
            .bind(stTarget.iCommentId)
            .execute(&mut *oTransaction)
            .await?;
        sqlx::query("UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id=$1")
            .bind(stTarget.iTopicId)
            .execute(&mut *oTransaction)
            .await?;
        oTransaction.commit().await?;
        Ok(())
    }
}

async fn vecReplyNodesPostorder(
    oTransaction: &mut Transaction<'_, Postgres>,
    iRootId: i32,
) -> Result<Vec<StReplyNode>> {
    let vecNodes: Vec<StReplyNode> = sqlx::query_as(
        r#"WITH RECURSIVE subtree AS (
             SELECT id,userid,replyto,0::int AS depth FROM comments
              WHERE replyto=$1 AND NOT deleted
             UNION ALL
             SELECT c.id,c.userid,c.replyto,st.depth+1
               FROM comments c JOIN subtree st ON c.replyto=st.id
              WHERE NOT c.deleted
           ) SELECT id AS i_id,userid AS i_author_id,replyto AS opt_reply_to,depth AS i_depth
               FROM subtree"#,
    )
    .bind(iRootId)
    .fetch_all(&mut **oTransaction)
    .await?;
    let mut mapChildren: HashMap<i32, Vec<StReplyNode>> = HashMap::new();
    for stNode in vecNodes {
        mapChildren
            .entry(stNode.opt_reply_to.unwrap_or(iRootId))
            .or_default()
            .push(stNode);
    }
    for vecChildren in mapChildren.values_mut() {
        vecChildren.sort_by_key(|stNode| stNode.i_id);
    }
    let mut vecPostorder = Vec::new();
    vAppendPostorder(iRootId, &mapChildren, &mut vecPostorder);
    Ok(vecPostorder)
}

fn vAppendPostorder(
    iParentId: i32,
    mapChildren: &HashMap<i32, Vec<StReplyNode>>,
    vecOutput: &mut Vec<StReplyNode>,
) {
    if let Some(vecChildren) = mapChildren.get(&iParentId) {
        for stChild in vecChildren {
            vAppendPostorder(stChild.i_id, mapChildren, vecOutput);
            vecOutput.push(*stChild);
        }
    }
}

fn stReplyBonusAndReason(bDropScore: bool, iDepth: i32) -> (i32, &'static str) {
    if !bDropScore {
        return (0, "7.1 Ответ на некорректное сообщение (авто)");
    }
    match iDepth {
        0 => (-2, "7.1 Ответ на некорректное сообщение (авто, уровень 0)"),
        1 => (-1, "7.1 Ответ на некорректное сообщение (авто, уровень 1)"),
        _ => (0, "7.1 Ответ на некорректное сообщение (авто, уровень >1)"),
    }
}

async fn bDeleteOne(
    oTransaction: &mut Transaction<'_, Postgres>,
    iCommentId: i32,
    iAuthorId: i32,
    iDeletedBy: i32,
    sReason: &str,
    iRequestedBonus: i32,
    iTopicId: i32,
    bNotify: bool,
) -> Result<bool> {
    let stUpdate = sqlx::query("UPDATE comments SET deleted=true WHERE id=$1 AND NOT deleted")
        .bind(iCommentId)
        .execute(&mut **oTransaction)
        .await?;
    if stUpdate.rows_affected() == 0 {
        return Ok(false);
    }
    let iEffectiveBonus = iEffectiveBonus(oTransaction, iAuthorId, iRequestedBonus).await?;
    if iEffectiveBonus != 0 {
        sqlx::query("UPDATE users SET score=score+$2 WHERE id=$1")
            .bind(iAuthorId)
            .bind(iEffectiveBonus)
            .execute(&mut **oTransaction)
            .await?;
    }
    sqlx::query(
        "INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES($1,$2,$3,CURRENT_TIMESTAMP,$4)",
    )
    .bind(iCommentId)
    .bind(iDeletedBy)
    .bind(sReason)
    .bind(iEffectiveBonus)
    .execute(&mut **oTransaction)
    .await?;
    if bNotify {
        vNotifyDeleted(
            oTransaction,
            iAuthorId,
            iDeletedBy,
            iTopicId,
            iCommentId,
            sReason,
        )
        .await?;
    }
    Ok(true)
}

async fn iEffectiveBonus(
    oTransaction: &mut Transaction<'_, Postgres>,
    iAuthorId: i32,
    iRequestedBonus: i32,
) -> Result<i32> {
    if iRequestedBonus == 0 || iAuthorId == I_ANONYMOUS_USER_ID {
        return Ok(iRequestedBonus);
    }
    let bFrozen: bool = sqlx::query_scalar(
        "SELECT COALESCE(frozen_until>CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
    )
    .bind(iAuthorId)
    .fetch_one(&mut **oTransaction)
    .await?;
    Ok(if bFrozen { 0 } else { iRequestedBonus })
}

async fn vNotifyDeleted(
    oTransaction: &mut Transaction<'_, Postgres>,
    iAuthorId: i32,
    iDeletedBy: i32,
    iTopicId: i32,
    iCommentId: i32,
    sReason: &str,
) -> Result<()> {
    if iAuthorId == iDeletedBy || iAuthorId == I_ANONYMOUS_USER_ID {
        return Ok(());
    }
    let bFrozen: bool = sqlx::query_scalar(
        "SELECT COALESCE(frozen_until>CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
    )
    .bind(iAuthorId)
    .fetch_one(&mut **oTransaction)
    .await?;
    if bFrozen {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO user_events(userid,type,private,message_id,comment_id,message) VALUES($1,'DEL',true,$2,$3,$4)",
    )
    .bind(iAuthorId)
    .bind(iTopicId)
    .bind(iCommentId)
    .bind(sReason)
    .execute(&mut **oTransaction)
    .await?;
    Ok(())
}

async fn vDeleteEvents(
    oTransaction: &mut Transaction<'_, Postgres>,
    vecCommentIds: &[i32],
) -> Result<()> {
    if vecCommentIds.is_empty() {
        return Ok(());
    }
    let vecAffectedUsers: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT userid FROM user_events WHERE comment_id=ANY($1) AND type IN ('REPLY','WATCH','REF','REACTION','WARNING')",
    )
    .bind(vecCommentIds)
    .fetch_all(&mut **oTransaction)
    .await?;
    sqlx::query(
        "DELETE FROM user_events WHERE comment_id=ANY($1) AND type IN ('REPLY','WATCH','REF','REACTION','WARNING')",
    )
    .bind(vecCommentIds)
    .execute(&mut **oTransaction)
    .await?;
    if !vecAffectedUsers.is_empty() {
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=ANY($1)")
            .bind(&vecAffectedUsers)
            .execute(&mut **oTransaction)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_preview_selects_warning_type_for_prepared_message_prefix() {
        assert!(S_DELETE_PREVIEW_SQL.contains("'warning_type',warning.warning_type::text"));
    }

    #[test]
    fn java_timestamp_without_timezone_edit_date_uses_the_shared_iana_parameter() {
        for sSql in [S_DELETE_PREVIEW_SQL, S_UNDELETE_PREVIEW_SQL] {
            assert!(sSql.contains(
                crate::infra::postgres::legacy_timestamp::S_LEGACY_TIMESTAMP_SQL_EXPRESSION
            ));
            assert!(!sSql.contains("AT TIME ZONE 'UTC'"));
        }
    }

    #[test]
    fn mutation_contract_is_additive_and_never_upserts_delete_info() {
        let sProductionSource = include_str!("comment_deletion_repository.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(sProductionSource.contains("SET score=score+$2"));
        assert!(sProductionSource.contains("SET score=score-$2"));
        assert!(!sProductionSource.contains("GREATEST(score"));
        assert!(!sProductionSource.contains("ON CONFLICT"));
    }

    #[test]
    fn reply_penalty_uses_the_raw_root_request() {
        assert_eq!(stReplyBonusAndReason(true, 0).0, -2);
        assert_eq!(stReplyBonusAndReason(true, 1).0, -1);
        assert_eq!(stReplyBonusAndReason(true, 2).0, 0);
        assert_eq!(stReplyBonusAndReason(false, 0).0, 0);
    }
}
