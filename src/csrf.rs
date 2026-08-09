//! Port of Java's `CommonContextFilter.csrfManipulation` +
//! `CSRFHandlerInterceptor`/`CSRFProtectionService`: a long-lived,
//! non-`HttpOnly` cookie carries a random token; every POST must echo it
//! back in a `csrf` form field, or it's rejected as a forged cross-site
//! request (double-submit cookie pattern). The cookie is deliberately
//! readable by JS (Java's isn't `HttpOnly` either) because `base.html`'s
//! logout/nav form is injected client-side after load and has no other way
//! to read a value the server computed for this request.
//!
//! Both URL-encoded and multipart forms are checked. Java's `@CSRFNoAuto`
//! write handlers (`/add.jsp`, `/add_comment.jsp`, `/edit_comment`) validate
//! the token only in their non-preview branch, so those paths are checked in
//! the route after form parsing rather than unconditionally here.
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{ConnectInfo, FromRequestParts, Request, State},
    http::{Method, StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use futures_util::stream;
use std::collections::HashMap;
use std::net::SocketAddr;
use time::Duration;

/// Matches Java's `CSRFProtectionService.CSRF_COOKIE` exactly - the
/// frontend JS shipped in `static/js/*.js` (`getCsrf()`) already reads this
/// literal cookie name, unmodified from the original.
pub const COOKIE_NAME: &str = "CSRF_TOKEN";
pub const FIELD_NAME: &str = "csrf";

fn generate_token() -> String {
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    // java.util.Base64.getEncoder(): standard alphabet, with padding.
    STANDARD.encode(bytes)
}

/// The token is echoed back into hidden form fields via raw `format!`
/// strings in several handlers (not just Askama templates, which
/// auto-escape). Accept only the exact 16-byte standard-Base64 shape Java
/// generates, so a tampered cookie containing HTML metacharacters can never
/// reach an unescaped `value="..."` attribute.
fn is_valid_token(token: &str) -> bool {
    let sToken = token.trim();
    STANDARD
        .decode(sToken)
        .is_ok_and(|vecToken| vecToken.len() == 16)
        // Java accepts any non-empty value. Keep its alphanumeric test/client
        // tokens (notably `csrf`) without allowing HTML metacharacters into
        // the port's remaining hand-rendered hidden inputs.
        || (!sToken.is_empty()
            && sToken.len() <= 64
            && sToken.chars().all(|cCharacter| cCharacter.is_ascii_alphanumeric()))
}

async fn optMultipartCsrf(sContentType: &str, vecBody: Bytes) -> Option<String> {
    let sBoundary = multer::parse_boundary(sContentType).ok()?;
    let stStream = stream::once(async move { Ok::<Bytes, std::io::Error>(vecBody) });
    let mut stMultipart = multer::Multipart::new(stStream, sBoundary);
    while let Some(stField) = stMultipart.next_field().await.ok()? {
        if stField.name() == Some(FIELD_NAME) {
            return stField.text().await.ok();
        }
    }
    None
}

/// The token a POST form must echo back in a hidden `csrf` field - pulled
/// out of request extensions inserted by [`apply`], not re-derived, so it's
/// always exactly the value that was in (or just set on) the request's
/// cookie jar.
#[derive(Debug, Clone)]
pub struct CsrfToken(pub String);

impl<S> FromRequestParts<S> for CsrfToken
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<CsrfToken>()
            .cloned()
            .unwrap_or_else(|| CsrfToken(String::new())))
    }
}

pub async fn apply(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let bSecurityIgnored = crate::security::bSpringSecurityIgnoredPath(req.uri().path());
    let jar = CookieJar::from_headers(req.headers());
    let existing = jar
        .get(COOKIE_NAME)
        .map(|c| c.value().trim().to_string())
        .filter(|v| is_valid_token(v));
    let token = existing.clone().unwrap_or_else(generate_token);
    let optPeerIp = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|stInfo| stInfo.0.ip());
    let is_secure = crate::security::is_secure_request(
        req.headers(),
        optPeerIp,
        &state.config.trusted_proxy_cidrs,
    );

    req.extensions_mut().insert(CsrfToken(token.clone()));

    let bManualCsrf = matches!(
        req.uri().path(),
        "/add.jsp" | "/add_comment.jsp" | "/edit_comment"
    );
    if req.method() == Method::POST && !bManualCsrf {
        // A missing content-type is still checked: a bodyless cross-site POST
        // is a browser "simple request" and must not bypass the interceptor.
        let optContentType = req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bMultipart = optContentType
            .as_deref()
            .is_some_and(|sValue| sValue.starts_with("multipart/form-data"));
        let (stParts, stBody) = req.into_parts();
        let iLimit = if bMultipart { 30_000_000 } else { 1_048_576 };
        let vecBytes = match axum::body::to_bytes(stBody, iLimit).await {
            Ok(vecBytes) => vecBytes,
            Err(_) => return (StatusCode::BAD_REQUEST, "invalid body").into_response(),
        };
        let optSubmitted = if bMultipart {
            optMultipartCsrf(
                optContentType.as_deref().unwrap_or_default(),
                vecBytes.clone(),
            )
            .await
        } else {
            let mapSubmitted: HashMap<String, String> =
                serde_urlencoded::from_bytes(&vecBytes).unwrap_or_default();
            mapSubmitted.get(FIELD_NAME).cloned()
        };
        let bValid = optSubmitted
            .as_deref()
            .is_some_and(|sValue| sValue.trim() == token.trim());
        if !bValid {
            return (
                StatusCode::FORBIDDEN,
                "Неправильный код защиты CSRF. Возможно сессия устарела",
            )
                .into_response();
        }
        req = Request::from_parts(stParts, Body::from(vecBytes));
    }

    let mut response = next.run(req).await;

    if existing.is_none()
        && !(bSecurityIgnored
            && response.status().as_u16() >= 200
            && response.status().as_u16() < 400)
    {
        let cookie = Cookie::build((COOKIE_NAME, token))
            .path("/")
            .max_age(Duration::seconds(60 * 60 * 24 * 31 * 24))
            .secure(is_secure)
            .build();
        if let Ok(header_value) = cookie.to_string().parse() {
            response
                .headers_mut()
                .append(axum::http::header::SET_COOKIE, header_value);
        }
    }

    response
}

#[cfg(test)]
mod tests {
    use super::{generate_token, is_valid_token, optMultipartCsrf};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use bytes::Bytes;

    #[test]
    fn token_shape_matches_java_standard_base64() {
        let sToken = generate_token();
        let vecDecoded = STANDARD.decode(&sToken).expect("standard Base64 token");
        assert_eq!(vecDecoded.len(), 16);
        assert!(sToken.ends_with("=="));
        assert!(is_valid_token(&sToken));
        assert!(!is_valid_token("base64url_token-without-padding"));
    }

    #[tokio::test]
    async fn reads_csrf_from_multipart_like_servlet_request_parameter() {
        let sBody = concat!(
            "--lor-boundary\r\n",
            "Content-Disposition: form-data; name=\"title\"\r\n\r\n",
            "Topic\r\n",
            "--lor-boundary\r\n",
            "Content-Disposition: form-data; name=\"csrf\"\r\n\r\n",
            "java-token==\r\n",
            "--lor-boundary--\r\n",
        );
        let optToken = optMultipartCsrf(
            "multipart/form-data; boundary=lor-boundary",
            Bytes::from_static(sBody.as_bytes()),
        )
        .await;
        assert_eq!(optToken.as_deref(), Some("java-token=="));
    }
}
