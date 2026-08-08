use crate::{
    application::topic::CTopicService, error::Result,
    infra::postgres::topic_repository::CTopicPgRepository, state::AppState,
};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, header},
};
use serde::Deserialize;

fn iDefaultSection() -> i32 {
    1
}

#[derive(Deserialize)]
pub struct RssQuery {
    #[serde(default = "iDefaultSection")]
    pub section: i32,
    #[serde(default)]
    pub group: i32,
    pub filter: Option<String>,
}

pub async fn section_rss(
    State(stState): State<AppState>,
    Query(stQuery): Query<RssQuery>,
) -> Result<(HeaderMap, String)> {
    let cTopicService = CTopicService::new(CTopicPgRepository::new(stState.pool.clone()));
    let stFeed = cTopicService
        .stRssFeed(stQuery.section, stQuery.group, stQuery.filter.as_deref())
        .await?;
    let sPublicUrl = stState.config.public_url.trim_end_matches('/');
    let sTitle = format!("Linux.org.ru: {}", stFeed.sTitle);
    let mut sBody =
        String::from(r#"<?xml version="1.0" encoding="utf-8"?><rss version="2.0"><channel>"#);
    sBody.push_str("<link>");
    sBody.push_str(&html_escape::encode_text(&format!("{sPublicUrl}/")));
    sBody.push_str("</link><language>ru</language><title>");
    sBody.push_str(&html_escape::encode_text(&sTitle));
    sBody.push_str("</title><description>");
    sBody.push_str(&html_escape::encode_text(&sTitle));
    sBody.push_str("</description><pubDate>");
    sBody.push_str(&chrono::Utc::now().to_rfc2822());
    sBody.push_str("</pubDate>");
    for stTopic in stFeed.vecTopics {
        let sLink = format!("{sPublicUrl}{}", stTopic.topic_url());
        sBody.push_str("<item><author>");
        sBody.push_str(&html_escape::encode_text(&stTopic.author));
        sBody.push_str("</author><link>");
        sBody.push_str(&html_escape::encode_text(&sLink));
        sBody.push_str("</link><guid>");
        sBody.push_str(&html_escape::encode_text(&sLink));
        sBody.push_str("</guid><title>");
        sBody.push_str(&html_escape::encode_text(&stTopic.title));
        sBody.push_str("</title><pubDate>");
        sBody.push_str(&stTopic.postdate.to_rfc2822());
        sBody.push_str("</pubDate></item>");
    }
    sBody.push_str("</channel></rss>");
    let mut stHeaders = HeaderMap::new();
    stHeaders.insert(
        header::CONTENT_TYPE,
        "application/rss+xml; charset=utf-8".parse().unwrap(),
    );
    Ok((stHeaders, sBody))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_section_matches_java_controller() {
        assert_eq!(iDefaultSection(), 1);
    }
}
