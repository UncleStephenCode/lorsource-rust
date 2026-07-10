use crate::{error::Result, state::AppState};
use axum::{extract::{Query, State}, http::{header, HeaderMap}};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct RssQuery { pub section: Option<String> }

pub async fn main_rss(State(state): State<AppState>) -> Result<(HeaderMap, String)> {
    render_rss(&state, None).await
}

pub async fn section_rss(State(state): State<AppState>, Query(q): Query<RssQuery>) -> Result<(HeaderMap, String)> {
    render_rss(&state, q.section).await
}

async fn render_rss(state: &AppState, optSection: Option<String>) -> Result<(HeaderMap, String)> {
    let section = optSection.as_deref();
    let topics = crate::routes::topics::list_topics(state, section, None, 0, 30).await?;
    let mut body = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel>"#);
    body.push_str("<title>LOR Rust</title><link>");
    body.push_str(&state.config.public_url);
    body.push_str("</link><description>lorsource-rust feed</description>");
    for t in topics {
        let link = format!("{}{}", state.config.public_url, t.topic_url());
        body.push_str("<item>");
        body.push_str(&format!("<title>{}</title>", html_escape::encode_text(&t.title)));
        body.push_str(&format!("<link>{}</link><guid>{}</guid>", link, link));
        body.push_str(&format!("<pubDate>{}</pubDate>", t.postdate.to_rfc2822()));
        body.push_str("</item>");
    }
    body.push_str("</channel></rss>");
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8".parse().unwrap());
    Ok((headers, body))
}
