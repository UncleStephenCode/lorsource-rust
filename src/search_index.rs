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
//! path (create/edit/delete topic or comment) writes a durable queue item and
//! returns without waiting for OpenSearch. The filesystem spool replaces the
//! original persistent embedded ActiveMQ queue while preserving its important
//! semantics: committed forum writes survive OpenSearch outages and process
//! restarts, retries are idempotent, and indexing remains fire-and-forget from
//! the HTTP request's point of view.

use crate::state::AppState;
use chrono::{Datelike, TimeZone};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const INDEX: &str = "messages";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum EnSearchQueueJob {
    Topic { id: i32, with_comments: bool },
    Comment { id: i32 },
}

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

fn stIndexDefinition() -> Value {
    json!({
        "settings": {
            "analysis": {
                "analyzer": {
                    "text_analyzer": {
                        "type": "custom",
                        "tokenizer": "text_tokenizer",
                        "filter": ["m_long_word", "lowercase", "m_my_snow_ru", "m_my_snow_en"],
                        "char_filter": ["html_strip", "m_ee"]
                    },
                    "exact_analyzer": {
                        "type": "custom",
                        "tokenizer": "text_tokenizer",
                        "filter": ["m_long_word", "lowercase"],
                        "char_filter": ["html_strip", "m_ee"]
                    }
                },
                "tokenizer": {
                    "text_tokenizer": {"type": "standard"}
                },
                "filter": {
                    "m_long_word": {"type": "length", "max": 100},
                    "m_my_snow_ru": {"type": "snowball", "language": "Russian"},
                    "m_my_snow_en": {"type": "snowball", "language": "English"}
                },
                "char_filter": {
                    "m_ee": {"type": "mapping", "mappings": ["ё => е", "Ё => Е"]}
                }
            }
        },
        "mappings": {
            "properties": {
                "section": {"type": "keyword"},
                "group": {"type": "keyword"},
                "topic_author": {"type": "keyword"},
                "author": {"type": "keyword"},
                "topic_id": {"type": "long"},
                "title": {"type": "text", "analyzer": "text_analyzer"},
                "topic_title": {"type": "text", "index": false},
                "message": {
                    "type": "text",
                    "analyzer": "text_analyzer",
                    "term_vector": "with_positions_offsets",
                    "fields": {
                        "raw": {
                            "type": "text",
                            "analyzer": "exact_analyzer",
                            "term_vector": "with_positions_offsets"
                        }
                    }
                },
                "postdate": {"type": "date"},
                "tag": {"type": "keyword"},
                "is_comment": {"type": "boolean"},
                "topic_awaits_commit": {"type": "boolean"}
            }
        }
    })
}

async fn vEnsureIndex(state: &AppState) -> Result<(), String> {
    let Some(base) = base_url(state) else {
        return Err("OPENSEARCH_URL is not configured".into());
    };
    let url = format!("{base}/{INDEX}");
    let stExistsResponse = state
        .http
        .head(&url)
        .send()
        .await
        .map_err(|stError| stError.to_string())?;
    if stExistsResponse.status().is_success() {
        return vValidateIndexContract(state, &url).await;
    }
    if stExistsResponse.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(format!(
            "OpenSearch index lookup failed with {}",
            stExistsResponse.status()
        ));
    }

    state
        .http
        .put(&url)
        .json(&stIndexDefinition())
        .send()
        .await
        .map_err(|stError| stError.to_string())?
        .error_for_status()
        .map_err(|stError| stError.to_string())?;
    vValidateIndexContract(state, &url).await
}

fn vecIndexContractProblems(stIndex: &Value) -> Vec<String> {
    let sRoot = format!("/{INDEX}/mappings/properties");
    let vecExpected = [
        ("section/type", json!("keyword")),
        ("group/type", json!("keyword")),
        ("topic_author/type", json!("keyword")),
        ("author/type", json!("keyword")),
        ("topic_id/type", json!("long")),
        ("title/type", json!("text")),
        ("title/analyzer", json!("text_analyzer")),
        ("topic_title/type", json!("text")),
        ("topic_title/index", json!(false)),
        ("message/type", json!("text")),
        ("message/analyzer", json!("text_analyzer")),
        ("message/term_vector", json!("with_positions_offsets")),
        ("message/fields/raw/type", json!("text")),
        ("message/fields/raw/analyzer", json!("exact_analyzer")),
        (
            "message/fields/raw/term_vector",
            json!("with_positions_offsets"),
        ),
        ("postdate/type", json!("date")),
        ("tag/type", json!("keyword")),
        ("is_comment/type", json!("boolean")),
        ("topic_awaits_commit/type", json!("boolean")),
    ];
    vecExpected
        .into_iter()
        .filter_map(|(sPath, stExpected)| {
            let sPointer = format!("{sRoot}/{sPath}");
            (stIndex.pointer(&sPointer) != Some(&stExpected)).then(|| {
                format!(
                    "{sPath}: expected {stExpected}, got {}",
                    stIndex
                        .pointer(&sPointer)
                        .map_or_else(|| "<missing>".to_owned(), Value::to_string)
                )
            })
        })
        .collect()
}

async fn vValidateIndexContract(state: &AppState, sUrl: &str) -> Result<(), String> {
    let stResponse = state
        .http
        .get(sUrl)
        .send()
        .await
        .map_err(|stError| stError.to_string())?
        .error_for_status()
        .map_err(|stError| stError.to_string())?;
    let stIndex: Value = stResponse
        .json()
        .await
        .map_err(|stError| stError.to_string())?;
    let vecProblems = vecIndexContractProblems(&stIndex);
    if vecProblems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "OpenSearch index {INDEX:?} is incompatible with the Java search contract; rebuild it from PostgreSQL before serving traffic:\n- {}",
            vecProblems.join("\n- ")
        ))
    }
}

pub async fn ensure_index(state: &AppState) -> Result<(), String> {
    if base_url(state).is_none() {
        return Ok(());
    }
    vEnsureIndex(state).await
}

struct TopicRow {
    section: String,
    group: String,
    author: String,
    title: String,
    message: String,
    markup: String,
    postdate: chrono::DateTime<chrono::Utc>,
    tags: Vec<String>,
    deleted: bool,
    draft: bool,
    awaits_commit: bool,
    /// TopicPermissionService.isTopicSearchable's remaining conditions:
    /// comments-hidden topics and anonymous-authored, not-yet-committed
    /// topics in a premoderated section are excluded from the index too.
    comments_hidden: bool,
    premoderated_anonymous_uncommitted: bool,
}

struct CommentRow {
    topic_id: i32,
    title: String,
    author: String,
    message: String,
    markup: String,
    postdate: chrono::DateTime<chrono::Utc>,
    deleted: bool,
}

type TySearchCommentRow = (
    i32,
    String,
    String,
    String,
    String,
    chrono::DateTime<chrono::Utc>,
    bool,
);

fn stCommentIndexDocument(stTopic: &TopicRow, stComment: &CommentRow) -> MessageIndexDocument {
    let optTitle =
        Some(html_escape::decode_html_entities(&stComment.title).into_owned()).filter(|sTitle| {
            !sTitle.is_empty() && *sTitle != stTopic.title && !sTitle.starts_with("Re:")
        });

    MessageIndexDocument {
        section: stTopic.section.clone(),
        topic_author: stTopic.author.clone(),
        topic_id: stComment.topic_id,
        author: stComment.author.clone(),
        group: stTopic.group.clone(),
        title: optTitle,
        topic_title: stTopic.title.clone(),
        // OpenSearchIndexService stores MessageTextService's sanitized HTML,
        // not a plain-text approximation. The analyzer strips tags while the
        // search page can retain safe formatting around highlighted text.
        message: markup::render_message_with_markup(
            &stComment.message,
            Some(&stComment.markup),
            None,
        ),
        postdate: stComment.postdate.to_rfc3339(),
        tag: stTopic.tags.clone(),
        is_comment: true,
        topic_awaits_commit: stTopic.awaits_commit,
    }
}

