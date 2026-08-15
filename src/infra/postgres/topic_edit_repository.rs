use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool, Postgres, Transaction};

use crate::{
    domain::topic::{
        edit::{
            StTopicEditCommand, StTopicEditEditor, StTopicEditGroup, StTopicEditMutationResult,
            StTopicEditPoll, StTopicEditPollValue, StTopicEditPollVariant, StTopicEditRestrictions,
            StTopicEditSnapshot, TrTopicEditRepository,
        },
        posting::StIpBlockInfo,
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
       t.draft AS b_draft,
       t.moderate AS b_committed,
       t.sticky AS b_sticky,
       (NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < CURRENT_TIMESTAMP-s.expire)
         AS b_expired,
       COALESCE(t.postscore,-9999) AS i_post_score,
       t.postdate AS dt_post_date,
       t.commitdate AS opt_commit_date,
       t.lastmod AS dt_last_mod,
       t.minor AS b_minor
  FROM topics t
  JOIN msgbase m ON m.id=t.id
  JOIN users u ON u.id=t.userid
  JOIN groups g ON g.id=t.groupid
  JOIN sections s ON s.id=g.section
 WHERE t.id=$1
"#;

const S_RESTRICTIONS_SQL: &str = r#"
SELECT COALESCE(u.frozen_until>CURRENT_TIMESTAMP,false),
       COALESCE((
         SELECT bi.ban_date IS NULL OR bi.ban_date>CURRENT_TIMESTAMP
           FROM b_ips bi WHERE bi.ip=$2::inet
       ),false),
       COALESCE((
         SELECT bi.allow_posting
           FROM b_ips bi WHERE bi.ip=$2::inet
       ),false)
  FROM users u WHERE u.id=$1
"#;

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
    b_expired: bool,
    i_post_score: i32,
    dt_post_date: DateTime<Utc>,
    opt_commit_date: Option<DateTime<Utc>>,
    dt_last_mod: DateTime<Utc>,
    b_minor: bool,
}

#[derive(Debug, Clone)]
pub struct CTopicEditPgRepository {
    oPool: PgPool,
}

impl CTopicEditPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrTopicEditRepository for CTopicEditPgRepository {
    async fn optSnapshot(&self, iTopicId: i32) -> Result<Option<StTopicEditSnapshot>> {
        let Some(stRow) = sqlx::query_as::<_, StSnapshotRow>(S_SNAPSHOT_SQL)
            .bind(iTopicId)
            .fetch_optional(&self.oPool)
            .await?
        else {
            return Ok(None);
        };
        let vecTags = sqlx::query_scalar(
            r#"SELECT tv.value FROM tags t
               JOIN tags_values tv ON tv.id=t.tagid
               WHERE t.msgid=$1 ORDER BY tv.value"#,
        )
        .bind(iTopicId)
        .fetch_all(&self.oPool)
        .await?;
        let vecGroups = sqlx::query_as::<_, (i32, String, i32)>(
            "SELECT id,title,section FROM groups WHERE section=$1 ORDER BY id",
        )
        .bind(stRow.i_section_id)
        .fetch_all(&self.oPool)
        .await?
        .into_iter()
        .map(|(iId, sTitle, iSectionId)| StTopicEditGroup {
            iId,
            sTitle,
            iSectionId,
        })
        .collect();
        let optLastEditMillis = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT editdate FROM edit_info WHERE msgid=$1 AND object_type='TOPIC'::edit_event_type ORDER BY id DESC LIMIT 1",
        )
        .bind(iTopicId)
        .fetch_optional(&self.oPool)
        .await?
        .map(|dtValue| dtValue.timestamp_millis());
        let vecEditors = sqlx::query_as::<_, (i32, String, i32, bool)>(
            r#"SELECT u.id,u.nick,COALESCE(u.score,0),COALESCE(u.blocked,false)
                 FROM users u
                WHERE u.id IN (
                  SELECT DISTINCT ei.editor FROM edit_info ei
                   WHERE ei.msgid=$1 AND ei.object_type='TOPIC'::edit_event_type
                     AND ei.editor<>$2
                )
                ORDER BY u.id"#,
        )
        .bind(iTopicId)
        .bind(stRow.i_author_id)
        .fetch_all(&self.oPool)
        .await?
        .into_iter()
        .map(|(iId, sNick, iScore, bBlocked)| StTopicEditEditor {
            iId,
            sNick,
            iScore,
            bBlocked,
        })
        .collect();
        let optPoll = optLoadPoll(&self.oPool, iTopicId).await?;

        Ok(Some(StTopicEditSnapshot {
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
            bExpired: stRow.b_expired,
            iPostScore: stRow.i_post_score,
            dtPostDate: stRow.dt_post_date,
            optCommitDate: stRow.opt_commit_date,
            dtLastMod: stRow.dt_last_mod,
            bMinor: stRow.b_minor,
            vecTags,
            optPoll,
            vecGroups,
            optLastEditMillis,
            vecEditors,
        }))
    }

    async fn stRestrictions(
        &self,
        iUserId: i32,
        sRemoteIp: &str,
    ) -> Result<StTopicEditRestrictions> {
        let (bFrozen, bBlocked, bAllowPosting): (bool, bool, bool) =
            sqlx::query_as(S_RESTRICTIONS_SQL)
                .bind(iUserId)
                .bind(sRemoteIp)
                .fetch_one(&self.oPool)
                .await?;
        Ok(StTopicEditRestrictions {
            bFrozen,
            stIpBlock: StIpBlockInfo {
                bBlocked,
                bAllowRegisteredPosting: !bBlocked || bAllowPosting,
            },
        })
    }

    async fn vecNewTags(&self, vecTags: &[String]) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            r#"SELECT input.tag
                 FROM unnest($1::text[]) WITH ORDINALITY AS input(tag,ord)
                WHERE NOT EXISTS(
                        SELECT 1 FROM tags_values tv
                         WHERE tv.value=input.tag AND tv.counter>0
                      )
                  AND NOT EXISTS(
                        SELECT 1 FROM tags_synonyms ts
                         WHERE ts.value=input.tag
                      )
                ORDER BY input.ord"#,
        )
        .bind(vecTags)
        .fetch_all(&self.oPool)
        .await?)
    }

    async fn stUpdateAndCommit(
        &self,
        stCommand: StTopicEditCommand,
    ) -> Result<StTopicEditMutationResult> {
        let mut oTransaction = self.oPool.begin().await?;
        let stCurrent = stLoadCurrent(&mut oTransaction, stCommand.iTopicId).await?;

        let vecOldTags = vecLoadTagsTx(&mut oTransaction, stCommand.iTopicId).await?;
        let optOldPoll = optLoadPollTx(&mut oTransaction, stCommand.iTopicId).await?;
        let vecOldImageIds: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM images WHERE topic=$1 AND NOT deleted ORDER BY main DESC,id",
        )
        .bind(stCommand.iTopicId)
        .fetch_all(&mut *oTransaction)
        .await?;

        let bMessageModified = stCommand
            .optMessage
            .as_deref()
            .is_some_and(|sValue| sValue != stCurrent.s_message);
        let bTitleModified = stCommand
            .optTitle
            .as_deref()
            .is_some_and(|sValue| sValue != stCurrent.s_title);
        let bUrlModified = stCommand.optUrl.as_deref().is_some_and(|sValue| {
            !bEqualNullableStrings(stCurrent.opt_url.as_deref(), Some(sValue))
        });
        let bLinkTextModified = stCommand.optLinkText.as_deref().is_some_and(|sValue| {
            !bEqualNullableStrings(stCurrent.opt_link_text.as_deref(), Some(sValue))
        });
        let bMinorModified = stCommand.bMinor != stCurrent.b_minor;
        let bTagsModified = stCommand
            .optTags
            .as_ref()
            .is_some_and(|vecTags| !bSameTags(&vecOldTags, vecTags));
        let bPollModified = stCommand.optPoll.as_ref().is_some_and(|vecPoll| {
            bPollModified(optOldPoll.as_ref(), vecPoll, stCommand.bMultiSelect)
        });
        let bImagesModified = !stCommand.vecPreviewNames.is_empty();
        let bModified = bMessageModified
            || bTitleModified
            || bUrlModified
            || bLinkTextModified
            || bMinorModified
            || bTagsModified
            || bPollModified
            || bImagesModified;

        if bMessageModified {
            sqlx::query("UPDATE msgbase SET message=$2 WHERE id=$1")
                .bind(stCommand.iTopicId)
                .bind(stCommand.optMessage.as_deref())
                .execute(&mut *oTransaction)
                .await?;
        }
        if bTitleModified {
            sqlx::query("UPDATE topics SET title=$2 WHERE id=$1")
                .bind(stCommand.iTopicId)
                .bind(stCommand.optTitle.as_deref())
                .execute(&mut *oTransaction)
                .await?;
        }
        if bUrlModified {
            sqlx::query("UPDATE topics SET url=$2 WHERE id=$1")
                .bind(stCommand.iTopicId)
                .bind(stCommand.optUrl.as_deref())
                .execute(&mut *oTransaction)
                .await?;
        }
        if bLinkTextModified {
            sqlx::query("UPDATE topics SET linktext=$2 WHERE id=$1")
                .bind(stCommand.iTopicId)
                .bind(stCommand.optLinkText.as_deref())
                .execute(&mut *oTransaction)
                .await?;
        }
        if bMinorModified {
            sqlx::query("UPDATE topics SET minor=$2 WHERE id=$1")
                .bind(stCommand.iTopicId)
                .bind(stCommand.bMinor)
                .execute(&mut *oTransaction)
                .await?;
        }
        if bTagsModified {
            vReplaceTagsTx(
                &mut oTransaction,
                stCommand.iTopicId,
                stCommand.optTags.as_deref().unwrap_or_default(),
            )
            .await?;
        }
        if bPollModified {
            let stOldPoll = optOldPoll.as_ref().ok_or_else(|| {
                AppError::Anyhow(anyhow::anyhow!(
                    "poll is missing for editable poll topic {}",
                    stCommand.iTopicId
                ))
            })?;
            vUpdatePollTx(
                &mut oTransaction,
                stOldPoll,
                stCommand.optPoll.as_deref().unwrap_or_default(),
                stCommand.bMultiSelect,
            )
            .await?;
        }

        let mut stImageRollback = StImageDirectoryRollback::default();
        if bImagesModified {
            for sPreviewName in &stCommand.vecPreviewNames {
                vSavePreviewImageTx(
                    &mut oTransaction,
                    stCommand.iTopicId,
                    stCommand.iEditorId,
                    &stCommand.sUploadRoot,
                    sPreviewName,
                    &mut stImageRollback,
                )
                .await?;
            }
        }

        if bModified {
            let optOldPollJson = bPollModified
                .then(|| {
                    optOldPoll
                        .as_ref()
                        .map(|stPoll| stPollHistoryJson(stPoll, stCommand.iTopicId))
                })
                .flatten();
            sqlx::query(
                r#"INSERT INTO edit_info(
                     msgid,editor,oldmessage,oldtitle,oldtags,oldlinktext,oldurl,
                     object_type,oldminor,oldpoll,oldaddimages
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,'TOPIC'::edit_event_type,$8,$9,$10)"#,
            )
            .bind(stCommand.iTopicId)
            .bind(stCommand.iEditorId)
            .bind(bMessageModified.then_some(stCurrent.s_message.as_str()))
            .bind(bTitleModified.then_some(stCurrent.s_title.as_str()))
            .bind(bTagsModified.then(|| vecOldTags.join(",")))
            .bind(
                bLinkTextModified
                    .then_some(stCurrent.opt_link_text.as_deref())
                    .flatten(),
            )
            .bind(
                bUrlModified
                    .then_some(stCurrent.opt_url.as_deref())
                    .flatten(),
            )
            .bind(bMinorModified.then_some(stCurrent.b_minor))
            .bind(optOldPollJson.map(sqlx::types::Json))
            .bind(bImagesModified.then_some(vecOldImageIds))
            .execute(&mut *oTransaction)
            .await?;
            sqlx::query("UPDATE topics SET lastmod=lastmod+'1 second'::interval WHERE id=$1")
                .bind(stCommand.iTopicId)
                .execute(&mut *oTransaction)
                .await?;
        }

        let vecNotifiedUserIds = if !stCommand.bNewMessageDraft && !stCurrent.b_expired {
            vecNotifyUsersTx(
                &mut oTransaction,
                stCommand.iTopicId,
                stCurrent.i_author_id,
                &stCommand.vecMentionedNicks,
                stCommand.bSendTagEvents,
                &stCurrent.s_markup,
            )
            .await?
        } else {
            Vec::new()
        };

        if stCommand.bPublish {
            sqlx::query(
                "UPDATE topics SET draft=false,postdate=CURRENT_TIMESTAMP,lastmod=CURRENT_TIMESTAMP WHERE id=$1 AND draft",
            )
            .bind(stCommand.iTopicId)
            .execute(&mut *oTransaction)
            .await?;
        }
        if stCommand.bCommit {
            if let Some(iChangeGroupId) = stCommand.optChangeGroupId
                && iChangeGroupId != stCurrent.i_group_id
            {
                let stResult = sqlx::query(
                    r#"UPDATE topics t SET groupid=$2,lastmod=CURRENT_TIMESTAMP
                        WHERE t.id=$1 AND EXISTS(
                          SELECT 1 FROM groups old_group,groups new_group
                           WHERE old_group.id=t.groupid AND new_group.id=$2
                             AND old_group.section=new_group.section
                        )"#,
                )
                .bind(stCommand.iTopicId)
                .bind(iChangeGroupId)
                .execute(&mut *oTransaction)
                .await?;
                if stResult.rows_affected() != 1 {
                    return Err(AppError::Forbidden);
                }
            }
            if stCurrent.b_draft {
                sqlx::query(
                    "UPDATE topics SET draft=false,postdate=CURRENT_TIMESTAMP,lastmod=CURRENT_TIMESTAMP WHERE id=$1 AND draft",
                )
                .bind(stCommand.iTopicId)
                .execute(&mut *oTransaction)
                .await?;
            }
            sqlx::query(
                "UPDATE topics SET moderate=true,commitby=$2,commitdate=CURRENT_TIMESTAMP,lastmod=CURRENT_TIMESTAMP WHERE id=$1",
            )
            .bind(stCommand.iTopicId)
            .bind(stCommand.iEditorId)
            .execute(&mut *oTransaction)
            .await?;
            vChangeScoreTx(&mut oTransaction, stCurrent.i_author_id, stCommand.iBonus).await?;
            for &(iEditorId, iBonus) in &stCommand.vecEditorBonus {
                vChangeScoreTx(&mut oTransaction, iEditorId, iBonus).await?;
            }
        }

        oTransaction.commit().await?;
        stImageRollback.vCommit();
        Ok(StTopicEditMutationResult {
            bModified,
            vecNotifiedUserIds,
        })
    }
}

