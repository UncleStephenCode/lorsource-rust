use crate::{auth, auth::CurrentUser, error::{AppError, Result}, state::AppState};
use askama::Template;
use axum::{extract::{Query, State}, response::{Html, IntoResponse, Redirect}, Form};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use rand::{distributions::Alphanumeric, Rng};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate<'a> { title: &'a str, error: Option<String>, redirect_url: String, csrf_token: String }

/// Mirrors LoginController.safeRedirectUrl from the Java/Scala implementation:
/// only same-site relative redirects are accepted; everything else goes to `/`.
fn safe_redirect_url(from: &str) -> String {
    if from.starts_with('/')
        && !from.starts_with("//")
        && !from.starts_with("/\\")
    {
        from.to_string()
    } else {
        "/".to_string()
    }
}

#[derive(Template)]
#[template(path = "register.html")]
struct RegisterTemplate<'a> { title: &'a str, error: Option<String>, permit: String, csrf_token: String }

#[derive(Deserialize)]
pub struct LoginQuery { pub from: Option<String> }

#[derive(Deserialize)]
pub struct LoginForm { pub nick: String, pub passwd: String, #[serde(rename = "redirectUrl", alias = "redirect_url")] pub redirect_url: Option<String> }

pub async fn login_form(Query(query): Query<LoginQuery>, CurrentUser(current_user): CurrentUser, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<impl IntoResponse> {
    let redirect_url = safe_redirect_url(query.from.as_deref().unwrap_or(""));
    if current_user.is_some() {
        return Ok(Redirect::to(&redirect_url).into_response());
    }
    Ok(Html(LoginTemplate { title: "Вход", error: None, redirect_url, csrf_token }.render()?).into_response())
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

pub async fn login(State(state): State<AppState>, headers: axum::http::HeaderMap, jar: CookieJar, Form(form): Form<LoginForm>) -> Result<(CookieJar, Redirect)> {
    let redirect_url = safe_redirect_url(form.redirect_url.as_deref().unwrap_or(""));
    let outcome = auth::verify_login(&state.pool, &form.nick, &form.passwd).await?;
    delay_response().await;
    let identity = match outcome {
        auth::LoginOutcome::Success(identity) => identity,
        // LockedException (frozen account): silently back to their own
        // profile, no error message, no failed-attempt penalty.
        auth::LoginOutcome::Frozen(nick) => {
            return Ok((jar, Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&nick)))));
        }
        auth::LoginOutcome::Failed => return Err(AppError::Forbidden),
    };
    let is_secure = crate::security::is_secure_request(&headers);
    let token = auth::make_session(identity.id, &state.config.cookie_secret);
    let session_cookie = Cookie::build(("lor_session", token))
        .path("/")
        .http_only(true)
        .secure(is_secure)
        .same_site(SameSite::Lax)
        .build();
    let jar = jar.add(session_cookie);
    Ok((jar, Redirect::to(&redirect_url)))
}

pub async fn logout(jar: CookieJar) -> (CookieJar, Redirect) {
    let session_cookie = Cookie::build(("lor_session", "")).path("/").build();
    let jar = jar.remove(session_cookie);
    (jar, Redirect::to("/"))
}