const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
/// UserConstants.ANONYMOUS_ID.
const ANONYMOUS_USER_ID: i32 = 2;

/// Java's `OpenSearchIndexService.topicAwaitsCommit`: `sections.moderate`
/// marks a premoderated section, while `topics.moderate` means that the topic
/// has already been committed by a moderator.
fn topic_awaits_commit(section_premoderated: bool, topic_committed: bool) -> bool {
    section_premoderated && !topic_committed
}

type TySearchTopicRow = (
    String,
    String,
    String,
    i32,
    String,
    String,
    String,
    chrono::DateTime<chrono::Utc>,
    bool,
    bool,
    bool,
    i32,
    bool,
);

async fn optLoadTopicRow(state: &AppState, topic_id: i32) -> Result<Option<TopicRow>, String> {
    let row: Option<TySearchTopicRow> = sqlx::query_as(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END,
                  g.urlname, u.nick, u.id, t.title, m.message, m.markup::text,
                  t.postdate, t.deleted, COALESCE(t.draft,false), t.moderate,
                  COALESCE(t.postscore, -9999), s.moderate
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
    .map_err(|stError| stError.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let (
        section,
        group,
        author,
        author_id,
        title,
        message,
        markup,
        postdate,
        deleted,
        draft,
        committed,
        postscore,
        section_premoderated,
    ) = row;
    let tags: Vec<String> = sqlx::query_scalar(
        "SELECT tv.value FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid WHERE tg.msgid=$1",
    )
    .bind(topic_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|stError| stError.to_string())?;
    let comments_hidden = postscore == POSTSCORE_HIDE_COMMENTS;
    let awaits_commit = topic_awaits_commit(section_premoderated, committed);
    // TopicPermissionService.isTopicSearchable excludes an anonymous topic
    // only while it awaits commit in a premoderated section.
    let premoderated_anonymous_uncommitted = awaits_commit && author_id == ANONYMOUS_USER_ID;
    Ok(Some(TopicRow {
        section,
        group,
        author,
        title: html_escape::decode_html_entities(&title).into_owned(),
        message,
        markup,
        postdate,
        tags,
        deleted,
        draft,
        awaits_commit,
        comments_hidden,
        premoderated_anonymous_uncommitted,
    }))
}

/// Reindex (or, if no longer searchable, remove) a topic and optionally its comments.
async fn vIndexTopic(state: &AppState, topic_id: i32, with_comments: bool) -> Result<(), String> {
    let Some(base) = base_url(state) else {
        return Ok(());
    };
    let Some(row) = optLoadTopicRow(state, topic_id).await? else {
        return Ok(());
    };

    if row.deleted || row.draft || row.comments_hidden || row.premoderated_anonymous_uncommitted {
        vDeleteDoc(state, base, topic_id).await?;
    } else {
        let doc = MessageIndexDocument {
            section: row.section.clone(),
            topic_author: row.author.clone(),
            topic_id,
            author: row.author.clone(),
            group: row.group.clone(),
            title: Some(row.title.clone()),
            topic_title: row.title.clone(),
            message: markup::render_message_with_markup(&row.message, Some(&row.markup), None),
            postdate: row.postdate.to_rfc3339(),
            tag: row.tags.clone(),
            is_comment: false,
            topic_awaits_commit: row.awaits_commit,
        };
        vPutDoc(state, base, topic_id, &doc).await?;
    }

    if with_comments {
        let comment_ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM comments WHERE topic=$1")
            .bind(topic_id)
            .fetch_all(&state.pool)
            .await
            .map_err(|stError| stError.to_string())?;
        for id in comment_ids {
            vIndexComment(state, id).await?;
        }
    }
    Ok(())
}

pub async fn index_topic(state: &AppState, topic_id: i32, with_comments: bool) {
    if let Err(stError) = vEnqueue(
        state,
        &EnSearchQueueJob::Topic {
            id: topic_id,
            with_comments,
        },
    ) {
        tracing::warn!(error = %stError, id = topic_id, "failed to queue topic reindex");
    }
}

async fn vIndexComment(state: &AppState, comment_id: i32) -> Result<(), String> {
    let Some(base) = base_url(state) else {
        return Ok(());
    };
    let row: Option<TySearchCommentRow> = sqlx::query_as(
        r#"SELECT c.topic,c.title,u.nick,m.message,m.markup::text,c.postdate,c.deleted
           FROM comments c
           JOIN users u ON u.id=c.userid
           JOIN msgbase m ON m.id=c.id
           WHERE c.id=$1"#,
    )
    .bind(comment_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|stError| stError.to_string())?;
    let Some((topic_id, title, author, message, markup, postdate, deleted)) = row else {
        return Ok(());
    };
    let comment = CommentRow {
        topic_id,
        title,
        author,
        message,
        markup,
        postdate,
        deleted,
    };
    let Some(topic) = optLoadTopicRow(state, topic_id).await? else {
        vDeleteDoc(state, base, comment_id).await?;
        return Ok(());
    };

    if comment.deleted
        || topic.deleted
        || topic.draft
        || topic.comments_hidden
        || topic.premoderated_anonymous_uncommitted
    {
        vDeleteDoc(state, base, comment_id).await?;
        return Ok(());
    }

    let doc = stCommentIndexDocument(&topic, &comment);
    vPutDoc(state, base, comment_id, &doc).await
}

pub async fn index_comment(state: &AppState, comment_id: i32) {
    if let Err(stError) = vEnqueue(state, &EnSearchQueueJob::Comment { id: comment_id }) {
        tracing::warn!(error = %stError, id = comment_id, "failed to queue comment reindex");
    }
}

fn stQueueDirectory(stState: &AppState, sName: &str) -> std::path::PathBuf {
    std::path::Path::new(&stState.config.upload_dir)
        .join("search-queue")
        .join(sName)
}

fn vEnqueue(stState: &AppState, stJob: &EnSearchQueueJob) -> Result<(), String> {
    if base_url(stState).is_none() {
        return Ok(());
    }
    let stPending = stQueueDirectory(stState, "pending");
    std::fs::create_dir_all(&stPending).map_err(|stError| stError.to_string())?;
    let sId = uuid::Uuid::new_v4().simple().to_string();
    let stTemporary = stPending.join(format!(".{sId}.tmp"));
    let stReady = stPending.join(format!("{sId}.json"));
    let vecPayload = serde_json::to_vec(stJob).map_err(|stError| stError.to_string())?;
    std::fs::write(&stTemporary, vecPayload).map_err(|stError| stError.to_string())?;
    std::fs::rename(&stTemporary, &stReady).map_err(|stError| stError.to_string())?;
    Ok(())
}

/// Drain a bounded batch from the durable spool. Renaming is the claim
/// operation, so concurrent replicas cannot process the same ready file.
/// Failed OpenSearch operations are returned to `pending` for the next pass.
pub(crate) async fn vDrainQueue(stState: &AppState) -> Result<(), String> {
    if base_url(stState).is_none() {
        return Ok(());
    }
    let stPending = stQueueDirectory(stState, "pending");
    let stProcessing = stQueueDirectory(stState, "processing");
    let stFailed = stQueueDirectory(stState, "failed");
    for stDirectory in [&stPending, &stProcessing, &stFailed] {
        std::fs::create_dir_all(stDirectory).map_err(|stError| stError.to_string())?;
    }

    vReclaimStaleQueueJobs(&stPending, &stProcessing)?;
    let mut vecEntries = std::fs::read_dir(&stPending)
        .map_err(|stError| stError.to_string())?
        .filter_map(Result::ok)
        .filter(|stEntry| {
            stEntry
                .path()
                .extension()
                .is_some_and(|sExt| sExt == "json")
        })
        .take(100)
        .collect::<Vec<_>>();
    vecEntries.sort_by_key(|stEntry| stEntry.file_name());

    for stEntry in vecEntries {
        let stPendingFile = stEntry.path();
        let stProcessingFile = stProcessing.join(stEntry.file_name());
        if std::fs::rename(&stPendingFile, &stProcessingFile).is_err() {
            continue;
        }
        let stJob = match std::fs::read(&stProcessingFile)
            .map_err(|stError| stError.to_string())
            .and_then(|vecPayload| {
                serde_json::from_slice::<EnSearchQueueJob>(&vecPayload)
                    .map_err(|stError| stError.to_string())
            }) {
            Ok(stJob) => stJob,
            Err(stError) => {
                let _ = std::fs::rename(&stProcessingFile, stFailed.join(stEntry.file_name()));
                tracing::error!(error = %stError, file = ?stEntry.file_name(), "invalid search queue job quarantined");
                continue;
            }
        };
        let stResult = match stJob {
            EnSearchQueueJob::Topic { id, with_comments } => {
                vIndexTopic(stState, id, with_comments).await
            }
            EnSearchQueueJob::Comment { id } => vIndexComment(stState, id).await,
        };
        match stResult {
            Ok(()) => {
                std::fs::remove_file(&stProcessingFile).map_err(|stError| stError.to_string())?
            }
            Err(stError) => {
                std::fs::rename(&stProcessingFile, &stPendingFile)
                    .map_err(|stRenameError| stRenameError.to_string())?;
                return Err(stError);
            }
        }
    }
    Ok(())
}

fn vReclaimStaleQueueJobs(
    stPending: &std::path::Path,
    stProcessing: &std::path::Path,
) -> Result<(), String> {
    let stThreshold = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(10 * 60))
        .unwrap_or(std::time::UNIX_EPOCH);
    for stEntry in std::fs::read_dir(stProcessing).map_err(|stError| stError.to_string())? {
        let stEntry = stEntry.map_err(|stError| stError.to_string())?;
        let stPath = stEntry.path();
        let bStale = stEntry
            .metadata()
            .and_then(|stMetadata| stMetadata.modified())
            .is_ok_and(|stModified| stModified < stThreshold);
        if bStale {
            let _ = std::fs::rename(&stPath, stPending.join(stEntry.file_name()));
        }
    }
    Ok(())
}

async fn vDeleteDoc(state: &AppState, base: &str, id: i32) -> Result<(), String> {
    let stResponse = state
        .http
        .delete(format!("{base}/{INDEX}/_doc/{id}"))
        .send()
        .await
        .map_err(|stError| stError.to_string())?;
    if stResponse.status().is_success() || stResponse.status() == reqwest::StatusCode::NOT_FOUND {
        Ok(())
    } else {
        Err(format!(
            "OpenSearch delete #{id} failed with {}",
            stResponse.status()
        ))
    }
}

async fn vPutDoc(
    state: &AppState,
    base: &str,
    id: i32,
    doc: &MessageIndexDocument,
) -> Result<(), String> {
    state
        .http
        .put(format!("{base}/{INDEX}/_doc/{id}"))
        .json(doc)
        .send()
        .await
        .map_err(|stError| stError.to_string())?
        .error_for_status()
        .map_err(|stError| stError.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StReindexMonth {
    iYear: i32,
    iMonth: u32,
}

fn stPreviousMonth(stMonth: StReindexMonth) -> StReindexMonth {
    if stMonth.iMonth == 1 {
        StReindexMonth {
            iYear: stMonth.iYear - 1,
            iMonth: 12,
        }
    } else {
        StReindexMonth {
            iYear: stMonth.iYear,
            iMonth: stMonth.iMonth - 1,
        }
    }
}

fn vecRecentReindexMonths(stNow: chrono::DateTime<chrono_tz::Tz>) -> Vec<StReindexMonth> {
    let mut stMonth = StReindexMonth {
        iYear: stNow.year(),
        iMonth: stNow.month(),
    };
    let mut vecMonths = Vec::with_capacity(3);
    for _ in 0..3 {
        vecMonths.push(stMonth);
        stMonth = stPreviousMonth(stMonth);
    }
    vecMonths
}

fn vecAllReindexMonths(
    stNow: chrono::DateTime<chrono_tz::Tz>,
    stFirstTopic: chrono::DateTime<chrono::Utc>,
    stTimezone: chrono_tz::Tz,
) -> Vec<StReindexMonth> {
    let stFirstTopic = stFirstTopic.with_timezone(&stTimezone);
    let stFirstMonth = StReindexMonth {
        iYear: stFirstTopic.year(),
        iMonth: stFirstTopic.month(),
    };
    let mut stMonth = StReindexMonth {
        iYear: stNow.year(),
        iMonth: stNow.month(),
    };
    let mut vecMonths = Vec::new();
    while stMonth.iYear > stFirstMonth.iYear
        || (stMonth.iYear == stFirstMonth.iYear && stMonth.iMonth >= stFirstMonth.iMonth)
    {
        vecMonths.push(stMonth);
        stMonth = stPreviousMonth(stMonth);
    }

    // SearchControlController.reindexAll always enqueues this sentinel month
    // after the regular range so epoch-dated legacy messages are reconciled.
    vecMonths.push(StReindexMonth {
        iYear: 1970,
        iMonth: 1,
    });
    vecMonths
}

fn stServerTimezone() -> chrono_tz::Tz {
    std::env::var("TZ")
        .ok()
        .and_then(|sTimezone| sTimezone.parse().ok())
        .unwrap_or(chrono_tz::Europe::Moscow)
}

fn stMonthBounds(
    stMonth: StReindexMonth,
    stTimezone: chrono_tz::Tz,
) -> Result<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>), String> {
    let stStart = stTimezone
        .with_ymd_and_hms(stMonth.iYear, stMonth.iMonth, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| {
            format!(
                "invalid reindex month {:04}-{:02}",
                stMonth.iYear, stMonth.iMonth
            )
        })?;
    let stNextMonth = if stMonth.iMonth == 12 {
        StReindexMonth {
            iYear: stMonth.iYear + 1,
            iMonth: 1,
        }
    } else {
        StReindexMonth {
            iYear: stMonth.iYear,
            iMonth: stMonth.iMonth + 1,
        }
    };
    let stEnd = stTimezone
        .with_ymd_and_hms(stNextMonth.iYear, stNextMonth.iMonth, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| {
            format!(
                "invalid reindex month {:04}-{:02}",
                stNextMonth.iYear, stNextMonth.iMonth
            )
        })?;
    Ok((stStart.to_utc(), stEnd.to_utc()))
}

async fn vReindexMonth(
    stState: &AppState,
    stMonth: StReindexMonth,
    stTimezone: chrono_tz::Tz,
) -> Result<(u64, u64), String> {
    let (stStart, stEnd) = stMonthBounds(stMonth, stTimezone)?;
    // Java's TopicDao.getMessageForMonth deliberately includes deleted and
    // draft topics: reindexMessage then removes their stale topic/comment
    // documents. Filtering them here would leave stale search results.
    let vecTopicIds: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM topics WHERE postdate >= $1 AND postdate < $2 ORDER BY id",
    )
    .bind(stStart)
    .bind(stEnd)
    .fetch_all(&stState.pool)
    .await
    .map_err(|stError| stError.to_string())?;
    let iCommentCount: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
             FROM comments c
             JOIN topics t ON t.id=c.topic
            WHERE t.postdate >= $1 AND t.postdate < $2"#,
    )
    .bind(stStart)
    .bind(stEnd)
    .fetch_one(&stState.pool)
    .await
    .map_err(|stError| stError.to_string())?;

    let iTopicCount = vecTopicIds.len() as u64;
    for iTopicId in vecTopicIds {
        vIndexTopic(stState, iTopicId, true).await?;
    }
    Ok((iTopicCount, iCommentCount.max(0) as u64))
}