async fn optLoadPoll(oPool: &PgPool, iTopicId: i32) -> Result<Option<StTopicEditPoll>> {
    let Some((iPollId, bMultiSelect)): Option<(i32, bool)> =
        sqlx::query_as("SELECT id,multiselect FROM polls WHERE topic=$1")
            .bind(iTopicId)
            .fetch_optional(oPool)
            .await?
    else {
        return Ok(None);
    };
    let vecVariants = sqlx::query_as::<_, (i32, String)>(
        "SELECT id,label FROM polls_variants WHERE vote=$1 ORDER BY id",
    )
    .bind(iPollId)
    .fetch_all(oPool)
    .await?
    .into_iter()
    .map(|(iId, sLabel)| StTopicEditPollVariant { iId, sLabel })
    .collect();
    Ok(Some(StTopicEditPoll {
        iId: iPollId,
        bMultiSelect,
        vecVariants,
    }))
}

#[derive(Debug, FromRow)]
struct StCurrentRow {
    i_author_id: i32,
    i_group_id: i32,
    s_title: String,
    s_message: String,
    s_markup: String,
    opt_url: Option<String>,
    opt_link_text: Option<String>,
    b_minor: bool,
    b_draft: bool,
    b_expired: bool,
}

async fn stLoadCurrent(
    txPg: &mut Transaction<'_, Postgres>,
    iTopicId: i32,
) -> Result<StCurrentRow> {
    sqlx::query_as::<_, StCurrentRow>(
        r#"SELECT t.userid AS i_author_id,t.groupid AS i_group_id,
                  t.title AS s_title,m.message AS s_message,m.markup::text AS s_markup,
                  t.url AS opt_url,t.linktext AS opt_link_text,t.minor AS b_minor,
                  t.draft AS b_draft,
                  (NOT t.sticky AND COALESCE(t.commitdate,t.postdate)<CURRENT_TIMESTAMP-s.expire)
                    AS b_expired
             FROM topics t JOIN msgbase m ON m.id=t.id
             JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
            WHERE t.id=$1"#,
    )
    .bind(iTopicId)
    .fetch_optional(&mut **txPg)
    .await?
    .ok_or(AppError::NotFound)
}