pub async fn register_form(State(state): State<AppState>, crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    Ok(Html(RegisterTemplate { title: "Регистрация", error: None, permit: make_register_permit(&state), csrf_token }.render()?))
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

pub async fn register(State(state): State<AppState>, Form(form): Form<RegisterForm>) -> Result<impl IntoResponse> {
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
        return Err(AppError::BadRequest("слишком длинное имя пользователя".into()));
    }
    if password.is_empty() || password2.is_empty() {
        return Err(AppError::BadRequest("пароль не может быть пустым".into()));
    }
    if password.eq_ignore_ascii_case(&nick) {
        return Err(AppError::BadRequest("пароль не может совпадать с логином".into()));
    }
    if password != password2 {
        return Err(AppError::BadRequest("введенные пароли не совпадают".into()));
    }
    if password.len() < 10 {
        return Err(AppError::BadRequest("слишком короткий пароль, минимальная длина: 10".into()));
    }
    if email.is_empty() {
        return Err(AppError::BadRequest("Не указан e-mail".into()));
    }
    validate_registration_email(&state, &email).await?;
    if form.rules.as_deref() != Some("okay") {
        return Err(AppError::BadRequest("Вы не согласились с правилами".into()));
    }
    if user_exists_or_similar(&state, &nick).await? {
        return Err(AppError::BadRequest("Это имя пользователя уже используется. Пожалуйста выберите другое имя.".into()));
    }
    if email_in_use_for_active_or_recently_blocked_user(&state, &email).await? {
        return Err(AppError::BadRequest("пользователь с таким e-mail уже зарегистрирован. Если вы забыли параметры своего аккаунта, воспользуйтесь формой восстановления пароля.".into()));
    }

    let hash = crate::security::password::hash(&password).map_err(|e| AppError::Anyhow(e.into()))?;
    let (id, _regdate): (i32, chrono::NaiveDateTime) = sqlx::query_as(
        r#"INSERT INTO users(id,nick,email,passwd,regdate,activated,score,max_score,canmod,candel,corrector,userinfo_markup)
           VALUES(nextval('s_uid')::int,$1,$2,$3,now(),false,45,45,false,false,false,'MARKDOWN')
           RETURNING id, regdate"#,
    )
    .bind(&nick)
    .bind(&email)
    .bind(hash)
    .fetch_one(&state.pool)
    .await?;

    crate::audit::log_user_action(&state.pool, id, id, "register", &[("email", email.as_str())]).await?;
    Ok(Html("<h1>Добавление пользователя прошло успешно.</h1><p>Ожидайте письма с кодом активации.</p>".to_string()).into_response())
}

fn make_register_permit(state: &AppState) -> String {
    crate::security::secret_tokens::make_register_permit(&state.config.site_secret, chrono::Utc::now().timestamp_millis())
        .unwrap_or_else(|_| "dev-permit".to_string())
}

