//! Backward-compatible original-schema model facade.
//! Actual structs live in `crate::domain::compat::model` with Hungarian `St*` names.

pub type OriginalUser = crate::domain::compat::model::StOriginalUser;
pub type OriginalSection = crate::domain::compat::model::StOriginalSection;
pub type OriginalGroup = crate::domain::compat::model::StOriginalGroup;
pub type OriginalTopic = crate::domain::compat::model::StOriginalTopic;
pub type OriginalComment = crate::domain::compat::model::StOriginalComment;
pub type MessageTextRow = crate::domain::compat::model::StMessageTextRow;
pub type TagValueRow = crate::domain::compat::model::StTagValueRow;
pub type TopicTagRow = crate::domain::compat::model::StTopicTagRow;
pub type PollRow = crate::domain::compat::model::StPollRow;
pub type PollVariantRow = crate::domain::compat::model::StPollVariantRow;
pub type VoteUserRow = crate::domain::compat::model::StVoteUserRow;
pub type EditInfoRow = crate::domain::compat::model::StEditInfoRow;
pub type DeleteInfoRow = crate::domain::compat::model::StDeleteInfoRow;
pub type MemoryRow = crate::domain::compat::model::StMemoryRow;
pub type IgnoreListRow = crate::domain::compat::model::StIgnoreListRow;
pub type UserAgentRow = crate::domain::compat::model::StUserAgentRow;
pub type BanInfoRow = crate::domain::compat::model::StBanInfoRow;
pub type UserEventRow = crate::domain::compat::model::StUserEventRow;
pub type ImageRow = crate::domain::compat::model::StImageRow;
pub type ReactionLogRow = crate::domain::compat::model::StReactionLogRow;
pub type WarningRow = crate::domain::compat::model::StWarningRow;
pub type UserSettingsRow = crate::domain::compat::model::StUserSettingsRow;
pub type UserLogRow = crate::domain::compat::model::StUserLogRow;
pub type VoteNameRow = PollRow;
pub type VoteVariantRow = PollVariantRow;
