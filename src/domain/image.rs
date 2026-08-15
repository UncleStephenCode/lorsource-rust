use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

use crate::{
    domain::topic::posting::{POSTSCORE_NO_COMMENTS, StIpBlockInfo, USER_ANONYMOUS_LEVEL_SCORE},
    error::Result,
};

const I_ARTICLES_SECTION: i32 = 6;
const I_EDIT_PERIOD_DAYS: i64 = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StImageReference {
    pub iId: i32,
    pub sExtension: String,
}

#[derive(Debug, Clone)]
pub struct StImageDeleteTarget {
    pub iImageId: i32,
    pub iTopicId: i32,
    pub sImageExtension: String,
    pub iAuthorId: i32,
    pub sTopicTitle: String,
    pub bTopicDeleted: bool,
    pub bDraft: bool,
    pub bCommitted: bool,
    pub bSticky: bool,
    pub bExpired: bool,
    pub iPostScore: i32,
    pub dtPostDate: DateTime<Utc>,
    pub optCommitDate: Option<DateTime<Utc>>,
    pub dtLastMod: DateTime<Utc>,
    pub iSectionId: i32,
    pub bSectionPremoderated: bool,
    pub bSectionImagePost: bool,
    pub sSectionPrefix: String,
    pub sGroupUrlName: String,
    pub sMarkup: String,
    pub vecActiveImages: Vec<StImageReference>,
}

impl StImageDeleteTarget {
    pub fn sTopicUrl(&self) -> String {
        format!(
            "/{}/{}/{}",
            self.sSectionPrefix, self.sGroupUrlName, self.iTopicId
        )
    }

