use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Section {
    pub id: i32,
    pub name: String,
    pub title: String,
    pub url_prefix: String,
    pub moderate: bool,
    pub imagepost: bool,
    pub preformat: bool,
    pub havelink: bool,
    pub vote: Option<bool>,
    pub add_info: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Group {
    pub id: i32,
    pub title: String,
    pub urlname: String,
    pub section: i32,
    pub section_name: String,
    pub section_prefix: String,
    pub info: Option<String>,
    pub longinfo: Option<String>,
    pub topics: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserSummary {
    pub id: i32,
    pub nick: String,
    pub name: Option<String>,
    pub score: Option<i32>,
    pub max_score: Option<i32>,
    pub photo: Option<String>,
    pub town: Option<String>,
    pub regdate: Option<NaiveDateTime>,
    pub canmod: bool,
    pub blocked: Option<bool>,
    pub userinfo: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TopicSummary {
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
    pub views: i32,
    pub deleted: bool,
    pub sticky: bool,
    pub resolved: Option<bool>,
    pub tags: Option<String>,
}

impl TopicSummary {
    pub fn topic_url(&self) -> String {
        format!("/{}/{}/{}", self.section_prefix, self.group_urlname, self.id)
    }

    pub fn tags_vec(&self) -> Vec<String> {
        self.tags
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TopicDetail {
    pub id: i32,
    pub title: String,
    pub message: String,
    pub bbcode: Option<bool>,
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
    pub comments: i32,
    pub views: i32,
    pub deleted: bool,
    pub sticky: bool,
    pub resolved: Option<bool>,
    pub tags: Option<String>,
}

impl TopicDetail {
    pub fn topic_url(&self) -> String {
        format!("/{}/{}/{}", self.section_prefix, self.group_urlname, self.id)
    }

    pub fn tags_vec(&self) -> Vec<String> {
        self.tags
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CommentItem {
    pub id: i32,
    pub topic: i32,
    pub replyto: Option<i32>,
    pub title: String,
    pub message: String,
    pub postdate: DateTime<Utc>,
    pub author_id: i32,
    pub author: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TagItem {
    pub value: String,
    pub counter: Option<i32>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct EventItem {
    pub id: i32,
    pub event_date: DateTime<Utc>,
    pub event_type: String,
    pub message: String,
    pub topic_id: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PagerQuery {
    pub offset: Option<i64>,
    pub page: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub offset: Option<i64>,
}
