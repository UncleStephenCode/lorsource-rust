use std::collections::BTreeMap;

pub mod tag_topic_count;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    domain::user::statistics::{
        StUserSectionCount, StUserTopicStatistics, TrUserStatisticsRepository, TyUserYearStats,
    },
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

    fn sBaseUrl(&self) -> Result<&str> {
        self.optBaseUrl
            .as_deref()
            .ok_or_else(|| AppError::Anyhow(anyhow::anyhow!("OPENSEARCH_URL is not configured")))
    }
}

#[derive(Debug, Deserialize)]
struct StCountResponse {
    count: i64,
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

#[derive(Debug, Deserialize)]
struct StTopicStatsResponse {
    #[serde(default)]
    timed_out: bool,
    aggregations: StTopicStatsAggregations,
}

#[derive(Debug, Deserialize)]
struct StTopicStatsAggregations {
    topic_stats: StTopicDateStats,
    sections: StTopicSectionBuckets,
}

#[derive(Debug, Deserialize)]
struct StTopicDateStats {
    count: i64,
    min: Option<f64>,
    max: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct StTopicSectionBuckets {
    buckets: Vec<StTopicSectionBucket>,
}

#[derive(Debug, Deserialize)]
struct StTopicSectionBucket {
    key: String,
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

fn stCommentCountQuery(sNick: &str) -> Value {
    json!({
        "query": {
            "bool": {
                "filter": [
                    {"term": {"author": {"value": sNick}}},
                    {"term": {"is_comment": {"value": true}}}
                ]
            }
        }
    })
}

fn stTopicStatsQuery(sNick: &str) -> Value {
    json!({
        "size": 0,
        "timeout": "5s",
        "query": {
            "bool": {
                "filter": [
                    {"term": {"author": {"value": sNick}}},
                    {"term": {"is_comment": {"value": false}}}
                ]
            }
        },
        "aggs": {
            "topic_stats": {
                "stats": {"field": "postdate"}
            },
            "sections": {
                "terms": {"field": "section", "size": 1000}
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

fn dtFromOpenSearchMillis(fValue: f64) -> Result<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp_millis(fValue as i64).ok_or_else(|| {
        AppError::Anyhow(anyhow::anyhow!(
            "OpenSearch returned an out-of-range postdate statistic"
        ))
    })
}

fn stMapTopicStatistics(stResponse: StTopicStatsResponse) -> Result<StUserTopicStatistics> {
    if stResponse.timed_out {
        return Err(AppError::Anyhow(anyhow::anyhow!(
            "OpenSearch topic statistics request timed out"
        )));
    }

    let stDateStats = stResponse.aggregations.topic_stats;
    let (optFirstTopic, optLastTopic) = if stDateStats.count > 0 {
        let fMin = stDateStats.min.ok_or_else(|| {
            AppError::Anyhow(anyhow::anyhow!(
                "OpenSearch omitted the minimum topic postdate"
            ))
        })?;
        let fMax = stDateStats.max.ok_or_else(|| {
            AppError::Anyhow(anyhow::anyhow!(
                "OpenSearch omitted the maximum topic postdate"
            ))
        })?;
        (
            Some(dtFromOpenSearchMillis(fMin)?),
            Some(dtFromOpenSearchMillis(fMax)?),
        )
    } else {
        (None, None)
    };
    let vecSectionCounts = stResponse
        .aggregations
        .sections
        .buckets
        .into_iter()
        .map(|stBucket| StUserSectionCount {
            sSectionUrlName: stBucket.key,
            iCount: stBucket.doc_count,
        })
        .collect();

    Ok(StUserTopicStatistics {
        optFirstTopic,
        optLastTopic,
        vecSectionCounts,
    })
}

#[async_trait]
impl TrUserStatisticsRepository for CUserStatisticsOpenSearchRepository {
    async fn mapYearStats(&self, sNick: &str, sTimezone: &str) -> Result<TyUserYearStats> {
        let sBaseUrl = self.sBaseUrl()?;
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

    async fn iCommentCount(&self, sNick: &str) -> Result<i64> {
        let sBaseUrl = self.sBaseUrl()?;
        let stResponse = self
            .oHttp
            .post(format!("{sBaseUrl}/{S_MESSAGE_INDEX}/_count"))
            .json(&stCommentCountQuery(sNick))
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

    async fn stTopicStatistics(&self, sNick: &str) -> Result<StUserTopicStatistics> {
        let sBaseUrl = self.sBaseUrl()?;
        let stResponse = self
            .oHttp
            .post(format!("{sBaseUrl}/{S_MESSAGE_INDEX}/_search"))
            .json(&stTopicStatsQuery(sNick))
            .send()
            .await
            .map_err(|stError| AppError::Anyhow(stError.into()))?
            .error_for_status()
            .map_err(|stError| AppError::Anyhow(stError.into()))?
            .json::<StTopicStatsResponse>()
            .await
            .map_err(|stError| AppError::Anyhow(stError.into()))?;
        stMapTopicStatistics(stResponse)
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
    fn comment_count_query_matches_java_boolean_filters() {
        let stQuery = stCommentCountQuery("test-user");
        assert_eq!(
            stQuery.pointer("/query/bool/filter/0/term/author/value"),
            Some(&json!("test-user"))
        );
        assert_eq!(
            stQuery.pointer("/query/bool/filter/1/term/is_comment/value"),
            Some(&json!(true))
        );
        assert!(stQuery.get("size").is_none());
    }

    #[test]
    fn topic_query_matches_java_filters_timeout_and_aggregations() {
        let stQuery = stTopicStatsQuery("test-user");
        assert_eq!(stQuery["size"], 0);
        assert_eq!(stQuery["timeout"], "5s");
        assert_eq!(
            stQuery.pointer("/query/bool/filter/0/term/author/value"),
            Some(&json!("test-user"))
        );
        assert_eq!(
            stQuery.pointer("/query/bool/filter/1/term/is_comment/value"),
            Some(&json!(false))
        );
        assert_eq!(
            stQuery.pointer("/aggs/topic_stats/stats/field"),
            Some(&json!("postdate"))
        );
        assert_eq!(
            stQuery.pointer("/aggs/sections/terms/field"),
            Some(&json!("section"))
        );
        assert_eq!(
            stQuery.pointer("/aggs/sections/terms/size"),
            Some(&json!(1000))
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

    #[test]
    fn topic_statistics_decode_dates_and_section_url_names() {
        let stResponse: StTopicStatsResponse = serde_json::from_value(json!({
            "timed_out": false,
            "aggregations": {
                "topic_stats": {
                    "count": 5,
                    "min": 1_725_148_800_123_f64,
                    "max": 1_725_235_200_987_f64,
                    "avg": 0,
                    "sum": 0
                },
                "sections": {
                    "buckets": [
                        {"key": "forum", "doc_count": 4},
                        {"key": "articles", "doc_count": 1}
                    ]
                }
            }
        }))
        .expect("valid OpenSearch response");

        let stActual = stMapTopicStatistics(stResponse).unwrap();
        assert_eq!(
            stActual.optFirstTopic.unwrap().timestamp_millis(),
            1_725_148_800_123
        );
        assert_eq!(
            stActual.optLastTopic.unwrap().timestamp_millis(),
            1_725_235_200_987
        );
        assert_eq!(
            stActual.vecSectionCounts,
            vec![
                StUserSectionCount {
                    sSectionUrlName: "forum".to_owned(),
                    iCount: 4,
                },
                StUserSectionCount {
                    sSectionUrlName: "articles".to_owned(),
                    iCount: 1,
                }
            ]
        );
    }

    #[test]
    fn successful_empty_topic_statistics_are_not_an_error() {
        let stResponse: StTopicStatsResponse = serde_json::from_value(json!({
            "timed_out": false,
            "aggregations": {
                "topic_stats": {"count": 0, "min": null, "max": null},
                "sections": {"buckets": []}
            }
        }))
        .unwrap();

        let stActual = stMapTopicStatistics(stResponse).unwrap();
        assert!(stActual.optFirstTopic.is_none());
        assert!(stActual.optLastTopic.is_none());
        assert!(stActual.vecSectionCounts.is_empty());
    }

    #[test]
    fn opensearch_timed_out_flag_is_a_recoverable_search_failure() {
        let stResponse: StTopicStatsResponse = serde_json::from_value(json!({
            "timed_out": true,
            "aggregations": {
                "topic_stats": {"count": 0, "min": null, "max": null},
                "sections": {"buckets": []}
            }
        }))
        .unwrap();

        assert!(matches!(
            stMapTopicStatistics(stResponse),
            Err(AppError::Anyhow(_))
        ));
    }
}
