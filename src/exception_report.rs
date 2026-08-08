use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::Response,
};

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
    let sHeaders = stRequest
        .headers()
        .iter()
        .filter(|(stName, _)| !stName.as_str().eq_ignore_ascii_case("cookie"))
        .filter_map(|(stName, stValue)| {
            stValue
                .to_str()
                .ok()
                .map(|sValue| format!("\n         {stName}: {sValue}"))
        })
        .collect::<String>();
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
    format!(
        "{}\n\n{sMethod}: {sPublicUrl}{sUri}\nIP: {sIp}\n{sCurrentUser}Headers: {sHeaders}\n\n{}",
        stError.sType, stError.sDebug,
    )
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
}
