//! OpenSearch indexing and querying for full-text search.
//!
//! Mirrors the Java original's `OpenSearchIndexService`/`SearchService`:
//! one `messages` index holding both topics and comments as separate
//! documents (`is_comment` distinguishes them), field names match
//! `MessageIndexDocument` exactly (section, topic_author, topic_id, author,
//! group, title, topic_title, message, postdate, tag, is_comment,
//! topic_awaits_commit) so a real Java-populated index is queryable as-is.
//!
//! `OPENSEARCH_URL` was configured but never used before this - every write
//! path (create/edit/delete topic or comment) now indexes/deletes the
//! corresponding document, best-effort: a search-indexing failure is logged
//! and does not fail the user-facing request, matching how a queue-backed
//! indexer degrades in the original (indexing is fire-and-forget from the
//! request's point of view there too, just via an actor mailbox instead of
//! inline HTTP).

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const INDEX: &str = "messages";

fn base_url(state: &AppState) -> Option<&str> {
    state.config.opensearch_url.as_deref()
}

#[derive(Debug, Serialize)]
struct MessageIndexDocument {
    section: String,
    topic_author: String,
    topic_id: i32,
    author: String,
    group: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    topic_title: String,
    message: String,
    postdate: String,
    tag: Vec<String>,
    is_comment: bool,
    topic_awaits_commit: bool,
}

pub async fn ensure_index(state: &AppState) {
    let Some(base) = base_url(state) else { return };
    let url = format!("{base}/{INDEX}");
    let exists = state.http.head(&url).send().await.map(|r| r.status().is_success()).unwrap_or(false);
    if exists {
        return;
    }
    let mapping = json!({
        "mappings": {
            "properties": {
                "section": {"type": "keyword"},
                "group": {"type": "keyword"},
                "topic_author": {"type": "keyword"},
                "author": {"type": "keyword"},
                "topic_id": {"type": "integer"},
                "title": {"type": "text"},
                "topic_title": {"type": "text"},
                "message": {"type": "text", "fields": {"raw": {"type": "text"}}},
                "postdate": {"type": "date"},
                "tag": {"type": "keyword"},
                "is_comment": {"type": "boolean"},
                "topic_awaits_commit": {"type": "boolean"}
            }
        }
    });
    if let Err(e) = state.http.put(&url).json(&mapping).send().await {
        tracing::warn!(error = %e, "failed to create opensearch index");
    }
}

struct TopicRow {
    section: String,
    group: String,
    author: String,
    title: String,
    message: String,
    postdate: chrono::DateTime<chrono::Utc>,
    tags: Vec<String>,
    deleted: bool,
    draft: bool,
    moderate: bool,
    /// TopicPermissionService.isTopicSearchable's remaining conditions:
    /// comments-hidden topics and anonymous-authored, not-yet-committed
    /// topics in a premoderated section are excluded from the index too.
    comments_hidden: bool,
    premoderated_anonymous_uncommitted: bool,
}

const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
/// UserConstants.ANONYMOUS_ID.
const ANONYMOUS_USER_ID: i32 = 2;

