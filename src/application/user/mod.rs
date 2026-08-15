use chrono::{DateTime, Duration, Months, Utc};

use crate::{
    domain::user::{
        moderation::{EnUserModerationMutation, StMassDeleteResult, StModerationUser},
        repository::TrUserModerationRepository,
    },
    error::{AppError, Result},
    models::UserSummary,
};

pub mod account;
pub mod statistics;
pub mod userpic;

const I_CORRECTOR_SCORE: i32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnUserModAction {
    Block,
    Unblock,
    Score50,
    ToggleCorrector,
    ResetPassword,
    RemoveUserInfo,
    RemoveTown,
    RemoveUrl,
    Freeze,
    BlockAndDelete,
}

impl EnUserModAction {
    pub fn optFromForm(sAction: &str) -> Option<Self> {
        Some(match sAction {
            "block" => Self::Block,
            "unblock" => Self::Unblock,
            "score50" => Self::Score50,
            "toggle_corrector" => Self::ToggleCorrector,
            "reset-password" => Self::ResetPassword,
            "remove_userinfo" => Self::RemoveUserInfo,
            "remove_town" => Self::RemoveTown,
            "remove_url" => Self::RemoveUrl,
            "freeze" => Self::Freeze,
            "block-n-delete-comments" => Self::BlockAndDelete,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct StUserModCommand {
    pub iTargetUserId: i32,
    pub enAction: EnUserModAction,
    pub optReason: Option<String>,
    pub optShift: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnUserModOutcome {
    ProfileRedirect { sNick: String },
    PasswordReset { sNick: String },
    MassDelete(StMassDeleteResult),
}

#[derive(Debug, Clone)]
pub struct CUserModerationService<R>
where
    R: TrUserModerationRepository,
{
    oRepository: R,
}

impl<R> CUserModerationService<R>
where
    R: TrUserModerationRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn enExecute(
        &self,
        stModerator: &UserSummary,
        stCommand: StUserModCommand,
    ) -> Result<EnUserModOutcome> {
        if !stModerator.canmod {
            return Err(AppError::Forbidden);
        }

        let stTarget = self
            .oRepository
            .optUser(stCommand.iTargetUserId)
            .await?
            .ok_or(AppError::NotFound)?;
        let iTargetUserId = stTarget.iId;
        let iModeratorId = stModerator.id;

        let enMutation = match stCommand.enAction {
            EnUserModAction::Block => {
                if !bIsBlockable(&stTarget, stModerator) {
                    return Err(AppError::Forbidden);
                }
                if stTarget.bBlocked {
                    return Err(AppError::BadRequest(
                        "Пользователь уже блокирован".to_owned(),
                    ));
                }
                EnUserModerationMutation::Block {
                    iTargetUserId,
                    iModeratorId,
                    // The Java parameter is optional, but the canonical
                    // `ban_info.reason NOT NULL` constraint makes omission
                    // fail and rolls back the transaction.
                    sReason: stCommand
                        .optReason
                        .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("ban reason is NULL")))?,
                }
            }
            EnUserModAction::Unblock => {
                if !bIsBlockable(&stTarget, stModerator) {
                    return Err(AppError::Forbidden);
                }
                EnUserModerationMutation::Unblock {
                    iTargetUserId,
                    iModeratorId,
                }
            }
            EnUserModAction::Score50 => {
                if stTarget.bBlocked || stTarget.bAnonymous || stTarget.iScore > 50 {
                    return Err(AppError::Forbidden);
                }
                EnUserModerationMutation::Score50 {
                    iTargetUserId,
                    iModeratorId,
                }
            }
            EnUserModAction::ToggleCorrector => {
                if stTarget.iScore < I_CORRECTOR_SCORE {
                    return Err(AppError::Forbidden);
                }
                EnUserModerationMutation::SetCorrector {
                    iTargetUserId,
                    iModeratorId,
                    bCorrector: !stTarget.bCorrector,
                }
            }
            EnUserModAction::ResetPassword => {
                if stTarget.bAnonymous {
                    return Err(AppError::Forbidden);
                }
                let sPassword = sGenerateJavaPassword();
                let sPasswordHash = crate::security::password::hash(&sPassword)
                    .map_err(|stError| AppError::Anyhow(stError.into()))?;
                EnUserModerationMutation::ResetPassword {
                    iTargetUserId,
                    iModeratorId,
                    sPasswordHash,
                }
            }
            EnUserModAction::RemoveUserInfo => {
                vRejectAnonymous(&stTarget)?;
                EnUserModerationMutation::RemoveUserInfo {
                    iTargetUserId,
                    iModeratorId,
                }
            }
            EnUserModAction::RemoveTown => {
                vRejectAnonymous(&stTarget)?;
                EnUserModerationMutation::RemoveTown {
                    iTargetUserId,
                    iModeratorId,
                }
            }
            EnUserModAction::RemoveUrl => {
                vRejectAnonymous(&stTarget)?;
                EnUserModerationMutation::RemoveUrl {
                    iTargetUserId,
                    iModeratorId,
                }
            }
            EnUserModAction::Freeze => {
                let sReason = stCommand.optReason.ok_or_else(|| {
                    AppError::BadRequest("Не задана причина заморозки".to_owned())
                })?;
                if sReason.encode_utf16().count() > 255 {
                    return Err(AppError::BadRequest(
                        "Причина слишком длиная, максимум 255 байт".to_owned(),
                    ));
                }
                let sShift = stCommand
                    .optShift
                    .as_deref()
                    .ok_or_else(|| AppError::BadRequest("Не задан срок заморозки".to_owned()))?;
                let dtNow = Utc::now();
                let (dtUntil, bDefrost) = optFreezeUntil(sShift, dtNow).ok_or_else(|| {
                    AppError::BadRequest("некорректный срок заморозки".to_owned())
                })?;

                if !bIsFreezable(&stTarget, stModerator) {
                    return Err(AppError::Forbidden);
                }
                // Java performs this check for both freeze and "Разморозить".
                if stTarget.bBlocked {
                    return Err(AppError::BadRequest(
                        "Пользователь блокирован, его нельзя заморозить".to_owned(),
                    ));
                }
                EnUserModerationMutation::Freeze {
                    iTargetUserId,
                    iModeratorId,
                    sReason,
                    dtUntil,
                    bDefrost,
                }
            }
            EnUserModAction::BlockAndDelete => {
                if !bIsBlockable(&stTarget, stModerator) {
                    return Err(AppError::Forbidden);
                }
                if stTarget.bBlocked {
                    return Err(AppError::BadRequest(
                        "Пользователь уже блокирован".to_owned(),
                    ));
                }
                EnUserModerationMutation::BlockAndDelete {
                    iTargetUserId,
                    iModeratorId,
                    sReason: stCommand
                        .optReason
                        .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("ban reason is NULL")))?,
                }
            }
        };

        let stResult = self.oRepository.stApply(enMutation).await?;
        Ok(match stCommand.enAction {
            EnUserModAction::ResetPassword => EnUserModOutcome::PasswordReset {
                sNick: stTarget.sNick,
            },
            EnUserModAction::BlockAndDelete => {
                EnUserModOutcome::MassDelete(stResult.optMassDelete.unwrap_or_default())
            }
            _ => EnUserModOutcome::ProfileRedirect {
                sNick: stTarget.sNick,
            },
        })
    }