fn vSpawnReindex(stState: AppState, vecMonths: Vec<StReindexMonth>, stTimezone: chrono_tz::Tz) {
    tokio::spawn(async move {
        if let Err(stError) = vEnsureIndex(&stState).await {
            tracing::error!(error = %stError, "search reindex could not ensure the index");
            return;
        }

        for stMonth in vecMonths {
            let stStarted = std::time::Instant::now();
            match vReindexMonth(&stState, stMonth, stTimezone).await {
                Ok((iTopics, iComments)) => tracing::info!(
                    year = stMonth.iYear,
                    month = stMonth.iMonth,
                    topics = iTopics,
                    comments = iComments,
                    elapsed_ms = stStarted.elapsed().as_millis(),
                    "search reindex month completed"
                ),
                Err(stError) => tracing::error!(
                    error = %stError,
                    year = stMonth.iYear,
                    month = stMonth.iMonth,
                    "search reindex month failed"
                ),
            }
        }
    });
}

/// SearchControlController.reindexCurrentMonth: enqueue current and previous
/// two calendar months and return to the administrator immediately.
pub fn vScheduleCurrentReindex(stState: AppState) {
    let stTimezone = stServerTimezone();
    let stNow = chrono::Utc::now().with_timezone(&stTimezone);
    vSpawnReindex(stState, vecRecentReindexMonths(stNow), stTimezone);
}

