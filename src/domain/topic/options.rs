use async_trait::async_trait;

use crate::error::Result;

pub const POSTSCORE_UNRESTRICTED: i32 = -9999;
pub const POSTSCORE_REGISTERED_ONLY: i32 = -50;
pub const POSTSCORE_MOD_AUTHOR: i32 = 9999;
pub const POSTSCORE_MODERATORS_ONLY: i32 = 10000;
pub const POSTSCORE_NO_COMMENTS: i32 = 10001;
pub const POSTSCORE_HIDE_COMMENTS: i32 = 10002;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTopicOptions {
    pub iTopicId: i32,
    pub iPostScore: i32,
    pub bSticky: bool,
    pub bNoTop: bool,
    pub bPremoderated: bool,
    pub sCanonicalUrl: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StSetTopicOptions {
    pub iTopicId: i32,
    pub iPostScore: i32,
    pub bSticky: bool,
    pub bNoTop: bool,
}

/// TopicModificationController accepts one sentinel plus one continuous
/// interval.  The values offered by the JSP are deliberately not a whitelist.
pub fn bValidPostScore(iPostScore: i32) -> bool {
    iPostScore == POSTSCORE_UNRESTRICTED
        || (POSTSCORE_REGISTERED_ONLY..=POSTSCORE_HIDE_COMMENTS).contains(&iPostScore)
}

/// `TopicPermissionService.getPostScoreInfo`: the topic card deliberately
/// renders no text for the unrestricted sentinel.  The options form uses the
/// `Full` wrapper below to label that otherwise-empty choice.
pub fn sPostScoreInfo(iPostScore: i32) -> String {
    match iPostScore {
        POSTSCORE_UNRESTRICTED => String::new(),
        50 => "Закрыто добавление комментариев для недавно зарегистрированных пользователей (со score < 50)"
            .to_owned(),
        100 | 200 | 300 | 400 | 500 => format!(
            "<b>Ограничение на отправку комментариев</b>: <span class=\"stars\">{}</span>",
            "★".repeat((iPostScore / 100) as usize)
        ),
        POSTSCORE_MOD_AUTHOR =>
            "<b>Ограничение на отправку комментариев</b>: только для модераторов и автора".to_owned(),
        POSTSCORE_MODERATORS_ONLY =>
            "<b>Ограничение на отправку комментариев</b>: только для модераторов".to_owned(),
        POSTSCORE_NO_COMMENTS =>
            "<b>Ограничение на отправку комментариев</b>: комментарии запрещены".to_owned(),
        POSTSCORE_HIDE_COMMENTS =>
            "<b>Ограничение на отправку комментариев</b>: без комментариев".to_owned(),
        POSTSCORE_REGISTERED_ONLY =>
            "<b>Ограничение на отправку комментариев</b>: только для зарегистрированных пользователей".to_owned(),
        _ => format!(
            "<b>Ограничение на отправку комментариев</b>: только для зарегистрированных пользователей, score>={iPostScore}"
        ),
    }
}

pub fn sPostScoreInfoFull(iPostScore: i32) -> String {
    let sInfo = sPostScoreInfo(iPostScore);
    if sInfo.is_empty() {
        "без ограничений".to_owned()
    } else {
        sInfo
    }
}

#[async_trait]
pub trait TrTopicOptionsRepository: Send + Sync {
    async fn optFind(&self, iTopicId: i32) -> Result<Option<StTopicOptions>>;

    /// TopicDao.setTopicOptions performs one unconditional update inside its
    /// local transaction. The controller's comparison snapshot is loaded
    /// separately before that transaction.
    async fn vSet(&self, stOptions: StSetTopicOptions) -> Result<()>;
}

#[async_trait]
pub trait TrTopicReindexQueue: Send + Sync {
    async fn vUpdateMessage(&self, iTopicId: i32, bWithComments: bool) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postscore_range_is_the_java_sentinel_plus_continuous_interval() {
        for iValue in [-9999, -50, -49, 0, 45, 1234, 9998, 10002] {
            assert!(bValidPostScore(iValue), "{iValue}");
        }
        for iValue in [-10000, -9998, -51, 10003, i32::MAX] {
            assert!(!bValidPostScore(iValue), "{iValue}");
        }
    }

    #[test]
    fn postscore_messages_match_topic_permission_service() {
        assert!(sPostScoreInfo(-9999).is_empty());
        assert_eq!(sPostScoreInfoFull(-9999), "без ограничений");
        assert_eq!(
            sPostScoreInfoFull(200),
            "<b>Ограничение на отправку комментариев</b>: <span class=\"stars\">★★</span>"
        );
        assert!(sPostScoreInfoFull(1234).ends_with("score>=1234"));
    }
}
