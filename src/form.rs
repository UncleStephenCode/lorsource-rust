use crate::error::{AppError, Result};
use axum::{
    body::Body,
    body::to_bytes,
    extract::Request,
    http::{Method, Uri, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

const I_SERVLET_PARAMETER_BODY_LIMIT: usize = 1_048_576;

static RE_ASSIGNED_PARAMETER_NAME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\p{Assigned}*$").expect("StrictHttpFirewall assigned-character regex")
});

pub(crate) fn bAllowedServletParameterName(sName: &str) -> bool {
    RE_ASSIGNED_PARAMETER_NAME.is_match(sName) && !sName.chars().any(char::is_control)
}

/// Decode the container's parameter representation without enumerating it.
///
/// `StrictFirewalledRequest` validates the *requested* name in
/// `getParameter(name)`/`getParameterValues(name)`, but validates every
/// client-supplied name only when `getParameterMap()` or
/// `getParameterNames()` is called. Named Spring arguments therefore ignore
/// an unrelated invalid key. Keep that lazy distinction instead of rejecting
/// the whole query while decoding it.
pub(crate) fn parse_pairs_for_named_access(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    serde_urlencoded::from_bytes(bytes)
        .map_err(|_| AppError::BadRequest("некорректные данные формы".into()))
}

/// `axum::Form<T>` deserializes via `serde_urlencoded`, which cannot turn
/// repeated keys (`vote=1&vote=2`, the standard HTML encoding for a
/// multi-select/checkbox group) into a `Vec<T>` field - it errors with
/// "invalid type: string ..., expected a sequence". Parsing into
/// `Vec<(String, String)>` instead preserves every occurrence in order, so
/// callers can pick out repeated fields by hand.
pub fn parse_pairs(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let vecPairs = parse_pairs_for_named_access(bytes)?;
    // This public parser is used when port code enumerates the complete form,
    // which corresponds to Servlet getParameterMap()/getParameterNames().
    // Those accessors validate every supplied name. Parameter values remain
    // unrestricted, matching the original firewall customization.
    if vecPairs
        .iter()
        .any(|(sName, _)| !bAllowedServletParameterName(sName))
    {
        return Err(AppError::RequestRejected);
    }
    Ok(vecPairs)
}

pub fn get<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub fn get_all<'a>(pairs: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    pairs
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .collect()
}

/// Reproduces the parameter view exposed by an unfiltered Tomcat servlet
/// request: query values come first and an URL-encoded request body is added
/// for POST only.  PUT/PATCH/DELETE bodies are not form-bound without
/// Spring's FormContentFilter, while duplicate values retain query-first
/// precedence for `@RequestParam` and bean binding.
pub async fn servlet_request_parameters(stRequest: Request) -> Result<Vec<(String, String)>> {
    let (stParts, stBody) = stRequest.into_parts();
    let mut vecParameters = match stParts.uri.query() {
        Some(sQuery) => parse_pairs_for_named_access(sQuery.as_bytes())?,
        None => Vec::new(),
    };
    let bUrlEncodedPost = stParts.method == Method::POST
        && stParts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|stValue| stValue.to_str().ok())
            .and_then(|sValue| sValue.split(';').next())
            .is_some_and(|sMediaType| {
                sMediaType
                    .trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            });
    if bUrlEncodedPost {
        let vecBody = to_bytes(stBody, I_SERVLET_PARAMETER_BODY_LIMIT)
            .await
            .map_err(|_| AppError::BadRequest("некорректные данные формы".to_owned()))?;
        if !vecBody.is_empty() {
            vecParameters.extend(parse_pairs_for_named_access(&vecBody)?);
        }
    }
    Ok(vecParameters)
}