/// SearchControlController.reindexAll: enqueue every month back to the first
/// non-epoch topic plus January 1970, without blocking the HTTP request for
/// the indexing work itself.
pub async fn vScheduleAllReindex(stState: AppState) -> Result<(), String> {
    let optFirstTopic: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT min(postdate) FROM topics WHERE postdate <> 'epoch'::timestamptz",
    )
    .fetch_one(&stState.pool)
    .await
    .map_err(|stError| stError.to_string())?;
    let stFirstTopic = optFirstTopic.ok_or_else(|| "no non-epoch topics to reindex".to_string())?;
    let stTimezone = stServerTimezone();
    let stNow = chrono::Utc::now().with_timezone(&stTimezone);
    vSpawnReindex(
        stState,
        vecAllReindexMonths(stNow, stFirstTopic, stTimezone),
        stTimezone,
    );
    Ok(())
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
    /// Java sends selected day bounds to OpenSearch as epoch-millisecond
    /// strings.  When present this filter takes precedence over `interval`.
    pub selected_day_ms: Option<(i64, i64)>,
    pub timezone: chrono_tz::Tz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSort {
    Relevance,
    Date,
    DateOldToNew,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchInterval {
    Month,
    ThreeMonth,
    Year,
    ThreeYear,
    All,
}

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
pub enum SearchRange {
    All,
    Topics,
    Comments,
}

pub const SEARCH_ROWS: i64 = 25;
pub const MAX_OFFSET: i64 = 10000 - SEARCH_ROWS;

#[derive(Debug, Deserialize)]
struct EsHit {
    _id: String,
    _score: Option<f64>,
    _source: EsSource,
    #[serde(default)]
    highlight: HashMap<String, Vec<String>>,
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
    pub title_html: String,
    pub url: String,
    pub author: String,
    pub postdate: String,
    pub postdate_iso: String,
    pub message_html: String,
    pub is_comment: bool,
    pub tags: Vec<SearchTag>,
    pub score: f64,
}

pub struct SearchTag {
    pub name: String,
    pub url: String,
}

pub struct FacetItem {
    pub key: String,
    pub label: String,
    pub selected: bool,
}

pub struct SearchResult {
    pub items: Vec<SearchItem>,
    pub total: i64,
    pub took_ms: i64,
    pub section_facet: Vec<FacetItem>,
    pub group_facet: Vec<FacetItem>,
    pub found_tags: Vec<SearchTag>,
    /// SearchService mutates an empty section when only one section bucket
    /// exists, so pagination and the group selector retain that section.
    pub effective_section: String,
}

#[derive(Debug, Clone)]
pub struct StSimilarTopic {
    pub title: String,
    pub link: String,
    pub year: i32,
    pub section: String,
}

type TySimilarCache = HashMap<i32, (std::time::Instant, Vec<Vec<StSimilarTopic>>)>;
static ST_SIMILAR_CACHE: Lazy<tokio::sync::RwLock<TySimilarCache>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));
const SIMILAR_CACHE_SIZE: usize = 10_000;
const SIMILAR_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

type TyActiveTagsCache = HashMap<(String, Option<String>), (std::time::Instant, Vec<String>)>;
static ST_ACTIVE_TAGS_CACHE: Lazy<tokio::sync::RwLock<TyActiveTagsCache>> =
    Lazy::new(|| tokio::sync::RwLock::new(HashMap::new()));
const ACTIVE_TAGS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(Debug, Deserialize)]
struct StSimilarHit {
    _id: String,
    _source: StSimilarSource,
}

#[derive(Debug, Deserialize)]
struct StSimilarSource {
    title: Option<String>,
    postdate: String,
    section: String,
    group: String,
}

// Lucene RussianAnalyzer's default stop set after the same empty-stop-set
// RussianAnalyzer normalization performed by MoreLikeThisService. Duplicates
// are intentional: Java collects every emitted token into a Vector.
const RUSSIAN_STOP_WORDS: &[&str] = &[
    "а",
    "без",
    "бол",
    "бы",
    "был",
    "был",
    "был",
    "был",
    "быт",
    "в",
    "вам",
    "вас",
    "ве",
    "во",
    "вот",
    "все",
    "всег",
    "всех",
    "вы",
    "где",
    "да",
    "даж",
    "для",
    "до",
    "ег",
    "е",
    "есл",
    "ест",
    "ещ",
    "же",
    "за",
    "зде",
    "и",
    "из",
    "ил",
    "им",
    "их",
    "к",
    "как",
    "ко",
    "когд",
    "кто",
    "ли",
    "либ",
    "мне",
    "может",
    "мы",
    "на",
    "над",
    "наш",
    "не",
    "нег",
    "не",
    "нет",
    "ни",
    "них",
    "но",
    "ну",
    "о",
    "об",
    "однак",
    "он",
    "он",
    "он",
    "он",
    "от",
    "очен",
    "по",
    "под",
    "при",
    "с",
    "со",
    "так",
    "так",
    "там",
    "те",
    "тем",
    "то",
    "тог",
    "тож",
    "то",
    "тольк",
    "том",
    "ты",
    "у",
    "уж",
    "хот",
    "чег",
    "че",
    "чем",
    "что",
    "чтоб",
    "чье",
    "чья",
    "эт",
    "эт",
    "эт",
    "я",
];

fn stSimilarRequestBody(iTopicId: i32, sTitle: &str, vecTags: &[String]) -> Value {
    let mut vecShould = vec![
        json!({"more_like_this": {
            "fields": ["title"],
            "like": [sTitle],
            "min_term_freq": 1,
            "min_doc_freq": 2,
            "stop_words": RUSSIAN_STOP_WORDS,
            "max_doc_freq": 5000
        }}),
        json!({"more_like_this": {
            "fields": ["message"],
            "like": [{"_index": INDEX, "_id": iTopicId.to_string()}],
            "min_term_freq": 1,
            "min_word_length": 3,
            "max_doc_freq": 100000
        }}),
    ];
    if !vecTags.is_empty() {
        vecShould.push(json!({"terms": {"tag": vecTags}}));
    }
    json!({
        "_source": ["title", "postdate", "section", "group"],
        "query": {"bool": {
            "should": vecShould,
            "filter": [
                {"term": {"is_comment": "false"}},
                {"term": {"topic_awaits_commit": "false"}}
            ],
            "minimum_should_match": "1",
            "must_not": [{"ids": {"values": [iTopicId.to_string()]}}]
        }}
    })
}

fn stRelatedTagsRequestBody(sTag: &str) -> Value {
    json!({
        "size": 0,
        "query": {"bool": {"filter": [
            {"term": {"is_comment": "false"}},
            {"term": {"tag": sTag}}
        ]}},
        "aggregations": {
            "related": {
                "significant_terms": {
                    "field": "tag",
                    "background_filter": {"term": {"is_comment": "false"}}
                }
            }
        }
    })
}

