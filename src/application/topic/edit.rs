use chrono::{Months, Utc};

use crate::{
    domain::topic::{
        edit::{
            StTopicEditActor, StTopicEditCommand, StTopicEditMutationResult, StTopicEditPermission,
            StTopicEditPollValue, StTopicEditRestrictions, StTopicEditSnapshot,
            TrTopicEditRealtimeNotifier, TrTopicEditRepository, stCheckCommit, stCheckContentEdit,
            stCheckTagsEdit,
        },
        options::TrTopicReindexQueue,
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone)]
pub struct StTopicEditInput {
    pub optTitle: Option<String>,
    pub optMessage: Option<String>,
    pub optUrl: Option<String>,
    pub optLinkText: Option<String>,
    pub optTags: Option<Vec<String>>,
    pub bMinor: bool,
    pub bPreview: bool,
    pub bCommit: bool,
    pub bPublish: bool,
    pub optChangeGroupId: Option<i32>,
    pub iBonus: i32,
    pub vecEditorBonus: Vec<(String, i32)>,
    pub optLastEditMillis: Option<i64>,
    pub optPoll: Option<Vec<StTopicEditPollValue>>,
    pub bMultiSelect: bool,
    pub vecPreviewNames: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StPreparedTopicEdit {
    pub stSnapshot: StTopicEditSnapshot,
    pub stContentPermission: StTopicEditPermission,
    pub stTagsPermission: StTopicEditPermission,
    pub stCommitPermission: StTopicEditPermission,
}

impl StPreparedTopicEdit {
    pub fn bAnythingEditable(&self) -> bool {
        self.stContentPermission.bPermitted() || self.stTagsPermission.bPermitted()
    }