async fn vecLoadTagsTx(txPg: &mut Transaction<'_, Postgres>, iTopicId: i32) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        r#"SELECT tv.value FROM tags t JOIN tags_values tv ON tv.id=t.tagid
            WHERE t.msgid=$1 ORDER BY tv.value"#,
    )
    .bind(iTopicId)
    .fetch_all(&mut **txPg)
    .await?)
}

async fn optLoadPollTx(
    txPg: &mut Transaction<'_, Postgres>,
    iTopicId: i32,
) -> Result<Option<StTopicEditPoll>> {
    let Some((iPollId, bMultiSelect)): Option<(i32, bool)> =
        sqlx::query_as("SELECT id,multiselect FROM polls WHERE topic=$1")
            .bind(iTopicId)
            .fetch_optional(&mut **txPg)
            .await?
    else {
        return Ok(None);
    };
    let vecVariants = sqlx::query_as::<_, (i32, String)>(
        "SELECT id,label FROM polls_variants WHERE vote=$1 ORDER BY id",
    )
    .bind(iPollId)
    .fetch_all(&mut **txPg)
    .await?
    .into_iter()
    .map(|(iId, sLabel)| StTopicEditPollVariant { iId, sLabel })
    .collect();
    Ok(Some(StTopicEditPoll {
        iId: iPollId,
        bMultiSelect,
        vecVariants,
    }))
}

async fn vReplaceTagsTx(
    txPg: &mut Transaction<'_, Postgres>,
    iTopicId: i32,
    vecTags: &[String],
) -> Result<()> {
    let vecOldTags: Vec<(String, i32)> = sqlx::query_as(
        r#"SELECT tv.value,tv.id FROM tags t
             JOIN tags_values tv ON tv.id=t.tagid
            WHERE t.msgid=$1 ORDER BY tv.value"#,
    )
    .bind(iTopicId)
    .fetch_all(&mut **txPg)
    .await?;
    let vecAddNames = vecTags
        .iter()
        .filter(|sTag| !vecOldTags.iter().any(|(sOld, _)| sOld == *sTag))
        .cloned()
        .collect::<Vec<_>>();
    let vecDeleteNames = vecOldTags
        .iter()
        .filter(|(sOld, _)| !vecTags.contains(sOld))
        .map(|(sOld, _)| sOld.clone())
        .collect::<Vec<_>>();

    let mut vecAddIds = Vec::with_capacity(vecAddNames.len());
    for sTag in vecAddNames {
        vecAddIds.push((sTag.clone(), iGetOrCreateTagTx(txPg, &sTag).await?));
    }
    let mut vecDeleteIds = Vec::with_capacity(vecDeleteNames.len());
    for sTag in vecDeleteNames {
        vecDeleteIds.push((sTag.clone(), iGetOrCreateTagTx(txPg, &sTag).await?));
    }
    let mut vecLockIds = vecAddIds
        .iter()
        .chain(&vecDeleteIds)
        .map(|(_, iTagId)| *iTagId)
        .collect::<Vec<_>>();
    vecLockIds.sort_unstable();
    vecLockIds.dedup();
    if !vecLockIds.is_empty() {
        sqlx::query("SELECT id FROM tags_values WHERE id=ANY($1) ORDER BY id FOR UPDATE")
            .bind(&vecLockIds)
            .fetch_all(&mut **txPg)
            .await?;
    }
    for (_, iTagId) in vecAddIds {
        let stInserted =
            sqlx::query("INSERT INTO tags(msgid,tagid) VALUES($1,$2) ON CONFLICT DO NOTHING")
                .bind(iTopicId)
                .bind(iTagId)
                .execute(&mut **txPg)
                .await?;
        if stInserted.rows_affected() > 0 {
            sqlx::query("UPDATE tags_values SET counter=counter+1 WHERE id=$1")
                .bind(iTagId)
                .execute(&mut **txPg)
                .await?;
        }
    }
    for (_, iTagId) in vecDeleteIds {
        // TopicTagDao deliberately leaves counter stale on deletion.  The
        // scheduled reCalculateAllCounters job repairs it later.
        sqlx::query("DELETE FROM tags WHERE msgid=$1 AND tagid=$2")
            .bind(iTopicId)
            .bind(iTagId)
            .execute(&mut **txPg)
            .await?;
    }
    Ok(())
}

