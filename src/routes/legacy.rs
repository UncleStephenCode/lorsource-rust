use axum::{http::StatusCode, response::{Html, IntoResponse}};

/// Route-level compatibility placeholder.
///
/// It makes unported legacy URLs explicit in the Rust router, so coverage and
/// HTTP compatibility tests can distinguish "route is known but behaviour is
/// pending" from accidental 404s.
pub async fn not_implemented() -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, Html("Legacy endpoint is mapped but the business logic has not been ported yet."))
}

pub async fn gone() -> impl IntoResponse {
    (StatusCode::GONE, Html("Legacy endpoint is no longer available."))
}
