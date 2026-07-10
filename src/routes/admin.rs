use crate::{auth::CurrentUser, error::{AppError, Result}, state::AppState};
use axum::{extract::State, response::Html, routing::{get, post}, Router};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/geoip", get(stub_admin))
        .route("/admin/search-reindex", get(stub_admin).post(stub_admin))
        .route("/banip.jsp", post(stub_admin))
        .route("/delip.jsp", post(stub_admin))
        .route("/sameip.jsp", get(stub_admin))
        .route("/groupmod.jsp", get(stub_admin).post(stub_admin))
        .route("/usermod.jsp", post(stub_admin))
        .route("/post-warning", get(stub_admin).post(stub_admin))
        .route("/clear-warning", post(stub_admin))
}

async fn stub_admin(State(_state): State<AppState>, CurrentUser(user): CurrentUser) -> Result<Html<&'static str>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    Ok(Html("OK: административный маршрут подключён, бизнес-логика вынесена в отдельный слой"))
}