/// Adapts a legacy read-only `ANY` route that still uses Axum's `Query<T>`
/// extractor. Spring exposes query plus URL-encoded POST form parameters to
/// `@RequestParam`; this middleware materializes their first-value view in the
/// URI while leaving the original body available to downstream extractors.
/// It must only be attached to read-only legacy routes.
pub async fn merge_servlet_post_form_into_query(stRequest: Request, cNext: Next) -> Response {
    let bMerge = stRequest.method() == Method::POST
        && stRequest
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|stValue| stValue.to_str().ok())
            .and_then(|sValue| sValue.split(';').next())
            .is_some_and(|sMediaType| {
                sMediaType
                    .trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            });
    if !bMerge {
        return cNext.run(stRequest).await;
    }

    let (mut stParts, stBody) = stRequest.into_parts();
    let vecBody = match to_bytes(stBody, I_SERVLET_PARAMETER_BODY_LIMIT).await {
        Ok(vecBody) => vecBody,
        Err(_) => {
            return AppError::BadRequest("некорректные данные формы".to_owned()).into_response();
        }
    };
    let mut vecMerged = Vec::new();
    let mut setNames = HashSet::new();
    let vecQuery = match stParts.uri.query() {
        Some(sQuery) => match parse_pairs_for_named_access(sQuery.as_bytes()) {
            Ok(vecQuery) => vecQuery,
            Err(stError) => return stError.into_response(),
        },
        None => Vec::new(),
    };
    for (sName, sValue) in vecQuery {
        if setNames.insert(sName.clone()) {
            vecMerged.push((sName, sValue));
        }
    }
    if !vecBody.is_empty() {
        let vecForm = match parse_pairs_for_named_access(&vecBody) {
            Ok(vecForm) => vecForm,
            Err(stError) => return stError.into_response(),
        };
        for (sName, sValue) in vecForm {
            if setNames.insert(sName.clone()) {
                vecMerged.push((sName, sValue));
            }
        }
    }

    let sQuery = match serde_urlencoded::to_string(&vecMerged) {
        Ok(sQuery) => sQuery,
        Err(_) => {
            return AppError::BadRequest("некорректные данные формы".to_owned()).into_response();
        }
    };
    let sPathAndQuery = if sQuery.is_empty() {
        stParts.uri.path().to_owned()
    } else {
        format!("{}?{sQuery}", stParts.uri.path())
    };
    let mut stUriParts = stParts.uri.into_parts();
    stUriParts.path_and_query = match sPathAndQuery.parse() {
        Ok(stPathAndQuery) => Some(stPathAndQuery),
        Err(_) => {
            return AppError::BadRequest("некорректные данные формы".to_owned()).into_response();
        }
    };
    stParts.uri = match Uri::from_parts(stUriParts) {
        Ok(stUri) => stUri,
        Err(_) => {
            return AppError::BadRequest("некорректные данные формы".to_owned()).into_response();
        }
    };
    cNext
        .run(Request::from_parts(stParts, Body::from(vecBody)))
        .await
}

#[cfg(test)]
mod servlet_parameter_tests {
    use axum::{
        Router,
        body::Body,
        extract::{Query, Request},
        http::{Method, StatusCode, header},
        middleware,
        routing::any,
    };
    use serde::Deserialize;
    use tower::ServiceExt;

    use super::{
        get, get_all, merge_servlet_post_form_into_query, parse_pairs,
        parse_pairs_for_named_access, servlet_request_parameters,
    };
    use crate::error::AppError;

    fn stRequest(stMethod: Method, sUri: &str, sBody: &str) -> Request {
        Request::builder()
            .method(stMethod)
            .uri(sUri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(sBody.to_owned()))
            .unwrap()
    }

    #[tokio::test]
    async fn servlet_parameters_are_query_first_and_post_only() {
        let vecPost = servlet_request_parameters(stRequest(
            Method::POST,
            "/endpoint?value=query&empty=",
            "value=form&body=present",
        ))
        .await
        .unwrap();
        assert_eq!(get(&vecPost, "value"), Some("query"));
        assert_eq!(get_all(&vecPost, "value"), ["query", "form"]);
        assert_eq!(get(&vecPost, "body"), Some("present"));
        assert_eq!(get(&vecPost, "empty"), Some(""));

        for stMethod in [Method::GET, Method::PUT, Method::PATCH, Method::DELETE] {
            let vecParameters = servlet_request_parameters(stRequest(
                stMethod,
                "/endpoint?value=query",
                "value=form&body=ignored",
            ))
            .await
            .unwrap();
            assert_eq!(get(&vecParameters, "value"), Some("query"));
            assert_eq!(get(&vecParameters, "body"), None);
        }
    }

