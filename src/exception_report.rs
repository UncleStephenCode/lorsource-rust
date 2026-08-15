use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use once_cell::sync::Lazy;
use regex::Regex;

use crate::{
    application::exception_reporting::StExceptionReport, error::StInternalErrorReport,
    state::AppState,
};

#[derive(Debug, Clone, Default)]
pub struct StExceptionRequestContext {
    optNick: Arc<Mutex<Option<String>>>,
}

impl StExceptionRequestContext {
    pub fn vSetCurrentNick(&self, sNick: &str) {
        if let Ok(mut optNick) = self.optNick.lock() {
            *optNick = Some(sNick.to_owned());
        }
    }

    fn optCurrentNick(&self) -> Option<String> {
        self.optNick.lock().ok().and_then(|optNick| optNick.clone())
    }
}

pub async fn apply(
    State(stState): State<AppState>,
    mut stRequest: Request,
    oNext: Next,
) -> Response {
    let sMethod = stRequest.method().to_string();
    let sUri = stRequest.uri().to_string();
    let sIp = stRequest
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|stPeer| {
            crate::security::stClientIp(
                stPeer.0.ip(),
                stRequest.headers(),
                &stState.config.trusted_proxy_cidrs,
            )
            .to_string()
        })
        .unwrap_or_default();
    let sHeaders = sSanitizedHeaders(stRequest.headers());
    let stRequestContext = StExceptionRequestContext::default();
    stRequest.extensions_mut().insert(stRequestContext.clone());

    let stResponse = oNext.run(stRequest).await;
    if let Some(stError) = stResponse.extensions().get::<StInternalErrorReport>() {
        let sBody = sReportBody(
            stError,
            &sMethod,
            stState.config.public_url.trim_end_matches('/'),
            &sUri,
            &sIp,
            stRequestContext.optCurrentNick().as_deref(),
            &sHeaders,
        );
        stState.exception_reporter.vReport(StExceptionReport {
            sType: stError.sType.clone(),
            sBody,
        });
    }
    stResponse
}

fn sReportBody(
    stError: &StInternalErrorReport,
    sMethod: &str,
    sPublicUrl: &str,
    sUri: &str,
    sIp: &str,
    optCurrentNick: Option<&str>,
    sHeaders: &str,
) -> String {
    let sCurrentUser = optCurrentNick
        .map(|sNick| format!("Current user: {sNick}\n"))
        .unwrap_or_default();
    let sSafeUri = sRedactedRequestUri(sUri);
    let sSafeDebug = sRedactSensitiveText(&stError.sDebug);
    format!(
        "{}\n\n{sMethod}: {sPublicUrl}{sSafeUri}\nIP: {sIp}\n{sCurrentUser}Headers: {sHeaders}\n\n{sSafeDebug}",
        stError.sType,
    )
}

const ARR_SENSITIVE_QUERY_MARKERS: [&str; 6] =
    ["activation", "reset", "code", "token", "password", "secret"];

static O_SENSITIVE_HEADER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)(?P<name>\b(?:authorization|proxy-authorization))\s*:[^\r\n]*")
        .expect("sensitive header regex")
});

static O_SENSITIVE_VALUE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)(?P<key>[a-z0-9_.~-]*(?:activation|reset|code|token|password|secret)[a-z0-9_.~-]*)(?P<separator>\s*[=:]\s*)(?:"[^"]*"|'[^']*'|[^&\s,;)\]}>]+)"#,
    )
    .expect("sensitive value regex")
});

fn sSanitizedHeaders(stHeaders: &HeaderMap) -> String {
    stHeaders
        .iter()
        .filter(|(stName, _)| {
            !matches!(
                stName.as_str().to_ascii_lowercase().as_str(),
                "cookie" | "authorization" | "proxy-authorization"
            )
        })
        .filter_map(|(stName, stValue)| {
            stValue
                .to_str()
                .ok()
                .map(|sValue| format!("\n         {stName}: {sValue}"))
        })
        .collect::<String>()
}

