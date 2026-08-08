use async_trait::async_trait;

use crate::error::Result;

pub const POSTSCORE_UNRESTRICTED: i32 = -9999;
pub const POSTSCORE_REGISTERED_ONLY: i32 = -50;
pub const POSTSCORE_MODERATORS_ONLY: i32 = 10000;
pub const POSTSCORE_NO_COMMENTS: i32 = 10001;
pub const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
pub const USER_ANONYMOUS_LEVEL_SCORE: i32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StAddTopicActor {
    pub optUserId: Option<i32>,
    pub bAnonymous: bool,
    pub bModerator: bool,
    pub bCorrector: bool,
    pub bBlocked: bool,
    pub iScore: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StIpBlockInfo {
    pub bBlocked: bool,
    pub bAllowRegisteredPosting: bool,
}

impl Default for StIpBlockInfo {
    fn default() -> Self {
        Self {
            bBlocked: false,
            // IpBlockInfo.isAllowRegisteredPosting is true for a missing or
            // expired block (`!isBlocked || allowPosting`).
            bAllowRegisteredPosting: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StAddTopicPermission {
    pub optReason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StTopicLimitInfo {
    pub iLimit: i32,
    pub iCurrentCount: i32,
    pub bReached: bool,
    pub bExempt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StSlowModeInfo {
    pub bCurrentlyFrozen: bool,
    pub bFrozenWithinThreeDays: bool,
    pub iRecentScoreLoss: i32,
}

impl StAddTopicPermission {
    pub fn bPermitted(&self) -> bool {
        self.optReason.is_none()
    }

    pub fn sReason(&self) -> &str {
        self.optReason.as_deref().unwrap_or("")
    }
}

/// Pure policy equivalent of the Java `AddTopicChecker` restriction chain.
/// The order matters: `RestrictionChain` preserves the first failed check.
pub fn stCheckAddTopic(
    stActor: StAddTopicActor,
    bFrozen: bool,
    stIpBlock: StIpBlockInfo,
    iRestriction: i32,
) -> StAddTopicPermission {
    let optReason = optFrozenReason(stActor, bFrozen)
        .or_else(|| optPostScoreReason(stActor, iRestriction))
        .or_else(|| optIpBlockReason(stActor, stIpBlock));
    StAddTopicPermission { optReason }
}

/// `GroupPermissionService.topicLimit`: at least two topics per 24 hours,
/// then one additional slot per full green star, capped by User.getGreenStars
/// at five.
pub fn iTopicDailyLimit(iScore: i32) -> i32 {
    let iClampedScore = iScore.clamp(0, 599);
    (iClampedScore / 100).max(2)
}

pub fn bSlowModeRestricted(stActor: StAddTopicActor, stInfo: StSlowModeInfo) -> bool {
    if stActor.bAnonymous || stActor.bBlocked || stInfo.bCurrentlyFrozen {
        false
    } else {
        stActor.iScore < 35 || stInfo.bFrozenWithinThreeDays || stInfo.iRecentScoreLoss >= 30
    }
}

pub fn stCheckTopicPublish(
    stAddPermission: StAddTopicPermission,
    stLimitInfo: StTopicLimitInfo,
) -> StAddTopicPermission {
    if !stAddPermission.bPermitted() {
        stAddPermission
    } else if !stLimitInfo.bExempt && stLimitInfo.bReached {
        StAddTopicPermission {
            optReason: Some("превышен лимит числа топиков в сутки".into()),
        }
    } else {
        StAddTopicPermission { optReason: None }
    }
}

fn optFrozenReason(stActor: StAddTopicActor, bFrozen: bool) -> Option<String> {
    if stActor.bAnonymous && bFrozen {
        Some("только для зарегистрированных".into())
    } else if bFrozen {
        Some("установлен режим только для чтения".into())
    } else {
        None
    }
}

fn optPostScoreReason(stActor: StAddTopicActor, iRestriction: i32) -> Option<String> {
    match iRestriction {
        POSTSCORE_UNRESTRICTED => None,
        POSTSCORE_MODERATORS_ONLY if !stActor.bModerator => Some("только для модераторов".into()),
        POSTSCORE_MODERATORS_ONLY => None,
        POSTSCORE_REGISTERED_ONLY if stActor.bAnonymous => {
            Some("только для зарегистрированных".into())
        }
        POSTSCORE_REGISTERED_ONLY => None,
        POSTSCORE_NO_COMMENTS | POSTSCORE_HIDE_COMMENTS => Some("постинг запрещен".into()),
        100 | 200 | 300 | 400 if stActor.bAnonymous || stActor.iScore < iRestriction => {
            Some(format!(
                "только для зарегистрированных, минимум {}",
                "★".repeat((iRestriction / 100) as usize)
            ))
        }
        500 if stActor.bAnonymous || stActor.iScore < iRestriction => {
            Some(format!("только для зарегистрированных, {}", "★".repeat(5)))
        }
        restriction if stActor.bAnonymous || stActor.iScore < restriction => Some(format!(
            "только для зарегистрированных, score>={restriction}"
        )),
        _ => None,
    }
}

fn optIpBlockReason(stActor: StAddTopicActor, stIpBlock: StIpBlockInfo) -> Option<String> {
    if stIpBlock.bBlocked && stActor.bAnonymous {
        Some("анонимный постинг с этого IP адреса заблокирован".into())
    } else if stIpBlock.bBlocked
        && (stActor.bAnonymous || stActor.bBlocked || stActor.iScore < USER_ANONYMOUS_LEVEL_SCORE)
    {
        Some(format!(
            "постинг с этого IP адреса ограничен для пользователей с score < {USER_ANONYMOUS_LEVEL_SCORE}"
        ))
    } else if stIpBlock.bBlocked && !stActor.bAnonymous && !stIpBlock.bAllowRegisteredPosting {
        Some("постинг с этого IP адреса заблокирован".into())
    } else {
        None
    }
}

#[async_trait]
pub trait TrAddTopicRepository: Send + Sync {
    async fn optGroupTopicRestriction(&self, iGroupId: i32) -> Result<Option<i32>>;
    async fn bIsUserFrozen(&self, iUserId: i32) -> Result<bool>;
    async fn stIpBlockInfo(&self, sIp: &str) -> Result<StIpBlockInfo>;
    async fn iCountRecentTopics(&self, iUserId: i32, iSectionId: i32) -> Result<i32>;
    async fn stSlowModeInfo(&self, iUserId: i32) -> Result<StSlowModeInfo>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stUser(iScore: i32) -> StAddTopicActor {
        StAddTopicActor {
            optUserId: Some(10),
            bAnonymous: false,
            bModerator: false,
            bCorrector: false,
            bBlocked: false,
            iScore,
        }
    }

    #[test]
    fn mirrors_java_postscore_special_values_and_star_messages() {
        assert!(stCheckAddTopic(stUser(0), false, Default::default(), -9999).bPermitted());
        assert_eq!(
            stCheckAddTopic(stUser(99), false, Default::default(), 100).sReason(),
            "только для зарегистрированных, минимум ★"
        );
        assert_eq!(
            stCheckAddTopic(stUser(499), false, Default::default(), 500).sReason(),
            "только для зарегистрированных, ★★★★★"
        );
        assert_eq!(
            stCheckAddTopic(stUser(100), false, Default::default(), 10001).sReason(),
            "постинг запрещен"
        );
    }

    #[test]
    fn moderator_only_and_registered_only_match_java() {
        let mut stModerator = stUser(0);
        stModerator.bModerator = true;
        assert!(stCheckAddTopic(stModerator, false, Default::default(), 10000).bPermitted());
        assert_eq!(
            stCheckAddTopic(stUser(1000), false, Default::default(), 10000).sReason(),
            "только для модераторов"
        );
        let stAnonymous = StAddTopicActor {
            optUserId: None,
            bAnonymous: true,
            bModerator: false,
            bCorrector: false,
            bBlocked: false,
            iScore: 0,
        };
        assert_eq!(
            stCheckAddTopic(stAnonymous, false, Default::default(), -50).sReason(),
            "только для зарегистрированных"
        );
    }

    #[test]
    fn frozen_restriction_wins_before_score_and_ip_checks() {
        let stIpBlock = StIpBlockInfo {
            bBlocked: true,
            bAllowRegisteredPosting: false,
        };
        assert_eq!(
            stCheckAddTopic(stUser(-100), true, stIpBlock, 10000).sReason(),
            "установлен режим только для чтения"
        );
    }

    #[test]
    fn ip_block_respects_low_score_and_allow_registered_posting() {
        let stStrictBlock = StIpBlockInfo {
            bBlocked: true,
            bAllowRegisteredPosting: false,
        };
        let stRegisteredAllowed = StIpBlockInfo {
            bBlocked: true,
            bAllowRegisteredPosting: true,
        };
        assert_eq!(
            stCheckAddTopic(stUser(49), false, stRegisteredAllowed, -9999).sReason(),
            "постинг с этого IP адреса ограничен для пользователей с score < 50"
        );
        assert_eq!(
            stCheckAddTopic(stUser(50), false, stStrictBlock, -9999).sReason(),
            "постинг с этого IP адреса заблокирован"
        );
        assert!(stCheckAddTopic(stUser(50), false, stRegisteredAllowed, -9999).bPermitted());
    }

    #[test]
    fn daily_limit_and_publish_check_match_java_green_stars() {
        assert_eq!(iTopicDailyLimit(-100), 2);
        assert_eq!(iTopicDailyLimit(0), 2);
        assert_eq!(iTopicDailyLimit(299), 2);
        assert_eq!(iTopicDailyLimit(300), 3);
        assert_eq!(iTopicDailyLimit(500), 5);
        assert_eq!(iTopicDailyLimit(9999), 5);

        let stLimit = StTopicLimitInfo {
            iLimit: 2,
            iCurrentCount: 2,
            bReached: true,
            bExempt: false,
        };
        assert_eq!(
            stCheckTopicPublish(StAddTopicPermission { optReason: None }, stLimit).sReason(),
            "превышен лимит числа топиков в сутки"
        );
    }

    #[test]
    fn slow_mode_uses_score_recent_freeze_and_three_day_score_loss() {
        assert!(bSlowModeRestricted(stUser(34), StSlowModeInfo::default()));
        assert!(bSlowModeRestricted(
            stUser(100),
            StSlowModeInfo {
                bCurrentlyFrozen: false,
                bFrozenWithinThreeDays: true,
                iRecentScoreLoss: 0,
            }
        ));
        assert!(bSlowModeRestricted(
            stUser(100),
            StSlowModeInfo {
                bCurrentlyFrozen: false,
                bFrozenWithinThreeDays: false,
                iRecentScoreLoss: 30,
            }
        ));
        assert!(!bSlowModeRestricted(stUser(100), StSlowModeInfo::default()));
        assert!(!bSlowModeRestricted(
            stUser(10),
            StSlowModeInfo {
                bCurrentlyFrozen: true,
                bFrozenWithinThreeDays: true,
                iRecentScoreLoss: 100,
            }
        ));
    }
}