    /// Port of `UserService.resetUserpic`. Unlike `/usermod.jsp`, this
    /// adjacent legacy action is also available to the profile owner.
    pub async fn sResetUserpic(&self, stActor: &UserSummary, iTargetUserId: i32) -> Result<String> {
        let stTarget = self
            .oRepository
            .optUser(iTargetUserId)
            .await?
            .ok_or(AppError::NotFound)?;
        if stActor.id != stTarget.iId && !stActor.canmod {
            return Err(AppError::Forbidden);
        }

        self.oRepository
            .stApply(EnUserModerationMutation::ResetUserpic {
                iTargetUserId: stTarget.iId,
                iActorUserId: stActor.id,
                bScorePenalty: stActor.canmod && stActor.id != stTarget.iId && !stTarget.bModerator,
            })
            .await?;

        Ok(stTarget.sNick)
    }
}

pub fn bIsBlockable(stTarget: &StModerationUser, stModerator: &UserSummary) -> bool {
    !stTarget.bAnonymous && stModerator.canmod && (!stTarget.bModerator || stModerator.candel)
}

pub fn bIsFreezable(stTarget: &StModerationUser, stModerator: &UserSummary) -> bool {
    stModerator.canmod && !stTarget.bModerator
}

fn vRejectAnonymous(stTarget: &StModerationUser) -> Result<()> {
    if stTarget.bAnonymous {
        Err(AppError::Forbidden)
    } else {
        Ok(())
    }
}

