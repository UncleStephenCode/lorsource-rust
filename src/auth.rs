use crate::{
    error::{AppError, Result},
    models::UserSummary,
    security,
    state::AppState,
};
use axum::{
    body::Body,
    extract::{FromRef, FromRequestParts, Request, State},
    http::request::Parts,
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};

#[derive(Debug, Clone)]
pub struct CurrentUser(pub Option<UserSummary>);

impl<S> FromRequestParts<S> for CurrentUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        if let Some(stCurrentUser) = parts.extensions.get::<CurrentUser>() {
            return Ok(stCurrentUser.clone());
        }
        let app = AppState::from_ref(state);
        let jar = CookieJar::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::Forbidden)?;
        let Some(user_id) = optUserIdFromCookies(&app.pool, &jar, &app.config.site_secret).await?
        else {
            return Ok(CurrentUser(None));
        };
        let user = sqlx::query_as::<_, UserSummary>(
            r#"SELECT id, nick, name, score, max_score, photo, town, regdate,
                      COALESCE(canmod,false) AS canmod,
                      COALESCE(candel,false) AS candel,
                      (COALESCE(corrector,false)
                       AND NOT COALESCE(frozen_until > CURRENT_TIMESTAMP,false)) AS corrector,
                      blocked, userinfo
               FROM users WHERE id=$1 AND activated AND NOT COALESCE(blocked,false)"#,
        )
        .bind(user_id)
        .fetch_optional(&app.pool)
        .await?;
        if user.is_some() {
            // LastLoginInterceptor/UserDao.updateLastlogin(force=false): keep
            // activity fresh without rewriting the row more than once/hour.
            sqlx::query(
                "UPDATE users SET lastlogin=CURRENT_TIMESTAMP \
                 WHERE id=$1 AND CURRENT_TIMESTAMP-lastlogin > interval '1 hour'",
            )
            .bind(user_id)
            .execute(&app.pool)
            .await?;
        }
        if let (Some(stContext), Some(stUser)) = (
            parts
                .extensions
                .get::<crate::exception_report::StExceptionRequestContext>(),
            user.as_ref(),
        ) {
            stContext.vSetCurrentNick(&stUser.nick);
        }
        Ok(CurrentUser(user))
    }
}

/// Java's `LastLoginInterceptor` executes for every DispatcherServlet request,
/// including handlers which do not explicitly request an authenticated user.
/// Resolve the session once at the application boundary, update activity with
/// the original one-hour throttle, and cache it for route extractors.
pub async fn hydrate(
    State(stState): State<AppState>,
    mut stRequest: Request<Body>,
    oNext: Next,
) -> Response {
    if crate::security::bSpringSecurityIgnoredPath(stRequest.uri().path()) {
        // These paths use `security="none"` in Spring. Keep an explicit
        // anonymous extension for a fallback handler, but avoid remember-me
        // verification, a user lookup and LastLogin writes for every asset.
        stRequest.extensions_mut().insert(CurrentUser(None));
        return oNext.run(stRequest).await;
    }
    let (mut stParts, stBody) = stRequest.into_parts();
    let stCurrentUser = match CurrentUser::from_request_parts(&mut stParts, &stState).await {
        Ok(stCurrentUser) => stCurrentUser,
        Err(stError) => return stError.into_response(),
    };
    stParts.extensions.insert(stCurrentUser);
    oNext.run(Request::from_parts(stParts, stBody)).await
}

type TyRememberMeRow = (i32, String, String, i32, bool);

pub async fn optRememberMeUserId(
    oPool: &sqlx::PgPool,
    sCookie: &str,
    sSecret: &str,
) -> std::result::Result<Option<i32>, sqlx::Error> {
    let Some(stToken) = security::remember_me::optDecode(sCookie) else {
        return Ok(None);
    };
    let optRow = sqlx::query_as::<_, TyRememberMeRow>(
        r#"SELECT id, nick, COALESCE(passwd,''), COALESCE(token_generation,0),
                  COALESCE(blocked,false)
           FROM users WHERE nick=$1"#,
    )
    .bind(&stToken.sUsername)
    .fetch_optional(oPool)
    .await?;
    let Some((iUserId, _sNick, sPasswordHash, iTokenGeneration, bBlocked)) = optRow else {
        return Ok(None);
    };
    if bBlocked || sPasswordHash.is_empty() {
        return Ok(None);
    }
    let bValid = security::remember_me::bVerify(
        &stToken,
        &sPasswordHash,
        sSecret,
        iTokenGeneration,
        chrono::Utc::now().timestamp_millis(),
    );
    Ok(bValid.then_some(iUserId))
}

