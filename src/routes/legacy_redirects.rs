//! Compatibility for redirects performed by the original Tuckey
//! `UrlRewriteFilter` before a request reaches Spring MVC.
//!
//! The Java configuration has `use-query-string="true"`.  Consequently the
//! regular expressions match against the UTF-8 percent-decoded path *plus*
//! the raw `?query`, rather than matching the path and automatically copying
//! the query string.  This distinction is observable: `/rss.jsp` redirects,
//! while `/rss.jsp?x=1` does not match the original anchored rule.

use std::borrow::Cow;

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderValue, StatusCode, Uri, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
const TOPIC_RSS_PREFIX: &str = "/topic-rss.jsp?topic=";
const PROFILE_PREFIX: &str = "/profile/";

const ARR_STATIC_RULES: [(&str, &str); 7] = [
    ("/index.jsp", "/"),
    ("/info.html", "/books"),
    ("/info-mirror.html", "/books"),
    ("/rss.jsp", "/section-rss.jsp"),
    ("/server.jsp", "/about"),
    ("/rss.xml", "/section-rss.jsp"),
    ("/rules.jsp", "/help/rules.md"),
];

/// Return the exact relative target produced by the legacy rules.
fn optRedirectTarget(stUri: &Uri) -> Option<String> {
    // UrlRewriter first decodes getRequestURI(), then (only if that result has
    // no '?') appends a non-empty, trimmed getQueryString().  It deliberately
    // does not decode that appended query string.
    let mut sMatchUrl = cowDecodePathLikeTuckey(stUri.path()).into_owned();
    if !sMatchUrl.contains('?')
        && let Some(sQuery) = stUri.query()
    {
        let sTrimmedQuery = sQuery.trim_matches(|cCharacter| cCharacter <= '\u{20}');
        if !sTrimmedQuery.is_empty() {
            sMatchUrl.push('?');
            sMatchUrl.push_str(sTrimmedQuery);
        }
    }

    if let Some((_, sTarget)) = ARR_STATIC_RULES
        .iter()
        .find(|(sFrom, _)| sMatchUrl.eq_ignore_ascii_case(sFrom))
    {
        return Some((*sTarget).to_owned());
    }

    if sMatchUrl
        .get(..TOPIC_RSS_PREFIX.len())
        .is_some_and(|sPrefix| sPrefix.eq_ignore_ascii_case(TOPIC_RSS_PREFIX))
    {
        let sTopic = &sMatchUrl[TOPIC_RSS_PREFIX.len()..];
        return Some(format!("/view-message.jsp?msgid={sTopic}&output=rss"));
    }

    let sProfileTail = sMatchUrl.get(PROFILE_PREFIX.len()..).filter(|_| {
        sMatchUrl
            .get(..PROFILE_PREFIX.len())
            .is_some_and(|sPrefix| sPrefix.eq_ignore_ascii_case(PROFILE_PREFIX))
    })?;
    let (sNick, sRest) = sProfileTail.split_once('/')?;
    (!sNick.is_empty()).then(|| format!("/{sRest}"))
}

/// Reproduce UrlRewriteFilter's UTF-8 path decoder.
///
/// Its decoder rejects the whole conversion when any `%` escape is malformed,
/// and Java's UTF-8 `String` constructor replaces malformed byte sequences.
/// The query is not passed through this function: Tuckey appends it later.
fn cowDecodePathLikeTuckey(sPath: &str) -> Cow<'_, str> {
    let arrBytes = sPath.as_bytes();
    let mut iIndex = 0;
    while iIndex < arrBytes.len() {
        if arrBytes[iIndex] == b'%' {
            if iIndex + 2 >= arrBytes.len()
                || !arrBytes[iIndex + 1].is_ascii_hexdigit()
                || !arrBytes[iIndex + 2].is_ascii_hexdigit()
            {
                return Cow::Borrowed(sPath);
            }
            iIndex += 3;
        } else {
            iIndex += 1;
        }
    }

    match urlencoding::decode_binary(arrBytes) {
        Cow::Borrowed(_) => Cow::Borrowed(sPath),
        Cow::Owned(vecDecoded) => Cow::Owned(String::from_utf8_lossy(&vecDecoded).into_owned()),
    }
}

