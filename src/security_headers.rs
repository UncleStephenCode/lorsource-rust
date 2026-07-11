//! Global response security headers, matching Java's `HstsInterceptor`
//! (applied to every response via a Spring `HandlerInterceptor`).
//!
//! - `X-Content-Type-Options: nosniff` and `X-Frame-Options: SAMEORIGIN` are
//!   unconditional.
//! - `Content-Security-Policy` is always set; the directive set mirrors
//!   Java's shape (default-src/base-uri/object-src/frame-ancestors/
//!   form-action/manifest-src/script-src/style-src/img-src/font-src/
//!   connect-src/frame-src) but drops the hCaptcha and WebSocket-specific
//!   origins Java adds, since neither hCaptcha nor a realtime WS hub is
//!   wired up in this port - adding those origins would advertise
//!   infrastructure that doesn't exist.
//! - `Strict-Transport-Security` is only added when the request is over
//!   HTTPS (via `X-Forwarded-Proto`, since TLS terminates at a reverse
//!   proxy in front of this app) AND `ENABLE_HSTS` is explicitly set,
//!   matching `SiteConfig.enableHsts()`'s property-absent-means-false
//!   default - HSTS is a one-way commitment in the browser, so it must be
//!   an explicit opt-in rather than silently on.
use crate::state::AppState;
use axum::{
    extract::{Request, State},
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};

const CSP: &str = concat!(
    "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'self'; ",
    "form-action 'self'; manifest-src 'self'; ",
    "script-src 'self' 'unsafe-inline'; ",
    "style-src 'self' 'unsafe-inline'; ",
    "img-src 'self' data: https://secure.gravatar.com; font-src 'self'; ",
    "connect-src 'self'; frame-src 'self'",
);

pub async fn apply(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let is_secure = crate::security::is_secure_request(req.headers());

    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    headers.insert(HeaderName::from_static("x-content-type-options"), HeaderValue::from_static("nosniff"));
    headers.insert(HeaderName::from_static("x-frame-options"), HeaderValue::from_static("SAMEORIGIN"));
    headers.insert(HeaderName::from_static("content-security-policy"), HeaderValue::from_static(CSP));

    if is_secure && state.config.enable_hsts {
        headers.insert(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }

    response
}
