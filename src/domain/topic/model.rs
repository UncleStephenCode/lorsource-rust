use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StTopicSummary {
    pub id: i32,
    pub title: String,
    pub url: Option<String>,
    pub postdate: DateTime<Utc>,
    pub lastmod: Option<DateTime<Utc>>,
    pub author_id: i32,
    pub author: String,
    pub group_id: i32,
    pub group_title: String,
    pub group_urlname: String,
    pub section_id: i32,
    pub section_name: String,
    pub section_prefix: String,
    pub comments: i32,
    pub deleted: bool,
    pub sticky: bool,
    pub resolved: Option<bool>,
    pub tags: Option<String>,
}

/// Main-page metadata which is intentionally absent from ordinary topic-list
/// rows.  The original controller needs `minor` only while splitting its one
/// commit-date-ordered result into full cards and the brief tail.
#[derive(Debug, Clone)]
pub struct StMainTopicSummary {
    pub stTopic: StTopicSummary,
    pub bMinor: bool,
}

impl StTopicSummary {
    pub fn sTitlePlain(&self) -> String {
        crate::domain::title::sTopicTitlePlainForDisplay(&self.title)
    }

    pub fn sTopicUrl(&self) -> String {
        format!(
            "/{}/{}/{}",
            self.section_prefix, self.group_urlname, self.id
        )
    }

    pub fn topic_url(&self) -> String {
        self.sTopicUrl()
    }

    pub fn vecTags(&self) -> Vec<String> {
        self.tags
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|sValue| !sValue.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    pub fn tags_vec(&self) -> Vec<String> {
        self.vecTags()
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StTopicDetail {
    pub id: i32,
    pub title: String,
    pub message: String,
    pub markup: String,
    pub url: Option<String>,
    pub linktext: Option<String>,
    pub postdate: DateTime<Utc>,
    pub lastmod: Option<DateTime<Utc>>,
    pub author_id: i32,
    pub author: String,
    #[serde(skip)]
    pub author_score: i32,
    #[serde(skip)]
    pub author_blocked: bool,
    #[serde(skip)]
    pub author_anonymous: bool,
    #[serde(skip)]
    pub author_frozen: bool,
    pub group_id: i32,
    pub group_title: String,
    pub group_urlname: String,
    pub section_id: i32,
    pub section_name: String,
    pub section_prefix: String,
    pub section_premoderated: bool,
    pub comments: i32,
    pub deleted: bool,
    pub sticky: bool,
    pub resolved: Option<bool>,
    pub tags: Option<String>,
    pub draft: bool,
    pub moderate: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StRssContext {
    pub sSectionName: String,
    pub optGroupTitle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StRssTag {
    pub sName: String,
    pub iCounter: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StRssImage {
    pub iId: i32,
    pub sExtension: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StRssPollVariant {
    pub sLabel: String,
    pub iVotes: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StRssPoll {
    pub bMultiSelect: bool,
    pub iVoterCount: i64,
    pub vecVariants: Vec<StRssPollVariant>,
}

/// Complete source data needed by `message-rss.tag`.
///
/// `sStoredTitle` deliberately remains in Java's HTML-escaped database form.
/// The RSS item title escapes that stored value once more, exactly like
/// `${fn:escapeXml(msg.message.title)}` in the original JSP.
#[derive(Debug, Clone)]
pub struct StRssTopic {
    pub iId: i32,
    pub sStoredTitle: String,
    pub dtPublished: DateTime<Utc>,
    pub dtLastModified: DateTime<Utc>,
    pub sAuthorNick: String,
    pub sGroupUrlName: String,
    pub sSectionPrefix: String,
    pub sMessage: String,
    pub sMarkup: String,
    pub bImagePost: bool,
    pub bImagesAllowed: bool,
    pub bPollPostAllowed: bool,
    pub bNofollow: bool,
    pub vecTags: Vec<StRssTag>,
    pub vecImages: Vec<StRssImage>,
    pub optPoll: Option<StRssPoll>,
}

impl StRssTopic {
    pub fn sTopicUrl(&self) -> String {
        format!(
            "/{}/{}/{}",
            self.sSectionPrefix, self.sGroupUrlName, self.iId
        )
    }
}

impl StTopicDetail {
    pub fn bNofollowAuthorLinks(&self) -> bool {
        !crate::domain::topic::link_policy::StAuthorLinkState {
            iScore: self.author_score,
            bBlocked: self.author_blocked,
            bAnonymous: self.author_anonymous,
            bFrozen: self.author_frozen,
        }
        .bFollowInTopic(self.moderate)
    }

    pub fn sTitlePlain(&self) -> String {
        crate::domain::title::sTopicTitlePlainForDisplay(&self.title)
    }

    pub fn sTopicUrl(&self) -> String {
        format!(
            "/{}/{}/{}",
            self.section_prefix, self.group_urlname, self.id
        )
    }

    pub fn topic_url(&self) -> String {
        self.sTopicUrl()
    }

    pub fn vecTags(&self) -> Vec<String> {
        self.tags
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|sValue| !sValue.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    pub fn tags_vec(&self) -> Vec<String> {
        self.vecTags()
    }
}
