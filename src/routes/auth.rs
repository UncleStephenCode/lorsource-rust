use crate::{
    auth,
    auth::CurrentUser,
    error::{AppError, Result},
    state::AppState,
};
use askama::Template;
use axum::{
    Form,
    extract::{ConnectInfo, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use std::net::SocketAddr;
use time::Duration;

pub(crate) fn cEmailService(
    stState: &AppState,
) -> crate::application::email::CEmailService<crate::infra::smtp::CSmtpEmailSender> {
    crate::application::email::CEmailService::new(
        crate::infra::smtp::CSmtpEmailSender::new(
            stState.config.smtp_host.clone(),
            stState.config.smtp_port,
            stState.config.smtp_helo_name.clone(),
        ),
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
    require_captcha: bool,
    captcha_site_key: String,
}

fn render_login_page(
    state: &AppState,
    error: Option<String>,
    nick: String,
    redirect_url: String,
    csrf_token: String,
    require_captcha: bool,
) -> Result<Response> {
    Ok(Html(
        LoginTemplate {
            title: "Login",
            error,
            nick,
            login_action: format!(
                "{}/login_process",
                state.config.public_url.trim_end_matches('/')
            ),
            redirect_url,
            csrf_token,
            require_captcha,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
        }
        .render()?,
    )
    .into_response())
}

pub(crate) async fn bIpCaptchaRequired(state: &AppState, sIp: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, Option<bool>>(
        "SELECT captcha_required FROM b_ips WHERE ip=$1::inet",
    )
    .bind(sIp)
    .fetch_optional(&state.pool)
    .await?
    .flatten()
    .unwrap_or(false))
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
    captcha_site_key: String,
    form_nick: String,
    form_email: String,
    rules_checked: bool,
}

fn render_register_page(
    state: &AppState,
    error: Option<String>,
    permit: String,
    csrf_token: String,
    form_nick: String,
    form_email: String,
    rules_checked: bool,
) -> Result<Response> {
    let mut stResponse = Html(
        RegisterTemplate {
            title: "Регистрация",
            error,
            permit,
            csrf_token,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
            form_nick,
            form_email,
            rules_checked,
        }
        .render()?,
    )
    .into_response();
    stResponse.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store, no-cache, must-revalidate".parse().unwrap(),
    );
    Ok(stResponse)
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
    #[serde(rename = "h-captcha-response")]
    pub captcha_response: Option<String>,
}

pub async fn login_form(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(query): Query<LoginQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<impl IntoResponse> {
    let redirect_url = safe_redirect_url(query.from.as_deref().unwrap_or(""));
    if current_user.is_some() {
        return Ok(found(&redirect_url));
    }
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let bRequireCaptcha = state.login_attempts.bRequireForIp(&sRemoteIp)
        || bIpCaptchaRequired(&state, &sRemoteIp).await?;
    render_login_page(
        &state,
        None,
        String::new(),
        redirect_url,
        csrf_token,
        bRequireCaptcha,
    )
}

/// LoginController.delayResponse: every response (success or failure) is
/// delayed by a random 1-3 seconds, which blunts both brute-force
/// throughput and timing-based username enumeration (a real account
/// lookup + bcrypt verify vs. an instant "not found" would otherwise be
/// distinguishable). Applied uniformly instead of only-on-success/failure
/// so the delay itself never becomes an extra timing signal.
async fn delay_response() {
    let millis = rand::random_range(1000..3000);
    tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
}

pub async fn login(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    jar: CookieJar,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    Form(form): Form<LoginForm>,
) -> Result<Response> {
    let redirect_url = safe_redirect_url(form.redirect_url.as_deref().unwrap_or(""));
    if current_user.is_some() {
        return Ok(found(&redirect_url));
    }
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let bRequireCaptcha = bIpCaptchaRequired(&state, &sRemoteIp).await?
        || state.login_attempts.bRequireForIp(&sRemoteIp)
        || state.login_attempts.bRequireForUser(&form.nick);
    if bRequireCaptcha
        && let Err(sError) = crate::application::auth::sValidateCaptcha(
            &state.config,
            &state.http,
            form.captcha_response.as_deref(),
            &sRemoteIp,
        )
        .await
    {
        delay_response().await;
        return render_login_page(
            &state,
            Some(sError),
            form.nick,
            redirect_url,
            csrf_token,
            true,
        );
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
            state
                .login_attempts
                .vRecordFailedAttempt(&sRemoteIp, &form.nick);
            return render_login_page(
                &state,
                Some("Регистрация не завершена! Инструкция по активации отправлена на указанный при регистрации email.".to_owned()),
                form.nick,
                redirect_url,
                csrf_token,
                true,
            );
        }
        auth::LoginOutcome::Failed => {
            state
                .login_attempts
                .vRecordFailedAttempt(&sRemoteIp, &form.nick);
            return render_login_page(
                &state,
                Some(
                    "Ошибка авторизации. Неправильное имя пользователя, e-mail или пароль."
                        .to_owned(),
                ),
                form.nick,
                redirect_url,
                csrf_token,
                true,
            );
        }
    };
    let is_secure = crate::security::is_secure_request(
        &headers,
        Some(stPeerAddress.ip()),
        &state.config.trusted_proxy_cidrs,
    );
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
    headers: axum::http::HeaderMap,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    if let Some(stUser) = current_user {
        return Ok(found(&format!(
            "/people/{}/profile",
            urlencoding::encode(&stUser.nick)
        )));
    }
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    if !bRegistrationAllowed(&state, &sRemoteIp).await? {
        return Ok(no_register_response());
    }
    render_register_page(
        &state,
        None,
        make_register_permit(&state),
        csrf_token,
        String::new(),
        String::new(),
        false,
    )
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
    #[serde(rename = "h-captcha-response")]
    pub captcha_response: Option<String>,
}

pub async fn register(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    Form(form): Form<RegisterForm>,
) -> Result<Response> {
    if !check_register_permit(&state, form.permit.as_deref()) {
        return Ok(no_register_response());
    }

    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();

    let nick = form.nick.trim().to_string();
    let email = form.email.unwrap_or_default().trim().to_lowercase();
    let password = form.password.or(form.passwd).unwrap_or_default();
    let password2 = form.password2.unwrap_or_default();
    let bRulesChecked = form.rules.as_deref() == Some("okay");
    if let Some(sError) = registration_error(
        &state,
        &nick,
        &email,
        &password,
        &password2,
        bRulesChecked,
        form.captcha_response.as_deref(),
        &sRemoteIp,
    )
    .await?
    {
        return render_register_page(
            &state,
            Some(sError),
            form.permit.unwrap_or_default(),
            csrf_token,
            nick,
            email,
            bRulesChecked,
        );
    }

    let hash =
        crate::security::password::hash(&password).map_err(|e| AppError::Anyhow(e.into()))?;
    let sUserAgent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|stValue| stValue.to_str().ok());
    let sAcceptLanguage = headers
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|stValue| stValue.to_str().ok());
    let mut tx = state.pool.begin().await?;
    let (id, regdate): (i32, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        r#"INSERT INTO users(id,nick,email,passwd,regdate,activated,score,max_score,canmod,candel,corrector,userinfo_markup)
           VALUES(nextval('s_uid')::int,$1,$2,$3,now(),false,45,45,false,false,false,'MARKDOWN')
           RETURNING id, regdate"#,
    )
    .bind(&nick)
    .bind(&email)
    .bind(hash)
    .fetch_one(&mut *tx)
    .await?;
    let iUserAgent: i32 = match sUserAgent {
        Some(sValue) => {
            sqlx::query_scalar("SELECT create_user_agent($1)")
                .bind(sValue.chars().take(511).collect::<String>())
                .fetch_one(&mut *tx)
                .await?
        }
        None => 0,
    };
    let sUserAgentId = iUserAgent.to_string();
    let mut vecInfo = vec![
        ("ip", sRemoteIp.as_str()),
        ("user_agent", sUserAgentId.as_str()),
    ];
    if let Some(sLanguage) = sAcceptLanguage {
        vecInfo.push(("accept_lang", sLanguage));
    }
    crate::audit::log_user_action_tx(&mut tx, id, id, "register", &vecInfo).await?;
    tx.commit().await?;
    cEmailService(&state)
        .vSendRegistration(&nick, &email, regdate.timestamp_millis(), true)
        .await?;
    Ok(Html(
        "<h1>Добавление пользователя прошло успешно.</h1><p>Ожидайте письма с кодом активации.</p>"
            .to_string(),
    )
    .into_response())
}

