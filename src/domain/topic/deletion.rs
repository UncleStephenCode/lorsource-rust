use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Local, Months, Utc};

use crate::error::Result;

pub const I_ANONYMOUS_USER_ID: i32 = 2;

/// `DeleteReasons.DeleteReasons`, exposed by `DeleteTopicController` as a
/// model attribute.  Order and spelling are part of the legacy form contract.
pub const VEC_TOPIC_DELETE_REASONS: &[&str] = &[
    "3.1 Дубль",
    "3.2 Неверная кодировка",
    "3.3 Некорректное форматирование",
    "3.4 Пустое сообщение",
    "4.1 Офтопик",
    "4.2 Вызывающе неверная информация",
    "4.3 Провокация flame",
    "4.4 Обсуждение действий модераторов",
    "4.5 Тестовые сообщения",
    "4.6 Спам",
    "4.7 Флуд",
    "4.8 Дискуссия не на русском языке",
    "4.9 Офтопик-лист, п. ",
    "5.1 Нецензурные выражения",
    "5.2 Оскорбление участников дискуссии",
    "5.3 Национальные/политические/религиозные споры",
    "5.4 Личная переписка",
    "5.5 Преднамеренное нарушение правил русского языка",
    "6 Нарушение copyright",
    "6.2 Warez",
    "7.1 Ответ на некорректное сообщение",
    "7.2 Чрезмерно исправленное сообщение",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StTopicDeletionActor<'a> {
    pub iUserId: i32,
    pub sNick: &'a str,
    pub bModerator: bool,
    pub bAdministrator: bool,
}

/// Controller snapshot loaded before `DeleteService.localTx`.  Besides the
/// policy fields it deliberately carries the complete base topic/author/group
/// model needed by the shared PreparedTopic renderer on the undelete form.
/// Tags, polls, images, warnings and reactions remain supplemental collections
/// loaded by that renderer; reducing this to just `msgid` would lose the Java
/// form's full `<lor:topic showMenu="false">` contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicDeletionSnapshot {
    pub iTopicId: i32,
    pub iAuthorId: i32,
    pub sAuthorNick: String,
    pub iAuthorScore: i32,
    pub iAuthorMaxScore: i32,
    pub bAuthorBlocked: bool,
    pub bAuthorAnonymous: bool,
    pub bAuthorFrozen: bool,
    pub sStoredTitle: String,
    pub sMessage: String,
    pub sMarkup: String,
    pub optUrl: Option<String>,
    pub optLinkText: Option<String>,
    pub iGroupId: i32,
    pub sGroupTitle: String,
    pub sGroupUrlName: String,
    pub iSectionId: i32,
    pub sSectionTitle: String,
    pub sSectionPrefix: String,
    pub bSectionPremoderated: bool,
    pub bSectionPollAllowed: bool,
    pub bSectionImagePost: bool,
    pub bSectionImageAllowed: bool,
    pub bLinksAllowed: bool,
    pub bDeleted: bool,
    pub bDraft: bool,
    pub bCommitted: bool,
    pub bSticky: bool,
    pub bResolved: bool,
    pub bExpired: bool,
    pub iCommentCount: i32,
    pub iPostScore: i32,
    pub bMinor: bool,
    pub dtPostdate: DateTime<Utc>,
    pub optCommitDate: Option<DateTime<Utc>>,
    pub dtLastMod: DateTime<Utc>,
    pub optDeleteDate: Option<DateTime<Utc>>,
    pub sPostIp: String,
    pub iUserAgentId: i32,
}

impl StTopicDeletionSnapshot {
    pub fn sCanonicalUrl(&self) -> String {
        format!(
            "/{}/{}/{}",
            self.sSectionPrefix, self.sGroupUrlName, self.iTopicId
        )
    }

    pub fn bDeleteBonusEligible(&self) -> bool {
        !self.bSectionPremoderated && !self.bDraft && !self.bExpired
    }

    pub fn bUncommitted(&self) -> bool {
        self.bSectionPremoderated && !self.bCommitted
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnTopicDeletionRestriction {
    AlreadyDeleted,
    CannotDelete,
    CannotUndelete,
}

impl EnTopicDeletionRestriction {
    pub const fn sReason(self) -> &'static str {
        match self {
            Self::AlreadyDeleted => "Сообщение уже удалено",
            Self::CannotDelete => "Вы не можете удалить это сообщение",
            Self::CannotUndelete => "это сообщение нельзя восстановить",
        }
    }
}

impl fmt::Display for EnTopicDeletionRestriction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.sReason())
    }
}