fn optFreezeUntil(sShift: &str, dtNow: DateTime<Utc>) -> Option<(DateTime<Utc>, bool)> {
    let stResult = match sShift {
        "Разморозить" => (dtNow, true),
        "30 минут" => (dtNow + Duration::minutes(30), false),
        "час" => (dtNow + Duration::hours(1), false),
        "2 часа" => (dtNow + Duration::hours(2), false),
        "3 часа" => (dtNow + Duration::hours(3), false),
        "6 часов" => (dtNow + Duration::hours(6), false),
        "9 часов" => (dtNow + Duration::hours(9), false),
        "12 часов" => (dtNow + Duration::hours(12), false),
        "сутки" => (dtNow + Duration::days(1), false),
        "двое суток" => (dtNow + Duration::days(2), false),
        "3 дня" => (dtNow + Duration::days(3), false),
        "5 дней" => (dtNow + Duration::days(5), false),
        "неделя" => (dtNow + Duration::weeks(1), false),
        "две недели" => (dtNow + Duration::weeks(2), false),
        "месяц" => (dtNow.checked_add_months(Months::new(1))?, false),
        "2 месяца" => (dtNow.checked_add_months(Months::new(2))?, false),
        "3 месяца" => (dtNow.checked_add_months(Months::new(3))?, false),
        _ => return None,
    };
    Some(stResult)
}