#[allow(clippy::too_many_arguments)]
async fn registration_error(
    state: &AppState,
    nick: &str,
    email: &str,
    password: &str,
    password2: &str,
    rules_checked: bool,
    captcha_response: Option<&str>,
    remote_ip: &str,
) -> Result<Option<String>> {
    let sError = if nick.is_empty() {
        Some("не задан nick")
    } else if !crate::routes::legacy::valid_login_name_for_java(nick) {
        Some("некорректное имя пользователя")
    } else if nick.len() > 19 {
        Some("слишком длинное имя пользователя")
    } else if password.is_empty() || password2.is_empty() {
        Some("пароль не может быть пустым")
    } else if password.eq_ignore_ascii_case(nick) {
        Some("пароль не может совпадать с логином")
    } else if password != password2 {
        Some("введенные пароли не совпадают")
    } else if password.len() < 10 {
        Some("слишком короткий пароль, минимальная длина: 10")
    } else if email.is_empty() {
        Some("Не указан e-mail")
    } else {
        None
    };
    if let Some(sError) = sError {
        return Ok(Some(sError.to_owned()));
    }
    if let Err(stError) = validate_registration_email(state, email).await {
        return match stError {
            AppError::BadRequest(sMessage) => Ok(Some(sMessage)),
            stOther => Err(stOther),
        };
    }
    if !rules_checked {
        return Ok(Some("Вы не согласились с правилами".to_owned()));
    }
    if let Err(sMessage) = crate::application::auth::sValidateCaptcha(
        &state.config,
        &state.http,
        captcha_response,
        remote_ip,
    )
    .await
    {
        return Ok(Some(sMessage));
    }
    if user_exists_or_similar(state, nick).await? {
        return Ok(Some(
            "Это имя пользователя уже используется. Пожалуйста выберите другое имя.".to_owned(),
        ));
    }
    if email_in_use_for_active_or_recently_blocked_user(state, email).await? {
        return Ok(Some("пользователь с таким e-mail уже зарегистрирован. Если вы забыли параметры своего аккаунта, воспользуйтесь формой восстановления пароля.".to_owned()));
    }
    Ok(None)
}

