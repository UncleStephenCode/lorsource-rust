use crate::{
    domain::{
        user::model::StUserSummary,
        warning::{
            model::{
                EnWarningType, StClearWarningMutation, StCreateWarningMutation, StWarningTopic,
            },
            repository::TrWarningRepository,
        },
    },
    error::{AppError, Result},
};

const I_MAX_WARNINGS_PER_HOUR: i64 = 5;
const I_HIDE_COMMENTS_POST_SCORE: i32 = 10002;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StWarningContext {
    pub bPremoderated: bool,
    pub sTopicUrl: String,
    pub vecEligibilityErrors: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StWarningPresentation {
    pub stContext: StWarningContext,
    pub vecTypes: Vec<EnWarningType>,
    pub vecErrors: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StCreateWarningCommand {
    pub iTopicId: i32,
    pub optCommentId: Option<i32>,
    pub optText: Option<String>,
    pub optWarningType: Option<String>,
    pub optRuleType: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnCreateWarningOutcome {
    Validation(StWarningPresentation),
    Created { sLink: String },
}

#[derive(Debug, Clone)]
pub struct CWarningService<R>
where
    R: TrWarningRepository,
{
    oRepository: R,
}

impl<R> CWarningService<R>
where
    R: TrWarningRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn stPrepare(
        &self,
        stUser: &StUserSummary,
        iTopicId: i32,
        optCommentId: Option<i32>,
    ) -> Result<StWarningPresentation> {
        let stContext = self.stContext(stUser, iTopicId, optCommentId).await?;
        let vecTypes = vecWarningTypes(stContext.bPremoderated, optCommentId);
        let mut vecErrors = stContext.vecEligibilityErrors.clone();
        if vecErrors.is_empty() {
            let iRecentWarnings = self.oRepository.iRecentWarnings(stUser.id).await?;
            if iRecentWarnings >= I_MAX_WARNINGS_PER_HOUR {
                vecErrors.push("Вы не можете отправить более 5 уведомлений в час");
            }
        }
        Ok(StWarningPresentation {
            stContext,
            vecTypes,
            vecErrors,
        })
    }

    pub async fn enCreate(
        &self,
        stUser: &StUserSummary,
        stCommand: StCreateWarningCommand,
    ) -> Result<EnCreateWarningOutcome> {
        let optCommentId = stCommand.optCommentId.filter(|iValue| *iValue != 0);
        let stContext = self
            .stContext(stUser, stCommand.iTopicId, optCommentId)
            .await?;
        let vecTypes = vecWarningTypes(stContext.bPremoderated, optCommentId);
        let sText = stCommand.optText.unwrap_or_default();
        let sRuleType = stCommand.optRuleType.unwrap_or_default();
        let bHasText = !sText.trim().is_empty();
        let bHasRuleType = !sRuleType.trim().is_empty();
        let optWarningType = stCommand
            .optWarningType
            .as_deref()
            .and_then(EnWarningType::optFromId)
            .or_else(|| (vecTypes.len() == 1).then(|| vecTypes[0]));

        // WarningController.checkRequest runs the hourly limit only when the
        // topic/comment eligibility checks produced no errors.  Spring then
        // appends every field error instead of stopping at the first one.
        let mut vecErrors = stContext.vecEligibilityErrors.clone();
        if vecErrors.is_empty() {
            let iRecentWarnings = self.oRepository.iRecentWarnings(stUser.id).await?;
            if iRecentWarnings >= I_MAX_WARNINGS_PER_HOUR {
                vecErrors.push("Вы не можете отправить более 5 уведомлений в час");
            }
        }
        if optWarningType != Some(EnWarningType::Rule) && bHasRuleType {
            vecErrors.push("Пункт правил можно выбрать только при уведомлении о нарушении");
        }
        if !optWarningType.is_some_and(|enType| vecTypes.contains(&enType)) {
            vecErrors.push("Не выбран тип уведомления");
        }
        if !bHasText && !bHasRuleType {
            vecErrors.push("Заполните комментарий");
        }
        if bHasText && sText.encode_utf16().count() > 256 {
            vecErrors.push("Сообщение не может быть более 256 символов");
        }
        if !vecErrors.is_empty() {
            return Ok(EnCreateWarningOutcome::Validation(StWarningPresentation {
                stContext,
                vecTypes,
                vecErrors,
            }));
        }

        let enWarningType = optWarningType.expect("validated warning type");
        let sMessage = if !bHasRuleType {
            sText
        } else if !bHasText {
            sRuleType
        } else {
            format!("[{sRuleType}] {sText}")
        };
        self.oRepository
            .iCreate(StCreateWarningMutation {
                iTopicId: stCommand.iTopicId,
                optCommentId,
                iAuthorId: stUser.id,
                sMessage,
                enWarningType,
            })
            .await?;

        let sLink = optCommentId
            .map(|iCommentId| format!("{}#comment-{iCommentId}", stContext.sTopicUrl))
            .unwrap_or(stContext.sTopicUrl);
        Ok(EnCreateWarningOutcome::Created { sLink })
    }

    pub async fn sClear(&self, stUser: &StUserSummary, iWarningId: i32) -> Result<String> {
        if !stUser.canmod && !stUser.corrector {
            return Err(AppError::Forbidden);
        }
        let stWarning = self
            .oRepository
            .optWarning(iWarningId)
            .await?
            .ok_or(AppError::NotFound)?;
        let stTopic = self
            .oRepository
            .optTopic(stWarning.iTopicId)
            .await?
            .ok_or(AppError::NotFound)?;
        self.oRepository
            .vClear(StClearWarningMutation {
                iWarningId,
                iActorId: stUser.id,
                iTopicId: stWarning.iTopicId,
                optCommentId: stWarning.optCommentId,
            })
            .await?;
        let sTopicUrl = stTopic.sTopicUrl();
        Ok(stWarning
            .optCommentId
            .map(|iCommentId| format!("{sTopicUrl}#comment-{iCommentId}"))
            .unwrap_or(sTopicUrl))
    }

    async fn stContext(
        &self,
        stUser: &StUserSummary,
        iTopicId: i32,
        optCommentId: Option<i32>,
    ) -> Result<StWarningContext> {
        let bFrozen = self.oRepository.bUserFrozen(stUser.id).await?;
        let stTopic = self
            .oRepository
            .optTopic(iTopicId)
            .await?
            .ok_or(AppError::NotFound)?;
        vCheckView(stUser, &stTopic)?;

        let mut bCommentDeleted = false;
        if let Some(iCommentId) = optCommentId {
            match self
                .oRepository
                .optCommentDeleted(iTopicId, iCommentId)
                .await?
            {
                Some(false) => {}
                Some(true) => bCommentDeleted = true,
                None => return Err(AppError::NotFound),
            }
        }
        let bCanPostWarning = !(stTopic.bDeleted
            || stTopic.bDraft
            || stTopic.bExpired
            || bCommentDeleted
            || stUser.score.unwrap_or(0) < 50
            || bFrozen);
        let mut vecEligibilityErrors = Vec::new();
        if !bCanPostWarning {
            vecEligibilityErrors.push("Вы не можете отправить уведомление");
        }
        if stTopic.bDeleted {
            vecEligibilityErrors.push("Топик удален");
        }
        if stTopic.bExpired {
            vecEligibilityErrors.push("Топик перемещен в архив");
        }
        if bCommentDeleted {
            vecEligibilityErrors.push("Комментарий удален");
        }
        Ok(StWarningContext {
            bPremoderated: stTopic.bPremoderated,
            sTopicUrl: stTopic.sTopicUrl(),
            vecEligibilityErrors,
        })
    }
}

fn vCheckView(stUser: &StUserSummary, stTopic: &StWarningTopic) -> Result<()> {
    if stTopic.iPostScore == I_HIDE_COMMENTS_POST_SCORE {
        return Err(AppError::Forbidden);
    }
    if stTopic.bPremoderated
        && !stTopic.bCommitted
        && stTopic.iAuthorId != stUser.id
        && !stUser.canmod
        && !stUser.corrector
    {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub fn vecWarningTypes(bPremoderated: bool, optCommentId: Option<i32>) -> Vec<EnWarningType> {
    if optCommentId.is_some() {
        vec![EnWarningType::Rule]
    } else if bPremoderated {
        vec![
            EnWarningType::Rule,
            EnWarningType::Spelling,
            EnWarningType::Tag,
            EnWarningType::Group,
        ]
    } else {
        vec![
            EnWarningType::Rule,
            EnWarningType::Tag,
            EnWarningType::Group,
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::{CWarningService, EnCreateWarningOutcome, StCreateWarningCommand, vecWarningTypes};
    use crate::{
        domain::{
            user::model::StUserSummary,
            warning::{
                model::{
                    EnWarningType, StClearWarningMutation, StCreateWarningMutation,
                    StWarningRecord, StWarningTopic,
                },
                repository::TrWarningRepository,
            },
        },
        error::Result,
    };

    #[derive(Debug, Clone)]
    struct CFakeWarningRepository {
        stState: Arc<Mutex<StFakeState>>,
    }

    #[derive(Debug, Clone)]
    struct StFakeState {
        optTopic: Option<StWarningTopic>,
        optCommentDeleted: Option<bool>,
        bFrozen: bool,
        iRecent: i64,
        optWarning: Option<StWarningRecord>,
        vecCreated: Vec<StCreateWarningMutation>,
        vecCleared: Vec<StClearWarningMutation>,
    }

    impl CFakeWarningRepository {
        fn new() -> Self {
            Self {
                stState: Arc::new(Mutex::new(StFakeState {
                    optTopic: Some(stTopic()),
                    optCommentDeleted: Some(false),
                    bFrozen: false,
                    iRecent: 0,
                    optWarning: None,
                    vecCreated: Vec::new(),
                    vecCleared: Vec::new(),
                })),
            }
        }
    }

    #[async_trait]
    impl TrWarningRepository for CFakeWarningRepository {
        async fn optTopic(&self, _iTopicId: i32) -> Result<Option<StWarningTopic>> {
            Ok(self.stState.lock().expect("fake state").optTopic.clone())
        }

        async fn optCommentDeleted(
            &self,
            _iTopicId: i32,
            _iCommentId: i32,
        ) -> Result<Option<bool>> {
            Ok(self.stState.lock().expect("fake state").optCommentDeleted)
        }

        async fn bUserFrozen(&self, _iUserId: i32) -> Result<bool> {
            Ok(self.stState.lock().expect("fake state").bFrozen)
        }

        async fn iRecentWarnings(&self, _iUserId: i32) -> Result<i64> {
            Ok(self.stState.lock().expect("fake state").iRecent)
        }

        async fn iCreate(&self, stMutation: StCreateWarningMutation) -> Result<i32> {
            self.stState
                .lock()
                .expect("fake state")
                .vecCreated
                .push(stMutation);
            Ok(71)
        }

        async fn optWarning(&self, _iWarningId: i32) -> Result<Option<StWarningRecord>> {
            Ok(self.stState.lock().expect("fake state").optWarning.clone())
        }

        async fn vClear(&self, stMutation: StClearWarningMutation) -> Result<()> {
            self.stState
                .lock()
                .expect("fake state")
                .vecCleared
                .push(stMutation);
            Ok(())
        }
    }

    fn stTopic() -> StWarningTopic {
        StWarningTopic {
            iId: 42,
            iAuthorId: 2,
            bDeleted: false,
            bDraft: false,
            iPostScore: 0,
            bExpired: false,
            bPremoderated: false,
            bCommitted: true,
            sGroupUrl: "general".to_owned(),
            sSectionPrefix: "forum".to_owned(),
        }
    }

    fn stUser(iScore: i32) -> StUserSummary {
        StUserSummary {
            id: 9,
            nick: "tester".to_owned(),
            name: None,
            score: Some(iScore),
            max_score: Some(iScore),
            photo: None,
            town: None,
            regdate: None,
            canmod: false,
            candel: false,
            corrector: false,
            blocked: Some(false),
            userinfo: None,
        }
    }

    fn stCommand(optCommentId: Option<i32>) -> StCreateWarningCommand {
        StCreateWarningCommand {
            iTopicId: 42,
            optCommentId,
            optText: Some("пояснение".to_owned()),
            optWarningType: Some("rule".to_owned()),
            optRuleType: None,
        }
    }

    #[test]
    fn warning_types_match_original_section_and_comment_rules() {
        assert_eq!(vecWarningTypes(false, Some(1)), [EnWarningType::Rule]);
        assert_eq!(
            vecWarningTypes(true, None),
            [
                EnWarningType::Rule,
                EnWarningType::Spelling,
                EnWarningType::Tag,
                EnWarningType::Group
            ]
        );
    }

    #[tokio::test]
    async fn low_score_is_a_form_validation_error_without_a_mutation() {
        let oRepository = CFakeWarningRepository::new();
        let cService = CWarningService::new(oRepository.clone());
        let enOutcome = cService
            .enCreate(&stUser(49), stCommand(None))
            .await
            .expect("validation outcome");
        let EnCreateWarningOutcome::Validation(stPresentation) = enOutcome else {
            panic!("expected validation outcome");
        };
        assert_eq!(
            stPresentation.vecErrors,
            ["Вы не можете отправить уведомление"]
        );
        assert!(
            oRepository
                .stState
                .lock()
                .expect("fake state")
                .vecCreated
                .is_empty()
        );
    }

    #[tokio::test]
    async fn comment_rule_composes_message_and_canonical_fragment() {
        let oRepository = CFakeWarningRepository::new();
        let cService = CWarningService::new(oRepository.clone());
        let mut stCommand = stCommand(Some(77));
        stCommand.optText = Some("детали".to_owned());
        stCommand.optRuleType = Some("4.1 Офтопик".to_owned());
        let enOutcome = cService
            .enCreate(&stUser(100), stCommand)
            .await
            .expect("created outcome");
        assert_eq!(
            enOutcome,
            EnCreateWarningOutcome::Created {
                sLink: "/forum/general/42#comment-77".to_owned()
            }
        );
        let stState = oRepository.stState.lock().expect("fake state");
        assert_eq!(stState.vecCreated.len(), 1);
        assert_eq!(stState.vecCreated[0].sMessage, "[4.1 Офтопик] детали");
        assert_eq!(stState.vecCreated[0].optCommentId, Some(77));
    }

    #[tokio::test]
    async fn hourly_limit_prevents_mutation() {
        let oRepository = CFakeWarningRepository::new();
        oRepository.stState.lock().expect("fake state").iRecent = 5;
        let cService = CWarningService::new(oRepository.clone());
        assert!(matches!(
            cService
                .enCreate(&stUser(100), stCommand(None))
                .await
                .expect("validation outcome"),
            EnCreateWarningOutcome::Validation(_)
        ));
        assert!(
            oRepository
                .stState
                .lock()
                .expect("fake state")
                .vecCreated
                .is_empty()
        );
    }

    #[tokio::test]
    async fn deleted_topic_reports_generic_and_specific_errors_in_spring_order() {
        let oRepository = CFakeWarningRepository::new();
        oRepository
            .stState
            .lock()
            .expect("fake state")
            .optTopic
            .as_mut()
            .expect("topic")
            .bDeleted = true;
        let cService = CWarningService::new(oRepository);
        let EnCreateWarningOutcome::Validation(stPresentation) = cService
            .enCreate(&stUser(100), stCommand(None))
            .await
            .expect("validation outcome")
        else {
            panic!("expected validation outcome");
        };
        assert_eq!(
            stPresentation.vecErrors,
            ["Вы не можете отправить уведомление", "Топик удален",]
        );
    }

    #[tokio::test]
    async fn post_validation_accumulates_all_matching_field_errors() {
        let oRepository = CFakeWarningRepository::new();
        let cService = CWarningService::new(oRepository);
        let mut stCommand = stCommand(None);
        stCommand.optWarningType = Some("tag".to_owned());
        stCommand.optRuleType = Some("4.1 Офтопик".to_owned());
        stCommand.optText = Some("🙂".repeat(129));
        let EnCreateWarningOutcome::Validation(stPresentation) = cService
            .enCreate(&stUser(100), stCommand)
            .await
            .expect("validation outcome")
        else {
            panic!("expected validation outcome");
        };
        assert_eq!(
            stPresentation.vecErrors,
            [
                "Пункт правил можно выбрать только при уведомлении о нарушении",
                "Сообщение не может быть более 256 символов",
            ]
        );
    }

    #[tokio::test]
    async fn corrector_can_clear_and_receives_comment_fragment() {
        let oRepository = CFakeWarningRepository::new();
        oRepository.stState.lock().expect("fake state").optWarning = Some(StWarningRecord {
            iTopicId: 42,
            optCommentId: Some(77),
        });
        let cService = CWarningService::new(oRepository.clone());
        let mut stCorrector = stUser(100);
        stCorrector.corrector = true;
        assert_eq!(
            cService
                .sClear(&stCorrector, 71)
                .await
                .expect("clear warning"),
            "/forum/general/42#comment-77"
        );
        assert_eq!(
            oRepository
                .stState
                .lock()
                .expect("fake state")
                .vecCleared
                .len(),
            1
        );
    }
}