pub async fn optUserIdFromCookies(
    oPool: &sqlx::PgPool,
    oJar: &CookieJar,
    sJavaSecret: &str,
) -> std::result::Result<Option<i32>, sqlx::Error> {
    if let Some(stCookie) = oJar.get(security::remember_me::COOKIE_NAME)
        && let Some(iUserId) = optRememberMeUserId(oPool, stCookie.value(), sJavaSecret).await?
    {
        return Ok(Some(iUserId));
    }

    Ok(None)
}

#[derive(Debug, Clone)]
pub struct StLoginIdentity {
    pub nick: String,
    pub password_hash: String,
    pub token_generation: i32,
}

pub type LoginIdentity = StLoginIdentity;

/// LoginController.loginProcess's exception branches: a blocked account
/// (Spring's `LockedException`) is silently redirected to the profile with
/// no error message and, crucially, does *not* count as a failed attempt -
/// unlike bad credentials or an unactivated account.
pub enum LoginOutcome {
    Success(LoginIdentity),
    Blocked,
    NotActivated,
    Failed,
}

#[derive(sqlx::FromRow)]
struct StLoginRow {
    id: i32,
    nick: String,
    passwd: Option<String>,
    activated: bool,
    blocked: bool,
    token_generation: i32,
}

pub async fn optLoadLoginIdentity(
    oPool: &sqlx::PgPool,
    iUserId: i32,
) -> std::result::Result<Option<LoginIdentity>, sqlx::Error> {
    let optRow = sqlx::query_as::<_, StLoginRow>(
        r#"SELECT id,nick,passwd,activated,COALESCE(blocked,false) AS blocked,
                  COALESCE(token_generation,0) AS token_generation
           FROM users WHERE id=$1"#,
    )
    .bind(iUserId)
    .fetch_optional(oPool)
    .await?;
    Ok(optRow.and_then(|stRow| {
        (!stRow.blocked && stRow.activated).then(|| LoginIdentity {
            nick: stRow.nick,
            password_hash: stRow.passwd.unwrap_or_default(),
            token_generation: stRow.token_generation,
        })
    }))
}

pub fn sMakeRememberMeCookieValue(stIdentity: &LoginIdentity, sSecret: &str) -> String {
    let iExpiryMillis =
        chrono::Utc::now().timestamp_millis() + security::remember_me::VALIDITY_SECONDS * 1_000;
    security::remember_me::sEncode(
        &stIdentity.nick,
        iExpiryMillis,
        &stIdentity.password_hash,
        sSecret,
        stIdentity.token_generation,
    )
}

/// Canonical Spring-compatible remember-me cookie used after authentication
/// and after a password change. Keeping the attributes here prevents a route
/// from refreshing the password-bound signature with weaker cookie flags.
pub fn stRememberMeCookie(
    stIdentity: &LoginIdentity,
    sSecret: &str,
    bSecure: bool,
) -> Cookie<'static> {
    Cookie::build((
        security::remember_me::COOKIE_NAME,
        sMakeRememberMeCookieValue(stIdentity, sSecret),
    ))
    .path("/")
    .max_age(time::Duration::seconds(
        security::remember_me::VALIDITY_SECONDS,
    ))
    .http_only(true)
    .secure(bSecure)
    .build()
}