fn no_register_response() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(
            r#"<div id="warning-body"><div id="warning-logo"><img src="/img/good-penguin.png" alt="good-penguin"></div><div id="warning-text"><p>Регистрация временно не доступна, попробуйте позже.</p></div></div><div id="warning-footer"></div>"#
                .to_owned(),
        ),
    )
        .into_response()
}

async fn bRegistrationAllowed(state: &AppState, sRemoteIp: &str) -> Result<bool> {
    let bIpBlocked: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
             SELECT 1 FROM b_ips
             WHERE ip=$1::inet AND (ban_date IS NULL OR ban_date>CURRENT_TIMESTAMP)
           )"#,
    )
    .bind(sRemoteIp)
    .fetch_one(&state.pool)
    .await?;
    if bIpBlocked {
        return Ok(false);
    }
    let iUnactivated: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM users u JOIN user_log ul ON u.id=ul.userid
           WHERE NOT u.activated AND NOT u.blocked
             AND u.regdate>CURRENT_TIMESTAMP-interval '1 day'
             AND ul.action='register'::user_log_action AND ul.info->'ip'=$1"#,
    )
    .bind(sRemoteIp)
    .fetch_one(&state.pool)
    .await?;
    Ok(iUnactivated < 2)
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

pub(crate) async fn validate_registration_email(state: &AppState, email: &str) -> Result<()> {
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
    // RegisterController first resolves exactly one account through
    // UserDao.getByEmail(searchBlocked=true): normalized address, active
    // account first, otherwise newest id. A blocked account reserves the
    // address only when UserService.wasRecentlyBlocker sees a block_user
    // audit event in the last 14 days; lastlogin is deliberately irrelevant.
    let found: bool = sqlx::query_scalar(EMAIL_IN_USE_SQL)
        .bind(email)
        .fetch_one(&state.pool)
        .await?;
    Ok(found)
}

const EMAIL_IN_USE_SQL: &str = r#"
SELECT EXISTS(
  SELECT 1
    FROM (
      SELECT id, COALESCE(blocked,false) AS blocked
        FROM users
       WHERE normalize_email(email)=normalize_email($1)
       ORDER BY blocked ASC, id DESC
       LIMIT 1
    ) candidate
   WHERE NOT candidate.blocked
      OR EXISTS (
        SELECT 1 FROM user_log ul
         WHERE ul.userid=candidate.id
           AND ul.action='block_user'::user_log_action
           AND ul.action_date>CURRENT_TIMESTAMP-'14 days'::interval
      )
)"#;

#[derive(Template)]
#[template(path = "lost_password.html")]
struct StLostPasswordTemplate {
    csrf_token: String,
    email: String,
    error: Option<String>,
    require_captcha: bool,
    captcha_site_key: String,
}

#[derive(Template)]
#[template(path = "action_done.html")]
struct StTopiclessActionDoneTemplate {
    message: String,
    big_message: Option<String>,
    link: Option<String>,
}

fn render_lost_password_page(
    state: &AppState,
    csrf_token: String,
    email: String,
    error: Option<String>,
    require_captcha: bool,
) -> Result<Response> {
    Ok(Html(
        StLostPasswordTemplate {
            csrf_token,
            email,
            error,
            require_captcha,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
        }
        .render()?,
    )
    .into_response())
}

pub async fn lost_password_form(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let bRequireCaptcha = current_user.is_none() || bIpCaptchaRequired(&state, &sRemoteIp).await?;
    render_lost_password_page(&state, csrf_token, String::new(), None, bRequireCaptcha)
}

