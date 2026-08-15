#[derive(Debug, Clone)]
pub struct Pager {
    pub offset: i64,
    pub limit: i64,
    pub next_offset: i64,
    pub prev_offset: Option<i64>,
}

/// `TopicListService`/`user-topics.jsp` use a fixed twenty-topic page and
/// clamp arbitrary request offsets independently of the viewer profile.
pub const TOPIC_FEED_PAGE_SIZE: i64 = 20;
pub const TOPIC_FEED_MAX_OFFSET: i64 = 300;
pub const TOPIC_FEED_NEXT_OFFSET_CEILING: i64 = 200;

impl Pager {
    pub fn new(offset: i64, limit: i64) -> Self {
        let offset = offset.max(0);
        Self {
            offset,
            limit,
            next_offset: offset + limit,
            prev_offset: if offset > 0 {
                Some((offset - limit).max(0))
            } else {
                None
            },
        }
    }
}

pub fn topic_feed_pager(raw_offset: i64) -> Pager {
    Pager::new(
        raw_offset.clamp(0, TOPIC_FEED_MAX_OFFSET),
        TOPIC_FEED_PAGE_SIZE,
    )
}

/// Mirrors the JSP condition exactly.  In particular this deliberately does
/// not issue a look-ahead query: Java treats a full page as sufficient proof
/// of a possible next page.
pub fn topic_feed_has_next(pager: &Pager, item_count: usize) -> bool {
    pager.offset < TOPIC_FEED_NEXT_OFFSET_CEILING && item_count as i64 == pager.limit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_feed_clamps_offsets_and_always_uses_twenty_items() {
        assert_eq!(topic_feed_pager(-1).offset, 0);
        assert_eq!(topic_feed_pager(999).offset, 300);
        assert_eq!(topic_feed_pager(40).limit, 20);
    }

    #[test]
    fn topic_feed_next_link_matches_java_jsp_condition() {
        assert!(topic_feed_has_next(&topic_feed_pager(180), 20));
        assert!(!topic_feed_has_next(&topic_feed_pager(180), 19));
        assert!(!topic_feed_has_next(&topic_feed_pager(200), 20));
    }
}
