use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

fn bShouldCount(sPath: &str, stStatus: StatusCode) -> bool {
    sPath.starts_with("/adv/") && (stStatus.is_success() || stStatus.is_redirection())
}

/// `AdvCounterInterceptor.postHandle`: count only successful `/adv/**`
/// resource responses and preserve `request.getRequestURI` (without query).
pub async fn apply(
    State(stState): State<AppState>,
    stRequest: Request<Body>,
    oNext: Next,
) -> Response {
    let sPath = stRequest.uri().path().to_owned();
    let stResponse = oNext.run(stRequest).await;
    if bShouldCount(&sPath, stResponse.status()) {
        stState.adv_counter.vCount(sPath);
    }
    stResponse
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_java_mapped_interceptor_and_status_window() {
        assert!(bShouldCount("/adv/banner.png", StatusCode::OK));
        assert!(bShouldCount("/adv/banner.png", StatusCode::NOT_MODIFIED));
        assert!(bShouldCount("/adv/banner.png", StatusCode::FOUND));
        assert!(!bShouldCount("/adv/banner.png", StatusCode::NOT_FOUND));
        assert!(!bShouldCount("/adv/", StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!bShouldCount("/adv", StatusCode::OK));
        assert!(!bShouldCount("/img/banner.png", StatusCode::OK));
    }
}
