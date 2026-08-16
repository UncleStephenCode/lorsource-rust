use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use async_trait::async_trait;
use serde_json::json;

use crate::{
    domain::{comment::deletion::TrCommentReindexQueue, topic::options::TrTopicReindexQueue},
    error::{AppError, Result},
};

static I_LAST_QUEUE_TIMESTAMP_MICROS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnSearchQueuePriority {
    Normal,
    Low,
}

impl EnSearchQueuePriority {
    fn sFilePrefix(self) -> &'static str {
        match self {
            Self::Normal => "h",
            Self::Low => "l",
        }
    }
}

/// Filesystem-backed equivalent of SearchQueueSender.updateMessage.  The
/// background OpenSearch worker already consumes this durable JSON contract.
#[derive(Debug, Clone)]
pub struct CSearchQueueSender {
    bEnabled: bool,
    pathUploadRoot: PathBuf,
}

impl CSearchQueueSender {
    pub fn new(optOpenSearchUrl: Option<&str>, pathUploadRoot: impl Into<PathBuf>) -> Self {
        Self {
            bEnabled: optOpenSearchUrl.is_some(),
            pathUploadRoot: pathUploadRoot.into(),
        }
    }

    /// Filesystem-backed equivalent of repeated low-priority Java
    /// `SearchQueueSender.updateMonth` calls. Preserve
    /// `SearchControlController`'s newest-to-oldest enqueue order.
    /// The loop deliberately stops on the first durable-spool failure, just
    /// as a failed synchronous JMS send aborts the Java controller action.
    pub async fn vUpdateMonths(&self, vecMonths: &[(i32, u32)]) -> Result<()> {
        if !self.bEnabled {
            return Ok(());
        }
        let pathUploadRoot = self.pathUploadRoot.clone();
        let vecMonths = vecMonths.to_vec();
        tokio::task::spawn_blocking(move || {
            for (iYear, iMonth) in vecMonths {
                vWriteMonthJob(&pathUploadRoot, iYear, iMonth)?;
            }
            Ok(())
        })
        .await
        .map_err(|stError| AppError::Anyhow(stError.into()))?
    }
}

#[async_trait]
impl TrTopicReindexQueue for CSearchQueueSender {
    async fn vUpdateMessage(&self, iTopicId: i32, bWithComments: bool) -> Result<()> {
        if !self.bEnabled {
            return Ok(());
        }
        let pathUploadRoot = self.pathUploadRoot.clone();
        tokio::task::spawn_blocking(move || {
            vWriteTopicJob(&pathUploadRoot, iTopicId, bWithComments)
        })
        .await
        .map_err(|stError| AppError::Anyhow(stError.into()))?
    }
}

#[async_trait]
impl TrCommentReindexQueue for CSearchQueueSender {
    async fn vUpdateComments(&self, vecCommentIds: &[i32]) -> Result<()> {
        if !self.bEnabled {
            return Ok(());
        }
        let pathUploadRoot = self.pathUploadRoot.clone();
        let vecCommentIds = vecCommentIds.to_vec();
        tokio::task::spawn_blocking(move || {
            vWriteJob(
                &pathUploadRoot,
                json!({"kind":"comments","ids":vecCommentIds}),
                EnSearchQueuePriority::Normal,
            )
        })
        .await
        .map_err(|stError| AppError::Anyhow(stError.into()))?
    }
}

fn vWriteTopicJob(pathUploadRoot: &Path, iTopicId: i32, bWithComments: bool) -> Result<()> {
    vWriteJob(
        pathUploadRoot,
        json!({
            "kind": "topic",
            "id": iTopicId,
            "with_comments": bWithComments,
        }),
        EnSearchQueuePriority::Normal,
    )
}

fn vWriteMonthJob(pathUploadRoot: &Path, iYear: i32, iMonth: u32) -> Result<()> {
    vWriteJob(
        pathUploadRoot,
        json!({"kind":"month","year":iYear,"month":iMonth}),
        EnSearchQueuePriority::Low,
    )
}

