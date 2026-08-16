//! Spring Security `StrictHttpFirewall` compatibility.
//!
//! The original configuration constructs an otherwise-default firewall, whose
//! allowed methods are DELETE, GET, HEAD, OPTIONS, PATCH, POST and PUT. Its
//! default encoded/decoded URL blocklists and normalization checks remain in
//! force; only the header-*value* predicate is relaxed. The deployed Java
//! stack also has Jetty 12 in front of the filter chain: it returns an empty
//! 403 for TRACE before URL rewriting, while CONNECT and ambiguous encoded
//! paths are parser-level 400 responses. Rust preserves those portable
//! statuses without copying Jetty-branded error HTML.

use axum::{
    extract::Request,
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

fn bAllowed(stMethod: &Method) -> bool {
    stMethod == Method::DELETE
        || stMethod == Method::GET
        || stMethod == Method::HEAD
        || stMethod == Method::OPTIONS
        || stMethod == Method::PATCH
        || stMethod == Method::POST
        || stMethod == Method::PUT
}

fn bNormalized(sPath: &str) -> bool {
    !sPath
        .split('/')
        .any(|sSegment| sSegment == "." || sSegment == "..")
}

fn bRejectedPath(sEncodedPath: &str) -> bool {
    // `rejectNonPrintableAsciiCharactersInFieldName(requestURI, ...)` is
    // unconditional. Raw UTF-8 is accepted by http::Uri but rejected by
    // StrictHttpFirewall; ordinary Unicode paths arrive percent-encoded.
    if !sEncodedPath
        .as_bytes()
        .iter()
        .all(|iByte| (0x20..=0x7e).contains(iByte))
    {
        return true;
    }
    let sLower = sEncodedPath.to_ascii_lowercase();
    // StrictHttpFirewall 6.5.11's encoded blocklist. The combined encoded
    // double-slash variants are already covered by `%2f` itself.
    if sEncodedPath.contains(';')
        || sEncodedPath.contains("//")
        || sEncodedPath.contains('\\')
        || ['\0', '\n', '\r']
            .into_iter()
            .any(|cValue| sEncodedPath.contains(cValue))
        || ["%3b", "%2f", "%5c", "%00", "%0a", "%0d", "%25", "%2e"]
            .into_iter()
            .any(|sForbidden| sLower.contains(sForbidden))
        || !bNormalized(sEncodedPath)
    {
        return true;
    }

    // Servlet containers expose a decoded servletPath/pathInfo to the second
    // blocklist. Decode once (like Tomcat); malformed escapes are rejected by
    // the container before MVC and therefore fail closed here too.
    let Ok(sDecoded) = urlencoding::decode(sEncodedPath) else {
        return true;
    };
    sDecoded.contains(';')
        || sDecoded.contains("//")
        || sDecoded.contains('\\')
        || sDecoded.contains('%')
        || ['\0', '\n', '\r', '\u{2028}', '\u{2029}']
            .into_iter()
            .any(|cValue| sDecoded.contains(cValue))
        || !bNormalized(&sDecoded)
}

pub async fn apply(stRequest: Request, cNext: Next) -> Response {
    // Jetty rejects ambiguous request targets while parsing them, before its
    // method handling. Keep that precedence for the observable status.
    if bRejectedPath(stRequest.uri().path()) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Jetty's HttpChannel rejects TRACE with 403 before the web.xml filter
    // chain. This is why `/rss.xml` is not rewritten for TRACE in the live
    // pinned runtime even though UrlRewriteFilter is declared first.
    if stRequest.method() == Method::TRACE {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !bAllowed(stRequest.method()) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    cNext.run(stRequest).await
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode, header},
        middleware,
        response::IntoResponse,
        routing::any,
    };
    use tower::ServiceExt;

    use super::{apply, bRejectedPath};

    #[test]
    fn matches_the_pinned_default_encoded_and_decoded_url_blocklists() {
        for sPath in [
            "/a;b",
            "/a%3bb",
            "/a%3Bb",
            "/a%2fb",
            "/a%2Fb",
            "/a//b",
            "/a%2f%2fb",
            "/a\\b",
            "/a%5cb",
            "/a%00b",
            "/a%0ab",
            "/a%0db",
            "/a%25b",
            "/a%2eb",
            "/a%2Eb",
            "/a%E2%80%A8b",
            "/a%E2%80%A9b",
            "/a/../b",
            "/a/./b",
            "/a/%2e%2e/b",
            "/раст",
            "/é",
        ] {
            assert!(bRejectedPath(sPath), "{sPath}");
        }
        for sPath in [
            "/ok",
            "/tag/c%2B%2B",
            "/tag/%D1%80%D0%B0%D1%81%D1%82",
            "/a%20b",
            "/a.b",
        ] {
            assert!(!bRejectedPath(sPath), "{sPath}");
        }
    }

    fn cApp(iCalls: Arc<AtomicUsize>) -> Router {
        Router::new()
            .fallback(any(move || {
                let iCalls = iCalls.clone();
                async move {
                    iCalls.fetch_add(1, Ordering::SeqCst);
                    StatusCode::NO_CONTENT.into_response()
                }
            }))
            .layer(middleware::from_fn(apply))
    }

    #[tokio::test]
    async fn allows_exact_strict_http_firewall_default_methods() {
        let iCalls = Arc::new(AtomicUsize::new(0));
        for stMethod in [
            Method::DELETE,
            Method::GET,
            Method::HEAD,
            Method::OPTIONS,
            Method::PATCH,
            Method::POST,
            Method::PUT,
        ] {
            let stResponse = cApp(iCalls.clone())
                .oneshot(
                    Request::builder()
                        .method(stMethod)
                        .uri("/probe")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(stResponse.status(), StatusCode::NO_CONTENT);
        }
        assert_eq!(iCalls.load(Ordering::SeqCst), 7);
    }

    #[tokio::test]
    async fn trace_and_connect_match_the_pinned_java_portable_contract() {
        let iCalls = Arc::new(AtomicUsize::new(0));
        for (stMethod, stExpected) in [
            (Method::TRACE, StatusCode::FORBIDDEN),
            (Method::CONNECT, StatusCode::BAD_REQUEST),
        ] {
            let stResponse = cApp(iCalls.clone())
                .oneshot(
                    Request::builder()
                        .method(stMethod)
                        .uri("/about")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(stResponse.status(), stExpected);
            assert!(stResponse.headers().get(header::CONTENT_TYPE).is_none());
            assert!(
                to_bytes(stResponse.into_body(), 1)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(iCalls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn rejected_raw_uris_keep_the_java_empty_400_shape_before_routing() {
        let iCalls = Arc::new(AtomicUsize::new(0));
        for sPath in [
            "/a;b",
            "/a%2fb",
            "/a//b",
            "/a%5cb",
            "/a%00b",
            "/a%25b",
            "/a%2eb",
            "/a%E2%80%A8b",
            "/a/../b",
            "/раст",
        ] {
            let stResponse = cApp(iCalls.clone())
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(sPath)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(stResponse.status(), StatusCode::BAD_REQUEST, "{sPath}");
            assert!(stResponse.headers().get(header::CONTENT_TYPE).is_none());
            assert!(
                to_bytes(stResponse.into_body(), 1)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(iCalls.load(Ordering::SeqCst), 0);

        let stAllowedQuery = cApp(iCalls.clone())
            .oneshot(
                Request::builder()
                    .uri("/ok?value=a;b")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stAllowedQuery.status(), StatusCode::NO_CONTENT);
        assert_eq!(iCalls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn container_trace_rejection_precedes_url_rewrite() {
        let cApp = Router::new()
            .fallback(any(|| async { StatusCode::NO_CONTENT }))
            .layer(middleware::from_fn(crate::routes::legacy_redirects::apply))
            .layer(middleware::from_fn(apply));

        let stRedirect = cApp
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::TRACE)
                    .uri("/rss.xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stRedirect.status(), StatusCode::FORBIDDEN);
        assert!(stRedirect.headers().get(header::LOCATION).is_none());

        let stRejected = cApp
            .oneshot(
                Request::builder()
                    .method(Method::TRACE)
                    .uri("/about")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stRejected.status(), StatusCode::FORBIDDEN);
    }
}