fn stActiveTagsRequestBody(sSection: &str, optGroup: Option<&str>) -> Value {
    let dtToday = chrono::Utc::now().date_naive();
    let dtOneYearAgo = dtToday
        .checked_sub_months(chrono::Months::new(12))
        .unwrap_or(dtToday);
    let dtTwoYearsAgo = dtToday
        .checked_sub_months(chrono::Months::new(24))
        .unwrap_or(dtToday);
    let mut vecFilters = vec![
        json!({"term": {"is_comment": "false"}}),
        json!({"term": {"section": sSection}}),
        json!({"range": {"postdate": {"gte": dtOneYearAgo.to_string()}}}),
    ];
    if let Some(sGroup) = optGroup {
        vecFilters.push(json!({"term": {"group": sGroup}}));
    }
    json!({
        "size": 0,
        "query": {"bool": {"filter": vecFilters}},
        "aggregations": {
            "active": {
                "significant_terms": {
                    "field": "tag",
                    "size": 15,
                    "min_doc_count": 5,
                    "background_filter": {"bool": {"filter": [
                        {"term": {"is_comment": "false"}},
                        {"term": {"section": sSection}},
                        {"range": {"postdate": {"gte": dtTwoYearsAgo.to_string()}}}
                    ]}}
                }
            }
        }
    })
}

