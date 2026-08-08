#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnWarningType {
    Rule,
    Tag,
    Spelling,
    Group,
}

impl EnWarningType {
    pub fn optFromId(sValue: &str) -> Option<Self> {
        Some(match sValue {
            "rule" => Self::Rule,
            "tag" => Self::Tag,
            "spelling" => Self::Spelling,
            "group" => Self::Group,
            _ => return None,
        })
    }

    pub fn sId(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Tag => "tag",
            Self::Spelling => "spelling",
            Self::Group => "group",
        }
    }

    pub fn sName(self) -> &'static str {
        match self {
            Self::Rule => "Нарушение правил",
            Self::Tag => "Некорректные теги",
            Self::Spelling => "Опечатка или форматирование",
            Self::Group => "Некорректная группа или раздел",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StWarningTopic {
    pub iId: i32,
    pub iAuthorId: i32,
    pub bDeleted: bool,
    pub bDraft: bool,
    pub iPostScore: i32,
    pub bExpired: bool,
    pub bPremoderated: bool,
    pub bCommitted: bool,
    pub sGroupUrl: String,
    pub sSectionPrefix: String,
}

impl StWarningTopic {
    pub fn sTopicUrl(&self) -> String {
        format!("/{}/{}/{}", self.sSectionPrefix, self.sGroupUrl, self.iId)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StWarningRecord {
    pub iTopicId: i32,
    pub optCommentId: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StCreateWarningMutation {
    pub iTopicId: i32,
    pub optCommentId: Option<i32>,
    pub iAuthorId: i32,
    pub sMessage: String,
    pub enWarningType: EnWarningType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StClearWarningMutation {
    pub iWarningId: i32,
    pub iActorId: i32,
    pub iTopicId: i32,
    pub optCommentId: Option<i32>,
}