impl std::error::Error for EnTopicDeletionRestriction {}

pub fn optDeleteRestriction(
    stActor: StTopicDeletionActor<'_>,
    stTopic: &StTopicDeletionSnapshot,
    dtNow: DateTime<Utc>,
) -> Option<EnTopicDeletionRestriction> {
    if stTopic.bDeleted {
        return Some(EnTopicDeletionRestriction::AlreadyDeleted);
    }
    if stActor.bAdministrator {
        return None;
    }

    let bByAuthor = if stActor.iUserId != stTopic.iAuthorId {
        false
    } else if stTopic.bDraft {
        true
    } else if stTopic.bSectionPremoderated && stTopic.bCommitted {
        false
    } else {
        stTopic.dtPostdate + Duration::days(3) > dtNow && stTopic.iCommentCount == 0
    };
    if bByAuthor {
        return None;
    }

    let bByModerator = stActor.bModerator
        && if !stTopic.bSectionPremoderated || !stTopic.bCommitted {
            true
        } else {
            optModeratorDeleteDeadline(stTopic.dtPostdate)
                .is_some_and(|dtDeadline| dtDeadline > dtNow)
        };

    (!bByModerator).then_some(EnTopicDeletionRestriction::CannotDelete)
}

pub fn optUndeleteRestriction(
    stActor: StTopicDeletionActor<'_>,
    stTopic: &StTopicDeletionSnapshot,
    dtNow: DateTime<Utc>,
) -> Option<EnTopicDeletionRestriction> {
    let bAllowed = stTopic.bDeleted
        && stActor.bModerator
        && (stActor.bAdministrator
            || !stTopic.bExpired
            || stTopic
                .optDeleteDate
                .is_some_and(|dtDeleted| dtDeleted > dtNow - Duration::days(14)));
    (!bAllowed).then_some(EnTopicDeletionRestriction::CannotUndelete)
}

/// `ZoneId.systemDefault().plusMonths(1)` is a calendar operation.  A fixed
/// 30-day duration changes the moderator boundary for short/long months and
/// around DST, so conversion through the process-local timezone is required.
fn optModeratorDeleteDeadline(dtPostdate: DateTime<Utc>) -> Option<DateTime<Utc>> {
    dtPostdate
        .with_timezone(&Local)
        .checked_add_months(Months::new(1))
        .map(|dtDeadline| dtDeadline.with_timezone(&Utc))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StDeleteTopicParameters {
    pub iTopicId: i32,
    pub sReason: String,
    pub iPenalty: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StUndeleteTopicParameters {
    pub iTopicId: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnTopicDeletionBindingError {
    Missing { sName: &'static str },
    InvalidInteger { sName: &'static str },
}

impl fmt::Display for EnTopicDeletionBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { sName } => write!(f, "Required parameter '{sName}' is missing"),
            Self::InvalidInteger { sName } => {
                write!(f, "Failed to convert parameter '{sName}'")
            }
        }
    }
}

impl std::error::Error for EnTopicDeletionBindingError {}

/// Spring resolves all `@RequestParam` values before entering
/// `AuthorizedOnly`; callers must therefore run this binder before auth and
/// map any error to HTTP 400 (Axum's typed extractor would otherwise emit
/// 422).  An absent or empty `bonus` uses the Java `defaultValue="0"`.
pub fn stBindDeleteTopicParameters(
    vecParameters: &[(String, String)],
) -> std::result::Result<StDeleteTopicParameters, EnTopicDeletionBindingError> {
    Ok(StDeleteTopicParameters {
        iTopicId: iRequiredI32(vecParameters, "msgid")?,
        sReason: sRequired(vecParameters, "reason")?.to_owned(),
        iPenalty: match optValue(vecParameters, "bonus") {
            None | Some("") => 0,
            Some(sValue) => iParseI32(sValue, "bonus")?,
        },
    })
}

pub fn stBindTopicId(
    vecParameters: &[(String, String)],
) -> std::result::Result<StUndeleteTopicParameters, EnTopicDeletionBindingError> {
    Ok(StUndeleteTopicParameters {
        iTopicId: iRequiredI32(vecParameters, "msgid")?,
    })
}

fn iRequiredI32(
    vecParameters: &[(String, String)],
    sName: &'static str,
) -> std::result::Result<i32, EnTopicDeletionBindingError> {
    iParseI32(sRequired(vecParameters, sName)?, sName)
}

fn iParseI32(
    sValue: &str,
    sName: &'static str,
) -> std::result::Result<i32, EnTopicDeletionBindingError> {
    sValue
        .trim()
        .parse::<i32>()
        .map_err(|_| EnTopicDeletionBindingError::InvalidInteger { sName })
}

fn sRequired<'a>(
    vecParameters: &'a [(String, String)],
    sName: &'static str,
) -> std::result::Result<&'a str, EnTopicDeletionBindingError> {
    optValue(vecParameters, sName).ok_or(EnTopicDeletionBindingError::Missing { sName })
}