fn sRedactedRequestUri(sUri: &str) -> String {
    let Some((sPath, sQuery)) = sUri.split_once('?') else {
        return sUri.to_owned();
    };
    let sSafeQuery = sQuery
        .split('&')
        .map(|sPair| {
            let (sKey, optValue) = sPair
                .split_once('=')
                .map_or((sPair, None), |(sKey, sValue)| (sKey, Some(sValue)));
            let bSensitiveKey = urlencoding::decode(sKey)
                .map(|sDecoded| bSensitiveQueryKey(&sDecoded))
                .unwrap_or_else(|_| bSensitiveQueryKey(sKey));
            let bNestedSensitiveQuery = optValue.is_some_and(bContainsSensitiveNestedQuery);
            if bSensitiveKey || bNestedSensitiveQuery {
                format!("{sKey}=[REDACTED]")
            } else if let Some(sValue) = optValue {
                format!("{sKey}={sValue}")
            } else {
                sKey.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{sPath}?{sSafeQuery}")
}

fn bSensitiveQueryKey(sKey: &str) -> bool {
    let sLower = sKey.to_ascii_lowercase();
    ARR_SENSITIVE_QUERY_MARKERS
        .iter()
        .any(|sMarker| sLower.contains(sMarker))
}

fn bContainsSensitiveNestedQuery(sValue: &str) -> bool {
    let mut sInspected = sValue.to_owned();
    for _ in 0..2 {
        if ARR_SENSITIVE_QUERY_MARKERS.iter().any(|sMarker| {
            let sLower = sInspected.to_ascii_lowercase();
            sLower.contains(&format!("{sMarker}=")) || sLower.contains(&format!("{sMarker}%3d"))
        }) {
            return true;
        }
        let Ok(sDecoded) = urlencoding::decode(&sInspected) else {
            break;
        };
        if sDecoded == sInspected {
            break;
        }
        sInspected = sDecoded.into_owned();
    }
    false
}

fn sRedactSensitiveText(sText: &str) -> String {
    let sWithoutHeaders = O_SENSITIVE_HEADER.replace_all(sText, "[REDACTED CREDENTIAL HEADER]");
    O_SENSITIVE_VALUE
        .replace_all(&sWithoutHeaders, "${key}${separator}[REDACTED]")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_body_matches_java_user_and_cookie_redaction_shape() {
        let sBody = sReportBody(
            &StInternalErrorReport {
                sType: "sqlx::Error".to_owned(),
                sDebug: "database unavailable".to_owned(),
            },
            "POST",
            "https://www.linux.org.ru",
            "/add.jsp?section=2",
            "192.0.2.10",
            Some("maxcom"),
            "\n         user-agent: test",
        );
        assert!(sBody.contains("POST: https://www.linux.org.ru/add.jsp?section=2"));
        assert!(sBody.contains("IP: 192.0.2.10\nCurrent user: maxcom\nHeaders:"));
        assert!(!sBody.to_ascii_lowercase().contains("cookie:"));
        assert!(sBody.ends_with("database unavailable"));
    }

    #[test]
    fn report_redacts_auth_headers_and_sensitive_query_values() {
        let mut stHeaders = HeaderMap::new();
        stHeaders.insert("authorization", "Bearer top-secret".parse().unwrap());
        stHeaders.insert(
            "proxy-authorization",
            "Basic cHJveHk6c2VjcmV0".parse().unwrap(),
        );
        stHeaders.insert("cookie", "session=also-secret".parse().unwrap());
        stHeaders.insert("user-agent", "compat-test".parse().unwrap());

        let sBody = sReportBody(
            &StInternalErrorReport {
                sType: "anyhow::Error".to_owned(),
                sDebug: concat!(
                    "upstream Authorization: Bearer debug-secret\n",
                    "password=\"debug-password\" resetCode: debug-reset"
                )
                .to_owned(),
            },
            "GET",
            "https://www.linux.org.ru",
            "/activate?nick=bird&activation=request-secret&safe=value&TOKEN=token-secret&redirect=%2Factivate%3Fcode%3Dnested-secret",
            "192.0.2.11",
            None,
            &sSanitizedHeaders(&stHeaders),
        );

        assert!(sBody.contains("nick=bird"));
        assert!(sBody.contains("safe=value"));
        assert!(sBody.contains("activation=[REDACTED]"));
        assert!(sBody.contains("TOKEN=[REDACTED]"));
        assert!(sBody.contains("redirect=[REDACTED]"));
        assert!(sBody.contains("user-agent: compat-test"));
        assert!(sBody.contains("password=[REDACTED]"));
        assert!(sBody.contains("resetCode: [REDACTED]"));
        assert!(!sBody.to_ascii_lowercase().contains("authorization:"));
        for sSecret in [
            "top-secret",
            "cHJveHk6c2VjcmV0",
            "also-secret",
            "request-secret",
            "token-secret",
            "nested-secret",
            "debug-secret",
            "debug-password",
            "debug-reset",
        ] {
            assert!(!sBody.contains(sSecret), "leaked {sSecret}: {sBody}");
        }
    }

    #[test]
    fn query_redaction_handles_case_and_percent_encoded_parameter_names() {
        let sUri = sRedactedRequestUri(
            "/reset?ResetCode=one&access_%74oken=two&client_secret=three&topic=42",
        );
        assert_eq!(
            sUri,
            "/reset?ResetCode=[REDACTED]&access_%74oken=[REDACTED]&client_secret=[REDACTED]&topic=42"
        );
    }
}