/// `TagService.getActiveTopTags`: significant tags from the last year in a
/// section (and optionally a group), using two years of section topics as the
/// background. The Java service caches this result for fifteen minutes.
pub async fn vecActiveTopTags(
    stState: &AppState,
    sSection: &str,
    optGroup: Option<&str>,
) -> Result<Vec<String>, String> {
    let stKey = (sSection.to_owned(), optGroup.map(str::to_owned));
    if let Some((stCreated, vecCached)) = ST_ACTIVE_TAGS_CACHE.read().await.get(&stKey)
        && stCreated.elapsed() < ACTIVE_TAGS_CACHE_TTL
    {
        return Ok(vecCached.clone());
    }
    let Some(sBase) = base_url(stState) else {
        return Ok(Vec::new());
    };
    let stResponse = stState
        .http
        .post(format!("{sBase}/{INDEX}/_search"))
        .json(&stActiveTagsRequestBody(sSection, optGroup))
        .send()
        .await
        .map_err(|stError| stError.to_string())?;
    if !stResponse.status().is_success() {
        let stStatus = stResponse.status();
        let sBody = stResponse.text().await.unwrap_or_default();
        return Err(format!("active-tags OpenSearch error {stStatus}: {sBody}"));
    }
    let stPayload: Value = stResponse
        .json()
        .await
        .map_err(|stError| stError.to_string())?;
    let mut vecTags: Vec<String> = stPayload
        .pointer("/aggregations/active/buckets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|stBucket| stBucket.get("key").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    vecTags.sort();
    ST_ACTIVE_TAGS_CACHE
        .write()
        .await
        .insert(stKey, (std::time::Instant::now(), vecTags.clone()));
    Ok(vecTags)
}

/// `TagService.getRelatedTags`: significant tag terms among topic documents
/// carrying the selected tag, using all topic documents as the background.
pub async fn vecRelatedTags(stState: &AppState, sTag: &str) -> Result<Vec<String>, String> {
    let Some(sBase) = base_url(stState) else {
        return Ok(Vec::new());
    };
    let stResponse = stState
        .http
        .post(format!("{sBase}/{INDEX}/_search"))
        .json(&stRelatedTagsRequestBody(sTag))
        .send()
        .await
        .map_err(|stError| stError.to_string())?;
    if !stResponse.status().is_success() {
        let stStatus = stResponse.status();
        let sBody = stResponse.text().await.unwrap_or_default();
        return Err(format!("related-tags OpenSearch error {stStatus}: {sBody}"));
    }
    let stPayload: Value = stResponse
        .json()
        .await
        .map_err(|stError| stError.to_string())?;
    let mut vecTags: Vec<String> = stPayload
        .pointer("/aggregations/related/buckets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|stBucket| stBucket.get("key").and_then(Value::as_str))
        .filter(|sRelated| *sRelated != sTag)
        .map(str::to_owned)
        .collect();
    vecTags.sort();
    Ok(vecTags)
}

pub async fn vecSimilarTopics(
    stState: &AppState,
    iTopicId: i32,
    sTitle: &str,
    vecTags: &[String],
) -> Result<Vec<Vec<StSimilarTopic>>, String> {
    if let Some((stCreated, vecCached)) = ST_SIMILAR_CACHE.read().await.get(&iTopicId)
        && stCreated.elapsed() < SIMILAR_CACHE_TTL
    {
        return Ok(vecCached.clone());
    }
    let Some(sBase) = base_url(stState) else {
        return Ok(Vec::new());
    };
    let stResponse = stState
        .http
        .post(format!("{sBase}/{INDEX}/_search"))
        .json(&stSimilarRequestBody(iTopicId, sTitle, vecTags))
        .send()
        .await
        .map_err(|stError| stError.to_string())?;
    if !stResponse.status().is_success() {
        let stStatus = stResponse.status();
        let sBody = stResponse.text().await.unwrap_or_default();
        return Err(format!(
            "similar-topics OpenSearch error {stStatus}: {sBody}"
        ));
    }
    let stPayload: Value = stResponse
        .json()
        .await
        .map_err(|stError| stError.to_string())?;
    let vecHits: Vec<StSimilarHit> = stPayload
        .pointer("/hits/hits")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|stError| stError.to_string())?
        .unwrap_or_default();
    let vecSectionRows: Vec<(i32, String)> = sqlx::query_as("SELECT id,name FROM sections")
        .fetch_all(&stState.pool)
        .await
        .map_err(|stError| stError.to_string())?;
    let mapSections: HashMap<String, String> = vecSectionRows
        .into_iter()
        .map(|(iId, sName)| {
            let sUrl = match iId {
                1 => "news".to_owned(),
                2 => "forum".to_owned(),
                3 => "gallery".to_owned(),
                5 => "polls".to_owned(),
                6 => "articles".to_owned(),
                _ => sName.to_lowercase(),
            };
            (sUrl, sName)
        })
        .collect();
    let stTimezone = stServerTimezone();
    let vecTopics: Vec<StSimilarTopic> = vecHits
        .into_iter()
        .map(|stHit| {
            let stSource = stHit._source;
            let iYear = chrono::DateTime::parse_from_rfc3339(&stSource.postdate)
                .map(|dtValue| dtValue.with_timezone(&stTimezone).year())
                .unwrap_or_default();
            StSimilarTopic {
                title: stSource.title.unwrap_or_default(),
                link: format!("/{}/{}/{}", stSource.section, stSource.group, stHit._id),
                year: iYear,
                section: mapSections
                    .get(&stSource.section)
                    .cloned()
                    .unwrap_or(stSource.section),
            }
        })
        .collect();
    let iHalf = vecTopics.len().div_ceil(2);
    let vecColumns = if iHalf == 0 {
        Vec::new()
    } else {
        vecTopics.chunks(iHalf).map(<[_]>::to_vec).collect()
    };
    let mut mapCache = ST_SIMILAR_CACHE.write().await;
    mapCache.retain(|_, (stCreated, _)| stCreated.elapsed() < SIMILAR_CACHE_TTL);
    if mapCache.len() >= SIMILAR_CACHE_SIZE
        && let Some(iOldestKey) = mapCache
            .iter()
            .max_by_key(|(_, (stCreated, _))| stCreated.elapsed())
            .map(|(iKey, _)| *iKey)
    {
        mapCache.remove(&iOldestKey);
    }
    mapCache.insert(iTopicId, (std::time::Instant::now(), vecColumns.clone()));
    Ok(vecColumns)
}

fn stAndFilter(mut vecFilters: Vec<Value>) -> Value {
    match vecFilters.len() {
        0 => json!({"match_all": {}}),
        1 => vecFilters.pop().expect("one filter"),
        _ => json!({"bool": {"must": vecFilters}}),
    }
}

fn stSearchRequestBody(p: &SearchParams) -> Value {
    let mut query_filters: Vec<Value> = Vec::new();
    match p.range {
        SearchRange::Topics => query_filters.push(json!({"term": {"is_comment": false}})),
        SearchRange::Comments => query_filters.push(json!({"term": {"is_comment": true}})),
        SearchRange::All => {}
    }
    if let Some(user) = p.user.as_deref() {
        let field = if p.usertopic {
            "topic_author"
        } else {
            "author"
        };
        query_filters.push(json!({"term": {field: user}}));
    }
    if let Some((iStart, iEnd)) = p.selected_day_ms {
        query_filters.push(json!({"range": {"postdate": {
            "gte": iStart.to_string(),
            "lt": iEnd.to_string()
        }}}));
    } else if let Some(gte) = p.interval.gte_expr() {
        query_filters.push(json!({"range": {"postdate": {"gt": gte}}}));
    }

    let mut post_filters: Vec<Value> = Vec::new();
    if let Some(section) = p.section.as_deref().filter(|s| !s.is_empty()) {
        post_filters.push(json!({"term": {"section": section}}));
    }
    if let Some(group) = p.group.as_deref().filter(|s| !s.is_empty()) {
        post_filters.push(json!({"term": {"group": group}}));
    }

    let text_query = if p.q.is_empty() {
        json!({"match_all": {}})
    } else {
        json!({
            "bool": {
                "must": [{"bool": {
                    "should": [
                        {"match": {"title": {"query": p.q, "minimum_should_match": "2"}}},
                        {"match": {"message": {"query": p.q, "minimum_should_match": "2"}}}
                    ],
                    "minimum_should_match": "1"
                }}],
                "should": [
                    {"match_phrase": {"message": p.q}},
                    {"match_phrase": {"title": p.q}},
                    {"match": {"message.raw": {"query": p.q, "minimum_should_match": "2"}}}
                ],
                "minimum_should_match": "0"
            }
        })
    };

    let boosted_query = json!({
        "function_score": {
            "query": text_query,
            "functions": [{
                "weight": 2.0,
                "filter": {"range": {"postdate": {"gte": "now/d-3y"}}}
            }]
        }
    });
    let query = if query_filters.is_empty() {
        boosted_query
    } else {
        json!({"bool": {"must": [boosted_query], "filter": query_filters}})
    };

    let sort = match p.sort {
        SearchSort::Relevance => {
            json!([{"_score": {"order": "desc"}}, {"postdate": {"order": "desc"}}])
        }
        SearchSort::Date => json!([{"postdate": {"order": "desc"}}]),
        SearchSort::DateOldToNew => json!([{"postdate": {"order": "asc"}}]),
    };

    let mut body = json!({
        "_source": [
            "title", "topic_title", "author", "postdate", "topic_id",
            "section", "message", "group", "is_comment", "tag"
        ],
        "query": query,
        "sort": sort,
        "from": p.offset,
        "size": SEARCH_ROWS,
        "aggs": {
            "sections": {
                "terms": {"field": "section", "size": 50},
                "aggs": {"groups": {"terms": {"field": "group", "size": 50}}}
            },
            "tags": {
                "significant_terms": {"field": "tag", "min_doc_count": 30}
            }
        },
        "highlight": {
            "pre_tags": ["<em class=search-hl>"],
            "post_tags": ["</em>"],
            "require_field_match": false,
            "fields": {
                "title": {"number_of_fragments": 0},
                "topicTitle": {"number_of_fragments": 0},
                "message": {
                    "type": "fvh",
                    "number_of_fragments": 1,
                    "fragment_size": 16384
                }
            }
        },
        "timeout": "60s",
        "track_total_hits": true
    });
    // Java always serializes post_filter: match_all for the empty case, the
    // sole query as-is, and bool.must for multiple filters.
    body["post_filter"] = stAndFilter(post_filters);
    body
}

fn sSanitizeSearchHtml(sHtml: &str) -> String {
    let mut stBuilder = ammonia::Builder::default();
    stBuilder.add_generic_attributes(&["class"]);
    stBuilder.clean(sHtml).to_string()
}

fn sFirstHighlight(mapHighlight: &HashMap<String, Vec<String>>, sField: &str) -> Option<String> {
    mapHighlight
        .get(sField)
        .and_then(|vecFragments| vecFragments.first())
        .cloned()
}

fn stTag(sName: String) -> SearchTag {
    SearchTag {
        url: format!("/tag/{}", urlencoding::encode(&sName)),
        name: sName,
    }
}

pub async fn search(state: &AppState, p: &SearchParams) -> Result<SearchResult, String> {
    let Some(base) = base_url(state) else {
        return Err("Поиск временно недоступен: не сконфигурирован OPENSEARCH_URL".into());
    };
    let body = stSearchRequestBody(p);

    let resp = state
        .http
        .post(format!("{base}/{INDEX}/_search"))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("opensearch error {status}: {text}"));
    }
    let payload: Value = resp.json().await.map_err(|e| e.to_string())?;

    let took_ms = payload.get("took").and_then(|v| v.as_i64()).unwrap_or(0);
    let total = payload
        .pointer("/hits/total/value")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let hits: Vec<EsHit> = payload
        .pointer("/hits/hits")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .unwrap_or(None)
        .unwrap_or_default();

    let items = hits
        .into_iter()
        .map(|h| {
            let s = h._source;
            let url = if s.is_comment {
                format!("/{}/{}/{}?cid={}", s.section, s.group, s.topic_id, h._id)
            } else {
                format!("/{}/{}/{}", s.section, s.group, s.topic_id)
            };
            let title_html = sFirstHighlight(&h.highlight, "title")
                .or_else(|| {
                    s.title
                        .map(|sTitle| html_escape::encode_text(&sTitle).into_owned())
                })
                .filter(|sTitle| !sTitle.trim().is_empty())
                .or_else(|| sFirstHighlight(&h.highlight, "topic_title"))
                .unwrap_or_else(|| html_escape::encode_text(&s.topic_title).into_owned());
            let sMessage = sFirstHighlight(&h.highlight, "message")
                .unwrap_or_else(|| s.message.chars().take(16384).collect());
            let tags = if s.is_comment {
                Vec::new()
            } else {
                s.tag.into_iter().map(stTag).collect()
            };
            let (postdate, postdate_iso) = match chrono::DateTime::parse_from_rfc3339(&s.postdate) {
                Ok(dtPostdate) => (
                    dtPostdate
                        .with_timezone(&p.timezone)
                        .format("%d.%m.%y %H:%M:%S %Z")
                        .to_string(),
                    dtPostdate.to_rfc3339(),
                ),
                Err(_) => (s.postdate.clone(), s.postdate),
            };
            SearchItem {
                title_html: sSanitizeSearchHtml(&title_html),
                url,
                author: s.author,
                postdate,
                postdate_iso,
                message_html: sSanitizeSearchHtml(&sMessage),
                is_comment: s.is_comment,
                tags,
                score: h._score.unwrap_or(0.0),
            }
        })
        .collect();

    let vecSections: Vec<(i32, String)> = sqlx::query_as("SELECT id,name FROM sections")
        .fetch_all(&state.pool)
        .await
        .map_err(|stError| stError.to_string())?;
    let mapSectionNames: HashMap<String, String> = vecSections
        .into_iter()
        .map(|(iId, sName)| {
            let sKey = match iId {
                1 => "news".to_owned(),
                2 => "forum".to_owned(),
                3 => "gallery".to_owned(),
                5 => "polls".to_owned(),
                6 => "articles".to_owned(),
                _ => sName.to_lowercase(),
            };
            (sKey, sName.to_lowercase())
        })
        .collect();
    let vecGroups: Vec<(String, String, String)> = sqlx::query_as(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum'
                    WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls'
                    WHEN 6 THEN 'articles' ELSE lower(s.name) END,
                  g.urlname, lower(g.title)
             FROM groups g JOIN sections s ON s.id=g.section"#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|stError| stError.to_string())?;
    let mapGroupNames: HashMap<(String, String), String> = vecGroups
        .into_iter()
        .map(|(sSection, sGroup, sTitle)| ((sSection, sGroup), sTitle))
        .collect();

    let mut section_facet = Vec::new();
    let mut group_facet = Vec::new();
    let mut effective_section = p.section.clone().unwrap_or_default();
    if let Some(buckets) = payload
        .pointer("/aggregations/sections/buckets")
        .and_then(|v| v.as_array())
    {
        let iAllCount = buckets
            .iter()
            .filter_map(|stBucket| stBucket.get("doc_count").and_then(Value::as_i64))
            .sum::<i64>();
        if buckets.len() > 1 || !effective_section.is_empty() {
            section_facet.push(FacetItem {
                key: String::new(),
                label: format!("все ({iAllCount})"),
                selected: effective_section.is_empty(),
            });
            if !effective_section.is_empty()
                && !buckets.iter().any(|b| {
                    b.get("key").and_then(Value::as_str) == Some(effective_section.as_str())
                })
            {
                let sLabel = mapSectionNames
                    .get(&effective_section)
                    .cloned()
                    .unwrap_or_else(|| effective_section.clone());
                section_facet.push(FacetItem {
                    key: effective_section.clone(),
                    label: format!("{sLabel} (0)"),
                    selected: true,
                });
            }
        } else if effective_section.is_empty() && buckets.len() == 1 {
            effective_section = buckets[0]
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
        }

        for b in buckets {
            let key = b
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let count = b.get("doc_count").and_then(|v| v.as_i64()).unwrap_or(0);
            if !section_facet.is_empty() {
                let sLabel = mapSectionNames
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| key.clone());
                section_facet.push(FacetItem {
                    key: key.clone(),
                    label: format!("{sLabel} ({count})"),
                    selected: effective_section == key,
                });
            }
            if effective_section == key
                && let Some(gbuckets) = b.pointer("/groups/buckets").and_then(|v| v.as_array())
            {
                for gb in gbuckets {
                    let gkey = gb
                        .get("key")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let gcount = gb.get("doc_count").and_then(|v| v.as_i64()).unwrap_or(0);
                    let sLabel = mapGroupNames
                        .get(&(key.clone(), gkey.clone()))
                        .cloned()
                        .unwrap_or_else(|| gkey.clone());
                    group_facet.push(FacetItem {
                        selected: p.group.as_deref() == Some(gkey.as_str()),
                        key: gkey,
                        label: format!("{sLabel} ({gcount})"),
                    });
                }
            }
        }

        if !effective_section.is_empty() {
            let sSelectedGroup = p.group.clone().unwrap_or_default();
            if !sSelectedGroup.is_empty()
                && !group_facet
                    .iter()
                    .any(|stFacet| stFacet.key == sSelectedGroup)
            {
                let sLabel = mapGroupNames
                    .get(&(effective_section.clone(), sSelectedGroup.clone()))
                    .cloned()
                    .unwrap_or_else(|| sSelectedGroup.clone());
                group_facet.push(FacetItem {
                    key: sSelectedGroup,
                    label: format!("{sLabel} (0)"),
                    selected: true,
                });
            }
            if group_facet.len() > 1 || !p.group.as_deref().unwrap_or_default().is_empty() {
                group_facet.insert(
                    0,
                    FacetItem {
                        key: String::new(),
                        label: format!(
                            "все ({})",
                            buckets
                                .iter()
                                .find(|b| {
                                    b.get("key").and_then(Value::as_str)
                                        == Some(effective_section.as_str())
                                })
                                .and_then(|b| b.get("doc_count"))
                                .and_then(Value::as_i64)
                                .unwrap_or(0)
                        ),
                        selected: p.group.as_deref().unwrap_or_default().is_empty(),
                    },
                );
            } else {
                group_facet.clear();
            }
        }
    }

    let found_tags = payload
        .pointer("/aggregations/tags/buckets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|stBucket| stBucket.get("key").and_then(Value::as_str))
        .map(|sName| stTag(sName.to_owned()))
        .collect();

    Ok(SearchResult {
        items,
        total,
        took_ms,
        section_facet,
        group_facet,
        found_tags,
        effective_section,
    })
}

