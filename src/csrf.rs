//! Port of Java's `CommonContextFilter.csrfManipulation` +
//! `CSRFHandlerInterceptor`/`CSRFProtectionService`: a long-lived,
//! non-`HttpOnly` cookie carries a random token; every POST must echo it
//! back in a `csrf` form field, or it's rejected as a forged cross-site
//! request (double-submit cookie pattern). The cookie is deliberately
//! readable by JS (Java's isn't `HttpOnly` either) because `base.html`'s
//! logout/nav form is injected client-side after load and has no other way
//! to read a value the server computed for this request.
//!
//! Both URL-encoded and multipart forms are checked. Java runs the interceptor
//! only after it has selected a handler: a POST to a GET-only mapping is 405,
//! while the DispatcherServlet's not-found fallback is still intercepted and
//! therefore returns 403 without a token. [`apply`] owns only token context and
//! cookie emission; [`validate_auto_post`] is attached to the selected Spring
//! mappings (and the not-found fallback) by the router. Java's `@CSRFNoAuto`
//! write handlers (`/add.jsp`, `/add_comment.jsp`, `/edit_comment`) validate in
//! their handlers after mapping/preview selection.
use crate::{error::AppError, state::AppState};
use axum::{
    body::Body,
    extract::{ConnectInfo, FromRequestParts, Request, State},
    http::{Method, StatusCode, Uri, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use futures_util::stream;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnPostParameterBody {
    UrlEncoded,
    Multipart,
    None,
}

fn enPostParameterBody(optContentType: Option<&str>) -> EnPostParameterBody {
    let Some(sMediaType) = optContentType
        .and_then(|sValue| sValue.split(';').next())
        .map(str::trim)
    else {
        return EnPostParameterBody::None;
    };
    if sMediaType.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
        EnPostParameterBody::UrlEncoded
    } else if sMediaType.eq_ignore_ascii_case("multipart/form-data") {
        EnPostParameterBody::Multipart
    } else {
        EnPostParameterBody::None
    }
}

async fn optMultipartCsrf(sContentType: &str, vecBody: Bytes) -> Result<Option<String>, AppError> {
    let Some(sBoundary) = multer::parse_boundary(sContentType).ok() else {
        return Ok(None);
    };
    let stStream = stream::once(async move { Ok::<Bytes, std::io::Error>(vecBody) });
    let mut stMultipart = multer::Multipart::new(stStream, sBoundary);
    let mut optToken = None;
    loop {
        let Some(stField) = stMultipart.next_field().await.ok().flatten() else {
            break;
        };
        let optName = stField.name().map(ToOwned::to_owned);
        if optName.as_deref() == Some(FIELD_NAME) && optToken.is_none() {
            optToken = stField.text().await.ok();
        }
    }
    Ok(optToken)
}

fn optQueryCsrf(stUri: &Uri) -> Result<Option<String>, AppError> {
    let Some(sQuery) = stUri.query() else {
        return Ok(None);
    };
    let vecQuery = crate::form::parse_pairs_for_named_access(sQuery.as_bytes())?;
    Ok(crate::form::get(&vecQuery, FIELD_NAME).map(ToOwned::to_owned))
}

/// Validate the first CSRF value in an already merged ServletRequest
/// parameter list. Parameter-conditioned handlers call this only after their
/// Spring-equivalent mapping has been selected.
pub(crate) fn bServletCsrfValid(vecParameters: &[(String, String)], sExpected: &str) -> bool {
    crate::form::get(vecParameters, FIELD_NAME)
        .is_some_and(|sSubmitted| !sSubmitted.is_empty() && sSubmitted.trim() == sExpected.trim())
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

/// Validate an automatically protected Spring handler after method/path
/// selection. The request body is restored byte-for-byte for the real handler.
pub async fn validate_auto_post(mut req: Request, next: Next) -> Response {
    if req.method() != Method::POST {
        return next.run(req).await;
    }

    let sExpected = req
        .extensions()
        .get::<CsrfToken>()
        .map(|stToken| stToken.0.as_str())
        .unwrap_or_default()
        .to_owned();
    // A missing content-type is still checked: a bodyless cross-site POST is
    // a browser "simple request" and must not bypass the interceptor.
    let optContentType = req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let enParameterBody = enPostParameterBody(optContentType.as_deref());
    let bMultipart = enParameterBody == EnPostParameterBody::Multipart;
    let optQuerySubmitted = match optQueryCsrf(req.uri()) {
        Ok(optToken) => optToken,
        Err(stError) => return stError.into_response(),
    };
    let (stParts, stBody) = req.into_parts();
    let iLimit = if bMultipart { 30_000_000 } else { 1_048_576 };
    let vecBytes = match axum::body::to_bytes(stBody, iLimit).await {
        Ok(vecBytes) => vecBytes,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid body").into_response(),
    };
    // ServletRequest.getParameter exposes the query string first, then
    // URL-encoded/multipart POST fields. CSRFHandlerInterceptor consumes
    // exactly that first value, so a query token wins on conflicts.
    let optSubmitted = if optQuerySubmitted.is_some() {
        optQuerySubmitted
    } else if bMultipart {
        match optMultipartCsrf(
            optContentType.as_deref().unwrap_or_default(),
            vecBytes.clone(),
        )
        .await
        {
            Ok(optToken) => optToken,
            Err(stError) => return stError.into_response(),
        }
    } else if enParameterBody == EnPostParameterBody::UrlEncoded {
        match crate::form::parse_pairs_for_named_access(&vecBytes) {
            Ok(vecForm) => crate::form::get(&vecForm, FIELD_NAME).map(ToOwned::to_owned),
            Err(_) => None,
        }
    } else {
        // ServletRequest does not expose JSON, text/plain or a body with no
        // content type as request parameters.
        None
    };
    let bValid = optSubmitted
        .as_deref()
        .is_some_and(|sValue| sValue.trim() == sExpected.trim());
    if !bValid {
        return AppError::Forbidden.into_response();
    }

    req = Request::from_parts(stParts, Body::from(vecBytes));
    next.run(req).await
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
    use super::{
        EnPostParameterBody, enPostParameterBody, generate_token, is_valid_token, optMultipartCsrf,
        optQueryCsrf,
    };
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
        .await
        .unwrap();
        assert_eq!(optToken.as_deref(), Some("java-token=="));
    }

    #[tokio::test]
    async fn multipart_csrf_ignores_an_unrequested_invalid_parameter_name() {
        let sBody = format!(
            concat!(
                "--lor-boundary\r\n",
                "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
                "value\r\n",
                "--lor-boundary\r\n",
                "Content-Disposition: form-data; name=\"csrf\"\r\n\r\n",
                "java-token==\r\n",
                "--lor-boundary--\r\n",
            ),
            '\u{0378}'
        );
        assert_eq!(
            optMultipartCsrf(
                "multipart/form-data; boundary=lor-boundary",
                Bytes::from(sBody),
            )
            .await
            .unwrap()
            .as_deref(),
            Some("java-token==")
        );
    }

    #[test]
    fn query_csrf_uses_the_first_servlet_parameter_value() {
        let stUri = "/submit?csrf=query-first&csrf=query-second"
            .parse()
            .unwrap();
        assert_eq!(
            optQueryCsrf(&stUri).unwrap().as_deref(),
            Some("query-first")
        );
        assert_eq!(optQueryCsrf(&"/submit".parse().unwrap()).unwrap(), None);
        assert_eq!(
            optQueryCsrf(&"/submit?%00evil=x&csrf=present".parse().unwrap())
                .unwrap()
                .as_deref(),
            Some("present")
        );
    }

    #[test]
    fn only_servlet_form_media_types_expose_post_body_parameters() {
        assert_eq!(
            enPostParameterBody(Some("application/x-www-form-urlencoded; charset=UTF-8")),
            EnPostParameterBody::UrlEncoded
        );
        assert_eq!(
            enPostParameterBody(Some("Multipart/Form-Data; boundary=x")),
            EnPostParameterBody::Multipart
        );
        for optContentType in [None, Some(""), Some("text/plain"), Some("application/json")] {
            assert_eq!(
                enPostParameterBody(optContentType),
                EnPostParameterBody::None
            );
        }
    }
}
