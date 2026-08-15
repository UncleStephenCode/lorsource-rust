use crate::{
    application::topic::{CTopicService, StRssFeed},
    auth::CurrentUser,
    error::Result,
    infra::postgres::topic_repository::CTopicPgRepository,
    state::AppState,
};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use chrono_tz::Europe::Moscow;
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
    CurrentUser(optUser): CurrentUser,
    stRequestHeaders: HeaderMap,
    Query(stQuery): Query<RssQuery>,
) -> Result<Response> {
    let cTopicService = CTopicService::new(CTopicPgRepository::new(stState.pool.clone()));
    let stSource = cTopicService
        .stRssSource(
            stQuery.section,
            stQuery.group,
            stQuery.filter.as_deref(),
            optUser.as_ref().map(|stUser| stUser.id),
        )
        .await?;
    if let Some(stResponse) = optNotModifiedResponse(&stRequestHeaders, stSource.optLastModified) {
        return Ok(stResponse);
    }
    let stMarkupUsers = stState
        .markup
        .stResolveBatch(
            stSource
                .vecTopics
                .iter()
                .map(|stTopic| (&*stTopic.sMessage, &*stTopic.sMarkup)),
        )
        .await?;
    let stFeed = cTopicService
        .stPrepareRssFeedWithUsers(
            stSource,
            &stState.config.public_url,
            &stState.config.upload_dir,
            Some(&stMarkupUsers),
        )
        .await?;
    Ok(stRssResponse(
        stFeed,
        &stRequestHeaders,
        &stState.config.public_url,
        Utc::now(),
    ))
}

fn stRssResponse(
    stFeed: StRssFeed,
    stRequestHeaders: &HeaderMap,
    sPublicUrl: &str,
    dtNow: DateTime<Utc>,
) -> Response {
    if let Some(stResponse) = optNotModifiedResponse(stRequestHeaders, stFeed.optLastModified) {
        return stResponse;
    }
    let mut stResponseHeaders = HeaderMap::new();
    if let Some(dtLastModified) = stFeed.optLastModified {
        stResponseHeaders.insert(
            header::LAST_MODIFIED,
            HeaderValue::from_str(&httpdate::fmt_http_date(dtLastModified.into()))
                .expect("HTTP date is a valid header value"),
        );
    }
    stResponseHeaders.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/rss+xml; charset=utf-8"),
    );
    let sBody = sRenderRss(&stFeed, sPublicUrl, dtNow);
    (stResponseHeaders, sBody).into_response()
}

fn optNotModifiedResponse(
    stRequestHeaders: &HeaderMap,
    optLastModified: Option<DateTime<Utc>>,
) -> Option<Response> {
    let dtLastModified = optLastModified?;
    if !bRequestNotModified(stRequestHeaders, dtLastModified) {
        return None;
    }
    let mut stHeaders = HeaderMap::new();
    stHeaders.insert(
        header::LAST_MODIFIED,
        HeaderValue::from_str(&httpdate::fmt_http_date(dtLastModified.into()))
            .expect("HTTP date is a valid header value"),
    );
    Some((StatusCode::NOT_MODIFIED, stHeaders).into_response())
}

fn bRequestNotModified(stRequestHeaders: &HeaderMap, dtLastModified: DateTime<Utc>) -> bool {
    let Some(stIfModifiedSince) = stRequestHeaders
        .get(header::IF_MODIFIED_SINCE)
        .and_then(|stValue| stValue.to_str().ok())
        .and_then(|sValue| httpdate::parse_http_date(sValue).ok())
    else {
        return false;
    };
    let dtIfModifiedSince = DateTime::<Utc>::from(stIfModifiedSince);
    // HTTP dates have one-second precision. Spring performs the same
    // truncation before comparing `lastModified` with If-Modified-Since.
    dtLastModified.timestamp() <= dtIfModifiedSince.timestamp()
}

