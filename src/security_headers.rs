//! Global response security headers, matching Java's `HstsInterceptor`
//! (applied to every response via a Spring `HandlerInterceptor`).
//!
//! - `X-Content-Type-Options: nosniff` and `X-Frame-Options: SAMEORIGIN` are
//!   unconditional.
//! - `Content-Security-Policy` is always set; the directive set mirrors
//!   Java's shape (default-src/base-uri/object-src/frame-ancestors/
//!   form-action/manifest-src/script-src/style-src/img-src/font-src/
//!   connect-src/frame-src). The configured `WS_URL` origin is included in
//!   `connect-src`, exactly as Java does, so the original browser realtime
//!   client is not blocked when the socket uses a separate origin.
//! - `Strict-Transport-Security` is only added when the request is over
//!   HTTPS (via `X-Forwarded-Proto`, since TLS terminates at a reverse
//!   proxy in front of this app) AND `ENABLE_HSTS` is explicitly set,
//!   matching `SiteConfig.enableHsts()`'s property-absent-means-false
//!   default - HSTS is a one-way commitment in the browser, so it must be
//!   an explicit opt-in rather than silently on.
use crate::state::AppState;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderName, HeaderValue, Uri},
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;

fn optOrigin(sUrl: &str) -> Option<String> {
    let stUri = sUrl.parse::<Uri>().ok()?;
    let sScheme = stUri.scheme_str()?;
    let stAuthority = stUri.authority()?;
    Some(format!("{sScheme}://{stAuthority}"))
}

fn sContentSecurityPolicy(sPublicUrl: &str, sWsUrl: &str) -> String {
    let sSecureOrigin = optOrigin(sPublicUrl)
        .map(|sOrigin| format!(" {sOrigin}"))
        .unwrap_or_default();
    let sWebSocketOrigin = optOrigin(sWsUrl)
        .map(|sOrigin| format!(" {sOrigin}"))
        .unwrap_or_default();
    format!(
        "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'self'; \
         form-action 'self'{sSecureOrigin}; manifest-src 'self'; \
         script-src 'self' 'unsafe-inline' https://hcaptcha.com https://*.hcaptcha.com; \
         style-src 'self' 'unsafe-inline' https://hcaptcha.com https://*.hcaptcha.com; \
         img-src 'self' data: https://images.ping-admin.ru https://cdn.jsdelivr.net \
         https://secure.gravatar.com; font-src 'self'; \
         connect-src 'self' https://hcaptcha.com https://*.hcaptcha.com{sWebSocketOrigin}; \
         frame-src 'self' https://hcaptcha.com https://*.hcaptcha.com"
    )
}

pub async fn apply(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let optPeerIp = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|stInfo| stInfo.0.ip());
    let is_secure = crate::security::is_secure_request(
        req.headers(),
        optPeerIp,
        &state.config.trusted_proxy_cidrs,
    );

    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("SAMEORIGIN"),
    );
    // CommonContextFilter marks every response private. Preserve an explicit
    // handler policy (for example no-store), otherwise add the Java default.
    if !headers.contains_key(axum::http::header::CACHE_CONTROL) {
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static("private"),
        );
    }
    let sCsp = sContentSecurityPolicy(&state.config.public_url, &state.config.ws_url);
    if let Ok(stCsp) = HeaderValue::from_str(&sCsp) {
        headers.insert(HeaderName::from_static("content-security-policy"), stCsp);
    }

    if is_secure && state.config.enable_hsts {
        headers.insert(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_origin_matches_java_csp_generation() {
        assert_eq!(
            optOrigin("wss://realtime.example:8443/socket-prefix/"),
            Some("wss://realtime.example:8443".to_string())
        );
        let sCsp = sContentSecurityPolicy(
            "https://www.example.org/",
            "wss://realtime.example:8443/socket-prefix/",
        );
        assert!(sCsp.contains("wss://realtime.example:8443;"));
        assert!(sCsp.contains("form-action 'self' https://www.example.org;"));
        assert!(sCsp.contains("https://*.hcaptcha.com"));
        assert!(!sCsp.contains("socket-prefix"));
    }

    #[test]
    fn invalid_websocket_url_does_not_weaken_connect_src() {
        let sCsp = sContentSecurityPolicy("not a URL", "not a URL");
        assert!(sCsp.contains("connect-src 'self' https://hcaptcha.com"));
    }
}
