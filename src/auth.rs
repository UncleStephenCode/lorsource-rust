use crate::{error::AppError, models::UserSummary, security, state::AppState};
use axum::{async_trait, extract::{FromRef, FromRequestParts}, http::request::Parts};
use axum_extra::extract::cookie::CookieJar;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

const SESSION_MAX_AGE_SECONDS: i64 = 365 * 24 * 60 * 60;

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
               FROM users WHERE id=$1 AND activated AND NOT COALESCE(blocked,false)"#,
        )
        .bind(user_id)
        .fetch_optional(&app.pool)
        .await?;
        Ok(CurrentUser(user))
    }
}

pub fn make_session(user_id: i32, secret: &str) -> String {
    security::make_timed_session(user_id, secret)
}

pub fn verify_session(value: &str, secret: &str) -> Option<i32> {
    security::verify_timed_session(value, secret, SESSION_MAX_AGE_SECONDS)
        .or_else(|| verify_legacy_session(value, secret))
}

/// Kept only for development cookies produced by the first MVP archive.
fn verify_legacy_session(value: &str, secret: &str) -> Option<i32> {
    let (payload64, sig) = value.split_once('.')?;
    let payload = String::from_utf8(URL_SAFE_NO_PAD.decode(payload64).ok()?).ok()?;
    if security::sign_payload(&payload, secret) == sig { payload.parse().ok() } else { None }
}

pub async fn verify_login(pool: &sqlx::PgPool, login: &str, password: &str) -> Result<Option<i32>, sqlx::Error> {
    let row: Option<(i32, Option<String>)> = sqlx::query_as(
        r#"SELECT id, passwd
           FROM users
           WHERE (lower(nick)=lower($1) OR lower(COALESCE(email,''))=lower($1))
             AND activated
             AND NOT COALESCE(blocked,false)"#,
    )
    .bind(login.trim())
    .fetch_optional(pool)
    .await?;

    let Some((id, encoded_password)) = row else { return Ok(None); };
    let Some(encoded_password) = encoded_password else { return Ok(None); };
    if security::password::verify(password, &encoded_password) {
        Ok(Some(id))
    } else {
        Ok(None)
    }
}