fn optValue<'a>(vecParameters: &'a [(String, String)], sName: &str) -> Option<&'a str> {
    vecParameters
        .iter()
        .find_map(|(sKey, sValue)| (sKey == sName).then_some(sValue.as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StDeleteTopicCommand {
    pub iTopicId: i32,
    pub sReason: String,
    /// Positive UI penalty in the inclusive Java controller range 0..=20.
    pub iPenalty: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StTopicDeleteMutation {
    pub bDeleted: bool,
    /// Raw additive delta actually written to `users.score` (zero or negative).
    pub iAppliedScoreDelta: i32,
}

#[async_trait]
pub trait TrTopicDeletionRepository: Send + Sync {
    async fn optSnapshot(&self, iTopicId: i32) -> Result<Option<StTopicDeletionSnapshot>>;
    async fn stDelete(
        &self,
        stActor: StTopicDeletionActor<'_>,
        stTopic: &StTopicDeletionSnapshot,
        stCommand: &StDeleteTopicCommand,
    ) -> Result<StTopicDeleteMutation>;
    async fn vUndelete(&self, stTopic: &StTopicDeletionSnapshot) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn stTopic() -> StTopicDeletionSnapshot {
        StTopicDeletionSnapshot {
            iTopicId: 42,
            iAuthorId: 7,
            sAuthorNick: "author".into(),
            iAuthorScore: 100,
            iAuthorMaxScore: 200,
            bAuthorBlocked: false,
            bAuthorAnonymous: false,
            bAuthorFrozen: false,
            sStoredTitle: "title".into(),
            sMessage: "body".into(),
            sMarkup: "MARKDOWN".into(),
            optUrl: None,
            optLinkText: None,
            iGroupId: 10,
            sGroupTitle: "General".into(),
            sGroupUrlName: "general".into(),
            iSectionId: 2,
            sSectionTitle: "Форум".into(),
            sSectionPrefix: "forum".into(),
            bSectionPremoderated: false,
            bSectionPollAllowed: false,
            bSectionImagePost: false,
            bSectionImageAllowed: false,
            bLinksAllowed: true,
            bDeleted: false,
            bDraft: false,
            bCommitted: true,
            bSticky: false,
            bResolved: false,
            bExpired: false,
            iCommentCount: 0,
            iPostScore: -9999,
            bMinor: false,
            dtPostdate: Utc.with_ymd_and_hms(2026, 1, 31, 12, 0, 0).unwrap(),
            optCommitDate: None,
            dtLastMod: Utc.with_ymd_and_hms(2026, 1, 31, 12, 0, 0).unwrap(),
            optDeleteDate: None,
            sPostIp: "127.0.0.1".into(),
            iUserAgentId: 1,
        }
    }

    fn stActor(
        iUserId: i32,
        bModerator: bool,
        bAdministrator: bool,
    ) -> StTopicDeletionActor<'static> {
        StTopicDeletionActor {
            iUserId,
            sNick: "actor",
            bModerator,
            bAdministrator,
        }
    }

    #[test]
    fn required_binding_is_400_ready_and_bonus_keeps_spring_default_semantics() {
        assert_eq!(
            stBindDeleteTopicParameters(&[
                ("msgid".into(), " 42 ".into()),
                ("reason".into(), "".into()),
            ])
            .unwrap(),
            StDeleteTopicParameters {
                iTopicId: 42,
                sReason: String::new(),
                iPenalty: 0,
            }
        );
        assert_eq!(
            stBindDeleteTopicParameters(&[
                ("msgid".into(), "42".into()),
                ("reason".into(), "spam".into()),
                ("bonus".into(), "".into()),
            ])
            .unwrap()
            .iPenalty,
            0
        );
        assert_eq!(
            stBindDeleteTopicParameters(&[("msgid".into(), "42".into())]).unwrap_err(),
            EnTopicDeletionBindingError::Missing { sName: "reason" }
        );
        assert_eq!(
            stBindDeleteTopicParameters(&[
                ("msgid".into(), "x".into()),
                ("reason".into(), "spam".into()),
            ])
            .unwrap_err(),
            EnTopicDeletionBindingError::InvalidInteger { sName: "msgid" }
        );
        assert_eq!(
            stBindDeleteTopicParameters(&[
                ("msgid".into(), "2147483648".into()),
                ("reason".into(), "spam".into()),
            ])
            .unwrap_err(),
            EnTopicDeletionBindingError::InvalidInteger { sName: "msgid" }
        );
    }

    #[test]
    fn author_rule_uses_a_strict_three_day_deadline_and_zero_comments() {
        let mut stTopic = stTopic();
        let dtDeadline = stTopic.dtPostdate + Duration::days(3);
        assert_eq!(
            optDeleteRestriction(
                stActor(7, false, false),
                &stTopic,
                dtDeadline - Duration::nanoseconds(1)
            ),
            None
        );
        assert_eq!(
            optDeleteRestriction(stActor(7, false, false), &stTopic, dtDeadline),
            Some(EnTopicDeletionRestriction::CannotDelete)
        );
        stTopic.iCommentCount = 1;
        assert_eq!(
            optDeleteRestriction(
                stActor(7, false, false),
                &stTopic,
                dtDeadline - Duration::hours(1)
            ),
            Some(EnTopicDeletionRestriction::CannotDelete)
        );
        stTopic.bDraft = true;
        assert_eq!(
            optDeleteRestriction(
                stActor(7, false, false),
                &stTopic,
                dtDeadline + Duration::days(30)
            ),
            None
        );
    }

    #[test]
    fn moderator_uses_a_calendar_month_not_thirty_days() {
        let mut stTopic = stTopic();
        stTopic.bSectionPremoderated = true;
        stTopic.bCommitted = true;
        let dtFebruaryEnd = Utc.with_ymd_and_hms(2026, 2, 28, 12, 0, 0).unwrap();
        assert_eq!(
            optDeleteRestriction(
                stActor(8, true, false),
                &stTopic,
                dtFebruaryEnd - Duration::seconds(1)
            ),
            None
        );
        assert_eq!(
            optDeleteRestriction(stActor(8, true, false), &stTopic, dtFebruaryEnd),
            Some(EnTopicDeletionRestriction::CannotDelete)
        );
        assert_eq!(
            optDeleteRestriction(
                stActor(9, false, true),
                &stTopic,
                dtFebruaryEnd + Duration::days(365)
            ),
            None
        );
    }

    #[test]
    fn undelete_window_is_strict_and_administrator_still_needs_deleted_state() {
        let mut stTopic = stTopic();
        stTopic.bDeleted = true;
        stTopic.bExpired = true;
        let dtNow = Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap();
        stTopic.optDeleteDate = Some(dtNow - Duration::days(14) + Duration::nanoseconds(1));
        assert_eq!(
            optUndeleteRestriction(stActor(8, true, false), &stTopic, dtNow),
            None
        );
        stTopic.optDeleteDate = Some(dtNow - Duration::days(14));
        assert_eq!(
            optUndeleteRestriction(stActor(8, true, false), &stTopic, dtNow),
            Some(EnTopicDeletionRestriction::CannotUndelete)
        );
        stTopic.bDeleted = false;
        assert_eq!(
            optUndeleteRestriction(stActor(9, true, true), &stTopic, dtNow),
            Some(EnTopicDeletionRestriction::CannotUndelete)
        );
    }
}
