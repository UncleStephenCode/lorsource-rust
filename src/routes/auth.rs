use crate::{
    auth,
    auth::CurrentUser,
    error::{AppError, Result},
    state::AppState,
};
use askama::Template;
use axum::{
    Form,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use rand::Rng;
use serde::Deserialize;
use time::Duration;

pub(crate) fn cEmailService(
    stState: &AppState,
) -> crate::application::email::CEmailService<crate::infra::smtp::CSmtpEmailSender> {
    crate::application::email::CEmailService::new(
        crate::infra::smtp::CSmtpEmailSender::from_env(),
        stState.config.site_secret.clone(),
    )
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate<'a> {
    title: &'a str,
    error: Option<String>,
    nick: String,
    login_action: String,
    redirect_url: String,
    csrf_token: String,
}

/// Mirrors LoginController.safeRedirectUrl from the Java/Scala implementation:
/// only same-site relative redirects are accepted; everything else goes to `/`.
fn safe_redirect_url(from: &str) -> String {
    if from.starts_with('/') && !from.starts_with("//") && !from.starts_with("/\\") {
        from.to_string()
    } else {
        "/".to_string()
    }
}

fn found(sLocation: &str) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, sLocation.to_owned())],
    )
        .into_response()
}

/// Redirect an unauthenticated browser page to the login form while retaining
/// the complete local request target.  Encoding the target as a single query
/// value is important: `/login.jsp` decodes it back into `LoginQuery::from`,
/// then carries the value in the original `redirectUrl` form field.
pub(crate) fn login_redirect(sFrom: &str) -> Response {
    let sFrom = safe_redirect_url(sFrom);
    Redirect::to(&format!("/login.jsp?from={}", urlencoding::encode(&sFrom))).into_response()
}

#[derive(Template)]
#[template(path = "register.html")]
struct RegisterTemplate<'a> {
    title: &'a str,
    error: Option<String>,
    permit: String,
    csrf_token: String,
}

#[derive(Deserialize)]
pub struct LoginQuery {
    pub from: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub nick: String,
    pub passwd: String,
    #[serde(rename = "redirectUrl", alias = "redirect_url")]
    pub redirect_url: Option<String>,
}

