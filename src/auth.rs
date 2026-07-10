use crate::{error::AppError, models::UserSummary, state::AppState};
use axum::{async_trait, extract::{FromRef, FromRequestParts}, http::request::Parts};
use axum_extra::extract::cookie::CookieJar;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct CurrentUser(pub Option<UserSummary>);

#[async_trait]
impl<S> FromRequestParts<S> for CurrentUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let jar = CookieJar::from_request_parts(parts, state).await.map_err(|_| AppError::Forbidden)?;
        let Some(cookie) = jar.get("lor_session") else { return Ok(CurrentUser(None)); };
        let Some(user_id) = verify_session(cookie.value(), &app.config.cookie_secret) else { return Ok(CurrentUser(None)); };
        let user = sqlx::query_as::<_, UserSummary>(
            r#"SELECT id, nick, name, score, max_score, photo, town, regdate, canmod, blocked, userinfo
               FROM users WHERE id=$1"#,
        )
        .bind(user_id)
        .fetch_optional(&app.pool)
        .await?;
        Ok(CurrentUser(user))
    }
}

pub fn make_session(user_id: i32, secret: &str) -> String {
    let payload = user_id.to_string();
    let sig = sign(&payload, secret);
    format!("{}.{}", URL_SAFE_NO_PAD.encode(payload), sig)
}

pub fn verify_session(value: &str, secret: &str) -> Option<i32> {
    let (payload64, sig) = value.split_once('.')?;
    let payload = String::from_utf8(URL_SAFE_NO_PAD.decode(payload64).ok()?).ok()?;
    if sign(&payload, secret) == sig { payload.parse().ok() } else { None }
}

fn sign(payload: &str, secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(payload.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}