fn sRenderRss(stFeed: &StRssFeed, sPublicUrl: &str, dtNow: DateTime<Utc>) -> String {
    let sPublicUrl = sPublicUrl.trim_end_matches('/');
    let sTitle = format!("Linux.org.ru: {}", stFeed.sTitle);
    let mut sBody =
        String::from(r#"<?xml version="1.0" encoding="utf-8"?><rss version="2.0"><channel>"#);
    // This is deliberately fixed in section-rss.jsp; only item links use the
    // configured secure URL.
    sBody.push_str("<link>https://www.linux.org.ru/</link><language>ru</language><title>");
    sBody.push_str(&html_escape::encode_text(&sTitle));
    sBody.push_str("</title><description>");
    sBody.push_str(&html_escape::encode_text(&sTitle));
    sBody.push_str("</description><pubDate>");
    sBody.push_str(&sRfc822Moscow(dtNow));
    sBody.push_str("</pubDate>");
    for stTopic in &stFeed.vecItems {
        let sLink = format!("{sPublicUrl}{}", stTopic.sTopicUrl);
        sBody.push_str("<item><author>");
        sBody.push_str(&html_escape::encode_text(&stTopic.sAuthorNick));
        sBody.push_str("</author><link>");
        sBody.push_str(&html_escape::encode_text(&sLink));
        sBody.push_str("</link><guid>");
        sBody.push_str(&html_escape::encode_text(&sLink));
        sBody.push_str("</guid><title>");
        // TopicListDao materializes a Topic through StringUtil.makeTitle, then
        // the JSP XML-escapes that still entity-bearing result. Do not decode
        // the storage value at either step.
        let sLegacyTitle = crate::domain::title::sMakeTitleForLegacyView(&stTopic.sStoredTitle);
        sBody.push_str(&html_escape::encode_text(&sLegacyTitle));
        sBody.push_str("</title><pubDate>");
        sBody.push_str(&sRfc822Moscow(stTopic.dtPublished));
        sBody.push_str("</pubDate>");
        sBody.push_str(&stTopic.sDescriptionElement);
        sBody.push_str("</item>");
    }
    sBody.push_str("</channel></rss>");
    sBody
}

fn sRfc822Moscow(dtValue: DateTime<Utc>) -> String {
    dtValue.with_timezone(&Moscow).to_rfc2822()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::topic::StPreparedRssTopic;
    use chrono::TimeZone;

    #[test]
    fn default_section_matches_java_controller() {
        assert_eq!(iDefaultSection(), 1);
    }

    fn stFeed() -> StRssFeed {
        let dtPublished = Utc.with_ymd_and_hms(2026, 8, 14, 12, 30, 0).unwrap();
        let dtLastModified = Utc.with_ymd_and_hms(2026, 8, 14, 12, 31, 42).unwrap()
            + chrono::Duration::milliseconds(987);
        StRssFeed {
            sTitle: "Новости".to_owned(),
            vecItems: vec![StPreparedRssTopic {
                sStoredTitle: "A &amp; B &lt; C &quot;Q&quot;".to_owned(),
                dtPublished,
                sAuthorNick: "author".to_owned(),
                sTopicUrl: "/news/opensource/42".to_owned(),
                sDescriptionElement: "<description><![CDATA[<p>body</p>]]></description>"
                    .to_owned(),
            }],
            optLastModified: Some(dtLastModified),
        }
    }

    #[test]
    fn item_metadata_body_and_encoded_storage_title_match_the_jsp_contract() {
        let dtNow = Utc.with_ymd_and_hms(2026, 8, 14, 13, 0, 0).unwrap();
        let sXml = sRenderRss(&stFeed(), "http://localhost:8181/", dtNow);
        assert!(sXml.contains("<link>https://www.linux.org.ru/</link>"));
        assert!(sXml.contains("<language>ru</language>"));
        assert!(sXml.contains("<author>author</author>"));
        assert!(sXml.contains(
            "<link>http://localhost:8181/news/opensource/42</link><guid>http://localhost:8181/news/opensource/42</guid>"
        ));
        assert!(sXml.contains("<title>A &amp;amp; B &amp;lt; C &amp;#171;Q&amp;#187;</title>"));
        assert!(sXml.contains("Fri, 14 Aug 2026 15:30:00 +0300"));
        assert!(sXml.contains("<description><![CDATA[<p>body</p>]]></description>"));
    }

    #[test]
    fn conditional_get_uses_second_precision_like_spring() {
        let dtLastModified = stFeed().optLastModified.unwrap();
        let mut stHeaders = HeaderMap::new();
        stHeaders.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Fri, 14 Aug 2026 12:31:42 GMT"),
        );
        assert!(bRequestNotModified(&stHeaders, dtLastModified));

        stHeaders.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Fri, 14 Aug 2026 12:31:41 GMT"),
        );
        assert!(!bRequestNotModified(&stHeaders, dtLastModified));
    }

    #[tokio::test]
    async fn not_modified_response_has_last_modified_and_no_feed_body() {
        let mut stHeaders = HeaderMap::new();
        stHeaders.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Fri, 14 Aug 2026 12:31:42 GMT"),
        );
        let stResponse =
            stRssResponse(stFeed(), &stHeaders, "https://www.linux.org.ru", Utc::now());
        assert_eq!(stResponse.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            stResponse.headers()[header::LAST_MODIFIED],
            "Fri, 14 Aug 2026 12:31:42 GMT"
        );
        assert!(!stResponse.headers().contains_key(header::CONTENT_TYPE));
        let bytes = axum::body::to_bytes(stResponse.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(bytes.is_empty());
    }
}
