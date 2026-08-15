use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;

pub const S_UNCOMMIT_EXPIRED: &str = "нельзя восстанавливать устаревшие сообщения";
pub const S_UNCOMMIT_DELETED: &str = "сообщение удалено";
pub const S_UNCOMMIT_NOT_COMMITTED: &str = "сообщение не подтверждено";
pub const S_MOVE_DELETED: &str = "Сообщение удалено";
pub const S_RESOLVE_GROUP_DISABLED: &str = "В данной группе нельзя помечать темы как решенные";
pub const S_RESOLVE_FORBIDDEN: &str = "У Вас нет прав на решение данной темы";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StTopicModerationActor<'a> {
    pub iUserId: i32,
    pub sNick: &'a str,
    pub bModerator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicModerationSnapshot {
    pub iTopicId: i32,
    pub iAuthorId: i32,
    pub sAuthorNick: String,
    pub iAuthorScore: i32,
    pub bAuthorBlocked: bool,
    pub sStoredTitle: String,
    pub sMessage: String,
    pub sMarkup: String,
    pub optUrl: Option<String>,
    pub optLinkText: Option<String>,
    pub iGroupId: i32,
    pub sGroupTitle: String,
    pub sGroupUrlName: String,
    pub iSectionId: i32,
    pub sSectionPrefix: String,
    pub bSectionPremoderated: bool,
    pub bSectionPollAllowed: bool,
    pub bLinksAllowed: bool,
    pub bGroupResolvable: bool,
    pub bDeleted: bool,
    pub bCommitted: bool,
    pub bSticky: bool,
    pub bExpired: bool,
    pub dtLastMod: DateTime<Utc>,
}

impl StTopicModerationSnapshot {
    pub fn sCanonicalUrl(&self) -> String {
        format!(
            "/{}/{}/{}",
            self.sSectionPrefix, self.sGroupUrlName, self.iTopicId
        )
    }

