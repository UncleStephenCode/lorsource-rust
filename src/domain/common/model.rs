use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct StPagerQuery {
    pub offset: Option<i64>,
    pub page: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StSearchQuery {
    pub q: Option<String>,
    pub offset: Option<i64>,
}
