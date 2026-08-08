use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StModerationUser {
    pub iId: i32,
    pub sNick: String,
    pub bModerator: bool,
    pub bAdministrator: bool,
    pub bAnonymous: bool,
    pub bCorrector: bool,
    pub bBlocked: bool,
    pub iScore: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnUserModerationMutation {
    Block {
        iTargetUserId: i32,
        iModeratorId: i32,
        sReason: String,
    },
    Unblock {
        iTargetUserId: i32,
        iModeratorId: i32,
    },
    Score50 {
        iTargetUserId: i32,
        iModeratorId: i32,
    },
    SetCorrector {
        iTargetUserId: i32,
        iModeratorId: i32,
        bCorrector: bool,
    },
    ResetPassword {
        iTargetUserId: i32,
        iModeratorId: i32,
        sPasswordHash: String,
    },
    ResetUserpic {
        iTargetUserId: i32,
        iActorUserId: i32,
        bScorePenalty: bool,
    },
    RemoveUserInfo {
        iTargetUserId: i32,
        iModeratorId: i32,
    },
    RemoveTown {
        iTargetUserId: i32,
        iModeratorId: i32,
    },
    RemoveUrl {
        iTargetUserId: i32,
        iModeratorId: i32,
    },
    Freeze {
        iTargetUserId: i32,
        iModeratorId: i32,
        sReason: String,
        dtUntil: DateTime<Utc>,
        bDefrost: bool,
    },
    BlockAndDelete {
        iTargetUserId: i32,
        iModeratorId: i32,
        sReason: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StMassDeleteResult {
    pub vecTopicIds: Vec<i32>,
    pub vecCommentIds: Vec<i32>,
    pub vecSkippedCommentIds: Vec<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StUserModerationMutationResult {
    pub optMassDelete: Option<StMassDeleteResult>,
}
