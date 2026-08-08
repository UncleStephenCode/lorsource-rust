use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    domain::user::statistics::{TrUserStatisticsRepository, TyUserYearStats},
    error::{AppError, Result},
};

const S_MESSAGE_INDEX: &str = "messages";

#[derive(Debug, Clone)]
pub struct CUserStatisticsOpenSearchRepository {
    optBaseUrl: Option<String>,
    oHttp: reqwest::Client,
}

impl CUserStatisticsOpenSearchRepository {
    pub fn new(optBaseUrl: Option<String>, oHttp: reqwest::Client) -> Self {
        Self { optBaseUrl, oHttp }
    }
}

#[derive(Debug, Deserialize)]
struct StYearStatsResponse {
    aggregations: StYearStatsAggregations,
}

#[derive(Debug, Deserialize)]
struct StYearStatsAggregations {
    days: StYearStatsDays,
}

#[derive(Debug, Deserialize)]
struct StYearStatsDays {
    buckets: Vec<StYearStatsBucket>,
}

#[derive(Debug, Deserialize)]
struct StYearStatsBucket {
    key: i64,
    doc_count: i64,
}

fn stYearStatsQuery(sNick: &str, sTimezone: &str) -> Value {
    json!({
        "size": 0,
        "query": {
            "bool": {
                "filter": [
                    {"term": {"author": {"value": sNick}}},
                    {"range": {"postdate": {"gt": "now-1y/M"}}}
                ]
            }
        },
        "aggs": {
            "days": {
                "date_histogram": {
                    "field": "postdate",
                    "time_zone": sTimezone,
                    "calendar_interval": "day",
                    "min_doc_count": 1
                }
            }
        }
    })
}

fn mapYearStats(stResponse: StYearStatsResponse) -> TyUserYearStats {
    stResponse
        .aggregations
        .days
        .buckets
        .into_iter()
        // OpenSearch returns date-histogram keys in milliseconds, while
        // cal-heatmap 3.x consumes epoch seconds, exactly as in Java.
        .map(|stBucket| (stBucket.key / 1000, stBucket.doc_count))
        .collect::<BTreeMap<_, _>>()
}

#[async_trait]
impl TrUserStatisticsRepository for CUserStatisticsOpenSearchRepository {
    async fn mapYearStats(&self, sNick: &str, sTimezone: &str) -> Result<TyUserYearStats> {
        let sBaseUrl = self
            .optBaseUrl
            .as_deref()
            .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("OPENSEARCH_URL is not configured")))?;
        let stResponse = self
            .oHttp
            .post(format!("{sBaseUrl}/{S_MESSAGE_INDEX}/_search"))
            .json(&stYearStatsQuery(sNick, sTimezone))
            .send()
            .await
            .map_err(|stError| AppError::Anyhow(stError.into()))?;
        let stResponse = stResponse
            .error_for_status()
            .map_err(|stError| AppError::Anyhow(stError.into()))?
            .json::<StYearStatsResponse>()
            .await
            .map_err(|stError| AppError::Anyhow(stError.into()))?;
        Ok(mapYearStats(stResponse))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_matches_java_year_stats_aggregation() {
        let stQuery = stYearStatsQuery("test-user", "Europe/Moscow");

        assert_eq!(stQuery["size"], 0);
        assert_eq!(
            stQuery.pointer("/query/bool/filter/0/term/author/value"),
            Some(&json!("test-user"))
        );
        assert_eq!(
            stQuery.pointer("/query/bool/filter/1/range/postdate/gt"),
            Some(&json!("now-1y/M"))
        );
        assert_eq!(
            stQuery.pointer("/aggs/days/date_histogram/time_zone"),
            Some(&json!("Europe/Moscow"))
        );
        assert_eq!(
            stQuery.pointer("/aggs/days/date_histogram/calendar_interval"),
            Some(&json!("day"))
        );
        assert_eq!(
            stQuery.pointer("/aggs/days/date_histogram/min_doc_count"),
            Some(&json!(1))
        );
    }

    #[test]
    fn histogram_milliseconds_become_cal_heatmap_epoch_seconds() {
        let stResponse: StYearStatsResponse = serde_json::from_value(json!({
            "aggregations": {
                "days": {
                    "buckets": [
                        {"key": 1_725_148_800_000_i64, "doc_count": 3},
                        {"key": 1_725_235_200_000_i64, "doc_count": 8}
                    ]
                }
            }
        }))
        .expect("valid OpenSearch response");

        assert_eq!(
            mapYearStats(stResponse),
            BTreeMap::from([(1_725_148_800, 3), (1_725_235_200, 8)])
        );
    }
}