    /// `TopicLinkBuilder.baseLink(topic).forceLastmod.build` uses the topic
    /// object loaded before `ImageService.deleteImage` updates `lastmod`.
    pub fn sForceLastModUrl(&self) -> String {
        format!(
            "{}?lastmod={}",
            self.sTopicUrl(),
            self.dtLastMod.timestamp_millis()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StImageDeleteActor {
    pub iUserId: i32,
    pub iScore: i32,
    pub bModerator: bool,
    pub bAdministrator: bool,
    pub bCorrector: bool,
    pub bBlocked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StImageDeleteRestrictions {
    pub bFrozen: bool,
    pub stIpBlock: StIpBlockInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StImageDeletePermission {
    pub optReason: Option<&'static str>,
}

impl StImageDeletePermission {
    pub fn bPermitted(&self) -> bool {
        self.optReason.is_none()
    }
}

/// Exact ordering of `EditTopicChecker.checkContentEdit`, followed by
/// `DeleteImageController.checkDelete`'s image-post rule.  The ordering is
/// observable because a restriction before a `permit` still applies, while a
/// restriction after it does not.
pub fn stCheckImageDelete(
    stTarget: &StImageDeleteTarget,
    stActor: StImageDeleteActor,
    stRestrictions: StImageDeleteRestrictions,
    iPreparedImageCount: usize,
    dtNow: DateTime<Utc>,
) -> StImageDeletePermission {
    let optReason = optContentEditReason(stTarget, stActor, stRestrictions, dtNow).or_else(|| {
        (stTarget.bSectionImagePost && iPreparedImageCount <= 1)
            .then_some("В этом разделе нельзя удалить единственное изображение")
    });
    StImageDeletePermission { optReason }
}

fn optContentEditReason(
    stTarget: &StImageDeleteTarget,
    stActor: StImageDeleteActor,
    stRestrictions: StImageDeleteRestrictions,
    dtNow: DateTime<Utc>,
) -> Option<&'static str> {
    if stTarget.bTopicDeleted {
        return Some("нельзя править удаленные топики");
    }
    if stRestrictions.bFrozen {
        return Some("установлен режим только для чтения");
    }
    if let Some(sReason) = optIpBlockReason(stActor, stRestrictions.stIpBlock) {
        return Some(sReason);
    }
    if !bLegacyMarkupEditable(&stTarget.sMarkup, stActor.bAdministrator) {
        return Some("запрещено редактирование текста в этом формате");
    }
    if stActor.bAdministrator {
        return None;
    }
    if stTarget.bExpired && !stTarget.bDraft {
        return Some("нельзя править архивные топики");
    }
    if stActor.bModerator {
        return None;
    }
    if stTarget.iPostScore == POSTSCORE_NO_COMMENTS {
        return Some("нельзя править топики с выключенными комментариями");
    }
    if stActor.bCorrector && stTarget.bSectionPremoderated {
        return None;
    }
    optAuthorEditReason(stTarget, stActor, dtNow)
}

fn optAuthorEditReason(
    stTarget: &StImageDeleteTarget,
    stActor: StImageDeleteActor,
    dtNow: DateTime<Utc>,
) -> Option<&'static str> {
    if stTarget.iAuthorId != stActor.iUserId {
        return Some("нельзя править чужие топики");
    }
    if stTarget.bDraft {
        return None;
    }
    if stTarget.bCommitted
        && stTarget.bSectionPremoderated
        && stTarget.iSectionId != I_ARTICLES_SECTION
    {
        return Some("в этом разделе нельзя править подтвержденные топики");
    }
    if !stTarget.bCommitted && (stTarget.bSticky || stTarget.bSectionPremoderated) {
        return None;
    }
    let dtBase = if stTarget.bCommitted && stTarget.iSectionId == I_ARTICLES_SECTION {
        stTarget.optCommitDate.unwrap_or(stTarget.dtPostDate)
    } else {
        stTarget.dtPostDate
    };
    (dtBase + Duration::days(I_EDIT_PERIOD_DAYS) < dtNow)
        .then_some("истек срок редактирования топика")
}

fn bLegacyMarkupEditable(sMarkup: &str, bAdministrator: bool) -> bool {
    matches!(sMarkup, "BBCODE_TEX" | "BBCODE_ULB" | "MARKDOWN")
        || (bAdministrator && sMarkup == "PLAIN")
}

fn optIpBlockReason(stActor: StImageDeleteActor, stIpBlock: StIpBlockInfo) -> Option<&'static str> {
    if !stIpBlock.bBlocked {
        None
    } else if stActor.bBlocked || stActor.iScore < USER_ANONYMOUS_LEVEL_SCORE {
        Some("постинг с этого IP адреса ограничен для пользователей с score < 50")
    } else if !stIpBlock.bAllowRegisteredPosting {
        Some("постинг с этого IP адреса заблокирован")
    } else {
        None
    }
}

#[async_trait]
pub trait TrImageDeleteRepository: Send + Sync {
    async fn optTarget(&self, iImageId: i32) -> Result<Option<StImageDeleteTarget>>;

    async fn stRestrictions(
        &self,
        iUserId: i32,
        sRemoteIp: &str,
    ) -> Result<StImageDeleteRestrictions>;

    async fn vDelete(&self, iImageId: i32, iTopicId: i32, iEditorId: i32) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn dtNow() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap()
    }

    fn stTarget() -> StImageDeleteTarget {
        StImageDeleteTarget {
            iImageId: 11,
            iTopicId: 10,
            sImageExtension: "png".into(),
            iAuthorId: 1,
            sTopicTitle: "topic".into(),
            bTopicDeleted: false,
            bDraft: false,
            bCommitted: false,
            bSticky: false,
            bExpired: false,
            iPostScore: -9999,
            dtPostDate: dtNow() - Duration::days(1),
            optCommitDate: None,
            dtLastMod: dtNow(),
            iSectionId: 2,
            bSectionPremoderated: false,
            bSectionImagePost: false,
            sSectionPrefix: "forum".into(),
            sGroupUrlName: "general".into(),
            sMarkup: "MARKDOWN".into(),
            vecActiveImages: vec![
                StImageReference {
                    iId: 11,
                    sExtension: "png".into(),
                },
                StImageReference {
                    iId: 12,
                    sExtension: "png".into(),
                },
            ],
        }
    }

    fn stActor(iUserId: i32) -> StImageDeleteActor {
        StImageDeleteActor {
            iUserId,
            iScore: 100,
            bModerator: false,
            bAdministrator: false,
            bCorrector: false,
            bBlocked: false,
        }
    }

    fn stRestrictions() -> StImageDeleteRestrictions {
        StImageDeleteRestrictions {
            bFrozen: false,
            stIpBlock: StIpBlockInfo::default(),
        }
    }

    fn optReason(
        stTarget: &StImageDeleteTarget,
        stActor: StImageDeleteActor,
        stRestrictions: StImageDeleteRestrictions,
        iPreparedImageCount: usize,
    ) -> Option<&'static str> {
        stCheckImageDelete(
            stTarget,
            stActor,
            stRestrictions,
            iPreparedImageCount,
            dtNow(),
        )
        .optReason
    }