/// Apply the subset of `urlrewrite.xml` rules that rewrite legacy public
/// paths.  Tuckey's `type="redirect"` calls Servlet `sendRedirect`, hence
/// these responses use 302 rather than Axum's 303/307 redirect helpers.
pub async fn apply(stRequest: Request, cNext: Next) -> Response {
    let Some(sTarget) = optRedirectTarget(stRequest.uri()) else {
        return cNext.run(stRequest).await;
    };

    let Ok(stLocation) = HeaderValue::from_str(&sTarget) else {
        // A syntactically valid HTTP request URI cannot normally reach this
        // branch.  Treat it as a bad request rather than panic in middleware.
        return StatusCode::BAD_REQUEST.into_response();
    };

    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, stLocation)
        .body(Body::empty())
        .expect("302 response with a validated Location header must build")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::any};

    fn optTarget(sUri: &str) -> Option<String> {
        optRedirectTarget(&sUri.parse().expect("test URI must be valid"))
    }

    #[test]
    fn static_rules_match_only_without_a_query_string() {
        let arrCases = [
            ("/index.jsp", "/"),
            ("/info.html", "/books"),
            ("/info-mirror.html", "/books"),
            ("/rss.jsp", "/section-rss.jsp"),
            ("/rss.xml", "/section-rss.jsp"),
            ("/server.jsp", "/about"),
            ("/rules.jsp", "/help/rules.md"),
        ];

        for (sSource, sTarget) in arrCases {
            assert_eq!(optTarget(sSource).as_deref(), Some(sTarget));
            assert_eq!(optTarget(&format!("{sSource}?x=1")), None);
        }

        // UrlRewriteFilter decodes getRequestURI() before applying <from>.
        assert_eq!(optTarget("/rss%2Ejsp").as_deref(), Some("/section-rss.jsp"));
        assert_eq!(optTarget("/rss%2Gjsp"), None);
    }

    #[test]
    fn matching_is_case_insensitive_like_tuckey_by_default() {
        assert_eq!(optTarget("/INDEX.JSP").as_deref(), Some("/"));
        assert_eq!(
            optTarget("/TOPIC-RSS.JSP?TOPIC=42").as_deref(),
            Some("/view-message.jsp?msgid=42&output=rss")
        );
        assert_eq!(
            optTarget("/PROFILE/Nick/view-message.jsp?msgid=7").as_deref(),
            Some("/view-message.jsp?msgid=7")
        );
    }

    #[test]
    fn topic_rss_preserves_the_raw_capture_and_parameter_order() {
        assert_eq!(
            optTarget("/topic-rss%2Ejsp?topic=123%2F456&mode=full").as_deref(),
            Some("/view-message.jsp?msgid=123%2F456&mode=full&output=rss")
        );
        assert_eq!(
            optTarget("/topic-rss.jsp?topic=").as_deref(),
            Some("/view-message.jsp?msgid=&output=rss")
        );
        assert_eq!(optTarget("/topic-rss.jsp?x=1&topic=2"), None);
        assert_eq!(optTarget("/topic-rss.jsp"), None);
        // A decoded '?' in getRequestURI() prevents the real query string
        // from being appended, exactly like UrlRewriter.getNewChain().
        assert_eq!(
            optTarget("/topic-rss.jsp%3Ftopic%3D42?ignored=1").as_deref(),
            Some("/view-message.jsp?msgid=42&output=rss")
        );
    }

    #[test]
    fn profile_rule_discards_nick_and_preserves_raw_suffix_and_query() {
        assert_eq!(
            optTarget("/profile/maxcom/view-message%2Ejsp?msgid=1993651&a=b").as_deref(),
            Some("/view-message.jsp?msgid=1993651&a=b")
        );
        assert_eq!(
            optTarget("/profile/maxcom/news%2Fopensource").as_deref(),
            Some("/news/opensource")
        );
        assert_eq!(optTarget("/profile/maxcom/").as_deref(), Some("/"));
        assert_eq!(optTarget("/profile/maxcom"), None);
    }

    #[tokio::test]
    async fn router_middleware_returns_302_for_any_http_method() {
        let cApp = Router::new()
            .fallback(any(|| async { StatusCode::NO_CONTENT }))
            .layer(axum::middleware::from_fn(apply));
        let (stAddress, hServer) = stStartServer(cApp).await;
        let cClient = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("test client must build");

        for eMethod in [
            reqwest::Method::GET,
            reqwest::Method::HEAD,
            reqwest::Method::POST,
        ] {
            let stResponse = cClient
                .request(eMethod, format!("http://{stAddress}/rss.xml"))
                .send()
                .await
                .expect("HTTP request to test router must succeed");

            assert_eq!(stResponse.status(), reqwest::StatusCode::FOUND);
            assert_eq!(
                stResponse
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|stValue| stValue.to_str().ok()),
                Some("/section-rss.jsp")
            );
        }

        hServer.abort();
    }

    #[tokio::test]
    async fn router_middleware_passes_non_matching_query_through() {
        let cApp = Router::new()
            .fallback(any(|| async { StatusCode::NO_CONTENT }))
            .layer(axum::middleware::from_fn(apply));
        let (stAddress, hServer) = stStartServer(cApp).await;
        let cClient = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("test client must build");

        let stResponse = cClient
            .get(format!("http://{stAddress}/rss.jsp?section=1"))
            .send()
            .await
            .expect("HTTP request to test router must succeed");

        assert_eq!(stResponse.status(), reqwest::StatusCode::NO_CONTENT);
        assert!(
            stResponse
                .headers()
                .get(reqwest::header::LOCATION)
                .is_none()
        );
        hServer.abort();
    }

    async fn stStartServer(cApp: Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let stListener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener must bind");
        let stAddress = stListener
            .local_addr()
            .expect("test listener must have an address");
        let hServer = tokio::spawn(async move {
            axum::serve(stListener, cApp)
                .await
                .expect("test router must serve");
        });
        (stAddress, hServer)
    }
}
