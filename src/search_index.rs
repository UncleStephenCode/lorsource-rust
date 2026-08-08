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
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
        return Ok(());
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
    Ok(())
}

pub async fn ensure_index(state: &AppState) {
    if let Err(stError) = vEnsureIndex(state).await {
        tracing::warn!(error = %stError, "failed to ensure opensearch index");
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
    postdate: chrono::DateTime<chrono::Utc>,
    deleted: bool,
}

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
        message: markup::plain_text_for_index(&stComment.message),
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
                  g.urlname, u.nick, u.id, t.title, m.message, t.postdate, t.deleted, COALESCE(t.draft,false), t.moderate,
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
        title,
        message,
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
            message: markup::plain_text_for_index(&row.message),
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
    let row: Option<(
        i32,
        String,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
        bool,
    )> = sqlx::query_as(
        r#"SELECT c.topic,c.title,u.nick,m.message,c.postdate,c.deleted
           FROM comments c
           JOIN users u ON u.id=c.userid
           JOIN msgbase m ON m.id=c.id
           WHERE c.id=$1"#,
    )
    .bind(comment_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|stError| stError.to_string())?;
    let Some((topic_id, title, author, message, postdate, deleted)) = row else {
        return Ok(());
    };
    let comment = CommentRow {
        topic_id,
        title,
        author,
        message,
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

pub struct FacetItem {
    pub key: String,
    pub label: String,
}

pub struct SearchResult {
    pub items: Vec<SearchItem>,
    pub total: i64,
    pub took_ms: i64,
    pub section_facet: Vec<FacetItem>,
    pub group_facet: Vec<FacetItem>,
}

pub async fn search(state: &AppState, p: &SearchParams) -> Result<SearchResult, String> {
    let Some(base) = base_url(state) else {
        return Err("Поиск временно недоступен: не сконфигурирован OPENSEARCH_URL".into());
    };

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
        let field = if p.usertopic {
            "topic_author"
        } else {
            "author"
        };
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
        SearchSort::Relevance => {
            json!([{"_score": {"order": "desc"}}, {"postdate": {"order": "desc"}}])
        }
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
            let title = s
                .title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(s.topic_title);
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
        })
        .collect();

    let mut section_facet = Vec::new();
    let mut group_facet = Vec::new();
    if let Some(buckets) = payload
        .pointer("/aggregations/sections/buckets")
        .and_then(|v| v.as_array())
    {
        for b in buckets {
            let key = b
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let count = b.get("doc_count").and_then(|v| v.as_i64()).unwrap_or(0);
            section_facet.push(FacetItem {
                key: key.clone(),
                label: format!("{key} ({count})"),
            });
            if p.section.as_deref() == Some(key.as_str())
                && let Some(gbuckets) = b.pointer("/groups/buckets").and_then(|v| v.as_array())
            {
                for gb in gbuckets {
                    let gkey = gb
                        .get("key")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let gcount = gb.get("doc_count").and_then(|v| v.as_i64()).unwrap_or(0);
                    group_facet.push(FacetItem {
                        key: gkey.clone(),
                        label: format!("{gkey} ({gcount})"),
                    });
                }
            }
        }
    }

    Ok(SearchResult {
        items,
        total,
        took_ms,
        section_facet,
        group_facet,
    })
}

use crate::markup;

#[cfg(test)]
mod moderation_semantics_tests {
    use chrono::{TimeZone, Utc};

    use super::{
        CommentRow, EnSearchQueueJob, StReindexMonth, TopicRow, stCommentIndexDocument,
        stIndexDefinition, topic_awaits_commit, vecAllReindexMonths, vecRecentReindexMonths,
    };

    #[test]
    fn index_definition_matches_java_analysis_and_term_vectors() {
        let stDefinition = stIndexDefinition();

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
            message: "Body".to_owned(),
            postdate: Utc.with_ymd_and_hms(2026, 2, 3, 4, 5, 6).unwrap(),
            deleted: false,
        };

        let stDocument = stCommentIndexDocument(&stTopic, &stComment);

        assert_eq!(stDocument.author, "comment-author");
        assert_eq!(stDocument.topic_author, "topic-author");
        assert_eq!(stDocument.postdate, "2026-02-03T04:05:06+00:00");
    }
}
