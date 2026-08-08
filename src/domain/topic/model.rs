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

impl StTopicSummary {
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

impl StTopicDetail {
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
