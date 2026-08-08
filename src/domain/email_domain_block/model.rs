use chrono::{DateTime, Utc};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StEmailDomainBlock {
    pub sDomain: String,
    pub dtBlockUntil: DateTime<Utc>,
    pub optModeratorNick: Option<String>,
    pub dtBlockedAt: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct StEmailDomainBlockPage {
    pub vecBlocks: Vec<StEmailDomainBlock>,
    pub iOffset: i32,
    pub iLimit: i32,
    pub bHasMore: bool,
}
