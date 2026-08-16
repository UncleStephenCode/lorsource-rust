use chrono::{DateTime, Utc};

use crate::domain::topic::model::StTopicSummary;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StTagSection {
    pub iId: i32,
    pub sName: String,
    pub sUrlName: String,
    pub bPremoderated: bool,
    pub iTopicsRestriction: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StTagInfo {
    pub iId: i32,
    pub sName: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StTagViewerProfile {
    pub iTopics: i32,
    pub iMessages: i32,
    pub bOldTracker: bool,
}

impl Default for StTagViewerProfile {
    fn default() -> Self {
        let stProfile = crate::profile::ProfileSettings::default();
        Self {
            iTopics: stProfile.topics,
            iMessages: stProfile.messages,
            bOldTracker: stProfile.old_tracker,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StTagViewerState {
    pub bFavorite: bool,
    pub bIgnored: bool,
    pub iFavoritesCount: i64,
    pub iIgnoreCount: i64,
}

/// `PreparedTopicsListItem` data used by `tracker-topics*.tag` in the
/// original forum-mode tag listing.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StTagForumTopic {
    pub iTopicId: i32,
    pub sStoredTitle: String,
    pub sTopicAuthor: String,
    pub bTopicAuthorBlocked: bool,
    pub sLastAuthor: String,
    pub bLastAuthorBlocked: bool,
    pub sGroupTitle: String,
    pub sGroupUrlName: String,
    pub sSectionUrlName: String,
    pub dtLastPost: DateTime<Utc>,
    pub iCommentCount: i32,
    pub optLastCommentId: Option<i32>,
    pub bResolved: bool,
    pub bUncommitted: bool,
    pub bDeleted: bool,
    pub iTopicPostscore: i32,
    pub optTags: Option<String>,
}

impl StTagForumTopic {
    pub fn sTitlePlain(&self) -> String {
        crate::domain::title::sTopicTitlePlainForDisplay(&self.sStoredTitle)
    }

    pub fn vecTags(&self) -> Vec<&str> {
        self.optTags
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|sValue| !sValue.is_empty())
            .collect()
    }

    pub fn iVisibleCommentCount(&self) -> i32 {
        if self.iTopicPostscore == 10_002 {
            0
        } else {
            self.iCommentCount
        }
    }

    pub fn bCommentsClosed(&self) -> bool {
        self.iTopicPostscore >= 10_000
    }

    pub fn sGroupUrl(&self) -> String {
        format!(
            "/{}/{}/",
            self.sSectionUrlName,
            urlencoding::encode(&self.sGroupUrlName)
        )
    }

    pub fn sLastPageUrl(&self, iMessagesPerPage: i32) -> String {
        let iMessages = iMessagesPerPage.max(1);
        let iPages = ((self.iCommentCount.max(0) + iMessages - 1) / iMessages).max(0);
        let iLastCommentId = self.optLastCommentId.unwrap_or(0);
        if iPages > 1 {
            format!(
                "{}{}/page{}?lastmod={iLastCommentId}",
                self.sGroupUrl(),
                self.iTopicId,
                iPages - 1
            )
        } else {
            format!(
                "{}{}?lastmod={iLastCommentId}",
                self.sGroupUrl(),
                self.iTopicId
            )
        }
    }
}

#[derive(Debug, Clone)]
pub enum EnTagSectionTopics {
    Forum(Vec<StTagForumTopic>),
    Feed(Vec<StTopicSummary>),
}

impl EnTagSectionTopics {
    pub fn iLen(&self) -> usize {
        match self {
            Self::Forum(vecTopics) => vecTopics.len(),
            Self::Feed(vecTopics) => vecTopics.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StTagSectionPage {
    pub stSection: StTagSection,
    pub vecSections: Vec<StTagSection>,
    pub stProfile: StTagViewerProfile,
    pub stViewerState: StTagViewerState,
    pub enTopics: EnTagSectionTopics,
    pub iOffset: i32,
    pub iPageSize: i32,
    pub iCounter: i64,
}

#[derive(Debug, Clone)]
pub enum EnTagSectionOutcome {
    Redirect { sMainTag: String, iSectionId: i32 },
    Page(StTagSectionPage),
}