fn vWriteJob(
    pathUploadRoot: &Path,
    stPayload: serde_json::Value,
    enPriority: EnSearchQueuePriority,
) -> Result<()> {
    let sId = format!(
        "{}-{:020}-{}",
        enPriority.sFilePrefix(),
        iNextQueueTimestampMicros(),
        uuid::Uuid::new_v4().simple()
    );
    vWriteJobWithId(pathUploadRoot, stPayload, &sId)
}

fn iNextQueueTimestampMicros() -> u64 {
    let iNow = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u64::MAX as u128) as u64;
    let mut iPrevious = I_LAST_QUEUE_TIMESTAMP_MICROS.load(Ordering::SeqCst);
    loop {
        let iNext = iNow.max(iPrevious.saturating_add(1));
        match I_LAST_QUEUE_TIMESTAMP_MICROS.compare_exchange(
            iPrevious,
            iNext,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return iNext,
            Err(iCurrent) => iPrevious = iCurrent,
        }
    }
}

fn vWriteJobWithId(pathUploadRoot: &Path, stPayload: serde_json::Value, sId: &str) -> Result<()> {
    let pathPending = pathUploadRoot.join("search-queue/pending");
    std::fs::create_dir_all(&pathPending)?;
    let pathTemporary = pathPending.join(format!(".{sId}.tmp"));
    let pathReady = pathPending.join(format!("{sId}.json"));
    let vecPayload =
        serde_json::to_vec(&stPayload).map_err(|stError| AppError::Anyhow(stError.into()))?;

    let mut stOptions = OpenOptions::new();
    stOptions.write(true).create_new(true);
    #[cfg(unix)]
    stOptions.mode(0o600);

    let mut oTemporary = stOptions.open(&pathTemporary)?;
    let mut stTemporaryGuard = StTemporaryFileGuard::new(pathTemporary.clone());
    let stWriteResult = oTemporary
        .write_all(&vecPayload)
        .and_then(|()| oTemporary.sync_all());
    drop(oTemporary);
    if let Err(stError) = stWriteResult {
        return vReturnAfterTemporaryCleanup(&mut stTemporaryGuard, stError);
    }

    if let Err(stError) = std::fs::rename(&pathTemporary, &pathReady) {
        return vReturnAfterTemporaryCleanup(&mut stTemporaryGuard, stError);
    }
    stTemporaryGuard.vDisarm();
    vSyncDirectory(&pathPending)?;
    Ok(())
}

#[derive(Debug)]
struct StTemporaryFileGuard {
    pathTemporary: PathBuf,
    bArmed: bool,
}

impl StTemporaryFileGuard {
    fn new(pathTemporary: PathBuf) -> Self {
        Self {
            pathTemporary,
            bArmed: true,
        }
    }

    fn vCleanup(&mut self) -> io::Result<()> {
        if !self.bArmed {
            return Ok(());
        }
        match std::fs::remove_file(&self.pathTemporary) {
            Ok(()) => {
                self.bArmed = false;
                Ok(())
            }
            Err(stError) if stError.kind() == io::ErrorKind::NotFound => {
                self.bArmed = false;
                Ok(())
            }
            Err(stError) => Err(stError),
        }
    }

    fn vDisarm(&mut self) {
        self.bArmed = false;
    }
}

impl Drop for StTemporaryFileGuard {
    fn drop(&mut self) {
        let _ = self.vCleanup();
    }
}

fn vReturnAfterTemporaryCleanup(
    stTemporaryGuard: &mut StTemporaryFileGuard,
    stOriginalError: io::Error,
) -> Result<()> {
    if let Err(stCleanupError) = stTemporaryGuard.vCleanup() {
        return Err(AppError::Anyhow(anyhow::anyhow!(
            "search queue operation failed ({stOriginalError}); temporary-file cleanup failed ({stCleanupError})"
        )));
    }
    Err(stOriginalError.into())
}

