use serde::Serialize;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StTagItem {
    pub value: String,
    pub counter: Option<i32>,
}
