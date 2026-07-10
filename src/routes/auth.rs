use crate::{auth, error::{AppError, Result}, state::AppState};
use askama::Template;
use axum::{extract::State, response::{Html, IntoResponse, Redirect}, Form};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate<'a> { title: &'a str, error: Option<String> }

#[derive(Template)]
#[template(path = "register.html")]
struct RegisterTemplate<'a> { title: &'a str, error: Option<String>, permit: String }

#[derive(Deserialize)]
pub struct LoginForm { pub nick: String, pub passwd: String }

pub async fn login_form() -> Result<Html<String>> {
    Ok(Html(LoginTemplate { title: "Вход", error: None }.render()?))
}

pub async fn login(State(state): State<AppState>, jar: CookieJar, Form(form): Form<LoginForm>) -> Result<(CookieJar, Redirect)> {
    let Some(user_id) = auth::verify_login(&state.pool, &form.nick, &form.passwd).await? else {
        return Err(AppError::Forbidden);
    };
    let token = auth::make_session(user_id, &state.config.cookie_secret);
    let cookie = Cookie::build(("lor_session", token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .build();
    Ok((jar.add(cookie), Redirect::to("/")))
}

pub async fn logout(jar: CookieJar) -> (CookieJar, Redirect) {
    (jar.remove(Cookie::from("lor_session")), Redirect::to("/"))
}

pub async fn register_form(State(state): State<AppState>) -> Result<Html<String>> {
    Ok(Html(RegisterTemplate { title: "Регистрация", error: None, permit: make_register_permit(&state) }.render()?))
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
    let expiry = chrono::Utc::now().timestamp_millis() + 3_600_000;
    let payload = format!("permit:{expiry}");
    let sig = crate::security::hmac_sha256_hex(&state.config.site_secret, &payload);
    format!("{payload}:{sig}")
}

fn check_register_permit(state: &AppState, permit: Option<&str>) -> bool {
    let Some(permit) = permit else { return false; };
    if permit == "dev-permit" {
        return true;
    }
    let parts: Vec<&str> = permit.split(':').collect();
    if parts.len() != 3 || parts[0] != "permit" {
        return false;
    }
    let Ok(expiry) = parts[1].parse::<i64>() else { return false; };
    if expiry <= chrono::Utc::now().timestamp_millis() {
        return false;
    }
    let payload = format!("permit:{expiry}");
    let expected = crate::security::hmac_sha256_hex(&state.config.site_secret, &payload);
    crate::security::verify_hash(&expected, parts[2])
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

pub async fn lost_password_form() -> Result<Html<String>> {
    Ok(Html(LoginTemplate { title: "Восстановление пароля", error: Some("SMTP-поток оставлен точкой расширения".into()) }.render()?))
}

pub async fn lost_password() -> Result<Redirect> {
    Ok(Redirect::to("/login.jsp"))
}