    #[test]
    fn enumerated_servlet_parameter_names_keep_strict_firewall_defaults() {
        assert!(matches!(
            parse_pairs(b"%00evil=x"),
            Err(AppError::RequestRejected)
        ));
        // U+0378 is an unassigned Unicode scalar value.
        assert!(matches!(
            parse_pairs(b"%CD%B8=x"),
            Err(AppError::RequestRejected)
        ));
        assert_eq!(
            parse_pairs("\u{0438}\u{043c}\u{044f}=value".as_bytes()).unwrap(),
            [("\u{0438}\u{043c}\u{044f}".to_owned(), "value".to_owned())]
        );
        // The project configuration relaxes header values, while Spring's
        // parameter-value predicate accepts this value in the pinned probe.
        assert_eq!(
            parse_pairs(b"name=%00value").unwrap()[0].1.as_bytes(),
            b"\0value"
        );
    }

    #[tokio::test]
    async fn named_parameter_access_ignores_unrequested_invalid_names() {
        for sRejectedName in ["%00evil", "%0Aevil", "%0Devil", "%CD%B8"] {
            let vecQuery = servlet_request_parameters(stRequest(
                Method::GET,
                &format!("/check-login?{sRejectedName}=x&nick=probe"),
                "",
            ))
            .await
            .unwrap();
            assert_eq!(get(&vecQuery, "nick"), Some("probe"), "{sRejectedName}");
            assert_eq!(get(&vecQuery, "missing"), None, "{sRejectedName}");
        }

        let vecForm = servlet_request_parameters(stRequest(
            Method::POST,
            "/check-login",
            "%00evil=x&nick=probe",
        ))
        .await
        .unwrap();
        assert_eq!(get(&vecForm, "nick"), Some("probe"));

        // Explicit full-map enumeration remains strict.
        assert!(matches!(
            parse_pairs(b"%00evil=x&nick=probe"),
            Err(AppError::RequestRejected)
        ));
        assert_eq!(
            get(
                &parse_pairs_for_named_access(b"%00evil=x&nick=probe").unwrap(),
                "nick"
            ),
            Some("probe")
        );
    }

    #[derive(Deserialize)]
    struct StProbeQuery {
        value: Option<String>,
        body: Option<String>,
    }

    #[tokio::test]
    async fn read_only_any_adapter_gives_query_values_post_form_precedence() {
        async fn probe(Query(stQuery): Query<StProbeQuery>) -> String {
            format!(
                "{}:{}",
                stQuery.value.as_deref().unwrap_or("missing"),
                stQuery.body.as_deref().unwrap_or("missing")
            )
        }
        let cApp = Router::new()
            .route("/probe", any(probe))
            .route_layer(middleware::from_fn(merge_servlet_post_form_into_query));
        let stResponse = cApp
            .oneshot(stRequest(
                Method::POST,
                "/probe?value=query",
                "value=form&body=present",
            ))
            .await
            .unwrap();
        assert_eq!(stResponse.status(), StatusCode::OK);
        let vecBody = axum::body::to_bytes(stResponse.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&vecBody[..], b"query:present");
    }

    #[tokio::test]
    async fn read_only_any_adapter_preserves_unused_invalid_names() {
        async fn probe(Query(stQuery): Query<StProbeQuery>) -> String {
            stQuery.value.unwrap_or_else(|| "missing".to_owned())
        }
        let cApp = Router::new()
            .route("/probe", any(probe))
            .route_layer(middleware::from_fn(merge_servlet_post_form_into_query));
        let stResponse = cApp
            .oneshot(stRequest(Method::POST, "/probe?%00evil=x", "value=present"))
            .await
            .unwrap();
        assert_eq!(stResponse.status(), StatusCode::OK);
        let vecBody = axum::body::to_bytes(stResponse.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&vecBody[..], b"present");
    }
}