async fn iGetOrCreateTagTx(txPg: &mut Transaction<'_, Postgres>, sTag: &str) -> Result<i32> {
    let optTagId: Option<i32> = sqlx::query_scalar(
        r#"SELECT id FROM (
             SELECT tv.id,0 AS priority FROM tags_values tv WHERE tv.value=$1
             UNION ALL
             SELECT ts.tagid AS id,1 AS priority FROM tags_synonyms ts WHERE ts.value=$1
           ) found ORDER BY priority LIMIT 1"#,
    )
    .bind(sTag)
    .fetch_optional(&mut **txPg)
    .await?;
    match optTagId {
        Some(iTagId) => Ok(iTagId),
        None => Ok(
            sqlx::query_scalar("INSERT INTO tags_values(value) VALUES($1) RETURNING id")
                .bind(sTag)
                .fetch_one(&mut **txPg)
                .await?,
        ),
    }
}

async fn vUpdatePollTx(
    txPg: &mut Transaction<'_, Postgres>,
    stPoll: &StTopicEditPoll,
    vecNew: &[StTopicEditPollValue],
    bMultiSelect: bool,
) -> Result<()> {
    let mapNew = vecNew
        .iter()
        .filter(|stVariant| stVariant.iVariantId != 0)
        .map(|stVariant| (stVariant.iVariantId, stVariant.sLabel.as_str()))
        .collect::<std::collections::HashMap<_, _>>();
    for stOld in &stPoll.vecVariants {
        match mapNew.get(&stOld.iId).copied() {
            None | Some("") => {
                sqlx::query("DELETE FROM polls_variants WHERE id=$1 AND vote=$2")
                    .bind(stOld.iId)
                    .bind(stPoll.iId)
                    .execute(&mut **txPg)
                    .await?;
            }
            Some(sLabel) if sLabel != stOld.sLabel => {
                sqlx::query("UPDATE polls_variants SET label=$1 WHERE id=$2 AND vote=$3")
                    .bind(sLabel)
                    .bind(stOld.iId)
                    .bind(stPoll.iId)
                    .execute(&mut **txPg)
                    .await?;
            }
            Some(_) => {}
        }
    }
    for stNew in vecNew
        .iter()
        .filter(|stVariant| stVariant.iVariantId == 0 && !stVariant.sLabel.is_empty())
    {
        sqlx::query("INSERT INTO polls_variants(id,vote,label) VALUES(nextval('votes_id'),$1,$2)")
            .bind(stPoll.iId)
            .bind(&stNew.sLabel)
            .execute(&mut **txPg)
            .await?;
    }
    if stPoll.bMultiSelect != bMultiSelect {
        sqlx::query("UPDATE polls SET multiselect=$2 WHERE id=$1")
            .bind(stPoll.iId)
            .bind(bMultiSelect)
            .execute(&mut **txPg)
            .await?;
    }
    Ok(())
}

fn stPollHistoryJson(stPoll: &StTopicEditPoll, iTopicId: i32) -> serde_json::Value {
    serde_json::json!({
        "id": stPoll.iId,
        "topic": iTopicId,
        "multiSelect": stPoll.bMultiSelect,
        "variants": stPoll.vecVariants.iter().map(|stVariant| {
            serde_json::json!({"id":stVariant.iId,"label":stVariant.sLabel})
        }).collect::<Vec<_>>()
    })
}

fn bPollModified(
    optOld: Option<&StTopicEditPoll>,
    vecNew: &[StTopicEditPollValue],
    bMultiSelect: bool,
) -> bool {
    let Some(stOld) = optOld else {
        return vecNew.iter().any(|stVariant| !stVariant.sLabel.is_empty());
    };
    if stOld.bMultiSelect != bMultiSelect {
        return true;
    }
    let mapNew = vecNew
        .iter()
        .filter(|stVariant| stVariant.iVariantId != 0)
        .map(|stVariant| (stVariant.iVariantId, stVariant.sLabel.as_str()))
        .collect::<std::collections::HashMap<_, _>>();
    stOld.vecVariants.iter().any(|stVariant| {
        mapNew
            .get(&stVariant.iId)
            .is_none_or(|sLabel| *sLabel != stVariant.sLabel)
    }) || vecNew
        .iter()
        .any(|stVariant| stVariant.iVariantId == 0 && !stVariant.sLabel.is_empty())
}

fn bSameTags(vecOld: &[String], vecNew: &[String]) -> bool {
    let setOld = vecOld.iter().collect::<std::collections::BTreeSet<_>>();
    let setNew = vecNew.iter().collect::<std::collections::BTreeSet<_>>();
    setOld == setNew
}

fn bEqualNullableStrings(optLeft: Option<&str>, optRight: Option<&str>) -> bool {
    match optLeft.filter(|sValue| !sValue.is_empty()) {
        None => optRight.is_none_or(str::is_empty),
        Some(sLeft) => optRight.is_some_and(|sRight| sLeft == sRight),
    }
}

#[derive(Default)]
struct StImageDirectoryRollback {
    vecDirectories: Vec<PathBuf>,
    bCommitted: bool,
}

impl StImageDirectoryRollback {
    fn vTrack(&mut self, pathDirectory: PathBuf) {
        self.vecDirectories.push(pathDirectory);
    }

    fn vCommit(&mut self) {
        self.bCommitted = true;
    }
}

impl Drop for StImageDirectoryRollback {
    fn drop(&mut self) {
        if self.bCommitted {
            return;
        }
        for pathDirectory in self.vecDirectories.iter().rev() {
            if let Err(stError) = std::fs::remove_dir_all(pathDirectory)
                && stError.kind() != std::io::ErrorKind::NotFound
            {
                tracing::error!(path=%pathDirectory.display(), error=%stError, "failed to roll back edited topic image");
            }
        }
    }
}