pub async fn login_form(
    State(state): State<AppState>,
    Query(query): Query<LoginQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<impl IntoResponse> {
    let redirect_url = safe_redirect_url(query.from.as_deref().unwrap_or(""));
    if current_user.is_some() {
        return Ok(found(&redirect_url));
    }
    Ok(Html(
        LoginTemplate {
            title: "Login",
            error: None,
            nick: String::new(),
            login_action: format!(
                "{}/login_process",
                state.config.public_url.trim_end_matches('/')
            ),
            redirect_url,
            csrf_token,
        }
        .render()?,
    )
    .into_response())
}

/// LoginController.delayResponse: every response (success or failure) is
/// delayed by a random 1-3 seconds, which blunts both brute-force
/// throughput and timing-based username enumeration (a real account
/// lookup + bcrypt verify vs. an instant "not found" would otherwise be
/// distinguishable). Applied uniformly instead of only-on-success/failure
/// so the delay itself never becomes an extra timing signal.
async fn delay_response() {
    let millis = rand::thread_rng().gen_range(1000..3000);
    tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
}

pub async fn login(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    jar: CookieJar,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    Form(form): Form<LoginForm>,
) -> Result<Response> {
    let redirect_url = safe_redirect_url(form.redirect_url.as_deref().unwrap_or(""));
    if current_user.is_some() {
        return Ok(found(&redirect_url));
    }
    let outcome = auth::verify_login(&state.pool, &form.nick, &form.passwd).await?;
    delay_response().await;
    let identity = match outcome {
        auth::LoginOutcome::Success(identity) => identity,
        // LockedException (blocked account): silently back to their own
        // profile, no error message, no failed-attempt penalty.
        auth::LoginOutcome::Blocked => {
            let sLocation = format!("/people/{}/profile", urlencoding::encode(form.nick.trim()));
            return Ok((jar, found(&sLocation)).into_response());
        }
        auth::LoginOutcome::NotActivated => {
            return Ok(Html(
                LoginTemplate {
                    title: "Login",
                    error: Some("Регистрация не завершена! Инструкция по активации отправлена на указанный при регистрации email.".to_owned()),
                    nick: form.nick.clone(),
                    login_action: format!(
                        "{}/login_process",
                        state.config.public_url.trim_end_matches('/')
                    ),
                    redirect_url,
                    csrf_token,
                }
                .render()?,
            )
            .into_response());
        }
        auth::LoginOutcome::Failed => {
            return Ok(Html(
                LoginTemplate {
                    title: "Login",
                    error: Some(
                        "Ошибка авторизации. Неправильное имя пользователя, e-mail или пароль."
                            .to_owned(),
                    ),
                    nick: form.nick.clone(),
                    login_action: format!(
                        "{}/login_process",
                        state.config.public_url.trim_end_matches('/')
                    ),
                    redirect_url,
                    csrf_token,
                }
                .render()?,
            )
            .into_response());
        }
    };
    let is_secure = crate::security::is_secure_request(&headers);
    let token = auth::sMakeRememberMeCookieValue(&identity, &state.config.site_secret);
    let session_cookie = Cookie::build((crate::security::remember_me::COOKIE_NAME, token))
        .path("/")
        .max_age(Duration::seconds(
            crate::security::remember_me::VALIDITY_SECONDS,
        ))
        .http_only(true)
        .secure(is_secure)
        .build();
    let jar = jar.remove(Cookie::build(("lor_session", "")).path("/").build());
    Ok((jar.add(session_cookie), found(&redirect_url)).into_response())
}

pub async fn logout(jar: CookieJar) -> Response {
    let stRememberMe = Cookie::build((crate::security::remember_me::COOKIE_NAME, ""))
        .path("/")
        .build();
    let stLegacy = Cookie::build(("lor_session", "")).path("/").build();
    (
        jar.remove(stRememberMe).remove(stLegacy),
        found("/login.jsp"),
    )
        .into_response()
}

pub async fn logout_link(CurrentUser(user): CurrentUser) -> Response {
    match user {
        Some(user) => found(&format!(
            "/people/{}/profile",
            urlencoding::encode(&user.nick)
        )),
        None => found("/login.jsp"),
    }
}

pub async fn logout_all_sessions(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    jar: CookieJar,
) -> Result<Response> {
    if let Some(user) = user {
        sqlx::query("UPDATE users SET token_generation=COALESCE(token_generation,0)+1 WHERE id=$1")
            .bind(user.id)
            .execute(&state.pool)
            .await?;
    }
    Ok(logout(jar).await)
}

pub async fn register_form(
    State(state): State<AppState>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    Ok(Html(
        RegisterTemplate {
            title: "Регистрация",
            error: None,
            permit: make_register_permit(&state),
            csrf_token,
        }
        .render()?,
    ))
}

#[derive(Deserialize)]
pub struct RegisterForm {
    pub nick: String,
    pub email: Option<String>,
    pub password: Option<String>,
    pub passwd: Option<String>,
    pub password2: Option<String>,
    pub rules: Option<String>,
    pub permit: Option<String>,
}

pub async fn register(
    State(state): State<AppState>,
    Form(form): Form<RegisterForm>,
) -> Result<impl IntoResponse> {
    if !check_register_permit(&state, form.permit.as_deref()) {
        return Ok(Html("<h1>Регистрация временно недоступна</h1>".to_string()).into_response());
    }

    let nick = form.nick.trim().to_string();
    let email = form.email.unwrap_or_default().trim().to_lowercase();
    let password = form.password.or(form.passwd).unwrap_or_default();
    let password2 = form.password2.unwrap_or_default();

    if nick.is_empty() {
        return Err(AppError::BadRequest("не задан nick".into()));
    }
    if !crate::routes::legacy::valid_login_name_for_java(&nick) {
        return Err(AppError::BadRequest("некорректное имя пользователя".into()));
    }
    if nick.len() > 19 {
        return Err(AppError::BadRequest(
            "слишком длинное имя пользователя".into(),
        ));
    }
    if password.is_empty() || password2.is_empty() {
        return Err(AppError::BadRequest("пароль не может быть пустым".into()));
    }
    if password.eq_ignore_ascii_case(&nick) {
        return Err(AppError::BadRequest(
            "пароль не может совпадать с логином".into(),
        ));
    }
    if password != password2 {
        return Err(AppError::BadRequest("введенные пароли не совпадают".into()));
    }
    if password.len() < 10 {
        return Err(AppError::BadRequest(
            "слишком короткий пароль, минимальная длина: 10".into(),
        ));
    }
    if email.is_empty() {
        return Err(AppError::BadRequest("Не указан e-mail".into()));
    }
    validate_registration_email(&state, &email).await?;
    if form.rules.as_deref() != Some("okay") {
        return Err(AppError::BadRequest("Вы не согласились с правилами".into()));
    }
    if user_exists_or_similar(&state, &nick).await? {
        return Err(AppError::BadRequest(
            "Это имя пользователя уже используется. Пожалуйста выберите другое имя.".into(),
        ));
    }
    if email_in_use_for_active_or_recently_blocked_user(&state, &email).await? {
        return Err(AppError::BadRequest("пользователь с таким e-mail уже зарегистрирован. Если вы забыли параметры своего аккаунта, воспользуйтесь формой восстановления пароля.".into()));
    }

    let hash =
        crate::security::password::hash(&password).map_err(|e| AppError::Anyhow(e.into()))?;
    let (id, regdate): (i32, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        r#"INSERT INTO users(id,nick,email,passwd,regdate,activated,score,max_score,canmod,candel,corrector,userinfo_markup)
           VALUES(nextval('s_uid')::int,$1,$2,$3,now(),false,45,45,false,false,false,'MARKDOWN')
           RETURNING id, regdate"#,
    )
    .bind(&nick)
    .bind(&email)
    .bind(hash)
    .fetch_one(&state.pool)
    .await?;

    crate::audit::log_user_action(
        &state.pool,
        id,
        id,
        "register",
        &[("email", email.as_str())],
    )
    .await?;
    cEmailService(&state)
        .vSendRegistration(&nick, &email, regdate.timestamp_millis(), true)
        .await?;
    Ok(Html(
        "<h1>Добавление пользователя прошло успешно.</h1><p>Ожидайте письма с кодом активации.</p>"
            .to_string(),
    )
    .into_response())
}

fn make_register_permit(state: &AppState) -> String {
    crate::security::secret_tokens::make_register_permit(
        &state.config.site_secret,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap_or_else(|_| "dev-permit".to_string())
}

fn check_register_permit(state: &AppState, permit: Option<&str>) -> bool {
    let Some(permit) = permit else {
        return false;
    };
    // Dev-only fallback (see ENABLE_DEV_BYPASSES) for local tests where the
    // form was created by older MVP archives - never reachable unless
    // explicitly opted into, since it would otherwise let anyone skip the
    // anti-bot registration-permit token entirely.
    if state.config.enable_dev_bypasses && permit == "dev-permit" {
        return true;
    }
    crate::security::secret_tokens::check_register_permit(
        &state.config.site_secret,
        permit,
        chrono::Utc::now().timestamp_millis(),
    )
}

async fn validate_registration_email(state: &AppState, email: &str) -> Result<()> {
    let Some((_local, domain)) = email.rsplit_once('@') else {
        return Err(AppError::BadRequest("Некорректный e-mail".into()));
    };
    if domain.is_empty() || !domain.contains('.') {
        return Err(AppError::BadRequest("Некорректный e-mail".into()));
    }
    let domain = domain.to_lowercase();
    let top_private = domain
        .split('.')
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(".");
    let cService = crate::application::email_domain_block::CEmailDomainBlockService::new(
        crate::infra::postgres::email_domain_block_repository::CEmailDomainBlockPgRepository::new(
            state.pool.clone(),
        ),
    );
    if cService.bIsBlocked(&domain).await? || cService.bIsBlocked(&top_private).await? {
        return Err(AppError::BadRequest("некорректный email домен".into()));
    }
    Ok(())
}

async fn user_exists_or_similar(state: &AppState, nick: &str) -> Result<bool> {
    let exact: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE lower(nick)=lower($1)")
        .bind(nick)
        .fetch_one(&state.pool)
        .await?;
    if exact > 0 {
        return Ok(true);
    }
    let similar: Option<i32> = sqlx::query_scalar(
        r#"SELECT id FROM users
           WHERE score>=200 AND lastlogin>CURRENT_TIMESTAMP - interval '3 years'
             AND levenshtein_less_equal(lower(nick), lower($1), 1)<=1
           LIMIT 1"#,
    )
    .bind(nick)
    .fetch_optional(&state.pool)
    .await?;
    Ok(similar.is_some())
}

async fn email_in_use_for_active_or_recently_blocked_user(
    state: &AppState,
    email: &str,
) -> Result<bool> {
    let found: Option<i32> = sqlx::query_scalar(
        r#"SELECT id FROM users
           WHERE lower(COALESCE(email,''))=lower($1)
             AND (NOT COALESCE(blocked,false) OR COALESCE(lastlogin, regdate) > CURRENT_TIMESTAMP - interval '90 days')
           LIMIT 1"#,
    )
    .bind(email)
    .fetch_optional(&state.pool)
    .await?;
    Ok(found.is_some())
}

pub async fn lost_password_form(
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    Ok(Html(format!(
        r#"
<h1>Восстановление пароля</h1>
<form method="post" action="/lostpwd.jsp" class="form">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <label>Email <input name="email" type="email" required></label>
  <button type="submit">Отправить инструкцию</button>
</form>
"#
    )))
}

#[derive(Deserialize)]
pub struct LostPasswordForm {
    pub email: String,
}

pub async fn lost_password(
    State(state): State<AppState>,
    CurrentUser(current_user): CurrentUser,
    Form(form): Form<LostPasswordForm>,
) -> Result<Html<String>> {
    let email = form.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(AppError::BadRequest("email не задан".into()));
    }

    let Some((id, nick, stored_email, blocked, activated, canmod, candel, anonymous)) =
        sqlx::query_as::<_, (i32, String, String, Option<bool>, bool, bool, bool, bool)>(
            r#"SELECT id,nick,email,blocked,activated,canmod,candel,
                      (passwd IS NULL OR passwd='') AS anonymous
           FROM users WHERE lower(email)=lower($1) LIMIT 1"#,
        )
        .bind(&email)
        .fetch_optional(&state.pool)
        .await?
    else {
        return Err(AppError::BadRequest(
            "Этот email не зарегистрирован!".into(),
        ));
    };

    if blocked.unwrap_or(false) || !activated || anonymous || candel {
        return Err(AppError::Forbidden);
    }
    let requester_is_moderator = current_user.as_ref().map(|u| u.canmod).unwrap_or(false);
    if canmod && !requester_is_moderator {
        return Err(AppError::Forbidden);
    }
    if !requester_is_moderator {
        let bRecentSelfRequest: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                 SELECT 1 FROM user_log
                  WHERE userid=$1
                    AND action='sent_password_reset'::user_log_action
                    AND action_date>CURRENT_TIMESTAMP-interval '1 day'
                    AND userid=action_userid
               )"#,
        )
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
        if bRecentSelfRequest {
            return Err(AppError::Forbidden);
        }
    }

    let now = chrono::Utc::now();
    let reset_code = crate::security::secret_tokens::reset_code(
        &state.config.site_secret,
        &nick,
        &stored_email,
        now.timestamp_millis(),
    );
    cEmailService(&state)
        .vSendPasswordReset(&nick, &stored_email, &reset_code)
        .await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE users SET lostpwd=$2 WHERE id=$1")
        .bind(id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    let action_user = current_user.as_ref().map(|u| u.id).unwrap_or(id);
    crate::audit::log_user_action_tx(
        &mut tx,
        id,
        action_user,
        "sent_password_reset",
        &[("email", stored_email.as_str())],
    )
    .await?;
    tx.commit().await?;

    Ok(Html(
        "<h1>Инструкция по сбросу пароля была отправлена на ваш email</h1>".to_string(),
    ))
}

