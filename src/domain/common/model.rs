use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct StPagerQuery {
    pub offset: Option<i64>,
    pub page: Option<i64>,
}