use crate::markup;

#[cfg(test)]
mod moderation_semantics_tests {
    use chrono::{TimeZone, Utc};

    use super::{
        CommentRow, EnSearchQueueJob, SearchInterval, SearchParams, SearchRange, SearchSort,
        StReindexMonth, TopicRow, stActiveTagsRequestBody, stCommentIndexDocument,
        stIndexDefinition, stRelatedTagsRequestBody, stSearchRequestBody, stSimilarRequestBody,
        topic_awaits_commit, vecAllReindexMonths, vecIndexContractProblems, vecRecentReindexMonths,
    };

    #[test]
    fn index_definition_matches_java_analysis_and_term_vectors() {
        let stDefinition = stIndexDefinition();
        let stOperationalDefinition: serde_json::Value =
            serde_json::from_str(include_str!("../compat/java-runtime/messages-index.json"))
                .unwrap();

        assert_eq!(
            stDefinition, stOperationalDefinition,
            "runtime creation and guarded rebuild must use one Java-compatible mapping"
        );

        assert_eq!(
            stDefinition.pointer("/settings/analysis/analyzer/text_analyzer/filter"),
            Some(&serde_json::json!([
                "m_long_word",
                "lowercase",
                "m_my_snow_ru",
                "m_my_snow_en"
            ]))
        );
        assert_eq!(
            stDefinition.pointer("/settings/analysis/char_filter/m_ee/mappings"),
            Some(&serde_json::json!(["ё => е", "Ё => Е"]))
        );
        assert_eq!(
            stDefinition.pointer("/mappings/properties/topic_title/index"),
            Some(&serde_json::json!(false))
        );
        assert_eq!(
            stDefinition.pointer("/mappings/properties/message/term_vector"),
            Some(&serde_json::json!("with_positions_offsets"))
        );
        assert_eq!(
            stDefinition.pointer("/mappings/properties/message/fields/raw/analyzer"),
            Some(&serde_json::json!("exact_analyzer"))
        );
    }

    #[test]
    fn existing_index_must_match_the_java_mapping_before_search_is_served() {
        let stDefinition = stIndexDefinition();
        let mut stExisting = serde_json::json!({
            "messages": {"mappings": stDefinition["mappings"].clone()}
        });
        assert!(vecIndexContractProblems(&stExisting).is_empty());

        stExisting["messages"]["mappings"]["properties"]["message"]
            .as_object_mut()
            .unwrap()
            .remove("term_vector");
        let vecProblems = vecIndexContractProblems(&stExisting);
        assert_eq!(vecProblems.len(), 1);
        assert!(vecProblems[0].starts_with("message/term_vector:"));
    }

    #[test]
    fn durable_queue_payload_round_trips_all_write_shapes() {
        for stJob in [
            EnSearchQueueJob::Topic {
                id: 42,
                with_comments: true,
            },
            EnSearchQueueJob::Comment { id: 43 },
        ] {
            let vecPayload = serde_json::to_vec(&stJob).unwrap();
            assert_eq!(
                serde_json::from_slice::<EnSearchQueueJob>(&vecPayload).unwrap(),
                stJob
            );
        }
    }

    #[test]
    fn current_reindex_schedules_exactly_three_months_across_year_boundary() {
        let stNow = chrono_tz::Europe::Moscow
            .with_ymd_and_hms(2026, 1, 20, 12, 0, 0)
            .unwrap();

        assert_eq!(
            vecRecentReindexMonths(stNow),
            vec![
                StReindexMonth {
                    iYear: 2026,
                    iMonth: 1
                },
                StReindexMonth {
                    iYear: 2025,
                    iMonth: 12
                },
                StReindexMonth {
                    iYear: 2025,
                    iMonth: 11
                }
            ]
        );
    }

