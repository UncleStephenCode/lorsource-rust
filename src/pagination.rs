#[derive(Debug, Clone)]
pub struct Pager {
    pub offset: i64,
    pub limit: i64,
    pub next_offset: i64,
    pub prev_offset: Option<i64>,
}

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
