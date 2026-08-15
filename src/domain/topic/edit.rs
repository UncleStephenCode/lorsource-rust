use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

use crate::{
    domain::topic::posting::{POSTSCORE_NO_COMMENTS, StIpBlockInfo, USER_ANONYMOUS_LEVEL_SCORE},
    error::Result,
};

const I_ARTICLES_SECTION: i32 = 6;
const I_NEWS_SECTION: i32 = 1;
const I_EDIT_PERIOD_DAYS: i64 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StTopicEditActor {
    pub iUserId: i32,
    pub iScore: i32,
    pub bModerator: bool,
    pub bAdministrator: bool,
    pub bCorrector: bool,
    pub bBlocked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StTopicEditRestrictions {
    pub bFrozen: bool,
    pub stIpBlock: StIpBlockInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicEditGroup {
    pub iId: i32,
    pub sTitle: String,
    pub iSectionId: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicEditEditor {
    pub iId: i32,
    pub sNick: String,
    pub iScore: i32,
    pub bBlocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicEditPollVariant {
    pub iId: i32,
    pub sLabel: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicEditPoll {
    pub iId: i32,
    pub bMultiSelect: bool,
    pub vecVariants: Vec<StTopicEditPollVariant>,
}

#[derive(Debug, Clone)]
pub struct StTopicEditSnapshot {
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
    pub bExpired: bool,
    pub iPostScore: i32,
    pub dtPostDate: DateTime<Utc>,
    pub optCommitDate: Option<DateTime<Utc>>,
    pub dtLastMod: DateTime<Utc>,
    pub bMinor: bool,
    pub vecTags: Vec<String>,
    pub optPoll: Option<StTopicEditPoll>,
    pub vecGroups: Vec<StTopicEditGroup>,
    pub optLastEditMillis: Option<i64>,
    pub vecEditors: Vec<StTopicEditEditor>,
}

impl StTopicEditSnapshot {
    pub fn bCommittable(&self) -> bool {
        !self.bCommitted && self.bSectionPremoderated
    }

    pub fn bCanBeMini(&self) -> bool {
        self.iSectionId == I_NEWS_SECTION
    }

    pub fn sCanonicalUrl(&self) -> String {
        format!(
            "/{}/{}/{}",
            self.sSectionPrefix, self.sGroupUrlName, self.iTopicId
        )
    }

    /// TopicLinkBuilder uses the Topic instance loaded before updateAndCommit.
    pub fn sForceLastModUrl(&self) -> String {
        format!(
            "{}?lastmod={}",
            self.sCanonicalUrl(),
            self.dtLastMod.timestamp_millis()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicEditPermission {
    pub optReason: Option<String>,
}

impl StTopicEditPermission {
    pub fn bPermitted(&self) -> bool {
        self.optReason.is_none()
    }

    pub fn sReason(&self) -> &str {
        self.optReason.as_deref().unwrap_or("")
    }
}

pub fn stCheckCommit(
    stTopic: &StTopicEditSnapshot,
    stActor: StTopicEditActor,
    stRestrictions: StTopicEditRestrictions,
) -> StTopicEditPermission {
    // EditTopicChecker.checkCommit starts with role restrictions and a
    // Restricted value keeps the first reason for the remainder of the
    // chain.  This ordering is observable for (for example) an ordinary
    // user attempting to commit a deleted topic.
    let optReason = (!stActor.bModerator && !stActor.bCorrector)
        .then_some("только для корректоров и модераторов".to_owned())
        .or_else(|| {
            (stActor.bCorrector && stTopic.iAuthorId == stActor.iUserId)
                .then_some("нельзя подтверждать собственные топики".to_owned())
        })
        .or_else(|| optPrecheckReason(stTopic, stActor, stRestrictions));
    StTopicEditPermission { optReason }
}

pub fn stCheckContentEdit(
    stTopic: &StTopicEditSnapshot,
    stActor: StTopicEditActor,
    stRestrictions: StTopicEditRestrictions,
    dtNow: DateTime<Utc>,
) -> StTopicEditPermission {
    // Permission.Restricted is sticky: a later `permit` never clears an
    // earlier restriction. Keep the source order explicit here.
    let optReason = if let Some(sReason) = optPrecheckReason(stTopic, stActor, stRestrictions) {
        Some(sReason)
    } else if let Some(sReason) = optLegacyMarkupReason(&stTopic.sMarkup, stActor.bAdministrator) {
        Some(sReason)
    } else if stActor.bAdministrator {
        None
    } else if stTopic.bExpired && !stTopic.bDraft {
        Some("нельзя править архивные топики".to_owned())
    } else if stActor.bModerator {
        None
    } else if stTopic.iPostScore == POSTSCORE_NO_COMMENTS {
        Some("нельзя править топики с выключенными комментариями".to_owned())
    } else if stActor.bCorrector && stTopic.bSectionPremoderated {
        None
    } else {
        optAuthorEditReason(stTopic, stActor, dtNow)
    };
    StTopicEditPermission { optReason }
}

pub fn stCheckTagsEdit(
    stTopic: &StTopicEditSnapshot,
    stActor: StTopicEditActor,
    stRestrictions: StTopicEditRestrictions,
    dtNow: DateTime<Utc>,
) -> StTopicEditPermission {
    let optReason = optPrecheckReason(stTopic, stActor, stRestrictions);
    let optReason = if optReason.is_some() {
        optReason
    } else if stActor.bAdministrator || stActor.bModerator || stActor.bCorrector {
        None
    } else {
        optAuthorEditReason(stTopic, stActor, dtNow)
    };
    StTopicEditPermission { optReason }
}

fn optPrecheckReason(
    stTopic: &StTopicEditSnapshot,
    stActor: StTopicEditActor,
    stRestrictions: StTopicEditRestrictions,
) -> Option<String> {
    if stTopic.bDeleted {
        Some("нельзя править удаленные топики".to_owned())
    } else if stRestrictions.bFrozen {
        Some("установлен режим только для чтения".to_owned())
    } else {
        optIpBlockReason(stActor, stRestrictions.stIpBlock)
    }
}

fn optIpBlockReason(stActor: StTopicEditActor, stIpBlock: StIpBlockInfo) -> Option<String> {
    if !stIpBlock.bBlocked {
        None
    } else if stActor.bBlocked || stActor.iScore < USER_ANONYMOUS_LEVEL_SCORE {
        Some(format!(
            "постинг с этого IP адреса ограничен для пользователей с score < {USER_ANONYMOUS_LEVEL_SCORE}"
        ))
    } else if !stIpBlock.bAllowRegisteredPosting {
        Some("постинг с этого IP адреса заблокирован".to_owned())
    } else {
        None
    }
}

fn optLegacyMarkupReason(sMarkup: &str, bAdministrator: bool) -> Option<String> {
    let bAllowed = matches!(sMarkup, "BBCODE_TEX" | "BBCODE_ULB" | "MARKDOWN")
        || (bAdministrator && sMarkup == "PLAIN");
    (!bAllowed).then(|| format!("запрещено редактирование текстов в формате {sMarkup}"))
}

fn optAuthorEditReason(
    stTopic: &StTopicEditSnapshot,
    stActor: StTopicEditActor,
    dtNow: DateTime<Utc>,
) -> Option<String> {
    if stTopic.iAuthorId != stActor.iUserId {
        return Some("нельзя править чужие топики".to_owned());
    }
    if stTopic.bDraft {
        return None;
    }
    if stTopic.bCommitted
        && stTopic.bSectionPremoderated
        && stTopic.iSectionId != I_ARTICLES_SECTION
    {
        return Some("в этом разделе нельзя править подтвержденные топики".to_owned());
    }
    if !stTopic.bCommitted && (stTopic.bSticky || stTopic.bSectionPremoderated) {
        return None;
    }
    let dtBase = if stTopic.bCommitted && stTopic.iSectionId == I_ARTICLES_SECTION {
        stTopic.optCommitDate.unwrap_or(stTopic.dtPostDate)
    } else {
        stTopic.dtPostDate
    };
    (dtBase + Duration::days(I_EDIT_PERIOD_DAYS) < dtNow)
        .then_some("истек срок редактирования топика".to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicEditPollValue {
    pub iVariantId: i32,
    pub sLabel: String,
}

#[derive(Debug, Clone)]
pub struct StTopicEditCommand {
    pub iTopicId: i32,
    pub iEditorId: i32,
    pub optTitle: Option<String>,
    pub optMessage: Option<String>,
    pub optUrl: Option<String>,
    pub optLinkText: Option<String>,
    pub optTags: Option<Vec<String>>,
    pub bMinor: bool,
    pub bCommit: bool,
    pub bPublish: bool,
    pub optChangeGroupId: Option<i32>,
    pub iBonus: i32,
    pub vecEditorBonus: Vec<(i32, i32)>,
    pub optPoll: Option<Vec<StTopicEditPollValue>>,
    pub bMultiSelect: bool,
    pub vecPreviewNames: Vec<String>,
    pub sUploadRoot: String,
    pub vecMentionedNicks: Vec<String>,
    pub bSendTagEvents: bool,
    pub bNewMessageDraft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicEditMutationResult {
    pub bModified: bool,
    pub vecNotifiedUserIds: Vec<i32>,
}

pub trait TrTopicEditRealtimeNotifier: Send + Sync {
    fn vNotifyEvents(&self, vecUserIds: &[i32]);
}

#[async_trait]
pub trait TrTopicEditRepository: Send + Sync {
    async fn optSnapshot(&self, iTopicId: i32) -> Result<Option<StTopicEditSnapshot>>;

    async fn stRestrictions(
        &self,
        iUserId: i32,
        sRemoteIp: &str,
    ) -> Result<StTopicEditRestrictions>;

    /// TagService.getNewTags: active canonical tags and synonyms are not new.
    async fn vecNewTags(&self, vecTags: &[String]) -> Result<Vec<String>>;

    async fn stUpdateAndCommit(
        &self,
        stCommand: StTopicEditCommand,
    ) -> Result<StTopicEditMutationResult>;
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn dtNow() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap()
    }

    fn stTopic() -> StTopicEditSnapshot {
        StTopicEditSnapshot {
            iTopicId: 10,
            iAuthorId: 1,
            sAuthorNick: "author".into(),
            iAuthorScore: 100,
            iAuthorMaxScore: 100,
            bAuthorBlocked: false,
            bAuthorAnonymous: false,
            bAuthorFrozen: false,
            sStoredTitle: "title".into(),
            sMessage: "body".into(),
            sMarkup: "MARKDOWN".into(),
            optUrl: None,
            optLinkText: None,
            iGroupId: 2,
            sGroupTitle: "group".into(),
            sGroupUrlName: "group".into(),
            iSectionId: 1,
            sSectionTitle: "Новости".into(),
            sSectionPrefix: "news".into(),
            bSectionPremoderated: true,
            bSectionPollAllowed: false,
            bSectionImagePost: false,
            bSectionImageAllowed: false,
            bLinksAllowed: true,
            bDeleted: false,
            bDraft: false,
            bCommitted: false,
            bSticky: false,
            bExpired: false,
            iPostScore: -9999,
            dtPostDate: dtNow() - Duration::days(1),
            optCommitDate: None,
            dtLastMod: dtNow(),
            bMinor: false,
            vecTags: vec!["tag".into()],
            optPoll: None,
            vecGroups: Vec::new(),
            optLastEditMillis: None,
            vecEditors: Vec::new(),
        }
    }

    fn stActor(iUserId: i32) -> StTopicEditActor {
        StTopicEditActor {
            iUserId,
            iScore: 100,
            bModerator: false,
            bAdministrator: false,
            bCorrector: false,
            bBlocked: false,
        }
    }

    fn stRestrictions() -> StTopicEditRestrictions {
        StTopicEditRestrictions {
            bFrozen: false,
            stIpBlock: StIpBlockInfo::default(),
        }
    }

    #[test]
    fn author_and_role_order_matches_edit_topic_checker() {
        let stTopic = stTopic();
        assert!(stCheckContentEdit(&stTopic, stActor(1), stRestrictions(), dtNow()).bPermitted());
        assert_eq!(
            stCheckContentEdit(&stTopic, stActor(2), stRestrictions(), dtNow()).sReason(),
            "нельзя править чужие топики"
        );
        let mut stModerator = stActor(2);
        stModerator.bModerator = true;
        assert!(stCheckContentEdit(&stTopic, stModerator, stRestrictions(), dtNow()).bPermitted());
    }

    #[test]
    fn precheck_cannot_be_cleared_by_administrator_or_moderator() {
        let mut stTopic = stTopic();
        stTopic.bDeleted = true;
        let mut stAdministrator = stActor(2);
        stAdministrator.bAdministrator = true;
        stAdministrator.bModerator = true;
        assert_eq!(
            stCheckContentEdit(&stTopic, stAdministrator, stRestrictions(), dtNow()).sReason(),
            "нельзя править удаленные топики"
        );
    }

    #[test]
    fn corrector_may_edit_tags_but_no_comments_blocks_content() {
        let mut stTopic = stTopic();
        stTopic.iPostScore = POSTSCORE_NO_COMMENTS;
        let mut stCorrector = stActor(2);
        stCorrector.bCorrector = true;
        assert!(!stCheckContentEdit(&stTopic, stCorrector, stRestrictions(), dtNow()).bPermitted());
        assert!(stCheckTagsEdit(&stTopic, stCorrector, stRestrictions(), dtNow()).bPermitted());
    }

    #[test]
    fn corrector_flag_blocks_own_commit_even_for_moderator_session() {
        let stTopic = stTopic();
        let mut stActor = stActor(1);
        stActor.bCorrector = true;
        stActor.bModerator = true;
        assert_eq!(
            stCheckCommit(&stTopic, stActor, stRestrictions()).sReason(),
            "нельзя подтверждать собственные топики"
        );
    }

    #[test]
    fn committed_article_deadline_is_anchored_on_commit_date() {
        let mut stTopic = stTopic();
        stTopic.iSectionId = I_ARTICLES_SECTION;
        stTopic.bCommitted = true;
        stTopic.dtPostDate = dtNow() - Duration::days(60);
        stTopic.optCommitDate = Some(dtNow() - Duration::days(1));
        assert!(stCheckContentEdit(&stTopic, stActor(1), stRestrictions(), dtNow()).bPermitted());
    }
}
