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
            r#"SELECT id, nick, name, score, max_score, photo, town, regdate, canmod, COALESCE(candel,false) AS candel, COALESCE(corrector,false) AS corrector, blocked, userinfo
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

#[derive(Debug, Clone)]
pub struct StLoginIdentity {
    pub id: i32,
}

pub type LoginIdentity = StLoginIdentity;

/// LoginController.loginProcess's exception branches: a frozen account
/// (Spring's `LockedException`) is silently redirected to the profile with
/// no error message and, crucially, does *not* count as a failed attempt -
/// unlike bad credentials or an unactivated account.
pub enum LoginOutcome {
    Success(LoginIdentity),
    Frozen(String),
    Failed,
}

pub async fn verify_login(pool: &sqlx::PgPool, login: &str, password: &str) -> Result<LoginOutcome, sqlx::Error> {
    let row: Option<(i32, String, Option<String>, bool, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        r#"SELECT u.id, u.nick, u.passwd, u.activated, u.frozen_until
           FROM users u
           WHERE (lower(u.nick)=lower($1) OR lower(COALESCE(u.email,''))=lower($1))
             AND NOT COALESCE(u.blocked,false)
           LIMIT 1"#,
    )
    .bind(login.trim())
    .fetch_optional(pool)
    .await?;

    let Some((id, nick, encoded_password, activated, frozen_until)) = row else { return Ok(LoginOutcome::Failed); };
    let Some(encoded_password) = encoded_password else { return Ok(LoginOutcome::Failed); };
    if !security::password::verify(password, &encoded_password) {
        return Ok(LoginOutcome::Failed);
    }
    if !activated {
        return Ok(LoginOutcome::Failed);
    }
    if frozen_until.map(|u| u > chrono::Utc::now()).unwrap_or(false) {
        return Ok(LoginOutcome::Frozen(nick));
    }
    Ok(LoginOutcome::Success(LoginIdentity { id }))
}