async fn load_topic_row(state: &AppState, topic_id: i32) -> Option<TopicRow> {
    let row: Option<(String, String, String, i32, String, String, chrono::DateTime<chrono::Utc>, bool, bool, bool, i32, bool)> = sqlx::query_as(
        r#"SELECT CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END,
                  g.urlname, u.nick, u.id, t.title, m.message, t.postdate, t.deleted, COALESCE(t.draft,false), t.moderate, t.postscore, s.moderate
           FROM topics t
           JOIN msgbase m ON m.id=t.id
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    let (section, group, author, author_id, title, message, postdate, deleted, draft, moderate, postscore, section_premoderated) = row?;
    let tags: Vec<String> = sqlx::query_scalar("SELECT tv.value FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid WHERE tg.msgid=$1")
        .bind(topic_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();
    let comments_hidden = postscore == POSTSCORE_HIDE_COMMENTS;
    // moderate==true means "awaiting commit" in this port's convention.
    let premoderated_anonymous_uncommitted = section_premoderated && moderate && author_id == ANONYMOUS_USER_ID;
    Some(TopicRow { section, group, author, title, message, postdate, tags, deleted, draft, moderate, comments_hidden, premoderated_anonymous_uncommitted })
}

/// Reindex (or, if no longer searchable, remove) a topic and optionally its comments.
pub async fn index_topic(state: &AppState, topic_id: i32, with_comments: bool) {
    let Some(base) = base_url(state) else { return };
    let Some(row) = load_topic_row(state, topic_id).await else { return };

    if row.deleted || row.draft || row.comments_hidden || row.premoderated_anonymous_uncommitted {
        let _ = state.http.delete(format!("{base}/{INDEX}/_doc/{topic_id}")).send().await;
    } else {
        let doc = MessageIndexDocument {
            section: row.section.clone(),
            topic_author: row.author.clone(),
            topic_id,
            author: row.author.clone(),
            group: row.group.clone(),
            title: Some(row.title.clone()),
            topic_title: row.title.clone(),
            message: markup::plain_text_for_index(&row.message),
            postdate: row.postdate.to_rfc3339(),
            tag: row.tags.clone(),
            is_comment: false,
            topic_awaits_commit: row.moderate,
        };
        put_doc(state, base, topic_id, &doc).await;
    }

    if with_comments {
        let comment_ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM comments WHERE topic=$1").bind(topic_id).fetch_all(&state.pool).await.unwrap_or_default();
        for id in comment_ids {
            index_comment(state, id).await;
        }
    }
}

pub async fn index_comment(state: &AppState, comment_id: i32) {
    let Some(base) = base_url(state) else { return };
    let row: Option<(i32, String, bool)> = sqlx::query_as(
        "SELECT topic, title, deleted FROM comments WHERE id=$1",
    )
    .bind(comment_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    let Some((topic_id, comment_title, comment_deleted)) = row else { return };
    let Some(topic) = load_topic_row(state, topic_id).await else {
        let _ = state.http.delete(format!("{base}/{INDEX}/_doc/{comment_id}")).send().await;
        return;
    };

    if comment_deleted || topic.deleted || topic.draft || topic.comments_hidden || topic.premoderated_anonymous_uncommitted {
        let _ = state.http.delete(format!("{base}/{INDEX}/_doc/{comment_id}")).send().await;
        return;
    }

    let message: Option<String> = sqlx::query_scalar("SELECT message FROM msgbase WHERE id=$1").bind(comment_id).fetch_optional(&state.pool).await.ok().flatten();
    let title = Some(comment_title).filter(|t| !t.is_empty() && *t != topic.title && !t.starts_with("Re:"));

    let doc = MessageIndexDocument {
        section: topic.section.clone(),
        topic_author: topic.author.clone(),
        topic_id,
        author: topic.author.clone(),
        group: topic.group.clone(),
        title,
        topic_title: topic.title.clone(),
        message: markup::plain_text_for_index(message.as_deref().unwrap_or("")),
        postdate: topic.postdate.to_rfc3339(),
        tag: topic.tags.clone(),
        is_comment: true,
        topic_awaits_commit: topic.moderate,
    };
    put_doc(state, base, comment_id, &doc).await;
}

async fn put_doc(state: &AppState, base: &str, id: i32, doc: &MessageIndexDocument) {
    if let Err(e) = state.http.put(format!("{base}/{INDEX}/_doc/{id}")).json(doc).send().await {
        tracing::warn!(error = %e, id, "failed to index search document");
    }
}

pub async fn delete_doc(state: &AppState, id: i32) {
    let Some(base) = base_url(state) else { return };
    let _ = state.http.delete(format!("{base}/{INDEX}/_doc/{id}")).send().await;
}

/// `/admin/search-reindex`: rebuild the whole index from Postgres.
pub async fn reindex_all(state: &AppState) -> Result<(u64, u64), String> {
    let Some(_) = base_url(state) else { return Err("OPENSEARCH_URL is not configured".into()) };
    ensure_index(state).await;
    let topic_ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM topics WHERE NOT deleted AND NOT draft")
        .fetch_all(&state.pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut topics = 0u64;
    let mut comments = 0u64;
    for id in topic_ids {
        index_topic(state, id, true).await;
        topics += 1;
    }
    let comment_count: i64 = sqlx::query_scalar("SELECT count(*) FROM comments c JOIN topics t ON t.id=c.topic WHERE NOT c.deleted AND NOT t.deleted AND NOT COALESCE(t.draft,false)")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
    comments += comment_count.max(0) as u64;
    Ok((topics, comments))
}

// --- search query ---

#[derive(Debug, Clone)]
pub struct SearchParams {
    pub q: String,
    pub section: Option<String>,
    pub group: Option<String>,
    pub user: Option<String>,
    pub usertopic: bool,
    pub sort: SearchSort,
    pub interval: SearchInterval,
    pub range: SearchRange,
    pub offset: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSort { Relevance, Date, DateOldToNew }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchInterval { Month, ThreeMonth, Year, ThreeYear, All }

impl SearchInterval {
    fn gte_expr(self) -> Option<&'static str> {
        match self {
            SearchInterval::Month => Some("now/h-1M"),
            SearchInterval::ThreeMonth => Some("now/d-3M"),
            SearchInterval::Year => Some("now/d-1y"),
            SearchInterval::ThreeYear => Some("now/w-3y"),
            SearchInterval::All => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchRange { All, Topics, Comments }

pub const SEARCH_ROWS: i64 = 25;
pub const MAX_OFFSET: i64 = 10000 - SEARCH_ROWS;

#[derive(Debug, Deserialize)]
struct EsHit {
    _id: String,
    _score: Option<f64>,
    _source: EsSource,
}

#[derive(Debug, Deserialize)]
struct EsSource {
    section: String,
    group: String,
    topic_id: i32,
    author: String,
    title: Option<String>,
    topic_title: String,
    message: String,
    postdate: String,
    #[serde(default)]
    tag: Vec<String>,
    is_comment: bool,
}

pub struct SearchItem {
    pub title: String,
    pub url: String,
    pub author: String,
    pub postdate: String,
    pub message_excerpt: String,
    pub is_comment: bool,
    pub tags: Vec<String>,
}

pub struct FacetItem { pub key: String, pub label: String }

pub struct SearchResult {
    pub items: Vec<SearchItem>,
    pub total: i64,
    pub took_ms: i64,
    pub section_facet: Vec<FacetItem>,
    pub group_facet: Vec<FacetItem>,
}

pub async fn search(state: &AppState, p: &SearchParams) -> Result<SearchResult, String> {
    let Some(base) = base_url(state) else { return Err("Поиск временно недоступен: не сконфигурирован OPENSEARCH_URL".into()) };

    // SearchService.performSearch: only section/group go into postFilter
    // (applied after aggregations are computed, so switching sections
    // doesn't zero out every other section's facet count) - range/
    // interval/user narrow the query (and therefore the aggregations)
    // itself, same as the free-text query.
    let mut query_filters: Vec<Value> = Vec::new();
    match p.range {
        SearchRange::Topics => query_filters.push(json!({"term": {"is_comment": false}})),
        SearchRange::Comments => query_filters.push(json!({"term": {"is_comment": true}})),
        SearchRange::All => {}
    }
    if let Some(user) = p.user.as_deref() {
        let field = if p.usertopic { "topic_author" } else { "author" };
        query_filters.push(json!({"term": {field: user}}));
    }
    if let Some(gte) = p.interval.gte_expr() {
        query_filters.push(json!({"range": {"postdate": {"gte": gte}}}));
    }

    let mut post_filters: Vec<Value> = Vec::new();
    if let Some(section) = p.section.as_deref().filter(|s| !s.is_empty()) {
        post_filters.push(json!({"term": {"section": section}}));
    }
    if let Some(group) = p.group.as_deref().filter(|s| !s.is_empty()) {
        post_filters.push(json!({"term": {"group": group}}));
    }

    let text_query = if p.q.trim().is_empty() {
        json!({"match_all": {}})
    } else {
        json!({
            "bool": {
                "should": [
                    {"match": {"title": {"query": p.q, "minimum_should_match": "2"}}},
                    {"match": {"message": {"query": p.q, "minimum_should_match": "2"}}},
                    {"match_phrase": {"message": p.q}},
                    {"match_phrase": {"title": p.q}}
                ],
                "minimum_should_match": 1
            }
        })
    };

    let sort = match p.sort {
        SearchSort::Relevance => json!([{"_score": {"order": "desc"}}, {"postdate": {"order": "desc"}}]),
        SearchSort::Date => json!([{"postdate": {"order": "desc"}}]),
        SearchSort::DateOldToNew => json!([{"postdate": {"order": "asc"}}]),
    };

    let mut body = json!({
        "query": {"bool": {"must": text_query, "filter": query_filters}},
        "sort": sort,
        "from": p.offset,
        "size": SEARCH_ROWS,
        "aggs": {
            "sections": {
                "terms": {"field": "section", "size": 50},
                "aggs": {"groups": {"terms": {"field": "group", "size": 50}}}
            }
        },
        "track_total_hits": true
    });
    if !post_filters.is_empty() {
        body["post_filter"] = json!({"bool": {"filter": post_filters}});
    }

    let resp = state.http.post(format!("{base}/{INDEX}/_search")).json(&body).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("opensearch error {status}: {text}"));
    }
    let payload: Value = resp.json().await.map_err(|e| e.to_string())?;

    let took_ms = payload.get("took").and_then(|v| v.as_i64()).unwrap_or(0);
    let total = payload.pointer("/hits/total/value").and_then(|v| v.as_i64()).unwrap_or(0);
    let hits: Vec<EsHit> = payload.pointer("/hits/hits").cloned().map(serde_json::from_value).transpose().unwrap_or(None).unwrap_or_default();

    let items = hits.into_iter().map(|h| {
        let s = h._source;
        let url = if s.is_comment {
            format!("/{}/{}/{}?cid={}", s.section, s.group, s.topic_id, h._id)
        } else {
            format!("/{}/{}/{}", s.section, s.group, s.topic_id)
        };
        let title = s.title.filter(|t| !t.trim().is_empty()).unwrap_or(s.topic_title);
        let excerpt: String = s.message.chars().take(300).collect();
        SearchItem {
            title,
            url,
            author: s.author,
            postdate: s.postdate,
            message_excerpt: excerpt,
            is_comment: s.is_comment,
            tags: s.tag,
        }
    }).collect();

    let mut section_facet = Vec::new();
    let mut group_facet = Vec::new();
    if let Some(buckets) = payload.pointer("/aggregations/sections/buckets").and_then(|v| v.as_array()) {
        for b in buckets {
            let key = b.get("key").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let count = b.get("doc_count").and_then(|v| v.as_i64()).unwrap_or(0);
            section_facet.push(FacetItem { key: key.clone(), label: format!("{key} ({count})") });
            if p.section.as_deref() == Some(key.as_str()) {
                if let Some(gbuckets) = b.pointer("/groups/buckets").and_then(|v| v.as_array()) {
                    for gb in gbuckets {
                        let gkey = gb.get("key").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                        let gcount = gb.get("doc_count").and_then(|v| v.as_i64()).unwrap_or(0);
                        group_facet.push(FacetItem { key: gkey.clone(), label: format!("{gkey} ({gcount})") });
                    }
                }
            }
        }
    }

    Ok(SearchResult { items, total, took_ms, section_facet, group_facet })
}

use crate::markup;