    #[test]
    fn author_window_and_committed_articles_use_java_deadlines() {
        let mut stTarget = stTarget();
        assert_eq!(optReason(&stTarget, stActor(1), stRestrictions(), 2), None);
        stTarget.dtPostDate = dtNow() - Duration::days(15);
        assert_eq!(
            optReason(&stTarget, stActor(1), stRestrictions(), 2),
            Some("истек срок редактирования топика")
        );

        stTarget.iSectionId = I_ARTICLES_SECTION;
        stTarget.bSectionPremoderated = true;
        stTarget.bCommitted = true;
        stTarget.optCommitDate = Some(dtNow() - Duration::days(1));
        assert_eq!(optReason(&stTarget, stActor(1), stRestrictions(), 2), None);
    }

    #[test]
    fn administrator_and_moderator_permits_remain_at_the_java_chain_positions() {
        let mut stImageTarget = stTarget();
        stImageTarget.bExpired = true;
        stImageTarget.iPostScore = POSTSCORE_NO_COMMENTS;
        let mut stModerator = stActor(2);
        stModerator.bModerator = true;
        assert_eq!(
            optReason(&stImageTarget, stModerator, stRestrictions(), 2),
            Some("нельзя править архивные топики")
        );
        stModerator.bAdministrator = true;
        assert_eq!(
            optReason(&stImageTarget, stModerator, stRestrictions(), 2),
            None
        );

        let mut stPlain = stTarget();
        stPlain.sMarkup = "PLAIN".into();
        assert!(optReason(&stPlain, stActor(1), stRestrictions(), 2).is_some());
        let mut stAdministrator = stActor(2);
        stAdministrator.bAdministrator = true;
        assert_eq!(
            optReason(&stPlain, stAdministrator, stRestrictions(), 2),
            None
        );
    }

    #[test]
    fn corrector_postscore_and_author_premoderation_order_matches_java() {
        let mut stTarget = stTarget();
        stTarget.bSectionPremoderated = true;
        let mut stCorrector = stActor(2);
        stCorrector.bCorrector = true;
        assert_eq!(optReason(&stTarget, stCorrector, stRestrictions(), 2), None);
        stTarget.iPostScore = POSTSCORE_NO_COMMENTS;
        assert_eq!(
            optReason(&stTarget, stCorrector, stRestrictions(), 2),
            Some("нельзя править топики с выключенными комментариями")
        );

        stTarget.iPostScore = -9999;
        stTarget.bCommitted = true;
        assert_eq!(
            optReason(&stTarget, stActor(1), stRestrictions(), 2),
            Some("в этом разделе нельзя править подтвержденные топики")
        );
    }

    #[test]
    fn frozen_ip_and_gallery_single_image_restrictions_apply_to_privileged_users() {
        let mut stAdministrator = stActor(2);
        stAdministrator.bAdministrator = true;
        let mut stFrozen = stRestrictions();
        stFrozen.bFrozen = true;
        assert!(optReason(&stTarget(), stAdministrator, stFrozen, 2).is_some());

        let mut stBlockedIp = stRestrictions();
        stBlockedIp.stIpBlock = StIpBlockInfo {
            bBlocked: true,
            bAllowRegisteredPosting: false,
        };
        assert!(optReason(&stTarget(), stAdministrator, stBlockedIp, 2).is_some());

        let mut stGallery = stTarget();
        stGallery.bSectionImagePost = true;
        assert_eq!(
            optReason(&stGallery, stActor(1), stRestrictions(), 1),
            Some("В этом разделе нельзя удалить единственное изображение")
        );
        assert_eq!(optReason(&stGallery, stActor(1), stRestrictions(), 2), None);
    }

    #[test]
    fn force_lastmod_uses_the_loaded_topic_and_real_section() {
        assert_eq!(
            stTarget().sForceLastModUrl(),
            "/forum/general/10?lastmod=1786795200000"
        );
    }
}