#[derive(Deserialize)]
pub struct LostPasswordForm {
    pub email: String,
    #[serde(rename = "h-captcha-response")]
    pub captcha_response: Option<String>,
}

pub async fn lost_password(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    Form(form): Form<LostPasswordForm>,
) -> Result<Response> {
    let email = form.email.trim().to_lowercase();
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let bRequireCaptcha = current_user.is_none() || bIpCaptchaRequired(&state, &sRemoteIp).await?;
    if email.is_empty() {
        delay_response().await;
        return render_lost_password_page(
            &state,
            csrf_token,
            email,
            Some("email не задан".into()),
            bRequireCaptcha,
        );
    }
    if bRequireCaptcha
        && let Err(sError) = crate::application::auth::sValidateCaptcha(
            &state.config,
            &state.http,
            form.captcha_response.as_deref(),
            &sRemoteIp,
        )
        .await
    {
        delay_response().await;
        return render_lost_password_page(&state, csrf_token, email, Some(sError), true);
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
        delay_response().await;
        return render_lost_password_page(
            &state,
            csrf_token,
            email,
            Some("Этот email не зарегистрирован!".into()),
            bRequireCaptcha,
        );
    };

    if blocked.unwrap_or(false) || !activated || anonymous || candel {
        delay_response().await;
        return Err(AppError::Forbidden);
    }
    let requester_is_moderator = current_user.as_ref().map(|u| u.canmod).unwrap_or(false);
    if canmod && !requester_is_moderator {
        delay_response().await;
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
            delay_response().await;
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
    if let Err(stError) = cEmailService(&state)
        .vSendPasswordReset(&nick, &stored_email, &reset_code)
        .await
    {
        delay_response().await;
        return match stError {
            AppError::BadRequest(sMessage) => render_lost_password_page(
                &state,
                csrf_token,
                email,
                Some(sMessage),
                bRequireCaptcha,
            ),
            stOther => Err(stOther),
        };
    }
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

    delay_response().await;
    Ok(Html(
        StTopiclessActionDoneTemplate {
            message: "Инструкция по сбросу пароля была отправлена на ваш email".into(),
            big_message: None,
            link: None,
        }
        .render()?,
    )
    .into_response())
}

#[derive(Deserialize)]
pub struct ResetPasswordCodeForm {
    pub nick: String,
    pub code: String,
}

#[derive(Template)]
#[template(path = "reset_password.html")]
struct StResetPasswordTemplate {
    csrf_token: String,
    error: Option<String>,
}

pub(crate) fn render_reset_password_form(
    csrf_token: String,
    error: Option<String>,
) -> Result<Html<String>> {
    Ok(Html(
        StResetPasswordTemplate { csrf_token, error }.render()?,
    ))
}

pub async fn reset_password_with_code(
    State(state): State<AppState>,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
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
        return render_reset_password_form(csrf_token, Some("Пользователь не найден".into()));
    };

    if blocked.unwrap_or(false) || !activated || anonymous || candel {
        return Err(AppError::Forbidden);
    }
    if lostpwd <= chrono::DateTime::<chrono::Utc>::from(std::time::UNIX_EPOCH)
        || lostpwd + chrono::Duration::days(1) < chrono::Utc::now()
    {
        return render_reset_password_form(
            csrf_token,
            Some("Срок действия кода истёк (24 часа). Запросите сброс пароля повторно.".into()),
        );
    }
    if !crate::security::secret_tokens::verify_reset_code(
        &state.config.site_secret,
        &nick,
        &email,
        lostpwd.timestamp_millis(),
        form.code.trim(),
    ) {
        tracing::warn!(nick = %nick, "password reset verification code does not match");
        return render_reset_password_form(csrf_token, Some("Код не совпадает".into()));
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

    Ok(Html(
        StTopiclessActionDoneTemplate {
            message: "Установлен новый пароль".into(),
            big_message: Some(format!("Ваш новый пароль: {new_password}")),
            link: None,
        }
        .render()?,
    ))
}

fn generate_java_like_password() -> String {
    (0..12)
        .map(|_| char::from(rand::random_range(33u8..126u8)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{EMAIL_IN_USE_SQL, login_redirect, safe_redirect_url};
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

    #[test]
    fn registration_email_reuse_matches_java_block_audit_window() {
        assert!(EMAIL_IN_USE_SQL.contains("normalize_email(email)=normalize_email($1)"));
        assert!(EMAIL_IN_USE_SQL.contains("ORDER BY blocked ASC, id DESC"));
        assert!(EMAIL_IN_USE_SQL.contains("'block_user'::user_log_action"));
        assert!(EMAIL_IN_USE_SQL.contains("'14 days'::interval"));
        assert!(!EMAIL_IN_USE_SQL.contains("lastlogin"));
    }
}