async fn vSavePreviewImageTx(
    txPg: &mut Transaction<'_, Postgres>,
    iTopicId: i32,
    iEditorId: i32,
    sUploadRoot: &str,
    sPreviewName: &str,
    stRollback: &mut StImageDirectoryRollback,
) -> Result<()> {
    let pathName = Path::new(sPreviewName);
    if pathName.file_name().and_then(|sValue| sValue.to_str()) != Some(sPreviewName)
        || !sPreviewName.starts_with(&format!("preview-{iEditorId}-"))
    {
        return Err(AppError::BadRequest(
            "Некорректное имя preview изображения".into(),
        ));
    }
    let sExtension = sPreviewName
        .rsplit_once('.')
        .map(|(_, sExtension)| sExtension)
        .filter(|sExtension| matches!(*sExtension, "jpg" | "png" | "gif"))
        .ok_or_else(|| AppError::BadRequest("Некорректное имя preview изображения".into()))?;
    let sStem = sPreviewName
        .rsplit_once('.')
        .map(|(sStem, _)| sStem)
        .unwrap_or(sPreviewName);
    let pathPreviewRoot = Path::new(sUploadRoot).join("gallery/preview");
    let pathOriginal = pathPreviewRoot.join(sPreviewName);
    let vecDerivatives =
        [500, 1000, 1500, 2000].map(|iSize| pathPreviewRoot.join(format!("{sStem}-{iSize}px.jpg")));
    if !pathOriginal.is_file() || vecDerivatives.iter().any(|pathValue| !pathValue.is_file()) {
        return Err(AppError::BadRequest(
            "Preview изображения истёк или повреждён".into(),
        ));
    }
    let iImageId: i32 =
        sqlx::query_scalar("SELECT nextval(pg_get_serial_sequence('images','id'))::int")
            .fetch_one(&mut **txPg)
            .await?;
    let pathDirectory = Path::new(sUploadRoot)
        .join("images")
        .join(iImageId.to_string());
    tokio::fs::create_dir(&pathDirectory).await?;
    stRollback.vTrack(pathDirectory.clone());
    tokio::fs::copy(
        pathOriginal,
        pathDirectory.join(format!("original.{sExtension}")),
    )
    .await?;
    for (pathSource, iSize) in vecDerivatives.iter().zip([500, 1000, 1500, 2000]) {
        tokio::fs::copy(pathSource, pathDirectory.join(format!("{iSize}px.jpg"))).await?;
    }
    sqlx::query("INSERT INTO images(id,topic,extension,main) VALUES($1,$2,$3,false)")
        .bind(iImageId)
        .bind(iTopicId)
        .bind(sExtension)
        .execute(&mut **txPg)
        .await?;
    Ok(())
}