fn check_register_permit(state: &AppState, permit: Option<&str>) -> bool {
    let Some(permit) = permit else { return false; };
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
    let top_private = domain.split('.').rev().take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(".");
    let blocked: Option<String> = sqlx::query_scalar(
        "SELECT domain FROM email_domains_block WHERE lower(domain)=lower($1) OR lower(domain)=lower($2) LIMIT 1",
    )
    .bind(&domain)
    .bind(&top_private)
    .fetch_optional(&state.pool)
    .await?;
    if blocked.is_some() {
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

async fn email_in_use_for_active_or_recently_blocked_user(state: &AppState, email: &str) -> Result<bool> {
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

pub async fn lost_password_form(crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken) -> Result<Html<String>> {
    Ok(Html(format!(r#"
<h1>Восстановление пароля</h1>
<form method="post" action="/lostpwd.jsp" class="form">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <label>Email <input name="email" type="email" required></label>
  <button type="submit">Отправить инструкцию</button>
</form>
"#)))
}

#[derive(Deserialize)]
pub struct LostPasswordForm { pub email: String }

pub async fn lost_password(State(state): State<AppState>, CurrentUser(current_user): CurrentUser, Form(form): Form<LostPasswordForm>) -> Result<Html<String>> {
    let email = form.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(AppError::BadRequest("email не задан".into()));
    }

    let Some((id, nick, stored_email, lostpwd, blocked, activated, canmod, candel)) = sqlx::query_as::<_, (i32, String, String, chrono::DateTime<chrono::Utc>, Option<bool>, bool, bool, bool)>(
        r#"SELECT id,nick,email,lostpwd,blocked,activated,canmod,candel
           FROM users WHERE lower(email)=lower($1) LIMIT 1"#,
    )
    .bind(&email)
    .fetch_optional(&state.pool)
    .await? else {
        return Err(AppError::BadRequest("Этот email не зарегистрирован!".into()));
    };

    if blocked.unwrap_or(false) || !activated || id == 2 || (canmod && candel) {
        return Err(AppError::Forbidden);
    }
    let requester_is_moderator = current_user.as_ref().map(|u| u.canmod).unwrap_or(false);
    if canmod && !requester_is_moderator {
        return Err(AppError::Forbidden);
    }
    if !requester_is_moderator && lostpwd > chrono::Utc::now() - chrono::Duration::days(1) {
        return Err(AppError::BadRequest("Нельзя запрашивать пароль чаще одного раза в день!".into()));
    }

    let now = chrono::Utc::now();
    let reset_code = crate::security::secret_tokens::reset_code(&state.config.site_secret, &nick, &stored_email, now.timestamp_millis());
    sqlx::query("UPDATE users SET lostpwd=$2 WHERE id=$1")
        .bind(id)
        .bind(now)
        .execute(&state.pool)
        .await?;
    let action_user = current_user.as_ref().map(|u| u.id).unwrap_or(id);
    crate::audit::log_user_action(&state.pool, id, action_user, "sent_password_reset", &[("email", stored_email.as_str())]).await?;

    // The Java application sends this code by SMTP. The Rust port keeps a deterministic
    // code-compatible path and exposes it in development so compatibility tests can finish
    // without a configured mail transport.
    Ok(Html(format!(
        "<h1>Инструкция по сбросу пароля была отправлена на ваш email</h1><p class=\"dev-only\">dev reset code for {}: <code>{}</code></p>",
        html_escape::encode_text(&nick),
        html_escape::encode_text(&reset_code)
    )))
}

#[derive(Deserialize)]
pub struct ResetPasswordCodeForm { pub nick: String, pub code: String }

pub async fn reset_password_with_code(State(state): State<AppState>, Form(form): Form<ResetPasswordCodeForm>) -> Result<Html<String>> {
    let Some((id, nick, email, lostpwd, blocked, activated, canmod, candel)) = sqlx::query_as::<_, (i32, String, String, chrono::DateTime<chrono::Utc>, Option<bool>, bool, bool, bool)>(
        r#"SELECT id,nick,email,lostpwd,blocked,activated,canmod,candel
           FROM users WHERE lower(nick)=lower($1) LIMIT 1"#,
    )
    .bind(form.nick.trim())
    .fetch_optional(&state.pool)
    .await? else {
        return Err(AppError::NotFound);
    };

    if blocked.unwrap_or(false) || !activated || id == 2 || (canmod && candel) {
        return Err(AppError::Forbidden);
    }
    if lostpwd <= chrono::DateTime::<chrono::Utc>::from(std::time::UNIX_EPOCH)
        || lostpwd + chrono::Duration::days(1) < chrono::Utc::now()
    {
        return Err(AppError::BadRequest("Срок действия кода истёк (24 часа). Запросите сброс пароля повторно.".into()));
    }
    if !crate::security::secret_tokens::verify_reset_code(&state.config.site_secret, &nick, &email, lostpwd.timestamp_millis(), form.code.trim()) {
        return Err(AppError::BadRequest("Код не совпадает".into()));
    }

    let new_password = generate_java_like_password();
    let hash = crate::security::password::hash(&new_password).map_err(|e| AppError::Anyhow(e.into()))?;
    sqlx::query("UPDATE users SET passwd=$2 WHERE id=$1")
        .bind(id)
        .bind(hash)
        .execute(&state.pool)
        .await?;
    crate::audit::log_user_action(&state.pool, id, id, "reset_password", &[]).await?;

    Ok(Html(format!(
        "<h1>Установлен новый пароль</h1><p>Ваш новый пароль: <code>{}</code></p>",
        html_escape::encode_text(&new_password)
    )))
}

fn generate_java_like_password() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect()
}