pub async fn verify_login(
    pool: &sqlx::PgPool,
    login: &str,
    password: &str,
) -> Result<LoginOutcome> {
    let sLogin = login.trim();
    let optRow: Option<StLoginRow> = if sLogin.contains('@') {
        if sLogin.chars().filter(|c| *c == '@').count() != 1
            || sLogin.chars().any(char::is_whitespace)
        {
            None
        } else {
            sqlx::query_as(
                r#"SELECT id,nick,passwd,activated,COALESCE(blocked,false) AS blocked,
                          COALESCE(token_generation,0) AS token_generation
                   FROM users
                   WHERE normalize_email(email)=normalize_email(lower($1))
                   ORDER BY blocked ASC, id DESC LIMIT 1"#,
            )
            .bind(sLogin)
            .fetch_optional(pool)
            .await?
        }
    } else {
        sqlx::query_as(
            r#"SELECT id,nick,passwd,activated,COALESCE(blocked,false) AS blocked,
                      COALESCE(token_generation,0) AS token_generation
               FROM users WHERE nick=$1"#,
        )
        .bind(sLogin)
        .fetch_optional(pool)
        .await?
    };

    let Some(stRow) = optRow else {
        return Ok(LoginOutcome::Failed);
    };
    // DaoAuthenticationProvider checks account status before credentials.
    if stRow.blocked {
        return Ok(LoginOutcome::Blocked);
    }
    let Some(sEncodedPassword) = stRow.passwd else {
        return Ok(LoginOutcome::Failed);
    };
    if !security::password::verify(password, &sEncodedPassword) {
        return Ok(LoginOutcome::Failed);
    }

    // `DaoAuthenticationProvider` upgrades the old Jasypt value before the
    // controller creates its remember-me cookie. The conditional UPDATE keeps
    // concurrent password changes from being overwritten.
    let sCurrentPasswordHash = if security::password::is_bcrypt(&sEncodedPassword) {
        sEncodedPassword
    } else {
        let sBcrypt = security::password::hash(password)
            .map_err(|stError| AppError::Anyhow(anyhow::Error::new(stError)))?;
        let stUpdated = sqlx::query("UPDATE users SET passwd=$1 WHERE id=$2 AND passwd=$3")
            .bind(&sBcrypt)
            .bind(stRow.id)
            .bind(&sEncodedPassword)
            .execute(pool)
            .await?;
        if stUpdated.rows_affected() == 1 {
            sBcrypt
        } else {
            sqlx::query_scalar::<_, Option<String>>("SELECT passwd FROM users WHERE id=$1")
                .bind(stRow.id)
                .fetch_one(pool)
                .await?
                .unwrap_or_default()
        }
    };

    if !stRow.activated {
        return Ok(LoginOutcome::NotActivated);
    }

    sqlx::query("UPDATE users SET lastlogin=CURRENT_TIMESTAMP WHERE id=$1")
        .bind(stRow.id)
        .execute(pool)
        .await?;
    Ok(LoginOutcome::Success(LoginIdentity {
        nick: stRow.nick,
        password_hash: sCurrentPasswordHash,
        token_generation: stRow.token_generation,
    }))
}

#[cfg(test)]
mod remember_me_cookie_tests {
    use super::{LoginIdentity, stRememberMeCookie};
    use crate::security::remember_me;

    #[test]
    fn password_refresh_cookie_has_login_attributes_and_uses_the_new_hash() {
        let stIdentity = LoginIdentity {
            nick: "alice".to_owned(),
            password_hash: "$2b$12$new-hash".to_owned(),
            token_generation: 7,
        };
        let stCookie = stRememberMeCookie(&stIdentity, "test-secret", true);

        assert_eq!(stCookie.name(), remember_me::COOKIE_NAME);
        assert_eq!(stCookie.path(), Some("/"));
        assert_eq!(
            stCookie
                .max_age()
                .map(|stDuration| stDuration.whole_seconds()),
            Some(remember_me::VALIDITY_SECONDS)
        );
        assert_eq!(stCookie.http_only(), Some(true));
        assert_eq!(stCookie.secure(), Some(true));

        let stToken = remember_me::optDecode(stCookie.value()).expect("remember-me token");
        assert!(remember_me::bVerify(
            &stToken,
            &stIdentity.password_hash,
            "test-secret",
            stIdentity.token_generation,
            chrono::Utc::now().timestamp_millis(),
        ));
        assert!(!remember_me::bVerify(
            &stToken,
            "$2b$12$old-hash",
            "test-secret",
            stIdentity.token_generation,
            chrono::Utc::now().timestamp_millis(),
        ));
    }
}
