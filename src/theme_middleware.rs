//! Server-side rendering of the selected theme's `data-theme` attribute.
//!
//! Java's original (`WEB-INF/jsp/head.jsp`) renders `data-theme` straight
//! from `template.style` - the logged-in user's saved theme, resolved
//! server-side, before any HTML reaches the browser. The Rust templates
//! previously baked in a static `data-theme="tango-auto"` and relied
//! entirely on a client-side script to read a cookie and patch the
//! attribute after the fact, which is both a flash-of-wrong-theme and
//! doesn't work without JS. Rather than threading a `theme` field through
//! all 14 templates that extend base.html, this middleware resolves the
//! theme once per request and rewrites the placeholder in the rendered
//! HTML - the base.html client script stays only as a same-tab, no-reload
//! hint for the moment a user changes their theme in Settings.

use crate::{profile::THEMES, state::AppState};
use axum::{
    body::Body,
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::Response,
};
use axum_extra::extract::cookie::CookieJar;

const PLACEHOLDER: &str = "data-theme=\"tango-auto\"";
const DEFAULT_THEME: &str = "tango-auto";

fn is_known_theme(value: &str) -> bool {
    THEMES.iter().any(|(id, _, _)| *id == value)
}

async fn resolve_theme(state: &AppState, jar: &CookieJar) -> String {
    if let Some(cookie) = jar.get("lor_theme") {
        if is_known_theme(cookie.value()) {
            return cookie.value().to_string();
        }
    }
    if let Some(session) = jar.get("lor_session") {
        if let Some(user_id) = crate::auth::verify_session(session.value(), &state.config.cookie_secret) {
            let style: Option<String> = sqlx::query_scalar("SELECT settings->'style' FROM user_settings WHERE id=$1")
                .bind(user_id)
                .fetch_optional(&state.pool)
                .await
                .ok()
                .flatten();
            if let Some(style) = style.filter(|s| is_known_theme(s)) {
                return style;
            }
        }
    }
    DEFAULT_THEME.to_string()
}

pub async fn apply_theme(State(state): State<AppState>, jar: CookieJar, req: Request, next: Next) -> Response {
    let theme = resolve_theme(&state, &jar).await;
    let response = next.run(req).await;

    if theme == DEFAULT_THEME {
        return response;
    }
    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.starts_with("text/html"))
        .unwrap_or(false);
    if !is_html {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
        return Response::from_parts(parts, Body::empty());
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    if !text.contains(PLACEHOLDER) {
        return Response::from_parts(parts, Body::from(bytes));
    }

    // `theme` is checked against the fixed THEMES allow-list in
    // resolve_theme above, so splicing it into the attribute here is safe -
    // this must stay an allow-list check, not escaping, since the value
    // ultimately comes from a client-controlled cookie.
    let rewritten = text.replacen(PLACEHOLDER, &format!("data-theme=\"{theme}\""), 1);
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(rewritten))
}
