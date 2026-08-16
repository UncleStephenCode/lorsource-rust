use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    domain::tag::repository::TrTagTopicCountRepository,
    error::{AppError, Result},
};

const S_MESSAGE_INDEX: &str = "messages";

#[derive(Debug, Clone)]
pub struct CTagTopicCountOpenSearchRepository {
    optBaseUrl: Option<String>,
    oHttp: reqwest::Client,
}

impl CTagTopicCountOpenSearchRepository {
    pub fn new(optBaseUrl: Option<String>, oHttp: reqwest::Client) -> Self {
        Self { optBaseUrl, oHttp }
    }
}

#[derive(Debug, Deserialize)]
struct StCountResponse {
    count: i64,
}

fn stTagCountQuery(sTag: &str, sSectionUrlName: &str) -> Value {
    // TagService.countTagTopics uses string FieldValues even for the boolean
    // OpenSearch fields. Preserve that serialized request contract.
    json!({
        "query": {
            "bool": {
                "filter": [
                    {"term": {"is_comment": {"value": "false"}}},
                    {"term": {"tag": {"value": sTag}}},
                    {"term": {"topic_awaits_commit": {"value": "false"}}},
                    {"term": {"section": {"value": sSectionUrlName}}}
                ]
            }
        }
    })
}

#[async_trait]
impl TrTagTopicCountRepository for CTagTopicCountOpenSearchRepository {
    async fn iCountTagTopics(&self, sTag: &str, sSectionUrlName: &str) -> Result<i64> {
        let sBaseUrl = self
            .optBaseUrl
            .as_deref()
            .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("OPENSEARCH_URL is not configured")))?;
        let stResponse = self
            .oHttp
            .post(format!("{sBaseUrl}/{S_MESSAGE_INDEX}/_count"))
            .json(&stTagCountQuery(sTag, sSectionUrlName))
            .send()
            .await
            .map_err(|stError| AppError::Anyhow(stError.into()))?
            .error_for_status()
            .map_err(|stError| AppError::Anyhow(stError.into()))?
            .json::<StCountResponse>()
            .await
            .map_err(|stError| AppError::Anyhow(stError.into()))?;
        Ok(stResponse.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_query_matches_java_tag_section_filters() {
        let stQuery = stTagCountQuery("rust", "forum");
        let vecFilters = stQuery
            .pointer("/query/bool/filter")
            .and_then(Value::as_array)
            .expect("filter array");
        assert_eq!(vecFilters.len(), 4);
        assert_eq!(
            stQuery.pointer("/query/bool/filter/0/term/is_comment/value"),
            Some(&json!("false"))
        );
        assert_eq!(
            stQuery.pointer("/query/bool/filter/1/term/tag/value"),
            Some(&json!("rust"))
        );
        assert_eq!(
            stQuery.pointer("/query/bool/filter/2/term/topic_awaits_commit/value"),
            Some(&json!("false"))
        );
        assert_eq!(
            stQuery.pointer("/query/bool/filter/3/term/section/value"),
            Some(&json!("forum"))
        );
    }
}