fn sGenerateJavaPassword() -> String {
    (0..12)
        .map(|_| char::from(rand::random_range(33u8..126u8)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};

    use super::{
        CUserModerationService, EnUserModAction, EnUserModOutcome, StUserModCommand, bIsBlockable,
        optFreezeUntil, sGenerateJavaPassword,
    };
    use crate::{
        domain::user::{
            moderation::{
                EnUserModerationMutation, StModerationUser, StUserModerationMutationResult,
            },
            repository::TrUserModerationRepository,
        },
        error::{AppError, Result},
        models::UserSummary,
    };

    #[derive(Clone)]
    struct CTestRepository {
        stTarget: StModerationUser,
        vecMutations: Arc<Mutex<Vec<EnUserModerationMutation>>>,
    }

    #[async_trait]
    impl TrUserModerationRepository for CTestRepository {
        async fn optUser(&self, _iUserId: i32) -> Result<Option<StModerationUser>> {
            Ok(Some(self.stTarget.clone()))
        }

        async fn stApply(
            &self,
            enMutation: EnUserModerationMutation,
        ) -> Result<StUserModerationMutationResult> {
            self.vecMutations
                .lock()
                .expect("mutation lock")
                .push(enMutation);
            Ok(StUserModerationMutationResult::default())
        }
    }

    fn stActor(bAdministrator: bool) -> UserSummary {
        UserSummary {
            id: 7,
            nick: "moderator".to_owned(),
            name: None,
            score: Some(1000),
            max_score: Some(1000),
            photo: None,
            town: None,
            regdate: None,
            canmod: true,
            candel: bAdministrator,
            corrector: false,
            blocked: Some(false),
            userinfo: None,
        }
    }

    fn stTarget() -> StModerationUser {
        StModerationUser {
            iId: 8,
            sNick: "target".to_owned(),
            bModerator: false,
            bAdministrator: false,
            bAnonymous: false,
            bCorrector: false,
            bBlocked: false,
            iScore: 250,
        }
    }

    #[test]
    fn block_policy_uses_password_derived_anonymous_and_admin_override() {
        let stModerator = stActor(false);
        let mut stTarget = stTarget();
        assert!(bIsBlockable(&stTarget, &stModerator));
        stTarget.bAnonymous = true;
        assert!(!bIsBlockable(&stTarget, &stModerator));
        stTarget.bAnonymous = false;
        stTarget.bModerator = true;
        assert!(!bIsBlockable(&stTarget, &stModerator));
        assert!(bIsBlockable(&stTarget, &stActor(true)));
    }

    #[test]
    fn freeze_months_use_calendar_arithmetic_and_unfreeze_is_now() {
        let dtNow = Utc.with_ymd_and_hms(2024, 1, 31, 12, 0, 0).unwrap();
        assert_eq!(
            optFreezeUntil("месяц", dtNow),
            Some((Utc.with_ymd_and_hms(2024, 2, 29, 12, 0, 0).unwrap(), false))
        );
        assert_eq!(optFreezeUntil("Разморозить", dtNow), Some((dtNow, true)));
        assert_eq!(optFreezeUntil("7 дней", dtNow), None);
    }

    #[test]
    fn generated_password_matches_java_printable_ascii_shape() {
        let sPassword = sGenerateJavaPassword();
        assert_eq!(sPassword.len(), 12);
        assert!(sPassword.bytes().all(|iByte| (33..=125).contains(&iByte)));
    }

    #[tokio::test]
    async fn score50_rejects_anonymous_target_even_when_id_is_not_two() {
        let mut stTarget = stTarget();
        stTarget.bAnonymous = true;
        stTarget.iId = 999;
        stTarget.iScore = 0;
        let cService = CUserModerationService::new(CTestRepository {
            stTarget,
            vecMutations: Arc::new(Mutex::new(Vec::new())),
        });

        assert!(matches!(
            cService
                .enExecute(
                    &stActor(false),
                    StUserModCommand {
                        iTargetUserId: 999,
                        enAction: EnUserModAction::Score50,
                        optReason: None,
                        optShift: None,
                    },
                )
                .await,
            Err(AppError::Forbidden)
        ));
    }

    #[tokio::test]
    async fn toggle_corrector_emits_exact_set_mutation_and_redirect() {
        let vecMutations = Arc::new(Mutex::new(Vec::new()));
        let cService = CUserModerationService::new(CTestRepository {
            stTarget: stTarget(),
            vecMutations: Arc::clone(&vecMutations),
        });
        let enOutcome = cService
            .enExecute(
                &stActor(false),
                StUserModCommand {
                    iTargetUserId: 8,
                    enAction: EnUserModAction::ToggleCorrector,
                    optReason: None,
                    optShift: None,
                },
            )
            .await
            .expect("toggle corrector");

        assert_eq!(
            enOutcome,
            EnUserModOutcome::ProfileRedirect {
                sNick: "target".to_owned()
            }
        );
        assert!(matches!(
            vecMutations.lock().expect("mutation lock").as_slice(),
            [EnUserModerationMutation::SetCorrector {
                iTargetUserId: 8,
                iModeratorId: 7,
                bCorrector: true,
            }]
        ));
    }

    #[tokio::test]
    async fn userpic_reset_allows_owner_without_penalty() {
        let vecMutations = Arc::new(Mutex::new(Vec::new()));
        let mut stOwner = stActor(false);
        stOwner.id = 8;
        let cService = CUserModerationService::new(CTestRepository {
            stTarget: stTarget(),
            vecMutations: Arc::clone(&vecMutations),
        });

        assert_eq!(
            cService
                .sResetUserpic(&stOwner, 8)
                .await
                .expect("owner resets userpic"),
            "target"
        );
        assert!(matches!(
            vecMutations.lock().expect("mutation lock").as_slice(),
            [EnUserModerationMutation::ResetUserpic {
                iTargetUserId: 8,
                iActorUserId: 8,
                bScorePenalty: false,
            }]
        ));
    }

    #[tokio::test]
    async fn moderator_userpic_reset_penalizes_only_non_moderator_target() {
        let vecMutations = Arc::new(Mutex::new(Vec::new()));
        let cService = CUserModerationService::new(CTestRepository {
            stTarget: stTarget(),
            vecMutations: Arc::clone(&vecMutations),
        });

        cService
            .sResetUserpic(&stActor(false), 8)
            .await
            .expect("moderator resets another userpic");
        assert!(matches!(
            vecMutations.lock().expect("mutation lock").as_slice(),
            [EnUserModerationMutation::ResetUserpic {
                iTargetUserId: 8,
                iActorUserId: 7,
                bScorePenalty: true,
            }]
        ));
    }
}
