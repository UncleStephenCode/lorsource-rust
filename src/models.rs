//! Backward-compatible model facade.
//!
//! Domain structs live under `crate::domain::*::model` and use Hungarian-style
//! names (`StTopicSummary`, `StUserSummary`, ...).  These type aliases keep the
//! existing route/templates surface stable while the port is refactored module by
//! module.

pub type Group = crate::domain::forum::model::StGroup;
pub type UserSummary = crate::domain::user::model::StUserSummary;
pub type TopicSummary = crate::domain::topic::model::StTopicSummary;
pub type TopicDetail = crate::domain::topic::model::StTopicDetail;
pub type CommentItem = crate::domain::comment::model::StCommentItem;
pub type TagItem = crate::domain::tag::model::StTagItem;
pub type PagerQuery = crate::domain::common::model::StPagerQuery;