#[derive(Deserialize)]
pub struct ResetPasswordCodeForm {
    pub nick: String,
    pub code: String,
}

pub async fn reset_password_with_code(
    State(state): State<AppState>,
    Form(form): Form<ResetPasswordCodeForm>,
) -> Result<Html<String>> {
    let Some((id, nick, email, lostpwd, blocked, activated, candel, anonymous)) = sqlx::query_as::<
        _,
        (
            i32,
            String,
            String,
            chrono::DateTime<chrono::Utc>,
            Option<bool>,
            bool,
            bool,
            bool,
        ),
    >(
        r#"SELECT id,nick,email,lostpwd,blocked,activated,candel,
                  (passwd IS NULL OR passwd='') AS anonymous
           FROM users WHERE lower(nick)=lower($1) LIMIT 1"#,
    )
    .bind(form.nick.trim())
    .fetch_optional(&state.pool)
    .await?
    else {
        return Err(AppError::NotFound);
    };

    if blocked.unwrap_or(false) || !activated || anonymous || candel {
        return Err(AppError::Forbidden);
    }
    if lostpwd <= chrono::DateTime::<chrono::Utc>::from(std::time::UNIX_EPOCH)
        || lostpwd + chrono::Duration::days(1) < chrono::Utc::now()
    {
        return Err(AppError::BadRequest(
            "Срок действия кода истёк (24 часа). Запросите сброс пароля повторно.".into(),
        ));
    }
    if !crate::security::secret_tokens::verify_reset_code(
        &state.config.site_secret,
        &nick,
        &email,
        lostpwd.timestamp_millis(),
        form.code.trim(),
    ) {
        return Err(AppError::BadRequest("Код не совпадает".into()));
    }

    let new_password = generate_java_like_password();
    let hash =
        crate::security::password::hash(&new_password).map_err(|e| AppError::Anyhow(e.into()))?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE users SET passwd=$2,lostpwd='epoch' WHERE id=$1")
        .bind(id)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    crate::audit::log_user_action_tx(&mut tx, id, id, "reset_password", &[]).await?;
    tx.commit().await?;

    Ok(Html(format!(
        "<h1>Установлен новый пароль</h1><p>Ваш новый пароль: <code>{}</code></p>",
        html_escape::encode_text(&new_password)
    )))
}