#[cfg(unix)]
fn vSyncDirectory(pathDirectory: &Path) -> io::Result<()> {
    std::fs::File::open(pathDirectory)?.sync_all()
}

#[cfg(not(unix))]
fn vSyncDirectory(_pathDirectory: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pathTestRoot(sCase: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lor-search-queue-{sCase}-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    #[tokio::test]
    async fn writes_the_existing_durable_topic_with_comments_contract() {
        let pathRoot = std::env::temp_dir().join(format!(
            "lor-setpostscore-queue-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let oQueue = CSearchQueueSender::new(Some("http://opensearch:9200"), &pathRoot);
        oQueue.vUpdateMessage(42, true).await.unwrap();
        let pathPending = pathRoot.join("search-queue/pending");
        let vecFiles = std::fs::read_dir(&pathPending)
            .unwrap()
            .map(|stEntry| stEntry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(vecFiles.len(), 1);
        let stPayload: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&vecFiles[0]).unwrap()).unwrap();
        assert_eq!(
            stPayload,
            json!({"kind":"topic","id":42,"with_comments":true})
        );
        std::fs::remove_dir_all(pathRoot).unwrap();
    }

    #[tokio::test]
    async fn disabled_search_has_the_same_noop_behavior_as_the_main_queue() {
        let pathRoot = std::env::temp_dir().join(format!(
            "lor-setpostscore-disabled-{}",
            uuid::Uuid::new_v4().simple()
        ));
        CSearchQueueSender::new(None, &pathRoot)
            .vUpdateMessage(42, true)
            .await
            .unwrap();
        assert!(!pathRoot.exists());
    }

    #[tokio::test]
    async fn writes_one_java_style_comment_batch_including_an_empty_race_result() {
        for vecIds in [vec![7, 8, 5], Vec::new()] {
            let pathRoot = std::env::temp_dir().join(format!(
                "lor-comment-delete-queue-{}",
                uuid::Uuid::new_v4().simple()
            ));
            let oQueue = CSearchQueueSender::new(Some("http://opensearch:9200"), &pathRoot);
            oQueue.vUpdateComments(&vecIds).await.unwrap();
            let vecFiles = std::fs::read_dir(pathRoot.join("search-queue/pending"))
                .unwrap()
                .map(|stEntry| stEntry.unwrap().path())
                .collect::<Vec<_>>();
            assert_eq!(vecFiles.len(), 1);
            let stPayload: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&vecFiles[0]).unwrap()).unwrap();
            assert_eq!(stPayload, json!({"kind":"comments","ids":vecIds}));
            std::fs::remove_dir_all(pathRoot).unwrap();
        }
    }

    #[tokio::test]
    async fn month_jobs_are_low_priority_durable_and_keep_controller_enqueue_order() {
        let pathRoot = pathTestRoot("month-order");
        let oQueue = CSearchQueueSender::new(Some("http://opensearch:9200"), &pathRoot);
        oQueue
            .vUpdateMonths(&[(2026, 1), (2025, 12), (2025, 11), (1970, 1)])
            .await
            .unwrap();

        let mut vecFiles = std::fs::read_dir(pathRoot.join("search-queue/pending"))
            .unwrap()
            .map(|stEntry| stEntry.unwrap().path())
            .collect::<Vec<_>>();
        vecFiles.sort();
        assert_eq!(vecFiles.len(), 4);
        assert!(vecFiles.iter().all(|pathFile| {
            pathFile
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("l-")
        }));
        let vecPayloads = vecFiles
            .iter()
            .map(|pathFile| {
                serde_json::from_slice::<serde_json::Value>(&std::fs::read(pathFile).unwrap())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vecPayloads,
            vec![
                json!({"kind":"month","year":2026,"month":1}),
                json!({"kind":"month","year":2025,"month":12}),
                json!({"kind":"month","year":2025,"month":11}),
                json!({"kind":"month","year":1970,"month":1}),
            ]
        );

        std::fs::remove_dir_all(pathRoot).unwrap();
    }

    #[tokio::test]
    async fn normal_jobs_sort_before_low_priority_month_jobs() {
        let pathRoot = pathTestRoot("priority");
        let oQueue = CSearchQueueSender::new(Some("http://opensearch:9200"), &pathRoot);
        oQueue.vUpdateMonths(&[(2026, 8)]).await.unwrap();
        oQueue.vUpdateMessage(42, true).await.unwrap();

        let mut vecNames = std::fs::read_dir(pathRoot.join("search-queue/pending"))
            .unwrap()
            .map(|stEntry| stEntry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        vecNames.sort();
        assert!(vecNames[0].starts_with("h-"));
        assert!(vecNames[1].starts_with("l-"));

        std::fs::remove_dir_all(pathRoot).unwrap();
    }

    #[tokio::test]
    async fn month_schedule_reports_spool_failure_instead_of_detaching_it() {
        let pathRoot = pathTestRoot("month-failure");
        std::fs::write(&pathRoot, b"not a directory").unwrap();
        let oQueue = CSearchQueueSender::new(Some("http://opensearch:9200"), &pathRoot);

        assert!(oQueue.vUpdateMonths(&[(2026, 8), (2026, 7)]).await.is_err());

        std::fs::remove_file(pathRoot).unwrap();
    }

    #[tokio::test]
    async fn durable_comment_send_reports_spool_failures_to_the_caller() {
        let pathRoot = std::env::temp_dir().join(format!(
            "lor-comment-queue-failure-{}",
            uuid::Uuid::new_v4().simple()
        ));
        // A regular file cannot contain search-queue/pending. This pins the
        // fallible post-commit contract used by comment create/edit routes.
        std::fs::write(&pathRoot, b"not a directory").unwrap();
        let oQueue = CSearchQueueSender::new(Some("http://opensearch:9200"), &pathRoot);
        assert!(oQueue.vUpdateComments(&[42]).await.is_err());
        std::fs::remove_file(pathRoot).unwrap();
    }

    #[test]
    fn durable_writer_publishes_a_private_ready_file_without_a_temporary_artifact() {
        let pathRoot = pathTestRoot("publish");
        vWriteJobWithId(&pathRoot, json!({"kind":"topic","id":42}), "fixed").unwrap();

        let pathPending = pathRoot.join("search-queue/pending");
        let pathReady = pathPending.join("fixed.json");
        assert!(!pathPending.join(".fixed.tmp").exists());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&pathReady).unwrap())
                .unwrap(),
            json!({"kind":"topic","id":42})
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(pathReady).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        std::fs::remove_dir_all(pathRoot).unwrap();
    }

    #[test]
    fn durable_writer_cleans_its_temporary_file_when_publish_fails() {
        let pathRoot = pathTestRoot("rename-failure");
        let pathPending = pathRoot.join("search-queue/pending");
        std::fs::create_dir_all(pathPending.join("fixed.json")).unwrap();

        assert!(vWriteJobWithId(&pathRoot, json!({"kind":"topic","id":42}), "fixed").is_err());
        assert!(!pathPending.join(".fixed.tmp").exists());
        assert!(pathPending.join("fixed.json").is_dir());

        std::fs::remove_dir_all(pathRoot).unwrap();
    }

    #[test]
    fn durable_writer_never_overwrites_or_removes_a_foreign_temporary_file() {
        let pathRoot = pathTestRoot("temporary-collision");
        let pathPending = pathRoot.join("search-queue/pending");
        std::fs::create_dir_all(&pathPending).unwrap();
        let pathTemporary = pathPending.join(".fixed.tmp");
        std::fs::write(&pathTemporary, b"foreign writer").unwrap();

        assert!(vWriteJobWithId(&pathRoot, json!({"kind":"topic","id":42}), "fixed").is_err());
        assert_eq!(std::fs::read(pathTemporary).unwrap(), b"foreign writer");
        assert!(!pathPending.join("fixed.json").exists());

        std::fs::remove_dir_all(pathRoot).unwrap();
    }
}
