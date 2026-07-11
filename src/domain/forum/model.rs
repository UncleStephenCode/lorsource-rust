use serde::Serialize;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StSection {
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
pub struct StGroup {
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