fn generate_java_like_password() -> String {
    let mut oRng = rand::thread_rng();
    (0..12)
        .map(|_| char::from(oRng.gen_range(33u8..126u8)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{login_redirect, safe_redirect_url};
    use axum::{
        http::{StatusCode, header},
        response::IntoResponse,
    };

    #[test]
    fn login_redirect_accepts_only_java_local_targets() {
        assert_eq!(safe_redirect_url("/"), "/");
        assert_eq!(safe_redirect_url("/forum/?offset=30"), "/forum/?offset=30");
        assert_eq!(safe_redirect_url("//evil.example"), "/");
        assert_eq!(safe_redirect_url("/\\evil.example"), "/");
        assert_eq!(safe_redirect_url("https://evil.example"), "/");
        assert_eq!(safe_redirect_url("forum/"), "/");
        assert_eq!(safe_redirect_url(""), "/");
    }

    #[test]
    fn protected_page_login_redirect_preserves_and_encodes_request_target() {
        let stResponse = login_redirect("/people/maxcom/settings?tab=display").into_response();

        assert_eq!(stResponse.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            stResponse
                .headers()
                .get(header::LOCATION)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("/login.jsp?from=%2Fpeople%2Fmaxcom%2Fsettings%3Ftab%3Ddisplay")
        );
    }

    #[test]
    fn protected_page_login_redirect_rejects_non_local_target() {
        let stResponse = login_redirect("https://evil.example/");

        assert_eq!(stResponse.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            stResponse
                .headers()
                .get(header::LOCATION)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("/login.jsp?from=%2F")
        );
    }
}
