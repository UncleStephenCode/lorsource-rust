use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

use crate::error::Result;

pub const VEC_DELETE_REASONS: &[&str] = &[
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
pub struct StCommentDeleteActor {
    pub iUserId: i32,
    pub bModerator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StCommentDeleteTarget {
    pub iCommentId: i32,
    pub iTopicId: i32,
    pub iAuthorId: i32,
    pub sAuthorNick: String,
    pub iAuthorScore: i32,
    pub bDeleted: bool,
    pub bTopicDeleted: bool,
    pub bTopicExpired: bool,
    pub bTopicDraft: bool,
    pub bCommentsHidden: bool,
    pub bHasReplies: bool,
    pub dtPostdate: DateTime<Utc>,
    pub optDeletedBy: Option<i32>,
    pub sPostIp: String,
    pub iUserAgentId: i32,
    pub sCanonicalTopicUrl: String,
}

impl StCommentDeleteTarget {
    pub fn bCanDelete(&self, stActor: StCommentDeleteActor, dtNow: DateTime<Utc>) -> bool {
        if self.bDeleted || self.bTopicDeleted {
            return false;
        }
        stActor.bModerator
            || (stActor.iUserId == self.iAuthorId
                && !self.bTopicExpired
                && !self.bHasReplies
                && self.dtPostdate + Duration::hours(3) > dtNow)
    }

    pub fn bCanUndelete(&self, stActor: StCommentDeleteActor) -> bool {
        stActor.bModerator
            && self.bDeleted
            && !self.bTopicDeleted
            && !self.bTopicExpired
            && self.optDeletedBy != Some(self.iAuthorId)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StDeleteCommentCommand {
    pub iCommentId: i32,
    pub sReason: String,
    pub iPenalty: i32,
    pub bDeleteReplies: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StCommentDeleteMutation {
    pub vecDeletedIds: Vec<i32>,
    pub optNextCommentId: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StCommentDeletePreview {
    pub iCommentId: i32,
    pub iAuthorId: i32,
    pub iTopicAuthorId: i32,
    pub bDeleted: bool,
    pub optDeletedById: Option<i32>,
    pub optDeletedByNick: Option<String>,
    pub optDeleteReason: Option<String>,
    pub optReplyTo: Option<i32>,
    pub bReplyDeleted: bool,
    pub optReplyTitle: Option<String>,
    pub optReplyAuthor: Option<String>,
    pub optReplyPostdate: Option<DateTime<Utc>>,
    pub iDepth: i32,
    pub sTitle: String,
    pub sMessage: String,
    pub sMarkup: String,
    pub dtPostdate: DateTime<Utc>,
    pub sAuthorNick: String,
    pub iAuthorScore: i32,
    pub iAuthorMaxScore: i32,
    pub bAuthorBlocked: bool,
    pub bAuthorAnonymous: bool,
    pub bAuthorFrozen: bool,
    pub optPhoto: Option<String>,
    pub optEmail: Option<String>,
    pub optRemark: Option<String>,
    pub iEditCount: i32,
    pub optEditDate: Option<DateTime<Utc>>,
    pub optEditorNick: Option<String>,
    pub sPostIp: String,
    pub iUserAgentId: i32,
    pub optUserAgent: Option<String>,
    pub sReactionsJson: String,
    pub sWarningsJson: String,
}

#[async_trait]
pub trait TrCommentDeletionRepository: Send + Sync {
    async fn optFindTarget(&self, iCommentId: i32) -> Result<Option<StCommentDeleteTarget>>;
    async fn vecDeletePreview(
        &self,
        iCommentId: i32,
        iViewerId: i32,
    ) -> Result<Vec<StCommentDeletePreview>>;
    async fn vecUndeletePreview(
        &self,
        iCommentId: i32,
        iViewerId: i32,
    ) -> Result<Vec<StCommentDeletePreview>>;
    async fn stDelete(
        &self,
        stActor: StCommentDeleteActor,
        stTarget: &StCommentDeleteTarget,
        stCommand: &StDeleteCommentCommand,
    ) -> Result<StCommentDeleteMutation>;
    async fn vUndelete(&self, stTarget: &StCommentDeleteTarget) -> Result<()>;
}

#[async_trait]
pub trait TrCommentReindexQueue: Send + Sync {
    async fn vUpdateComments(&self, vecCommentIds: &[i32]) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stTarget() -> StCommentDeleteTarget {
        StCommentDeleteTarget {
            iCommentId: 2,
            iTopicId: 1,
            iAuthorId: 10,
            sAuthorNick: "author".into(),
            iAuthorScore: 3,
            bDeleted: false,
            bTopicDeleted: false,
            bTopicExpired: false,
            bTopicDraft: false,
            bCommentsHidden: false,
            bHasReplies: false,
            dtPostdate: Utc::now() - Duration::hours(1),
            optDeletedBy: None,
            sPostIp: "127.0.0.1".into(),
            iUserAgentId: 0,
            sCanonicalTopicUrl: "/forum/general/1".into(),
        }
    }

    #[test]
    fn moderator_never_bypasses_a_deleted_topic() {
        let mut stTarget = stTarget();
        stTarget.bTopicDeleted = true;
        assert!(!stTarget.bCanDelete(
            StCommentDeleteActor {
                iUserId: 20,
                bModerator: true,
            },
            Utc::now()
        ));
    }

    #[test]
    fn author_window_and_reply_rule_match_topic_permission_service() {
        let stActor = StCommentDeleteActor {
            iUserId: 10,
            bModerator: false,
        };
        let mut stTarget = stTarget();
        assert!(stTarget.bCanDelete(stActor, Utc::now()));
        stTarget.bHasReplies = true;
        assert!(!stTarget.bCanDelete(stActor, Utc::now()));
        stTarget.bHasReplies = false;
        stTarget.dtPostdate = Utc::now() - Duration::hours(4);
        assert!(!stTarget.bCanDelete(stActor, Utc::now()));
    }
}
