use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::json;

use crate::{
    domain::{comment::deletion::TrCommentReindexQueue, topic::options::TrTopicReindexQueue},
    error::{AppError, Result},
};

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
    )
}

fn vWriteJob(pathUploadRoot: &Path, stPayload: serde_json::Value) -> Result<()> {
    let pathPending = pathUploadRoot.join("search-queue/pending");
    std::fs::create_dir_all(&pathPending)?;
    let sId = uuid::Uuid::new_v4().simple().to_string();
    let pathTemporary = pathPending.join(format!(".{sId}.tmp"));
    let pathReady = pathPending.join(format!("{sId}.json"));
    let vecPayload =
        serde_json::to_vec(&stPayload).map_err(|stError| AppError::Anyhow(stError.into()))?;
    std::fs::write(&pathTemporary, vecPayload)?;
    std::fs::rename(pathTemporary, pathReady)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