async fn vecNotifyUsersTx(
    txPg: &mut Transaction<'_, Postgres>,
    iTopicId: i32,
    iAuthorId: i32,
    vecMentionedNicks: &[String],
    bIncludeTags: bool,
    sMarkup: &str,
) -> Result<Vec<i32>> {
    let mut vecNotified: Vec<i32> = if vecMentionedNicks.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(
            r#"SELECT u.id FROM users u
                WHERE u.nick=ANY($1) AND u.id<>$2
                  AND ($4 OR NOT COALESCE(u.blocked,false))
                  AND NOT EXISTS(SELECT 1 FROM topic_users_notified tun
                                  WHERE tun.topic=$3 AND tun.userid=u.id)
                  AND NOT EXISTS(SELECT 1 FROM ignore_list il
                                  WHERE il.userid=u.id AND il.ignored=$2)"#,
        )
        .bind(vecMentionedNicks)
        .bind(iAuthorId)
        .bind(iTopicId)
        .bind(crate::markup::mentions_include_blocked_users(sMarkup))
        .fetch_all(&mut **txPg)
        .await?
    };
    for &iUserId in &vecNotified {
        sqlx::query(
            "INSERT INTO topic_users_notified(topic,userid) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(iTopicId)
        .bind(iUserId)
        .execute(&mut **txPg)
        .await?;
        sqlx::query(
            "INSERT INTO user_events(userid,type,private,message_id) VALUES($1,'REF',false,$2)",
        )
        .bind(iUserId)
        .bind(iTopicId)
        .execute(&mut **txPg)
        .await?;
    }
    let vecTagUsers: Vec<i32> = if bIncludeTags {
        sqlx::query_scalar(
            r#"SELECT DISTINCT ut.user_id FROM user_tags ut
                JOIN tags tg ON tg.tagid=ut.tag_id
               WHERE tg.msgid=$1 AND ut.is_favorite AND ut.user_id<>$2
                 AND NOT ut.user_id=ANY($3)
                 AND NOT EXISTS(SELECT 1 FROM topic_users_notified tun
                                 WHERE tun.topic=$1 AND tun.userid=ut.user_id)
                 AND NOT EXISTS(SELECT 1 FROM ignore_list il
                                 WHERE il.userid=ut.user_id AND il.ignored=$2)
                 AND NOT EXISTS(
                   SELECT 1 FROM user_tags ignored_tag
                   JOIN tags topic_tag ON topic_tag.tagid=ignored_tag.tag_id
                    WHERE ignored_tag.user_id=ut.user_id
                      AND NOT ignored_tag.is_favorite AND topic_tag.msgid=$1
                 )"#,
        )
        .bind(iTopicId)
        .bind(iAuthorId)
        .bind(&vecNotified)
        .fetch_all(&mut **txPg)
        .await?
    } else {
        Vec::new()
    };
    for &iUserId in &vecTagUsers {
        sqlx::query(
            "INSERT INTO topic_users_notified(topic,userid) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(iTopicId)
        .bind(iUserId)
        .execute(&mut **txPg)
        .await?;
        sqlx::query(
            "INSERT INTO user_events(userid,type,private,message_id) VALUES($1,'TAG',false,$2)",
        )
        .bind(iUserId)
        .bind(iTopicId)
        .execute(&mut **txPg)
        .await?;
    }
    vecNotified.extend(vecTagUsers);
    vecNotified.sort_unstable();
    vecNotified.dedup();
    // Canonical PostgreSQL owns the unread counter through new_event_t.
    // Java's UserEventDao.addEvent performs only the INSERT above; a manual
    // recount here would introduce different concurrency semantics.
    Ok(vecNotified)
}

async fn vChangeScoreTx(
    txPg: &mut Transaction<'_, Postgres>,
    iUserId: i32,
    iDelta: i32,
) -> Result<()> {
    let stResult = sqlx::query("UPDATE users SET score=score+$2 WHERE id=$1")
        .bind(iUserId)
        .bind(iDelta)
        .execute(&mut **txPg)
        .await?;
    if stResult.rows_affected() != 1 {
        return Err(AppError::BadRequest(format!(
            "Пользователь {iUserId} не найден"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    type TyEditHistoryRow = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<bool>,
    );

    const I_SUCCESS_TOPIC: i32 = 2_130_000_001;
    const I_ROLLBACK_TOPIC: i32 = 2_130_000_002;
    const VEC_TEST_TAGS: [&str; 5] = [
        "edit-tx-old-a-2130000001",
        "edit-tx-old-b-2130000001",
        "edit-tx-new-2130000001",
        "edit-tx-rollback-a-2130000002",
        "edit-tx-rollback-b-2130000002",
    ];

    async fn vCleanupIntegrationFixtures(oPool: &PgPool) -> Result<()> {
        let vecTopicIds = vec![I_SUCCESS_TOPIC, I_ROLLBACK_TOPIC];
        let vecTags = VEC_TEST_TAGS.to_vec();
        let mut vecErrors = Vec::new();
        let sDeleteEvents = r#"
WITH deleted AS (
    DELETE FROM user_events
     WHERE message_id=ANY($1)
        OR comment_id IN(SELECT id FROM comments WHERE topic=ANY($1))
     RETURNING userid,unread
), removed AS (
    SELECT userid,count(*) FILTER(WHERE unread)::integer AS amount
      FROM deleted GROUP BY userid
)
UPDATE users u SET unread_events=GREATEST(0,u.unread_events-r.amount)
FROM removed r WHERE u.id=r.userid
"#;
        if let Err(stError) = sqlx::query(sDeleteEvents)
            .bind(&vecTopicIds)
            .execute(oPool)
            .await
        {
            vecErrors.push(format!(
                "delete fixture events and restore unread counters: {stError}"
            ));
        }
        for sSql in [
            "DELETE FROM reactions_log WHERE topic_id=ANY($1) OR comment_id IN(SELECT id FROM comments WHERE topic=ANY($1))",
            "DELETE FROM message_warnings WHERE topic=ANY($1) OR comment IN(SELECT id FROM comments WHERE topic=ANY($1))",
            "DELETE FROM comments WHERE topic=ANY($1)",
            "DELETE FROM images WHERE topic=ANY($1)",
            "DELETE FROM memories WHERE topic=ANY($1)",
            "DELETE FROM telegram_posts WHERE topic_id=ANY($1)",
            "DELETE FROM topic_users_notified WHERE topic=ANY($1)",
            "DELETE FROM edit_info WHERE msgid=ANY($1)",
            "DELETE FROM tags WHERE msgid=ANY($1)",
        ] {
            if let Err(stError) = sqlx::query(sSql).bind(&vecTopicIds).execute(oPool).await {
                vecErrors.push(format!("{sSql}: {stError}"));
            }
        }

        // The canonical topins_t trigger increments groups.stat3 on every
        // topic insert and there is no matching delete trigger. Couple the
        // fixture-topic delete and compensation in one statement so a stale
        // fixture from an interrupted run is repaired exactly once.
        let sDeleteTopics = r#"
WITH deleted AS (
    DELETE FROM topics WHERE id=ANY($1) RETURNING groupid
), removed AS (
    SELECT groupid,count(*)::integer AS amount FROM deleted GROUP BY groupid
)
UPDATE groups g SET stat3=g.stat3-r.amount
FROM removed r WHERE g.id=r.groupid
"#;
        if let Err(stError) = sqlx::query(sDeleteTopics)
            .bind(&vecTopicIds)
            .execute(oPool)
            .await
        {
            vecErrors.push(format!("delete topics and restore group stat3: {stError}"));
        }
        if let Err(stError) = sqlx::query("DELETE FROM msgbase WHERE id=ANY($1)")
            .bind(&vecTopicIds)
            .execute(oPool)
            .await
        {
            vecErrors.push(format!("delete fixture msgbase rows: {stError}"));
        }
        if let Err(stError) = sqlx::query(
            "DELETE FROM tags_values tv WHERE tv.value=ANY($1) AND NOT EXISTS(SELECT 1 FROM tags t WHERE t.tagid=tv.id)",
        )
        .bind(&vecTags)
        .execute(oPool)
        .await
        {
            vecErrors.push(format!("delete fixture tag values: {stError}"));
        }

        if vecErrors.is_empty() {
            Ok(())
        } else {
            Err(AppError::Anyhow(anyhow::anyhow!(vecErrors.join("; "))))
        }
    }

    async fn stInsertIntegrationFixture(
        oPool: &PgPool,
        iTopicId: i32,
    ) -> Result<(i32, i32, DateTime<Utc>, String, String)> {
        let iAuthorId: i32 = sqlx::query_scalar("SELECT id FROM users ORDER BY id LIMIT 1")
            .fetch_one(oPool)
            .await?;
        let iGroupId: i32 = sqlx::query_scalar("SELECT id FROM groups ORDER BY id LIMIT 1")
            .fetch_one(oPool)
            .await?;
        let dtLastMod = Utc::now() - chrono::Duration::hours(2);
        let sOldTagA = if iTopicId == I_SUCCESS_TOPIC {
            VEC_TEST_TAGS[0]
        } else {
            VEC_TEST_TAGS[3]
        };
        let sOldTagB = VEC_TEST_TAGS[1];
        let sOldTagB = if iTopicId == I_SUCCESS_TOPIC {
            sOldTagB
        } else {
            VEC_TEST_TAGS[4]
        };

        sqlx::query("INSERT INTO msgbase(id,message,markup) VALUES($1,'old body','MARKDOWN')")
            .bind(iTopicId)
            .execute(oPool)
            .await?;
        sqlx::query(
            r#"INSERT INTO topics(
                 id,groupid,userid,title,url,moderate,postdate,linktext,deleted,
                 stat1,stat3,lastmod,commitby,notop,commitdate,postscore,postip,
                 sticky,resolved,minor,draft,allow_anonymous,reactions,open_warnings
               ) VALUES(
                 $1,$2,$3,'old title','https://old.example/',false,
                 CURRENT_TIMESTAMP-interval '1 hour','old link',false,
                 0,0,$4,NULL,false,NULL,-9999,'192.0.2.200',false,false,false,
                 false,true,'{}',0
               )"#,
        )
        .bind(iTopicId)
        .bind(iGroupId)
        .bind(iAuthorId)
        .bind(dtLastMod)
        .execute(oPool)
        .await?;
        // topins_t overwrites lastmod with CURRENT_TIMESTAMP. Java edit
        // history assertions require a deterministic pre-edit value.
        sqlx::query("UPDATE topics SET lastmod=$2 WHERE id=$1")
            .bind(iTopicId)
            .bind(dtLastMod)
            .execute(oPool)
            .await?;
        let dtStoredLastMod: DateTime<Utc> =
            sqlx::query_scalar("SELECT lastmod FROM topics WHERE id=$1")
                .bind(iTopicId)
                .fetch_one(oPool)
                .await?;
        for sTag in [sOldTagA, sOldTagB] {
            let iTagId: i32 = sqlx::query_scalar(
                "INSERT INTO tags_values(value,counter) VALUES($1,1) RETURNING id",
            )
            .bind(sTag)
            .fetch_one(oPool)
            .await?;
            sqlx::query("INSERT INTO tags(msgid,tagid) VALUES($1,$2)")
                .bind(iTopicId)
                .bind(iTagId)
                .execute(oPool)
                .await?;
        }
        Ok((
            iAuthorId,
            iGroupId,
            dtStoredLastMod,
            sOldTagA.to_owned(),
            sOldTagB.to_owned(),
        ))
    }

    fn stIntegrationCommand(
        iTopicId: i32,
        iEditorId: i32,
        vecTags: Vec<String>,
    ) -> StTopicEditCommand {
        StTopicEditCommand {
            iTopicId,
            iEditorId,
            optTitle: Some("new title".into()),
            optMessage: Some("new body".into()),
            optUrl: Some("https://new.example/".into()),
            optLinkText: Some("new link".into()),
            optTags: Some(vecTags),
            bMinor: true,
            bCommit: false,
            bPublish: false,
            optChangeGroupId: None,
            iBonus: 3,
            vecEditorBonus: Vec::new(),
            optPoll: None,
            bMultiSelect: false,
            vecPreviewNames: Vec::new(),
            sUploadRoot: std::env::temp_dir().to_string_lossy().into_owned(),
            vecMentionedNicks: Vec::new(),
            bSendTagEvents: false,
            bNewMessageDraft: false,
        }
    }

    #[test]
    fn optimistic_edit_check_matches_java_controller_without_stronger_tx_guard() {
        let sSource = include_str!("topic_edit_repository.rs");
        let sProduction = sSource.split("#[cfg(test)]").next().unwrap();
        assert!(!sProduction.contains("FOR UPDATE OF t"));
        assert!(!sProduction.contains("optLatestEdit"));
        assert!(!sProduction.contains("StaleEdit"));
    }

    #[test]
    fn all_topic_deltas_and_side_effects_share_one_transaction() {
        let sSource = include_str!("topic_edit_repository.rs");
        let sProduction = sSource.split("#[cfg(test)]").next().unwrap();
        for sFragment in [
            "UPDATE msgbase SET message",
            "UPDATE topics SET title",
            "UPDATE topics SET minor",
            "vReplaceTagsTx",
            "vUpdatePollTx",
            "vSavePreviewImageTx",
            "INSERT INTO edit_info",
            "UPDATE topics SET draft=false",
            "UPDATE topics SET moderate=true",
            "UPDATE users SET score=score+",
            "vecNotifyUsersTx",
        ] {
            assert!(sProduction.contains(sFragment), "{sFragment}");
        }
        assert_eq!(
            sProduction.matches("oTransaction.commit().await?").count(),
            1
        );
    }

    #[test]
    fn snapshot_contains_original_form_and_permission_sources() {
        for sFragment in [
            "m.markup::text",
            "s.moderate AS b_section_premoderated",
            "s.vote",
            "s.imagepost",
            "s.imageallowed",
            "s.havelink",
            "t.commitdate",
            "t.lastmod",
            "t.minor",
        ] {
            assert!(S_SNAPSHOT_SQL.contains(sFragment), "{sFragment}");
        }
    }

    #[test]
    fn tag_delta_keeps_java_counter_and_history_bytes() {
        let sSource = include_str!("topic_edit_repository.rs");
        let sProduction = sSource.split("#[cfg(test)]").next().unwrap();
        assert!(sProduction.contains("UPDATE tags_values SET counter=counter+1"));
        assert!(sProduction.contains("DELETE FROM tags WHERE msgid=$1 AND tagid=$2"));
        assert!(!sProduction.contains("SET counter=(SELECT count(*)"));
        assert!(sProduction.contains("vecOldTags.join(\",\")"));
        assert!(!sProduction.contains("vecOldTags.join(\", \")"));
    }

    #[test]
    fn notification_counter_is_owned_by_canonical_new_event_trigger() {
        let sSource = include_str!("topic_edit_repository.rs");
        let sProduction = sSource.split("#[cfg(test)]").next().unwrap();
        assert!(sProduction.contains("INSERT INTO user_events"));
        assert!(!sProduction.contains("UPDATE users SET unread_events"));
    }

    #[tokio::test]
    #[ignore = "requires an explicitly selected disposable Java/Liquibase PostgreSQL database"]
    async fn transaction_deltas_and_late_failure_rollback_match_java() {
        assert_eq!(
            std::env::var("LOR_EDIT_INTEGRATION_CONFIRM").as_deref(),
            Ok("mutate-disposable-edit-fixture"),
            "set LOR_EDIT_INTEGRATION_CONFIRM=mutate-disposable-edit-fixture"
        );
        let sDatabaseUrl = std::env::var("LOR_EDIT_INTEGRATION_DATABASE_URL")
            .expect("set LOR_EDIT_INTEGRATION_DATABASE_URL to a disposable canonical database");
        let oPool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&sDatabaseUrl)
            .await
            .expect("disposable canonical database must be reachable");
        vCleanupIntegrationFixtures(&oPool)
            .await
            .expect("clean stale edit integration fixtures");
        let (iFixtureGroupId, iGroupStat3Before): (i32, i32) =
            sqlx::query_as("SELECT id,stat3 FROM groups ORDER BY id LIMIT 1")
                .fetch_one(&oPool)
                .await
                .expect("fixture group");
        let iFixtureAuthorId: i32 = sqlx::query_scalar("SELECT id FROM users ORDER BY id LIMIT 1")
            .fetch_one(&oPool)
            .await
            .expect("fixture author");
        let (iEventUserId, sEventUserNick, iUnreadBefore): (i32, String, i32) = sqlx::query_as(
            r#"SELECT u.id,u.nick,u.unread_events FROM users u
                WHERE u.id<>$1 AND NOT COALESCE(u.blocked,false)
                  AND NOT EXISTS(SELECT 1 FROM ignore_list il
                                  WHERE il.userid=u.id AND il.ignored=$1)
                ORDER BY u.id LIMIT 1"#,
        )
        .bind(iFixtureAuthorId)
        .fetch_one(&oPool)
        .await
        .expect("fixture notification recipient");

        let stRun: Result<_> = async {
            let (iAuthorId, _, dtOldLastMod, sOldTagA, sOldTagB) =
                stInsertIntegrationFixture(&oPool, I_SUCCESS_TOPIC).await?;
            let cRepository = CTopicEditPgRepository::new(oPool.clone());
            let mut stSuccessCommand = stIntegrationCommand(
                I_SUCCESS_TOPIC,
                iAuthorId,
                vec![sOldTagB.clone(), VEC_TEST_TAGS[2].to_owned()],
            );
            stSuccessCommand.vecMentionedNicks = vec![sEventUserNick.clone()];
            let stResult = cRepository
                .stUpdateAndCommit(stSuccessCommand)
                .await?;
            let (sTitle, sMessage, dtLastMod): (String, String, DateTime<Utc>) = sqlx::query_as(
                "SELECT t.title,m.message,t.lastmod FROM topics t JOIN msgbase m ON m.id=t.id WHERE t.id=$1",
            )
            .bind(I_SUCCESS_TOPIC)
            .fetch_one(&oPool)
            .await?;
            let iUnreadAfter: i32 =
                sqlx::query_scalar("SELECT unread_events FROM users WHERE id=$1")
                    .bind(iEventUserId)
                    .fetch_one(&oPool)
                    .await?;
            let iReferenceEvents: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM user_events WHERE userid=$1 AND message_id=$2 AND type='REF'",
            )
            .bind(iEventUserId)
            .bind(I_SUCCESS_TOPIC)
            .fetch_one(&oPool)
            .await?;
            let vecRelations: Vec<String> = sqlx::query_scalar(
                "SELECT tv.value FROM tags t JOIN tags_values tv ON tv.id=t.tagid WHERE t.msgid=$1 ORDER BY tv.value",
            )
            .bind(I_SUCCESS_TOPIC)
            .fetch_all(&oPool)
            .await?;
            let (iOldCounter, iKeptCounter, iNewCounter): (i32, i32, i32) = sqlx::query_as(
                "SELECT max(counter) FILTER(WHERE value=$1)::int,max(counter) FILTER(WHERE value=$2)::int,max(counter) FILTER(WHERE value=$3)::int FROM tags_values WHERE value=ANY($4)",
            )
            .bind(&sOldTagA)
            .bind(&sOldTagB)
            .bind(VEC_TEST_TAGS[2])
            .bind(vec![sOldTagA.clone(), sOldTagB.clone(), VEC_TEST_TAGS[2].to_owned()])
            .fetch_one(&oPool)
            .await?;
            let stHistory: TyEditHistoryRow = sqlx::query_as(
                "SELECT oldmessage,oldtitle,oldtags,oldlinktext,oldurl,oldminor FROM edit_info WHERE msgid=$1 ORDER BY id DESC LIMIT 1",
            )
            .bind(I_SUCCESS_TOPIC)
            .fetch_one(&oPool)
            .await?;

            let (_, _, _, sRollbackTag, sRollbackKeptTag) =
                stInsertIntegrationFixture(&oPool, I_ROLLBACK_TOPIC).await?;
            let mut stRollbackCommand = stIntegrationCommand(
                I_ROLLBACK_TOPIC,
                iAuthorId,
                vec![sRollbackKeptTag, VEC_TEST_TAGS[2].to_owned()],
            );
            stRollbackCommand.vecPreviewNames = vec![format!(
                "preview-{iAuthorId}-missing-{}.png",
                I_ROLLBACK_TOPIC
            )];
            let bLateFailure = cRepository
                .stUpdateAndCommit(stRollbackCommand)
                .await
                .is_err();
            let (sRollbackTitle, sRollbackMessage): (String, String) = sqlx::query_as(
                "SELECT t.title,m.message FROM topics t JOIN msgbase m ON m.id=t.id WHERE t.id=$1",
            )
            .bind(I_ROLLBACK_TOPIC)
            .fetch_one(&oPool)
            .await?;
            let vecRollbackRelations: Vec<String> = sqlx::query_scalar(
                "SELECT tv.value FROM tags t JOIN tags_values tv ON tv.id=t.tagid WHERE t.msgid=$1 ORDER BY tv.value",
            )
            .bind(I_ROLLBACK_TOPIC)
            .fetch_all(&oPool)
            .await?;
            let iRollbackHistory: i64 =
                sqlx::query_scalar("SELECT count(*) FROM edit_info WHERE msgid=$1")
                    .bind(I_ROLLBACK_TOPIC)
                    .fetch_one(&oPool)
                    .await?;

            Ok((
                stResult,
                sTitle,
                sMessage,
                dtOldLastMod,
                dtLastMod,
                vecRelations,
                iOldCounter,
                iKeptCounter,
                iNewCounter,
                stHistory,
                iUnreadAfter,
                iReferenceEvents,
                bLateFailure,
                sRollbackTag,
                sRollbackTitle,
                sRollbackMessage,
                vecRollbackRelations,
                iRollbackHistory,
            ))
        }
        .await;

        // Cleanup runs before assertions and after every fallible operation,
        // so a failed regression never leaves permanent fixture rows.
        let stCleanup = vCleanupIntegrationFixtures(&oPool).await;
        let stCleanupProof: std::result::Result<(i64, i64, i64, i32, i32), sqlx::Error> =
            sqlx::query_as(
                r#"SELECT
                     (SELECT count(*) FROM topics WHERE id=ANY($1)),
                     (SELECT count(*) FROM msgbase WHERE id=ANY($1)),
                     (SELECT count(*) FROM tags_values WHERE value=ANY($2)),
                     (SELECT stat3 FROM groups WHERE id=$3),
                     (SELECT unread_events FROM users WHERE id=$4)"#,
            )
            .bind(vec![I_SUCCESS_TOPIC, I_ROLLBACK_TOPIC])
            .bind(VEC_TEST_TAGS.to_vec())
            .bind(iFixtureGroupId)
            .bind(iEventUserId)
            .fetch_one(&oPool)
            .await;
        oPool.close().await;
        stCleanup.expect("clean edit integration fixtures after the run");
        let (iTopicRows, iMessageRows, iTagRows, iGroupStat3After, iUnreadAfterCleanup) =
            stCleanupProof.expect("verify edit integration cleanup");
        assert_eq!((iTopicRows, iMessageRows, iTagRows), (0, 0, 0));
        assert_eq!(iGroupStat3After, iGroupStat3Before);
        assert_eq!(iUnreadAfterCleanup, iUnreadBefore);
        let (
            stResult,
            sTitle,
            sMessage,
            dtOldLastMod,
            dtLastMod,
            vecRelations,
            iOldCounter,
            iKeptCounter,
            iNewCounter,
            stHistory,
            iUnreadAfter,
            iReferenceEvents,
            bLateFailure,
            sRollbackTag,
            sRollbackTitle,
            sRollbackMessage,
            vecRollbackRelations,
            iRollbackHistory,
        ) = stRun.expect("edit integration operations");

        assert!(stResult.bModified);
        assert_eq!(
            (sTitle.as_str(), sMessage.as_str()),
            ("new title", "new body")
        );
        assert_eq!(dtLastMod, dtOldLastMod + chrono::Duration::seconds(1));
        assert_eq!(
            vecRelations,
            [VEC_TEST_TAGS[2].to_owned(), VEC_TEST_TAGS[1].to_owned()]
        );
        assert_eq!((iOldCounter, iKeptCounter, iNewCounter), (1, 1, 1));
        assert_eq!(
            stHistory,
            (
                Some("old body".into()),
                Some("old title".into()),
                Some(format!("{},{}", VEC_TEST_TAGS[0], VEC_TEST_TAGS[1])),
                Some("old link".into()),
                Some("https://old.example/".into()),
                Some(false),
            )
        );
        assert_eq!(stResult.vecNotifiedUserIds, [iEventUserId]);
        assert_eq!(iReferenceEvents, 1);
        assert_eq!(iUnreadAfter, iUnreadBefore + 1);
        assert!(bLateFailure);
        assert_eq!(
            (sRollbackTitle.as_str(), sRollbackMessage.as_str()),
            ("old title", "old body")
        );
        assert_eq!(
            vecRollbackRelations,
            [sRollbackTag, VEC_TEST_TAGS[4].to_owned()]
        );
        assert_eq!(iRollbackHistory, 0);
    }
}