    pub fn bMiniEditable(&self) -> bool {
        self.stSnapshot.bCanBeMini() && self.stCommitPermission.bPermitted()
    }
}

#[derive(Debug, Clone)]
pub enum EnTopicEditOutcome {
    Render {
        stPrepared: Box<StPreparedTopicEdit>,
        vecErrors: Vec<String>,
        bCommitForm: bool,
        sHeading: String,
    },
    Applied {
        sRedirectUrl: String,
        bModeratedConfirmation: bool,
    },
}

#[derive(Debug, Clone)]
pub struct CTopicEditService<R, Q, N>
where
    R: TrTopicEditRepository,
    Q: TrTopicReindexQueue,
    N: TrTopicEditRealtimeNotifier,
{
    oRepository: R,
    oReindexQueue: Q,
    oRealtimeNotifier: N,
}

impl<R, Q, N> CTopicEditService<R, Q, N>
where
    R: TrTopicEditRepository,
    Q: TrTopicReindexQueue,
    N: TrTopicEditRealtimeNotifier,
{
    pub fn new(oRepository: R, oReindexQueue: Q, oRealtimeNotifier: N) -> Self {
        Self {
            oRepository,
            oReindexQueue,
            oRealtimeNotifier,
        }
    }

    pub async fn stPrepare(
        &self,
        iTopicId: i32,
        stActor: StTopicEditActor,
        sRemoteIp: &str,
    ) -> Result<StPreparedTopicEdit> {
        let (optSnapshot, stRestrictions) = tokio::try_join!(
            self.oRepository.optSnapshot(iTopicId),
            self.oRepository.stRestrictions(stActor.iUserId, sRemoteIp),
        )?;
        let stSnapshot = optSnapshot.ok_or(AppError::NotFound)?;
        Ok(stPrepared(stSnapshot, stActor, stRestrictions))
    }

    pub async fn stPrepareEditForm(
        &self,
        iTopicId: i32,
        stActor: StTopicEditActor,
        sRemoteIp: &str,
    ) -> Result<StPreparedTopicEdit> {
        let stPrepared = self.stPrepare(iTopicId, stActor, sRemoteIp).await?;
        if !stPrepared.bAnythingEditable() {
            return Err(AppError::Forbidden);
        }
        Ok(stPrepared)
    }

    pub async fn stPrepareCommitForm(
        &self,
        iTopicId: i32,
        stActor: StTopicEditActor,
        sRemoteIp: &str,
    ) -> Result<StPreparedTopicEdit> {
        if !stActor.bCorrector && !stActor.bModerator {
            return Err(AppError::Forbidden);
        }
        let stPrepared = self.stPrepare(iTopicId, stActor, sRemoteIp).await?;
        if stPrepared.stSnapshot.bCommitted {
            return Err(AppError::BadRequest("Топик уже подтвержден".into()));
        }
        if !stPrepared.stSnapshot.bCommittable() {
            return Err(AppError::BadRequest("Этот топик нельзя подтвердить".into()));
        }
        if !stPrepared.stCommitPermission.bPermitted() {
            return Err(AppError::Forbidden);
        }
        Ok(stPrepared)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn stSubmit(
        &self,
        iTopicId: i32,
        stActor: StTopicEditActor,
        sRemoteIp: &str,
        mut stInput: StTopicEditInput,
        mut vecErrors: Vec<String>,
        bPublishAllowed: bool,
        sPublishReason: &str,
        sUploadRoot: &str,
    ) -> Result<EnTopicEditOutcome> {
        let stPrepared = self.stPrepareEditForm(iTopicId, stActor, sRemoteIp).await?;
        let stTopic = &stPrepared.stSnapshot;

        // EditTopicController converts a present but fully sanitized empty
        // tag string to None.  In particular, posting an empty tags field is
        // not an instruction to delete all tags.
        if stInput.optTags.as_ref().is_some_and(Vec::is_empty) {
            stInput.optTags = None;
        }

        if !(0..=20).contains(&stInput.iBonus) {
            vecErrors.push("Некорректное значение bonus".into());
        }
        if stInput
            .vecEditorBonus
            .iter()
            .any(|(_, iBonus)| !(0..=5).contains(iBonus))
        {
            vecErrors.push("Некорректное значение editorBonus".into());
        }
        let mut vecEditorBonus = Vec::with_capacity(stInput.vecEditorBonus.len());
        for (sNick, iBonus) in &stInput.vecEditorBonus {
            let Some(stEditor) = stTopic
                .vecEditors
                .iter()
                .find(|stEditor| stEditor.sNick == *sNick)
            else {
                vecErrors.push("некорректный корректор?!".into());
                continue;
            };
            vecEditorBonus.push((stEditor.iId, *iBonus));
        }

        // EditTopicRequestValidator checks the optimistic token after all
        // content/bonus fields and on every POST, including preview. Java
        // does not repeat it in updateAndCommit, so retain that race.
        if stTopic.optLastEditMillis.is_some()
            && stTopic.optLastEditMillis != stInput.optLastEditMillis
        {
            vecErrors.push("Сообщение было отредактировано независимо".into());
        }

        let bCommitForm = stTopic.bCommittable() && stPrepared.stCommitPermission.bPermitted();
        if stInput.bCommit {
            if stTopic.bCommitted {
                vecErrors.push("Топик уже подтвержден".into());
            } else if !stTopic.bCommittable() {
                vecErrors.push("Этот топик нельзя подтвердить".into());
            }
            if vecErrors.is_empty() && !stPrepared.stCommitPermission.bPermitted() {
                vecErrors.push(format!(
                    "Ограничение: {}",
                    stPrepared.stCommitPermission.sReason()
                ));
            }
        }

        if stInput.bMinor != stTopic.bMinor
            && (!stTopic.bCanBeMini() || !stPrepared.stCommitPermission.bPermitted())
        {
            vecErrors.push("вы не можете менять статус новости".into());
        }

        if let Some(iChangeGroupId) = stInput.optChangeGroupId
            && iChangeGroupId != stTopic.iGroupId
        {
            let Some(stGroup) = stTopic
                .vecGroups
                .iter()
                .find(|stGroup| stGroup.iId == iChangeGroupId)
            else {
                return Err(AppError::Forbidden);
            };
            if stGroup.iSectionId != stTopic.iSectionId {
                return Err(AppError::Forbidden);
            }
        }

        let optStoredTitle = stInput
            .optTitle
            .as_deref()
            .map(crate::domain::title::sEscapeForStorage);
        let optFixedUrl = stInput.optUrl.as_deref().map(sFixUrlLikeJava);
        let bMessageModified = stInput
            .optMessage
            .as_deref()
            .is_some_and(|sMessage| sMessage != stTopic.sMessage);
        let bTitleModified = optStoredTitle
            .as_deref()
            .is_some_and(|sTitle| sTitle != stTopic.sStoredTitle);
        let bUrlModified = optFixedUrl
            .as_deref()
            .is_some_and(|sUrl| !bEqualNullableStrings(stTopic.optUrl.as_deref(), Some(sUrl)));
        let bLinkTextModified = stInput.optLinkText.as_deref().is_some_and(|sLinkText| {
            !bEqualNullableStrings(stTopic.optLinkText.as_deref(), Some(sLinkText))
        });
        let bPollModified = stInput
            .optPoll
            .as_ref()
            .is_some_and(|vecPoll| bPollModified(stTopic, vecPoll, stInput.bMultiSelect));
        let bContentModified = bMessageModified
            || bTitleModified
            || bUrlModified
            || bLinkTextModified
            || bPollModified
            || !stInput.vecPreviewNames.is_empty();
        // Deliberate hardening: Java's controller accidentally omits poll
        // changes and changes to already non-null URL/linktext values from
        // its pre-service permission delta. Reproducing that would let a
        // tags-only editor mutate protected content with a crafted POST.
        if bContentModified && !stPrepared.stContentPermission.bPermitted() {
            return Err(AppError::Forbidden);
        }

        if let Some(vecTags) = stInput.optTags.as_deref()
            && !stTopic.bSectionPremoderated
            && stActor.iScore < 200
        {
            let vecNewTags = self.oRepository.vecNewTags(vecTags).await?;
            if !vecNewTags.is_empty() {
                vecErrors.push(format!(
                    "Вы не можете создавать новые теги ({})",
                    vecNewTags.join(",")
                ));
            }
        }
        let bDoPublish = stTopic.bDraft && stInput.bPublish;
        if bDoPublish && !stInput.bPreview && vecErrors.is_empty() && !bPublishAllowed {
            vecErrors.push(format!("Ограничение: {sPublishReason}"));
        }

        let sHeading = if stInput.bCommit && bCommitForm {
            "Подтверждение"
        } else if stInput.bPreview {
            "Предпросмотр"
        } else {
            "Редактирование"
        }
        .to_owned();

        if stInput.bPreview || !vecErrors.is_empty() {
            return Ok(EnTopicEditOutcome::Render {
                stPrepared: Box::new(stPrepared),
                vecErrors,
                bCommitForm,
                sHeading,
            });
        }

        let sMessageForMentions = stInput.optMessage.as_deref().unwrap_or(&stTopic.sMessage);
        let vecMentionedNicks =
            crate::markup::extract_mentions(sMessageForMentions, &stTopic.sMarkup);
        let bNeedCommit = stTopic.bSectionPremoderated && !stTopic.bCommitted;
        let dtEffective = if stTopic.bCommitted {
            stTopic.optCommitDate.unwrap_or(stTopic.dtPostDate)
        } else {
            stTopic.dtPostDate
        };
        let dtFreshBoundary = Utc::now()
            .checked_sub_months(Months::new(1))
            .expect("one month before a valid current date");
        let bFresh = dtEffective > dtFreshBoundary;
        let bSendTagEvents =
            stInput.optTags.is_some() && (stInput.bCommit || (!bNeedCommit && bFresh));
        let bNewMessageDraft = if stInput.bPublish {
            false
        } else {
            stTopic.bDraft
        };
        let stCommand = StTopicEditCommand {
            iTopicId,
            iEditorId: stActor.iUserId,
            optTitle: optStoredTitle,
            optMessage: stInput.optMessage.clone(),
            optUrl: optFixedUrl,
            optLinkText: stInput.optLinkText.clone(),
            optTags: stInput.optTags.clone(),
            bMinor: stInput.bMinor,
            bCommit: stInput.bCommit,
            bPublish: bDoPublish,
            optChangeGroupId: stInput.optChangeGroupId,
            iBonus: stInput.iBonus,
            vecEditorBonus,
            optPoll: stInput.optPoll.clone(),
            bMultiSelect: stInput.bMultiSelect,
            vecPreviewNames: stInput.vecPreviewNames.clone(),
            sUploadRoot: sUploadRoot.to_owned(),
            vecMentionedNicks,
            bSendTagEvents,
            bNewMessageDraft,
        };

        let StTopicEditMutationResult {
            bModified,
            vecNotifiedUserIds,
        } = self.oRepository.stUpdateAndCommit(stCommand).await?;

        // Java still enters updateAndCommit for a valid no-op POST (and its
        // notification transaction), then returns the form-level error.
        if !bModified && !stInput.bCommit && !bDoPublish {
            return Ok(EnTopicEditOutcome::Render {
                stPrepared: Box::new(stPrepared),
                vecErrors: vec!["Нет изменений".into()],
                bCommitForm,
                sHeading,
            });
        }

        // TopicService performs the queue send after committing PostgreSQL.
        // A durable-queue error therefore cannot roll back the accepted edit.
        if (bModified || stInput.bCommit || bDoPublish) && !bNewMessageDraft {
            self.oRealtimeNotifier.vNotifyEvents(&vecNotifiedUserIds);
            self.oReindexQueue.vUpdateMessage(iTopicId, true).await?;
        }

        let sRedirectUrl = stPrepared.stSnapshot.sForceLastModUrl();
        let bModeratedConfirmation = bDoPublish && stPrepared.stSnapshot.bSectionPremoderated;
        Ok(EnTopicEditOutcome::Applied {
            sRedirectUrl,
            bModeratedConfirmation,
        })
    }
}

fn stPrepared(
    stSnapshot: StTopicEditSnapshot,
    stActor: StTopicEditActor,
    stRestrictions: StTopicEditRestrictions,
) -> StPreparedTopicEdit {
    let dtNow = Utc::now();
    StPreparedTopicEdit {
        stContentPermission: stCheckContentEdit(&stSnapshot, stActor, stRestrictions, dtNow),
        stTagsPermission: stCheckTagsEdit(&stSnapshot, stActor, stRestrictions, dtNow),
        stCommitPermission: stCheckCommit(&stSnapshot, stActor, stRestrictions),
        stSnapshot,
    }
}

fn bEqualNullableStrings(optLeft: Option<&str>, optRight: Option<&str>) -> bool {
    match optLeft.filter(|sValue| !sValue.is_empty()) {
        None => optRight.is_none_or(str::is_empty),
        Some(sLeft) => optRight.is_some_and(|sRight| sLeft == sRight),
    }
}

pub(crate) fn sFixUrlLikeJava(sUrl: &str) -> String {
    let sTrimmed = sUrl.trim();
    let sLower = sTrimmed.to_ascii_lowercase();
    if sLower.starts_with("www.") {
        format!("http://{sTrimmed}")
    } else if sLower.starts_with("ftp.") {
        format!("ftp://{sTrimmed}")
    } else {
        sTrimmed.to_owned()
    }
}

fn bPollModified(
    stTopic: &StTopicEditSnapshot,
    vecNew: &[StTopicEditPollValue],
    bMultiSelect: bool,
) -> bool {
    let Some(stOld) = &stTopic.optPoll else {
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
    if stOld.vecVariants.iter().any(|stOldVariant| {
        mapNew
            .get(&stOldVariant.iId)
            .is_none_or(|sLabel| *sLabel != stOldVariant.sLabel)
    }) {
        return true;
    }
    vecNew
        .iter()
        .any(|stVariant| stVariant.iVariantId == 0 && !stVariant.sLabel.is_empty())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{Duration, TimeZone};

    use crate::domain::topic::{
        edit::{StTopicEditGroup, StTopicEditPoll, StTopicEditPollVariant},
        posting::StIpBlockInfo,
    };

    use super::*;

    #[derive(Clone)]
    struct CRepository {
        stSnapshot: StTopicEditSnapshot,
        stResult: StTopicEditMutationResult,
        vecCommands: Arc<Mutex<Vec<StTopicEditCommand>>>,
    }

    #[async_trait]
    impl TrTopicEditRepository for CRepository {
        async fn optSnapshot(&self, iTopicId: i32) -> Result<Option<StTopicEditSnapshot>> {
            Ok((iTopicId == self.stSnapshot.iTopicId).then(|| self.stSnapshot.clone()))
        }

        async fn stRestrictions(
            &self,
            _iUserId: i32,
            _sRemoteIp: &str,
        ) -> Result<StTopicEditRestrictions> {
            Ok(StTopicEditRestrictions {
                bFrozen: false,
                stIpBlock: StIpBlockInfo::default(),
            })
        }

        async fn vecNewTags(&self, _vecTags: &[String]) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn stUpdateAndCommit(
            &self,
            stCommand: StTopicEditCommand,
        ) -> Result<StTopicEditMutationResult> {
            self.vecCommands.lock().unwrap().push(stCommand);
            Ok(self.stResult.clone())
        }
    }

    #[derive(Clone, Default)]
    struct CQueue(Arc<Mutex<Vec<(i32, bool)>>>);

    #[async_trait]
    impl TrTopicReindexQueue for CQueue {
        async fn vUpdateMessage(&self, iTopicId: i32, bWithComments: bool) -> Result<()> {
            self.0.lock().unwrap().push((iTopicId, bWithComments));
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct CNotifier(Arc<Mutex<Vec<i32>>>);

    impl TrTopicEditRealtimeNotifier for CNotifier {
        fn vNotifyEvents(&self, vecUserIds: &[i32]) {
            self.0.lock().unwrap().extend_from_slice(vecUserIds);
        }
    }

    #[derive(Clone)]
    struct COrderedFailingQueue(Arc<Mutex<Vec<&'static str>>>);

    #[async_trait]
    impl TrTopicReindexQueue for COrderedFailingQueue {
        async fn vUpdateMessage(&self, _iTopicId: i32, _bWithComments: bool) -> Result<()> {
            self.0.lock().unwrap().push("queue");
            Err(AppError::Anyhow(anyhow::anyhow!("queue failed")))
        }
    }

    #[derive(Clone)]
    struct COrderedNotifier(Arc<Mutex<Vec<&'static str>>>);

    impl TrTopicEditRealtimeNotifier for COrderedNotifier {
        fn vNotifyEvents(&self, _vecUserIds: &[i32]) {
            self.0.lock().unwrap().push("realtime");
        }
    }

    fn stSnapshot() -> StTopicEditSnapshot {
        let dtNow = Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap();
        StTopicEditSnapshot {
            iTopicId: 42,
            iAuthorId: 7,
            sAuthorNick: "author".into(),
            iAuthorScore: 100,
            iAuthorMaxScore: 100,
            bAuthorBlocked: false,
            bAuthorAnonymous: false,
            bAuthorFrozen: false,
            sStoredTitle: "old".into(),
            sMessage: "old body".into(),
            sMarkup: "MARKDOWN".into(),
            optUrl: None,
            optLinkText: None,
            iGroupId: 2,
            sGroupTitle: "group".into(),
            sGroupUrlName: "group".into(),
            iSectionId: 2,
            sSectionTitle: "Форум".into(),
            sSectionPrefix: "forum".into(),
            bSectionPremoderated: false,
            bSectionPollAllowed: false,
            bSectionImagePost: false,
            bSectionImageAllowed: true,
            bLinksAllowed: false,
            bDeleted: false,
            bDraft: false,
            bCommitted: false,
            bSticky: false,
            bExpired: false,
            iPostScore: -9999,
            dtPostDate: dtNow - Duration::days(1),
            optCommitDate: None,
            dtLastMod: dtNow,
            bMinor: false,
            vecTags: vec!["rust".into()],
            optPoll: None,
            vecGroups: vec![StTopicEditGroup {
                iId: 2,
                sTitle: "group".into(),
                iSectionId: 2,
            }],
            optLastEditMillis: Some(123),
            vecEditors: Vec::new(),
        }
    }

    fn stActor() -> StTopicEditActor {
        StTopicEditActor {
            iUserId: 7,
            iScore: 100,
            bModerator: false,
            bAdministrator: false,
            bCorrector: false,
            bBlocked: false,
        }
    }

    fn stInput() -> StTopicEditInput {
        StTopicEditInput {
            optTitle: Some("new".into()),
            optMessage: Some("new body".into()),
            optUrl: None,
            optLinkText: None,
            optTags: Some(vec!["rust".into()]),
            bMinor: false,
            bPreview: false,
            bCommit: false,
            bPublish: false,
            optChangeGroupId: None,
            iBonus: 3,
            vecEditorBonus: Vec::new(),
            optLastEditMillis: Some(123),
            optPoll: None,
            bMultiSelect: false,
            vecPreviewNames: Vec::new(),
        }
    }

    #[tokio::test]
    async fn missing_last_edit_is_rejected_before_preview_or_repository_call() {
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let oQueue = CQueue::default();
        let cService = CTopicEditService::new(
            CRepository {
                stSnapshot: stSnapshot(),
                stResult: StTopicEditMutationResult {
                    bModified: true,
                    vecNotifiedUserIds: Vec::new(),
                },
                vecCommands: vecCommands.clone(),
            },
            oQueue.clone(),
            CNotifier::default(),
        );
        let mut stInput = stInput();
        stInput.optLastEditMillis = None;
        stInput.bPreview = true;
        let enOutcome = cService
            .stSubmit(
                42,
                stActor(),
                "127.0.0.1",
                stInput,
                Vec::new(),
                true,
                "",
                "/tmp",
            )
            .await
            .unwrap();
        let EnTopicEditOutcome::Render { vecErrors, .. } = enOutcome else {
            panic!("expected render")
        };
        assert_eq!(vecErrors, ["Сообщение было отредактировано независимо"]);
        assert!(vecCommands.lock().unwrap().is_empty());
        assert!(oQueue.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn stale_last_edit_is_rejected_before_preview_or_repository_call() {
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let oQueue = CQueue::default();
        let cService = CTopicEditService::new(
            CRepository {
                stSnapshot: stSnapshot(),
                stResult: StTopicEditMutationResult {
                    bModified: true,
                    vecNotifiedUserIds: Vec::new(),
                },
                vecCommands: vecCommands.clone(),
            },
            oQueue.clone(),
            CNotifier::default(),
        );
        let mut stInput = stInput();
        stInput.optLastEditMillis = Some(122);
        stInput.bPreview = true;
        let enOutcome = cService
            .stSubmit(
                42,
                stActor(),
                "127.0.0.1",
                stInput,
                Vec::new(),
                true,
                "",
                "/tmp",
            )
            .await
            .unwrap();
        let EnTopicEditOutcome::Render { vecErrors, .. } = enOutcome else {
            panic!("expected render")
        };
        assert_eq!(vecErrors, ["Сообщение было отредактировано независимо"]);
        assert!(vecCommands.lock().unwrap().is_empty());
        assert!(oQueue.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tags_only_missing_content_fields_preserve_original_values() {
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let cService = CTopicEditService::new(
            CRepository {
                stSnapshot: stSnapshot(),
                stResult: StTopicEditMutationResult {
                    bModified: true,
                    vecNotifiedUserIds: Vec::new(),
                },
                vecCommands: vecCommands.clone(),
            },
            CQueue::default(),
            CNotifier::default(),
        );
        let stCorrector = StTopicEditActor {
            iUserId: 8,
            iScore: 100,
            bModerator: false,
            bAdministrator: false,
            bCorrector: true,
            bBlocked: false,
        };
        let mut stInput = stInput();
        stInput.optTitle = None;
        stInput.optMessage = None;
        stInput.optUrl = None;
        stInput.optLinkText = None;
        stInput.optTags = Some(vec!["changed-tag".into()]);

        let enOutcome = cService
            .stSubmit(
                42,
                stCorrector,
                "127.0.0.1",
                stInput,
                Vec::new(),
                true,
                "",
                "/tmp",
            )
            .await
            .unwrap();
        assert!(matches!(enOutcome, EnTopicEditOutcome::Applied { .. }));
        let vecCommands = vecCommands.lock().unwrap();
        assert_eq!(vecCommands.len(), 1);
        assert!(vecCommands[0].optTitle.is_none());
        assert!(vecCommands[0].optMessage.is_none());
        assert!(vecCommands[0].optUrl.is_none());
        assert!(vecCommands[0].optLinkText.is_none());
    }

    #[tokio::test]
    async fn absent_poll_map_reaches_repository_as_none_and_preserves_poll() {
        let mut stSnapshot = stSnapshot();
        stSnapshot.bSectionPollAllowed = true;
        stSnapshot.optPoll = Some(StTopicEditPoll {
            iId: 5,
            bMultiSelect: false,
            vecVariants: vec![StTopicEditPollVariant {
                iId: 10,
                sLabel: "old choice".into(),
            }],
        });
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let cService = CTopicEditService::new(
            CRepository {
                stSnapshot,
                stResult: StTopicEditMutationResult {
                    bModified: false,
                    vecNotifiedUserIds: Vec::new(),
                },
                vecCommands: vecCommands.clone(),
            },
            CQueue::default(),
            CNotifier::default(),
        );
        let mut stInput = stInput();
        stInput.optTitle = Some("old".into());
        stInput.optMessage = Some("old body".into());
        stInput.optTags = Some(vec!["rust".into()]);
        stInput.optPoll = None;

        let enOutcome = cService
            .stSubmit(
                42,
                stActor(),
                "127.0.0.1",
                stInput,
                Vec::new(),
                true,
                "",
                "/tmp",
            )
            .await
            .unwrap();
        assert!(matches!(enOutcome, EnTopicEditOutcome::Render { .. }));
        let vecCommands = vecCommands.lock().unwrap();
        assert_eq!(vecCommands.len(), 1);
        assert!(vecCommands[0].optPoll.is_none());
    }

    #[tokio::test]
    async fn tags_only_editor_cannot_exploit_java_existing_link_or_poll_delta_hole() {
        let mut stSnapshot = stSnapshot();
        stSnapshot.iAuthorId = 7;
        stSnapshot.optUrl = Some("https://old.example/".into());
        stSnapshot.optLinkText = Some("old link".into());
        stSnapshot.bSectionPollAllowed = true;
        stSnapshot.optPoll = Some(StTopicEditPoll {
            iId: 5,
            bMultiSelect: false,
            vecVariants: vec![StTopicEditPollVariant {
                iId: 10,
                sLabel: "old choice".into(),
            }],
        });
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let cService = CTopicEditService::new(
            CRepository {
                stSnapshot,
                stResult: StTopicEditMutationResult {
                    bModified: true,
                    vecNotifiedUserIds: Vec::new(),
                },
                vecCommands: vecCommands.clone(),
            },
            CQueue::default(),
            CNotifier::default(),
        );
        let stCorrector = StTopicEditActor {
            iUserId: 8,
            iScore: 100,
            bModerator: false,
            bAdministrator: false,
            bCorrector: true,
            bBlocked: false,
        };
        let mut stInput = stInput();
        stInput.optTitle = Some("old".into());
        stInput.optMessage = Some("old body".into());
        stInput.optUrl = Some("https://new.example/".into());
        stInput.optLinkText = Some("new link".into());
        stInput.optPoll = Some(vec![StTopicEditPollValue {
            iVariantId: 10,
            sLabel: "new choice".into(),
        }]);

        let stError = cService
            .stSubmit(
                42,
                stCorrector,
                "127.0.0.1",
                stInput,
                Vec::new(),
                true,
                "",
                "/tmp",
            )
            .await
            .expect_err("protected content delta must not reach the repository");

        assert!(matches!(stError, AppError::Forbidden));
        assert!(vecCommands.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn applied_edit_queues_topic_and_comments_after_repository_success() {
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let oQueue = CQueue::default();
        let oNotifier = CNotifier::default();
        let cService = CTopicEditService::new(
            CRepository {
                stSnapshot: stSnapshot(),
                stResult: StTopicEditMutationResult {
                    bModified: true,
                    vecNotifiedUserIds: vec![9],
                },
                vecCommands: vecCommands.clone(),
            },
            oQueue.clone(),
            oNotifier.clone(),
        );
        let enOutcome = cService
            .stSubmit(
                42,
                stActor(),
                "127.0.0.1",
                stInput(),
                Vec::new(),
                true,
                "",
                "/tmp",
            )
            .await
            .unwrap();
        assert!(matches!(enOutcome, EnTopicEditOutcome::Applied { .. }));
        assert_eq!(&*oQueue.0.lock().unwrap(), &[(42, true)]);
        assert_eq!(&*oNotifier.0.lock().unwrap(), &[9]);
    }

    #[tokio::test]
    async fn realtime_is_emitted_before_a_failing_post_commit_queue_send() {
        let vecCommands = Arc::new(Mutex::new(Vec::new()));
        let vecOrder = Arc::new(Mutex::new(Vec::new()));
        let cService = CTopicEditService::new(
            CRepository {
                stSnapshot: stSnapshot(),
                stResult: StTopicEditMutationResult {
                    bModified: true,
                    vecNotifiedUserIds: vec![9],
                },
                vecCommands,
            },
            COrderedFailingQueue(vecOrder.clone()),
            COrderedNotifier(vecOrder.clone()),
        );

        let stError = cService
            .stSubmit(
                42,
                stActor(),
                "127.0.0.1",
                stInput(),
                Vec::new(),
                true,
                "",
                "/tmp",
            )
            .await
            .expect_err("the external queue error remains observable");

        assert!(matches!(stError, AppError::Anyhow(_)));
        assert_eq!(&*vecOrder.lock().unwrap(), &["realtime", "queue"]);
    }

    #[test]
    fn poll_delta_covers_delete_add_label_and_multiselect() {
        let mut stSnapshot = stSnapshot();
        stSnapshot.optPoll = Some(StTopicEditPoll {
            iId: 5,
            bMultiSelect: false,
            vecVariants: vec![StTopicEditPollVariant {
                iId: 10,
                sLabel: "yes".into(),
            }],
        });
        assert!(!bPollModified(
            &stSnapshot,
            &[StTopicEditPollValue {
                iVariantId: 10,
                sLabel: "yes".into(),
            }],
            false,
        ));
        assert!(bPollModified(
            &stSnapshot,
            &[StTopicEditPollValue {
                iVariantId: 10,
                sLabel: String::new(),
            }],
            false,
        ));
        assert!(bPollModified(
            &stSnapshot,
            &[StTopicEditPollValue {
                iVariantId: 10,
                sLabel: "yes".into(),
            }],
            true,
        ));
    }
}
