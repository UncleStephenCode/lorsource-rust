//! Port of Java's `CommonContextFilter.csrfManipulation` +
//! `CSRFHandlerInterceptor`/`CSRFProtectionService`: a long-lived,
//! non-`HttpOnly` cookie carries a random token; every POST must echo it
//! back in a `csrf` form field, or it's rejected as a forged cross-site
//! request (double-submit cookie pattern). The cookie is deliberately
//! readable by JS (Java's isn't `HttpOnly` either) because `base.html`'s
//! logout/nav form is injected client-side after load and has no other way
//! to read a value the server computed for this request.
//!
//! Only `application/x-www-form-urlencoded` bodies are checked - multipart
//! uploads (photo upload) rely on the `SameSite=Lax` session cookie
//! instead, which already stops the cross-site POST from carrying the
//! session in the first place on any modern browser.
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{FromRequestParts, Request, State},
    http::{request::Parts, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use std::collections::HashMap;
use time::Duration;

/// Matches Java's `CSRFProtectionService.CSRF_COOKIE` exactly - the
/// frontend JS shipped in `static/js/*.js` (`getCsrf()`) already reads this
/// literal cookie name, unmodified from the original.
pub const COOKIE_NAME: &str = "CSRF_TOKEN";
pub const FIELD_NAME: &str = "csrf";

fn generate_token() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The token is echoed back into hidden form fields via raw `format!`
/// strings in several handlers (not just Askama templates, which
/// auto-escape) - reject anything outside the base64url alphabet
/// `generate_token` produces, so a cookie value tampered with client-side
/// (e.g. HTML metacharacters) can never reach an unescaped `value="..."`
/// attribute.
fn is_valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= 64 && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The token a POST form must echo back in a hidden `csrf` field - pulled
/// out of request extensions inserted by [`apply`], not re-derived, so it's
/// always exactly the value that was in (or just set on) the request's
/// cookie jar.
#[derive(Debug, Clone)]
pub struct CsrfToken(pub String);

#[axum::async_trait]
impl<S> FromRequestParts<S> for CsrfToken
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<CsrfToken>().cloned().unwrap_or_else(|| CsrfToken(String::new())))
    }
}

pub async fn apply(State(_state): State<AppState>, mut req: Request, next: Next) -> Response {
    let jar = CookieJar::from_headers(req.headers());
    let existing = jar.get(COOKIE_NAME).map(|c| c.value().to_string()).filter(|v| is_valid_token(v));
    let token = existing.clone().unwrap_or_else(generate_token);
    let is_secure = crate::security::is_secure_request(req.headers());

    req.extensions_mut().insert(CsrfToken(token.clone()));

    if req.method() == Method::POST {
        // Only multipart bodies are punted on (see module docs) - anything
        // else, *including a missing/absent content-type* (a bodyless
        // `fetch(..., {method:"POST"})` is a valid same-origin-looking
        // simple request an attacker can issue cross-site without ever
        // setting a content-type), is treated as "must present a matching
        // `csrf` field", and an empty body simply fails to match.
        let is_multipart = req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.starts_with("multipart/form-data"))
            .unwrap_or(false);

        if !is_multipart {
            let (parts, body) = req.into_parts();
            let bytes = match axum::body::to_bytes(body, usize::MAX).await {
                Ok(b) => b,
                Err(_) => return (StatusCode::BAD_REQUEST, "invalid body").into_response(),
            };
            let submitted: HashMap<String, String> = serde_urlencoded::from_bytes(&bytes).unwrap_or_default();
            let ok = submitted.get(FIELD_NAME).map(|v| *v == token).unwrap_or(false);
            if !ok {
                return (StatusCode::FORBIDDEN, "Неправильный код защиты CSRF. Возможно сессия устарела").into_response();
            }
            req = Request::from_parts(parts, Body::from(bytes));
        }
    }

    let mut response = next.run(req).await;

    if existing.is_none() {
        let cookie = Cookie::build((COOKIE_NAME, token))
            .path("/")
            .max_age(Duration::days(365 * 2))
            .secure(is_secure)
            .same_site(axum_extra::extract::cookie::SameSite::Lax)
            .build();
        if let Ok(header_value) = cookie.to_string().parse() {
            response.headers_mut().append(axum::http::header::SET_COOKIE, header_value);
        }
    }

    response
}
