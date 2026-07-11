use crate::{
    error::Result,
    search_index::{self, FacetItem, SearchInterval, SearchItem, SearchParams, SearchRange, SearchSort, MAX_OFFSET, SEARCH_ROWS},
    state::AppState,
};
use askama::Template;
use axum::{extract::{Query, State}, response::Html};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "search.html")]
struct SearchTemplate {
    q: String,
    section: String,
    group: String,
    sort: String,
    interval: String,
    range: String,
    error: Option<String>,
    items: Vec<SearchItem>,
    total: i64,
    took_ms: i64,
    section_facet: Vec<FacetItem>,
    group_facet: Vec<FacetItem>,
    prev_link: Option<String>,
    next_link: Option<String>,
    searched: bool,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub section: Option<String>,
    pub group: Option<String>,
    pub user: Option<String>,
    pub usertopic: Option<String>,
    pub sort: Option<String>,
    pub interval: Option<String>,
    pub range: Option<String>,
    pub offset: Option<i64>,
}

fn parse_sort(s: Option<&str>) -> SearchSort {
    match s {
        Some("DATE") | Some("2") => SearchSort::Date,
        Some("DATE_OLD_TO_NEW") => SearchSort::DateOldToNew,
        _ => SearchSort::Relevance,
    }
}

fn sort_id(s: SearchSort) -> &'static str {
    match s {
        SearchSort::Relevance => "RELEVANCE",
        SearchSort::Date => "DATE",
        SearchSort::DateOldToNew => "DATE_OLD_TO_NEW",
    }
}

fn parse_interval(s: Option<&str>) -> SearchInterval {
    match s {
        Some("MONTH") => SearchInterval::Month,
        Some("THREE_MONTH") => SearchInterval::ThreeMonth,
        Some("YEAR") => SearchInterval::Year,
        Some("THREE_YEAR") => SearchInterval::ThreeYear,
        _ => SearchInterval::All,
    }
}

fn interval_id(i: SearchInterval) -> &'static str {
    match i {
        SearchInterval::Month => "MONTH",
        SearchInterval::ThreeMonth => "THREE_MONTH",
        SearchInterval::Year => "YEAR",
        SearchInterval::ThreeYear => "THREE_YEAR",
        SearchInterval::All => "ALL",
    }
}

fn parse_range(s: Option<&str>) -> SearchRange {
    match s {
        Some("TOPICS") => SearchRange::Topics,
        Some("COMMENTS") => SearchRange::Comments,
        _ => SearchRange::All,
    }
}

fn range_id(r: SearchRange) -> &'static str {
    match r {
        SearchRange::All => "ALL",
        SearchRange::Topics => "TOPICS",
        SearchRange::Comments => "COMMENTS",
    }
}

/// SearchController.search / SearchService.performSearch, backed by
/// OpenSearch instead of an in-Postgres ILIKE/tsvector scan - the previous
/// implementation never used OPENSEARCH_URL at all despite it being
/// configured, and had no facets/sort/interval/range options.
pub async fn search(State(state): State<AppState>, Query(q): Query<SearchQuery>) -> Result<Html<String>> {
    let query_text = q.q.unwrap_or_default();
    let section = q.section.unwrap_or_default();
    let group = q.group.unwrap_or_default();
    let sort = parse_sort(q.sort.as_deref());
    let interval = parse_interval(q.interval.as_deref());
    let range = parse_range(q.range.as_deref());
    let offset = q.offset.unwrap_or(0).clamp(0, MAX_OFFSET);
    let usertopic = q.usertopic.as_deref() == Some("true");

    let searched = !query_text.trim().is_empty() || q.user.is_some();

    let (items, total, took_ms, section_facet, group_facet, error, prev_link, next_link) = if searched {
        let params = SearchParams {
            q: query_text.clone(),
            section: Some(section.clone()).filter(|s| !s.is_empty()),
            group: Some(group.clone()).filter(|s| !s.is_empty()),
            user: q.user.clone(),
            usertopic,
            sort,
            interval,
            range,
            offset,
        };
        match search_index::search(&state, &params).await {
            Ok(result) => {
                let build_link = |o: i64| {
                    let mut parts = vec![format!("q={}", urlencoding::encode(&query_text))];
                    if range != SearchRange::All { parts.push(format!("range={}", range_id(range))); }
                    if interval != SearchInterval::All { parts.push(format!("interval={}", interval_id(interval))); }
                    if let Some(u) = &q.user { parts.push(format!("user={}", urlencoding::encode(u))); }
                    if usertopic { parts.push("usertopic=true".to_string()); }
                    if sort != SearchSort::Relevance { parts.push(format!("sort={}", sort_id(sort))); }
                    if !section.is_empty() { parts.push(format!("section={}", urlencoding::encode(&section))); }
                    if !group.is_empty() { parts.push(format!("group={}", urlencoding::encode(&group))); }
                    if o != 0 { parts.push(format!("offset={o}")); }
                    format!("/search.jsp?{}", parts.join("&"))
                };
                let next_offset = offset + SEARCH_ROWS;
                let next_link = (next_offset < MAX_OFFSET && result.total > next_offset).then(|| build_link(next_offset));
                let prev_link = (offset - SEARCH_ROWS >= 0).then(|| build_link(offset - SEARCH_ROWS));
                (result.items, result.total, result.took_ms, result.section_facet, result.group_facet, None, prev_link, next_link)
            }
            Err(e) => (vec![], 0, 0, vec![], vec![], Some(e), None, None),
        }
    } else {
        (vec![], 0, 0, vec![], vec![], None, None, None)
    };

    Ok(Html(SearchTemplate {
        q: query_text,
        section,
        group,
        sort: sort_id(sort).to_string(),
        interval: interval_id(interval).to_string(),
        range: range_id(range).to_string(),
        error,
        items,
        total,
        took_ms,
        section_facet,
        group_facet,
        prev_link,
        next_link,
        searched,
    }.render()?))
}
