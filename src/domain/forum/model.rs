use serde::Serialize;

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
    pub topics_per_day: i32,
}