    #[test]
    fn full_reindex_reaches_first_topic_month_and_appends_epoch_sentinel() {
        let stTimezone = chrono_tz::Europe::Moscow;
        let stNow = stTimezone.with_ymd_and_hms(2026, 3, 20, 12, 0, 0).unwrap();
        let stFirstTopic = Utc.with_ymd_and_hms(2025, 5, 10, 0, 0, 0).unwrap();

        let vecMonths = vecAllReindexMonths(stNow, stFirstTopic, stTimezone);

        assert_eq!(
            vecMonths.first(),
            Some(&StReindexMonth {
                iYear: 2026,
                iMonth: 3
            })
        );
        assert_eq!(
            vecMonths.get(vecMonths.len() - 2),
            Some(&StReindexMonth {
                iYear: 2025,
                iMonth: 5
            })
        );
        assert_eq!(
            vecMonths.last(),
            Some(&StReindexMonth {
                iYear: 1970,
                iMonth: 1
            })
        );
    }

    #[test]
    fn only_uncommitted_topics_in_premoderated_sections_await_commit() {
        assert!(topic_awaits_commit(true, false));
        assert!(!topic_awaits_commit(true, true));
        assert!(!topic_awaits_commit(false, false));
        assert!(!topic_awaits_commit(false, true));
    }

    #[test]
    fn comment_documents_use_comment_author_and_postdate() {
        let stTopic = TopicRow {
            section: "forum".to_owned(),
            group: "linux".to_owned(),
            author: "topic-author".to_owned(),
            title: "Topic".to_owned(),
            message: String::new(),
            markup: "LORCODE".to_owned(),
            postdate: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            tags: vec!["rust".to_owned()],
            deleted: false,
            draft: false,
            awaits_commit: false,
            comments_hidden: false,
            premoderated_anonymous_uncommitted: false,
        };
        let stComment = CommentRow {
            topic_id: 42,
            title: "Комментарий".to_owned(),
            author: "comment-author".to_owned(),
            message: "[b]Body[/b]".to_owned(),
            markup: "LORCODE".to_owned(),
            postdate: Utc.with_ymd_and_hms(2026, 2, 3, 4, 5, 6).unwrap(),
            deleted: false,
        };

        let stDocument = stCommentIndexDocument(&stTopic, &stComment);

        assert_eq!(stDocument.author, "comment-author");
        assert_eq!(stDocument.topic_author, "topic-author");
        assert_eq!(stDocument.postdate, "2026-02-03T04:05:06+00:00");
        assert_eq!(stDocument.message, "<p><strong>Body</strong></p>");
    }

    fn stSearchParams() -> SearchParams {
        SearchParams {
            q: "rust search".to_owned(),
            section: Some("forum".to_owned()),
            group: Some("linux-org-ru".to_owned()),
            user: Some("tester".to_owned()),
            usertopic: true,
            sort: SearchSort::Relevance,
            interval: SearchInterval::Month,
            range: SearchRange::Topics,
            offset: 25,
            selected_day_ms: None,
            timezone: chrono_tz::Europe::Moscow,
        }
    }

    #[test]
    fn search_request_matches_java_query_boost_highlight_and_aggregations() {
        let stBody = stSearchRequestBody(&stSearchParams());

        assert_eq!(
            stBody.pointer("/query/bool/must/0/function_score/functions/0/weight"),
            Some(&serde_json::json!(2.0))
        );
        assert_eq!(
            stBody.pointer(
                "/query/bool/must/0/function_score/query/bool/should/2/match/message.raw/minimum_should_match"
            ),
            Some(&serde_json::json!("2"))
        );
        assert_eq!(
            stBody.pointer("/highlight/fields/message/type"),
            Some(&serde_json::json!("fvh"))
        );
        assert_eq!(
            stBody.pointer("/aggs/tags/significant_terms/min_doc_count"),
            Some(&serde_json::json!(30))
        );
        assert_eq!(
            stBody.pointer("/post_filter/bool/must/1/term/group"),
            Some(&serde_json::json!("linux-org-ru"))
        );
        assert_eq!(stBody.pointer("/timeout"), Some(&serde_json::json!("60s")));
    }

    #[test]
    fn active_tags_query_matches_java_section_group_and_background_scope() {
        let stBody = stActiveTagsRequestBody("gallery", Some("screenshots"));

        assert_eq!(
            stBody.pointer("/query/bool/filter/0/term/is_comment"),
            Some(&serde_json::json!("false"))
        );
        assert_eq!(
            stBody.pointer("/query/bool/filter/1/term/section"),
            Some(&serde_json::json!("gallery"))
        );
        assert_eq!(
            stBody.pointer("/query/bool/filter/3/term/group"),
            Some(&serde_json::json!("screenshots"))
        );
        assert_eq!(
            stBody.pointer("/aggregations/active/significant_terms/field"),
            Some(&serde_json::json!("tag"))
        );
        assert_eq!(
            stBody.pointer("/aggregations/active/significant_terms/size"),
            Some(&serde_json::json!(15))
        );
        assert_eq!(
            stBody.pointer("/aggregations/active/significant_terms/min_doc_count"),
            Some(&serde_json::json!(5))
        );
        assert_eq!(
            stBody.pointer(
                "/aggregations/active/significant_terms/background_filter/bool/filter/1/term/section"
            ),
            Some(&serde_json::json!("gallery"))
        );
        assert!(
            stBody
                .pointer("/aggregations/active/significant_terms/background_filter/bool/filter/3")
                .is_none(),
            "the Java background scope is the whole section, not the selected group"
        );
    }

    #[test]
    fn selected_date_replaces_interval_filter_with_java_epoch_millisecond_bounds() {
        let mut stParams = stSearchParams();
        stParams.selected_day_ms = Some((1_700_000_000_000, 1_700_086_400_000));
        let stBody = stSearchRequestBody(&stParams);
        let vecFilters = stBody
            .pointer("/query/bool/filter")
            .and_then(serde_json::Value::as_array)
            .unwrap();

        assert!(vecFilters.iter().any(|stFilter| {
            stFilter.pointer("/range/postdate/gte") == Some(&serde_json::json!("1700000000000"))
                && stFilter.pointer("/range/postdate/lt")
                    == Some(&serde_json::json!("1700086400000"))
        }));
        assert!(!vecFilters.iter().any(|stFilter| {
            stFilter.pointer("/range/postdate/gt") == Some(&serde_json::json!("now/h-1M"))
        }));
    }

    #[test]
    fn similar_topics_query_matches_java_mlt_tags_visibility_and_self_exclusion() {
        let stBody = stSimilarRequestBody(
            42,
            "Rust search topic",
            &["rust".to_owned(), "opensearch".to_owned()],
        );

        assert_eq!(
            stBody.pointer("/query/bool/should/0/more_like_this/fields/0"),
            Some(&serde_json::json!("title"))
        );
        assert_eq!(
            stBody.pointer("/query/bool/should/1/more_like_this/like/0/_id"),
            Some(&serde_json::json!("42"))
        );
        assert_eq!(
            stBody.pointer("/query/bool/should/2/terms/tag"),
            Some(&serde_json::json!(["rust", "opensearch"]))
        );
        assert_eq!(
            stBody.pointer("/query/bool/filter/1/term/topic_awaits_commit"),
            Some(&serde_json::json!("false"))
        );
        assert_eq!(
            stBody.pointer("/query/bool/must_not/0/ids/values/0"),
            Some(&serde_json::json!("42"))
        );
    }

    #[test]
    fn related_tags_query_matches_java_significant_terms_scope() {
        let stBody = stRelatedTagsRequestBody("rust");
        assert_eq!(stBody["size"], serde_json::json!(0));
        assert_eq!(
            stBody.pointer("/query/bool/filter/0/term/is_comment"),
            Some(&serde_json::json!("false"))
        );
        assert_eq!(
            stBody.pointer("/query/bool/filter/1/term/tag"),
            Some(&serde_json::json!("rust"))
        );
        assert_eq!(
            stBody.pointer("/aggregations/related/significant_terms/field"),
            Some(&serde_json::json!("tag"))
        );
        assert_eq!(
            stBody.pointer(
                "/aggregations/related/significant_terms/background_filter/term/is_comment"
            ),
            Some(&serde_json::json!("false"))
        );
    }
}
