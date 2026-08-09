//! Host/scheme redirects from the original Tuckey `urlrewrite.xml`.

use std::net::SocketAddr;

use crate::state::AppState;
use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, StatusCode, Uri, header, uri::Authority},
    middleware::Next,
    response::{IntoResponse, Response},
};

const ARR_ALLOWED_HOSTS: [&str; 6] = [
    "www.linux.org.ru",
    "beta.linux.org.ru",
    "test-lor",
    "localhost",
    "10.0.2.2",
    "127.0.0.1",
];

fn optCanonicalTarget(sHost: &str, bSecure: bool, stUri: &Uri) -> Option<String> {
    if sHost.is_empty() {
        return None;
    }
    let Ok(stAuthority) = sHost.parse::<Authority>() else {
        return Some(format!("https://www.linux.org.ru{stUri}"));
    };
    let sHostname = stAuthority.host().to_ascii_lowercase();
    if sHostname == "stoplinux.org.ru" {
        return Some(format!("http://127.0.0.1{stUri}"));
    }

    let bAllowed = ARR_ALLOWED_HOSTS.contains(&sHostname.as_str());
    if !bAllowed {
        return Some(format!("https://www.linux.org.ru{stUri}"));
    }
    if sHostname == "www.linux.org.ru" && !bSecure {
        return Some(format!("https://www.linux.org.ru{stUri}"));
    }
    None
}

pub async fn apply(State(stState): State<AppState>, stRequest: Request, oNext: Next) -> Response {
    let sHost = stRequest
        .headers()
        .get(header::HOST)
        .and_then(|stValue| stValue.to_str().ok())
        .unwrap_or_default();
    let optPeerIp = stRequest
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|stPeer| stPeer.0.ip());
    let bSecure = crate::security::is_secure_request(
        stRequest.headers(),
        optPeerIp,
        &stState.config.trusted_proxy_cidrs,
    );
    let Some(sTarget) = optCanonicalTarget(sHost, bSecure, stRequest.uri()) else {
        return oNext.run(stRequest).await;
    };
    let Ok(stLocation) = HeaderValue::from_str(&sTarget) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, stLocation)
        .body(Body::empty())
        .expect("canonical redirect with a validated Location must build")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn optTarget(sHost: &str, bSecure: bool, sUri: &str) -> Option<String> {
        optCanonicalTarget(sHost, bSecure, &sUri.parse().expect("valid test URI"))
    }

    #[test]
    fn enforces_exact_host_and_scheme_rules() {
        assert_eq!(
            optTarget("unknown.example:8181", false, "/forum/?x=1"),
            Some("https://www.linux.org.ru/forum/?x=1".to_owned())
        );
        assert_eq!(
            optTarget("www.linux.org.ru", false, "/news/"),
            Some("https://www.linux.org.ru/news/".to_owned())
        );
        assert_eq!(optTarget("www.linux.org.ru", true, "/news/"), None);
        assert_eq!(optTarget("beta.linux.org.ru", false, "/"), None);
        assert_eq!(optTarget("localhost:8181", false, "/readyz"), None);
        assert_eq!(optTarget("127.0.0.1:8181", false, "/readyz"), None);
        assert_eq!(
            optTarget("www.linux.org.ru.example", true, "/"),
            Some("https://www.linux.org.ru/".to_owned())
        );
        assert_eq!(
            optTarget("stoplinux.org.ru.example", true, "/"),
            Some("https://www.linux.org.ru/".to_owned())
        );
    }

    #[test]
    fn stoplinux_legacy_rule_precedes_the_canonical_host_rule() {
        assert_eq!(
            optTarget("stoplinux.org.ru", true, "/path?q=1"),
            Some("http://127.0.0.1/path?q=1".to_owned())
        );
    }
}