    /// `TopicLinkBuilder.baseLink(topic).forceLastmod` receives the Topic
    /// instance loaded before move/resolve.  Both path and milliseconds must
    /// therefore remain stale after the mutation.
    pub fn sForceLastModUrl(&self) -> String {
        format!(
            "{}?lastmod={}",
            self.sCanonicalUrl(),
            self.dtLastMod.timestamp_millis()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicMoveGroup {
    pub iId: i32,
    pub sTitle: String,
    pub iSectionId: i32,
    pub sSectionTitle: String,
    /// Java exposes this as `Group.linksAllowed`, but GroupDao obtains it from
    /// the joined `sections.havelink` column rather than from `groups`.
    pub bLinksAllowed: bool,
    pub bResolvable: bool,
}

impl StTopicMoveGroup {
    pub fn sFormLabel(&self) -> String {
        format!("{}: {}", self.sSectionTitle, self.sTitle)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnTopicMoveForm {
    /// `/mt.jsp`: Forum (2), followed by Articles (6), groups ordered by id.
    ForumAndArticles,
    /// `/mtn.jsp`: the current section, except that a non-poll premoderated
    /// topic may move among all non-poll premoderated sections.
    PremoderatedCompanion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnTopicMoveGroupScope {
    ForumAndArticles,
    CurrentSection(i32),
    PremoderatedNonPoll,
}

pub fn enMoveGroupScope(
    enForm: EnTopicMoveForm,
    stTopic: &StTopicModerationSnapshot,
) -> EnTopicMoveGroupScope {
    match enForm {
        EnTopicMoveForm::ForumAndArticles => EnTopicMoveGroupScope::ForumAndArticles,
        EnTopicMoveForm::PremoderatedCompanion
            if stTopic.bSectionPremoderated && !stTopic.bSectionPollAllowed =>
        {
            EnTopicMoveGroupScope::PremoderatedNonPoll
        }
        EnTopicMoveForm::PremoderatedCompanion => {
            EnTopicMoveGroupScope::CurrentSection(stTopic.iSectionId)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnTopicModerationRestriction {
    UncommitExpired,
    UncommitDeleted,
    UncommitNotCommitted,
    MoveDeleted,
    ResolveGroupDisabled,
    ResolveForbidden,
}

impl EnTopicModerationRestriction {
    pub const fn sReason(self) -> &'static str {
        match self {
            Self::UncommitExpired => S_UNCOMMIT_EXPIRED,
            Self::UncommitDeleted => S_UNCOMMIT_DELETED,
            Self::UncommitNotCommitted => S_UNCOMMIT_NOT_COMMITTED,
            Self::MoveDeleted => S_MOVE_DELETED,
            Self::ResolveGroupDisabled => S_RESOLVE_GROUP_DISABLED,
            Self::ResolveForbidden => S_RESOLVE_FORBIDDEN,
        }
    }
}

pub fn optUncommitRestriction(
    stTopic: &StTopicModerationSnapshot,
) -> Option<EnTopicModerationRestriction> {
    // TopicModificationController.checkUncommitable preserves this order.
    if stTopic.bExpired {
        Some(EnTopicModerationRestriction::UncommitExpired)
    } else if stTopic.bDeleted {
        Some(EnTopicModerationRestriction::UncommitDeleted)
    } else if !stTopic.bCommitted {
        Some(EnTopicModerationRestriction::UncommitNotCommitted)
    } else {
        None
    }
}

pub fn optMoveRestriction(
    stTopic: &StTopicModerationSnapshot,
) -> Option<EnTopicModerationRestriction> {
    stTopic
        .bDeleted
        .then_some(EnTopicModerationRestriction::MoveDeleted)
}

pub fn optResolveRestriction(
    stTopic: &StTopicModerationSnapshot,
    stActor: StTopicModerationActor<'_>,
) -> Option<EnTopicModerationRestriction> {
    // ResolveController checks the group's feature flag before ownership.
    if !stTopic.bGroupResolvable {
        Some(EnTopicModerationRestriction::ResolveGroupDisabled)
    } else if !stActor.bModerator && stActor.iUserId != stTopic.iAuthorId {
        Some(EnTopicModerationRestriction::ResolveForbidden)
    } else {
        None
    }
}

/// The Java controller does not parse a boolean: only the exact, case-
/// sensitive string `yes` means resolved. Every other present value means
/// unresolved.
pub fn bResolveValue(sValue: &str) -> bool {
    sValue == "yes"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnTopicMarkup {
    Lorcode,
    LorcodeUserLineBreak,
    Html,
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StUnsupportedTopicMarkup(pub String);

impl fmt::Display for StUnsupportedTopicMarkup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported topic markup {}", self.0)
    }
}

impl std::error::Error for StUnsupportedTopicMarkup {}

impl TryFrom<&str> for EnTopicMarkup {
    type Error = StUnsupportedTopicMarkup;

    fn try_from(sMarkup: &str) -> std::result::Result<Self, Self::Error> {
        match sMarkup {
            "BBCODE_TEX" | "LORCODE" => Ok(Self::Lorcode),
            "BBCODE_ULB" => Ok(Self::LorcodeUserLineBreak),
            "PLAIN" => Ok(Self::Html),
            "MARKDOWN" => Ok(Self::Markdown),
            _ => Err(StUnsupportedTopicMarkup(sMarkup.to_owned())),
        }
    }
}

/// Exact `MessageTextService.moveInfo` output, including its historic leading
/// and repeated newlines.  These bytes are appended directly to msgbase and
/// are therefore a migration-compatibility contract, not presentation-only
/// whitespace.
pub fn sMoveInfo(
    enMarkup: EnTopicMarkup,
    optUrl: Option<&str>,
    optLinkText: Option<&str>,
    sModeratorNick: &str,
    sOldGroupUrlName: &str,
) -> String {
    let sLink = optUrl
        .filter(|sUrl| !sUrl.is_empty())
        .map_or_else(String::new, |sUrl| {
            let sEffectiveLinkText = optLinkText
                .filter(|sLinkText| !sLinkText.is_empty())
                .unwrap_or("Подробности");
            match enMarkup {
                EnTopicMarkup::Html => format!(
                    "<br><a href=\"{}\">{}</a>\n<br>\n",
                    sEscapeHtmlLikeJava(sUrl),
                    sEscapeHtmlLikeJava(sEffectiveLinkText)
                ),
                EnTopicMarkup::Lorcode | EnTopicMarkup::LorcodeUserLineBreak => {
                    format!("\n[url={sUrl}]{sEffectiveLinkText}[/url]\n")
                }
                EnTopicMarkup::Markdown => format!(
                    "\n[{}]({sUrl})\n",
                    sEscapeInlineMarkdown(sEffectiveLinkText)
                ),
            }
        });

    match enMarkup {
        EnTopicMarkup::Lorcode | EnTopicMarkup::LorcodeUserLineBreak => {
            format!("\n{sLink}\n\n[i]Перемещено {sModeratorNick} из {sOldGroupUrlName}[/i]\n")
        }
        EnTopicMarkup::Html => {
            format!("\n{sLink}<br><i>Перемещено {sModeratorNick} из {sOldGroupUrlName}</i>\n")
        }
        EnTopicMarkup::Markdown => {
            format!("\n{sLink}\n\nПеремещено {sModeratorNick} из {sOldGroupUrlName}\n")
        }
    }
}

fn sEscapeHtmlLikeJava(sValue: &str) -> String {
    // Guava HtmlEscapers.htmlEscaper(), used by StringUtil.escapeHtml.
    let mut sEscaped = String::with_capacity(sValue.len());
    for cValue in sValue.chars() {
        match cValue {
            '"' => sEscaped.push_str("&quot;"),
            '\'' => sEscaped.push_str("&#39;"),
            '&' => sEscaped.push_str("&amp;"),
            '<' => sEscaped.push_str("&lt;"),
            '>' => sEscaped.push_str("&gt;"),
            _ => sEscaped.push(cValue),
        }
    }
    sEscaped
}

fn sEscapeInlineMarkdown(sValue: &str) -> String {
    // FlexmarkMarkdownFormatter.escapeInlineMarkdown escapes precisely these
    // four characters, with backslash first to prevent double escaping.
    let mut sEscaped = String::with_capacity(sValue.len() + 16);
    for cValue in sValue.chars() {
        match cValue {
            '\\' => sEscaped.push_str("\\\\"),
            '[' => sEscaped.push_str("\\["),
            ']' => sEscaped.push_str("\\]"),
            '`' => sEscaped.push_str("\\`"),
            _ => sEscaped.push(cValue),
        }
    }
    sEscaped
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StUncommitParameters {
    pub iTopicId: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StMoveParameters {
    pub iTopicId: i32,
    pub iMoveToGroupId: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StResolveParameters {
    pub iTopicId: i32,
    pub sResolve: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnLegacyRequiredBindingError {
    Missing { sName: &'static str },
    InvalidInteger { sName: &'static str },
}

impl fmt::Display for EnLegacyRequiredBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { sName } => write!(f, "Required parameter '{sName}' is missing"),
            Self::InvalidInteger { sName } => {
                write!(f, "Failed to convert parameter '{sName}'")
            }
        }
    }
}

impl std::error::Error for EnLegacyRequiredBindingError {}

/// Spring `@RequestParam` binding is method-independent and rejects missing or
/// malformed required values with 400.  Axum's `Query`/`Form` rejection is
/// 422, so the moderation routes deliberately bind decoded pairs themselves.
pub fn stBindUncommitParameters(
    vecParameters: &[(String, String)],
) -> std::result::Result<StUncommitParameters, EnLegacyRequiredBindingError> {
    Ok(StUncommitParameters {
        iTopicId: iRequiredI32(vecParameters, "msgid")?,
    })
}

pub fn stBindMoveParameters(
    vecParameters: &[(String, String)],
) -> std::result::Result<StMoveParameters, EnLegacyRequiredBindingError> {
    Ok(StMoveParameters {
        iTopicId: iRequiredI32(vecParameters, "msgid")?,
        iMoveToGroupId: iRequiredI32(vecParameters, "moveto")?,
    })
}

pub fn stBindResolveParameters(
    vecParameters: &[(String, String)],
) -> std::result::Result<StResolveParameters, EnLegacyRequiredBindingError> {
    Ok(StResolveParameters {
        iTopicId: iRequiredI32(vecParameters, "msgid")?,
        sResolve: sRequired(vecParameters, "resolve")?.to_owned(),
    })
}

fn iRequiredI32(
    vecParameters: &[(String, String)],
    sName: &'static str,
) -> std::result::Result<i32, EnLegacyRequiredBindingError> {
    sRequired(vecParameters, sName)?
        .trim()
        .parse::<i32>()
        .map_err(|_| EnLegacyRequiredBindingError::InvalidInteger { sName })
}

fn sRequired<'a>(
    vecParameters: &'a [(String, String)],
    sName: &'static str,
) -> std::result::Result<&'a str, EnLegacyRequiredBindingError> {
    vecParameters
        .iter()
        .find_map(|(sKey, sValue)| (sKey == sName).then_some(sValue.as_str()))
        .ok_or(EnLegacyRequiredBindingError::Missing { sName })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StMoveTopicCommand {
    pub iTopicId: i32,
    pub iTargetGroupId: i32,
    pub bTargetLinksAllowed: bool,
    /// TopicService receives these values from the stale Topic/current-user
    /// objects held by the controller, but reads msgbase.markup later, inside
    /// the move transaction.
    pub optOriginalUrl: Option<String>,
    pub optOriginalLinkText: Option<String>,
    pub sOriginalGroupUrlName: String,
    pub sModeratorNick: String,
}

#[async_trait]
pub trait TrTopicModerationRepository: Send + Sync {
    async fn optSnapshot(&self, iTopicId: i32) -> Result<Option<StTopicModerationSnapshot>>;

    async fn optMoveGroup(&self, iGroupId: i32) -> Result<Option<StTopicMoveGroup>>;

    async fn vecMoveGroups(&self, enScope: EnTopicMoveGroupScope) -> Result<Vec<StTopicMoveGroup>>;

    /// Exact one-statement TopicDao.uncommit transaction. Lastmod is not
    /// touched and score/history/events are intentionally absent.
    async fn vUncommit(&self, iTopicId: i32) -> Result<()>;

    /// TopicDao.moveTopic row-lock/update/link clearing plus MsgbaseDao append
    /// form one PostgreSQL transaction. The application skips this entirely
    /// when its stale controller snapshot already has the target group.
    async fn vMove(&self, stCommand: StMoveTopicCommand) -> Result<()>;

    /// TopicDao.resolveMessage increments lastmod by exactly one second even
    /// when the resolved value did not change.
    async fn vResolve(&self, iTopicId: i32, bResolved: bool) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::TimeZone;
    use pretty_assertions::assert_eq;

    use super::*;

    fn stTopic() -> StTopicModerationSnapshot {
        StTopicModerationSnapshot {
            iTopicId: 42,
            iAuthorId: 7,
            sAuthorNick: "author".into(),
            iAuthorScore: 300,
            bAuthorBlocked: false,
            sStoredTitle: "title".into(),
            sMessage: "body".into(),
            sMarkup: "MARKDOWN".into(),
            optUrl: Some("https://example.test/a".into()),
            optLinkText: Some("details".into()),
            iGroupId: 10,
            sGroupTitle: "Old".into(),
            sGroupUrlName: "old-group".into(),
            iSectionId: 1,
            sSectionPrefix: "news".into(),
            bSectionPremoderated: true,
            bSectionPollAllowed: false,
            bLinksAllowed: true,
            bGroupResolvable: true,
            bDeleted: false,
            bCommitted: true,
            bSticky: false,
            bExpired: false,
            dtLastMod: Utc.with_ymd_and_hms(2026, 8, 15, 10, 11, 12).unwrap(),
        }
    }

    #[test]
    fn uncommit_restrictions_keep_the_java_order_and_exact_messages() {
        let mut stTopic = stTopic();
        stTopic.bExpired = true;
        stTopic.bDeleted = true;
        stTopic.bCommitted = false;
        assert_eq!(
            optUncommitRestriction(&stTopic),
            Some(EnTopicModerationRestriction::UncommitExpired)
        );
        assert_eq!(
            optUncommitRestriction(&stTopic).unwrap().sReason(),
            "нельзя восстанавливать устаревшие сообщения"
        );

        stTopic.bExpired = false;
        assert_eq!(
            optUncommitRestriction(&stTopic).unwrap().sReason(),
            "сообщение удалено"
        );
        stTopic.bDeleted = false;
        assert_eq!(
            optUncommitRestriction(&stTopic).unwrap().sReason(),
            "сообщение не подтверждено"
        );
    }

    #[test]
    fn resolve_checks_group_before_actor_and_only_exact_yes_is_true() {
        let mut stTopic = stTopic();
        let stOther = StTopicModerationActor {
            iUserId: 99,
            sNick: "other",
            bModerator: false,
        };
        stTopic.bGroupResolvable = false;
        assert_eq!(
            optResolveRestriction(&stTopic, stOther).unwrap().sReason(),
            S_RESOLVE_GROUP_DISABLED
        );
        stTopic.bGroupResolvable = true;
        assert_eq!(
            optResolveRestriction(&stTopic, stOther).unwrap().sReason(),
            S_RESOLVE_FORBIDDEN
        );

        assert!(bResolveValue("yes"));
        for sValue in ["", "no", "YES", "true", " yes"] {
            assert!(!bResolveValue(sValue), "{sValue:?}");
        }
    }

    #[test]
    fn force_lastmod_url_uses_the_pre_mutation_path_and_milliseconds() {
        assert_eq!(
            stTopic().sForceLastModUrl(),
            "/news/old-group/42?lastmod=1786788672000"
        );
    }

    #[test]
    fn move_info_matches_all_four_database_markup_modes_byte_for_byte() {
        let sLorcode = "\n\n[url=https://example.test/a]details[/url]\n\n\n[i]Перемещено mod из old-group[/i]\n";
        assert_eq!(
            sMoveInfo(
                EnTopicMarkup::Lorcode,
                Some("https://example.test/a"),
                Some("details"),
                "mod",
                "old-group"
            ),
            sLorcode
        );
        assert_eq!(
            sMoveInfo(
                EnTopicMarkup::LorcodeUserLineBreak,
                Some("https://example.test/a"),
                Some("details"),
                "mod",
                "old-group"
            ),
            sLorcode
        );
        assert_eq!(
            sMoveInfo(
                EnTopicMarkup::Html,
                Some("https://e.test/?a='x'&b=\"y\""),
                Some("<go> & 'now'"),
                "mod",
                "old-group"
            ),
            "\n<br><a href=\"https://e.test/?a=&#39;x&#39;&amp;b=&quot;y&quot;\">&lt;go&gt; &amp; &#39;now&#39;</a>\n<br>\n<br><i>Перемещено mod из old-group</i>\n"
        );
        assert_eq!(
            sMoveInfo(
                EnTopicMarkup::Markdown,
                Some("https://example.test/(raw)"),
                Some("a\\[b]`c"),
                "mod",
                "old-group"
            ),
            "\n\n[a\\\\\\[b\\]\\`c](https://example.test/(raw))\n\n\nПеремещено mod из old-group\n"
        );
    }

    #[test]
    fn move_info_keeps_default_link_text_and_no_url_whitespace() {
        assert_eq!(
            sMoveInfo(
                EnTopicMarkup::Markdown,
                Some("https://example.test"),
                Some(""),
                "mod",
                "old"
            ),
            "\n\n[Подробности](https://example.test)\n\n\nПеремещено mod из old\n"
        );
        assert_eq!(
            sMoveInfo(EnTopicMarkup::Html, None, None, "mod", "old"),
            "\n<br><i>Перемещено mod из old</i>\n"
        );
        assert_eq!(
            sMoveInfo(EnTopicMarkup::Lorcode, None, None, "mod", "old"),
            "\n\n\n[i]Перемещено mod из old[/i]\n"
        );
    }

    #[test]
    fn required_legacy_parameters_never_degrade_to_optional_toggle_semantics() {
        let mapEmpty = HashMap::<String, String>::new();
        let vecEmpty = mapEmpty.into_iter().collect::<Vec<_>>();
        assert_eq!(
            stBindUncommitParameters(&vecEmpty).unwrap_err(),
            EnLegacyRequiredBindingError::Missing { sName: "msgid" }
        );
        assert_eq!(
            stBindResolveParameters(&[("msgid".into(), "x".into())]).unwrap_err(),
            EnLegacyRequiredBindingError::InvalidInteger { sName: "msgid" }
        );
        assert_eq!(
            stBindResolveParameters(&[("msgid".into(), "1".into())]).unwrap_err(),
            EnLegacyRequiredBindingError::Missing { sName: "resolve" }
        );

        let stEmptyResolve = stBindResolveParameters(&[
            ("msgid".into(), " 42 ".into()),
            ("resolve".into(), String::new()),
        ])
        .unwrap();
        assert_eq!(stEmptyResolve.iTopicId, 42);
        assert_eq!(stEmptyResolve.sResolve, "");
        assert!(!bResolveValue(&stEmptyResolve.sResolve));

        assert_eq!(
            stBindMoveParameters(&[("msgid".into(), "42".into()), ("moveto".into(), "6".into()),])
                .unwrap(),
            StMoveParameters {
                iTopicId: 42,
                iMoveToGroupId: 6,
            }
        );
    }

    #[test]
    fn mtn_scope_matches_current_section_or_all_non_poll_premoderated_sections() {
        let mut stTopic = stTopic();
        assert_eq!(
            enMoveGroupScope(EnTopicMoveForm::ForumAndArticles, &stTopic),
            EnTopicMoveGroupScope::ForumAndArticles
        );
        assert_eq!(
            enMoveGroupScope(EnTopicMoveForm::PremoderatedCompanion, &stTopic),
            EnTopicMoveGroupScope::PremoderatedNonPoll
        );
        stTopic.bSectionPollAllowed = true;
        assert_eq!(
            enMoveGroupScope(EnTopicMoveForm::PremoderatedCompanion, &stTopic),
            EnTopicMoveGroupScope::CurrentSection(1)
        );
    }
}
