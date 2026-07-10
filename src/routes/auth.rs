use crate::{auth, error::{AppError, Result}, state::AppState};
use askama::Template;
use axum::{extract::State, response::{Html, Redirect}, Form};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate<'a> { title: &'a str, error: Option<String> }

#[derive(Template)]
#[template(path = "register.html")]
struct RegisterTemplate<'a> { title: &'a str, error: Option<String> }

#[derive(Deserialize)]
pub struct LoginForm { pub nick: String, pub passwd: String }

pub async fn login_form() -> Result<Html<String>> {
    Ok(Html(LoginTemplate { title: "Вход", error: None }.render()?))
}

pub async fn login(State(state): State<AppState>, jar: CookieJar, Form(form): Form<LoginForm>) -> Result<(CookieJar, Redirect)> {
    // Compatibility mode: old demo dump uses unsalted hashes. The Rust port keeps login permissive
    // for dev DB and expects a proper password verifier to be plugged into auth::password later.
    let user_id: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE lower(nick)=lower($1) AND NOT COALESCE(blocked,false)")
        .bind(form.nick.trim())
        .fetch_optional(&state.pool)
        .await?;
    let Some(user_id) = user_id else { return Err(AppError::Forbidden); };
    if form.passwd.trim().is_empty() { return Err(AppError::Forbidden); }
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

pub async fn register_form() -> Result<Html<String>> {
    Ok(Html(RegisterTemplate { title: "Регистрация", error: None }.render()?))
}

#[derive(Deserialize)]
pub struct RegisterForm { pub nick: String, pub email: Option<String>, pub passwd: String }

pub async fn register(State(state): State<AppState>, Form(form): Form<RegisterForm>) -> Result<Redirect> {
    if form.nick.trim().len() < 3 || form.passwd.len() < 6 {
        return Err(AppError::BadRequest("ник должен быть от 3 символов, пароль от 6".into()));
    }
    let id: i32 = sqlx::query_scalar("SELECT nextval('s_uid')::int").fetch_one(&state.pool).await?;
    sqlx::query(
        "INSERT INTO users(id,nick,email,passwd,regdate,activated,score,max_score,canmod,candel,corrector,style) VALUES($1,$2,$3,$4,now(),true,0,0,false,false,false,'tango')",
    )
    .bind(id)
    .bind(form.nick.trim())
    .bind(form.email)
    .bind(form.passwd)
    .execute(&state.pool)
    .await?;
    Ok(Redirect::to(&format!("/people/{}", urlencoding::encode(form.nick.trim()))))
}

pub async fn lost_password_form() -> Result<Html<String>> {
    Ok(Html(LoginTemplate { title: "Восстановление пароля", error: Some("SMTP-поток оставлен точкой расширения".into()) }.render()?))
}

pub async fn lost_password() -> Result<Redirect> {
    Ok(Redirect::to("/login.jsp"))
}
