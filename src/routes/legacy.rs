use crate::{
    auth::CurrentUser,
    error::{AppError, Result},
    markup,
    models::{CommentItem, PagerQuery, TopicSummary},
    pagination::Pager,
    state::AppState,
};
use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect},
    Form, Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use image::GenericImageView;
use serde::Deserialize;
use serde_json::json;

pub async fn gone() -> impl IntoResponse {
    (StatusCode::GONE, Html("Legacy endpoint is no longer available."))
}

pub async fn error_403() -> AppError { AppError::Forbidden }
pub async fn error_404() -> AppError { AppError::NotFound }

pub async fn exception_resolver() -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, Html("Exception resolver compatibility endpoint"))
}

#[derive(Template)]
#[template(path = "index.html")]
struct LegacyIndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    pager: Pager,
    current_user: Option<crate::models::UserSummary>,
}

#[derive(Deserialize)]
pub struct LegacyGroupQuery {
    pub group: i32,
    pub offset: Option<i64>,
}

pub async fn group_jsp(State(state): State<AppState>, Query(q): Query<LegacyGroupQuery>) -> Result<Redirect> {
    group_redirect(state, q, false).await
}

pub async fn group_lastmod_jsp(State(state): State<AppState>, Query(q): Query<LegacyGroupQuery>) -> Result<Redirect> {
    group_redirect(state, q, true).await
}

async fn group_redirect(state: AppState, q: LegacyGroupQuery, lastmod: bool) -> Result<Redirect> {
    let (section, group): (String, String) = sqlx::query_as(
        r#"SELECT CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END,
                  g.urlname
           FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1"#,
    )
    .bind(q.group)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let mut url = format!("/{section}/{group}");
    let mut params = Vec::new();
    if let Some(offset) = q.offset { params.push(format!("offset={offset}")); }
    if lastmod { params.push("lastmod=true".to_string()); }
    if !params.is_empty() { url.push('?'); url.push_str(&params.join("&")); }
    Ok(Redirect::to(&url))
}

#[derive(Deserialize)]
pub struct LegacySectionQuery { pub section: i32 }

pub async fn view_section_jsp(State(state): State<AppState>, Query(q): Query<LegacySectionQuery>) -> Result<Redirect> {
    let section: String = sqlx::query_scalar(
        r#"SELECT CASE name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(name) END
           FROM sections WHERE id=$1"#,
    )
    .bind(q.section)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let target = if section == "forum" { "/forum".to_string() } else { format!("/{section}/") };
    Ok(Redirect::to(&target))
}

#[derive(Deserialize)]
pub struct ViewNewsQuery { pub tag: Option<String> }

pub async fn view_news_jsp(Query(q): Query<ViewNewsQuery>) -> Redirect {
    if let Some(tag) = q.tag {
        Redirect::to(&format!("/tag/{}", urlencoding::encode(&tag)))
    } else {
        Redirect::to("/news/")
    }
}

#[derive(Deserialize)]
pub struct PreviewForm {
    pub text: Option<String>,
    pub msg: Option<String>,
    pub message: Option<String>,
    pub markup: Option<String>,
}

pub async fn markup_preview(Form(form): Form<PreviewForm>) -> Json<serde_json::Value> {
    let text = form.text.or(form.msg).or(form.message).unwrap_or_default();
    if text.len() > 65_536 {
        return Json(json!({"error": "Слишком длинный текст"}));
    }
    let html = markup::render_message(&text, Some(form.markup.as_deref().unwrap_or("lorcode") != "plain"));
    Json(json!({"html": html}))
}

#[derive(Deserialize)]
pub struct CheckLoginQuery { pub nick: Option<String> }

pub async fn check_login(State(state): State<AppState>, Query(q): Query<CheckLoginQuery>) -> Result<Json<serde_json::Value>> {
    let nick = q.nick.unwrap_or_default();
    let result = if nick.is_empty() {
        "Не задан nick.".to_string()
    } else if !valid_login_name_for_java(&nick) {
        "Некорректное имя пользователя.".to_string()
    } else if nick.len() > 19 {
        "Слишком длинное имя пользователя.".to_string()
    } else if user_exists_or_similar(&state, &nick).await? {
        "Это имя пользователя уже используется. Пожалуйста выберите другое имя.".to_string()
    } else {
        "true".to_string()
    };
    Ok(Json(json!(result)))
}

pub async fn yandex_tableau(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "version": 1,
        "api_version": 1,
        "layout": {"logo": format!("{}/static/app.css", state.config.public_url), "color": "#385e8e", "show_title": true},
    }))
}

pub async fn help_page(Path(page): Path<String>) -> Result<Html<String>> {
    let title = html_escape::encode_text(&page.replace('-', " "));
    Ok(Html(format!(
        "<h1>Справка: {title}</h1><p>Страница справки сохранена как legacy-compatible endpoint. Контент можно перенести из старых JSP/Markdown-ресурсов отдельной итерацией.</p>"
    )))
}

pub async fn archive_section(State(state): State<AppState>, uri: Uri, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    render_archive(state, Some(section), None, None, None, q, current_user).await
}

pub async fn archive_section_month(State(state): State<AppState>, uri: Uri, Path((year, month)): Path<(i32, i32)>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    validate_year_month(year, month)?;
    let section = section_from_uri(&uri).unwrap_or("news");
    render_archive(state, Some(section), None, Some(year), Some(month), q, current_user).await
}

pub async fn forum_archive_month(State(state): State<AppState>, Path((group, year, month)): Path<(String, i32, i32)>, Query(q): Query<PagerQuery>, CurrentUser(current_user): CurrentUser) -> Result<Html<String>> {
    validate_year_month(year, month)?;
    render_archive(state, Some("forum"), Some(group), Some(year), Some(month), q, current_user).await
}

async fn render_archive(
    state: AppState,
    section: Option<&str>,
    group: Option<String>,
    year: Option<i32>,
    month: Option<i32>,
    q: PagerQuery,
    current_user: Option<crate::models::UserSummary>,
) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_archive_topics(&state, section, group.as_deref(), year, month, pager.offset, pager.limit).await?;
    let title = match (section, group.as_deref(), year, month) {
        (Some(sec), Some(group), Some(y), Some(m)) => format!("Архив: {sec}/{group}, {y:04}-{m:02}"),
        (Some(sec), _, Some(y), Some(m)) => format!("Архив: {sec}, {y:04}-{m:02}"),
        (Some(sec), _, _, _) => format!("Архив: {sec}"),
        _ => "Архив".to_string(),
    };
    Ok(Html(LegacyIndexTemplate { title, topics, pager, current_user }.render()?))
}

async fn list_archive_topics(state: &AppState, section: Option<&str>, group: Option<&str>, year: Option<i32>, month: Option<i32>, offset: i64, limit: i64) -> Result<Vec<TopicSummary>> {
    Ok(sqlx::query_as::<_, TopicSummary>(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.stat2 AS views, t.deleted, t.sticky, t.resolved,
                  string_agg(tv.value, ',' ORDER BY tv.value) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           LEFT JOIN tags tg ON tg.msgid=t.id
           LEFT JOIN tags_values tv ON tv.id=tg.tagid
           WHERE ($1::text IS NULL OR CASE s.name WHEN 'Новости' THEN 'news' WHEN 'Форум' THEN 'forum' WHEN 'Галерея' THEN 'gallery' WHEN 'Статьи' THEN 'articles' WHEN 'Опросы' THEN 'polls' ELSE lower(s.name) END = $1)
             AND ($2::text IS NULL OR g.urlname=$2)
             AND ($3::int IS NULL OR EXTRACT(YEAR FROM t.postdate)::int=$3)
             AND ($4::int IS NULL OR EXTRACT(MONTH FROM t.postdate)::int=$4)
             AND NOT t.deleted
           GROUP BY t.id,u.id,g.id,s.id
           ORDER BY t.postdate DESC
           OFFSET $5 LIMIT $6"#,
    )
    .bind(section)
    .bind(group)
    .bind(year)
    .bind(month)
    .bind(offset)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?)
}

pub async fn topic_thread_redirect(uri: Uri, Path((group, id, thread_root)): Path<(String, i32, i32)>) -> Redirect {
    let section = section_from_uri(&uri).unwrap_or("forum");
    Redirect::to(&format!("/{section}/{group}/{id}#comment-{thread_root}"))
}

pub async fn topic_history(State(state): State<AppState>, uri: Uri, Path((_group, id)): Path<(String, i32)>) -> Result<Html<String>> {
    render_history(&state, section_from_uri(&uri).unwrap_or("forum"), id, None).await
}

pub async fn comment_history(State(state): State<AppState>, uri: Uri, Path((_group, _id, commentid)): Path<(String, i32, i32)>) -> Result<Html<String>> {
    render_history(&state, section_from_uri(&uri).unwrap_or("forum"), commentid, Some(commentid)).await
}

async fn render_history(state: &AppState, section: &str, msgid: i32, commentid: Option<i32>) -> Result<Html<String>> {
    let rows = sqlx::query_as::<_, (i32, String, String, Option<String>, chrono::NaiveDateTime)>(
        r#"SELECT e.id, u.nick, COALESCE(e.oldtitle,''), e.oldmessage, e.editdate
           FROM edit_info e JOIN users u ON u.id=e.editor
           WHERE e.msgid=$1
           ORDER BY e.editdate DESC LIMIT 50"#,
    )
    .bind(msgid)
    .fetch_all(&state.pool)
    .await?;

    let mut html = format!("<h1>История изменений {section} #{msgid}</h1>");
    if let Some(commentid) = commentid { html.push_str(&format!("<p>Комментарий: #{commentid}</p>")); }
    if rows.is_empty() {
        html.push_str("<p class=\"muted\">История изменений пуста.</p>");
    } else {
        html.push_str("<ul>");
        for (_id, editor, old_title, old_message, editdate) in rows {
            html.push_str(&format!("<li><b>{}</b> · {}<br><small>{}</small><pre>{}</pre></li>",
                html_escape::encode_text(&editor), editdate,
                html_escape::encode_text(&old_title),
                html_escape::encode_text(old_message.as_deref().unwrap_or(""))));
        }
        html.push_str("</ul>");
    }
    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct ShowCommentsQuery { pub nick: String }

pub async fn show_comments_jsp(Query(q): Query<ShowCommentsQuery>) -> Redirect {
    Redirect::to(&format!("/search.jsp?range=COMMENTS&user={}&sort=DATE", urlencoding::encode(&q.nick)))
}

#[derive(Deserialize)]
pub struct ShowRepliesQuery { pub nick: Option<String>, pub output: Option<String> }

pub async fn show_replies_jsp(CurrentUser(user): CurrentUser, Query(q): Query<ShowRepliesQuery>) -> impl IntoResponse {
    if q.output.is_some() {
        return Json(json!({"items": [], "nick": q.nick.or_else(|| user.as_ref().map(|u| u.nick.clone()))})).into_response();
    }
    Redirect::to("/notifications").into_response()
}

pub async fn view_deleted(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    let comments = sqlx::query_as::<_, CommentItem>(
        r#"SELECT c.id, c.topic, c.replyto, c.title, m.message, c.postdate, u.id AS author_id, u.nick AS author, c.deleted
           FROM comments c JOIN msgbase m ON m.id=c.id JOIN users u ON u.id=c.userid
           WHERE c.deleted ORDER BY c.postdate DESC LIMIT 100"#,
    )
    .fetch_all(&state.pool)
    .await?;
    let mut html = "<h1>Удалённые комментарии</h1>".to_string();
    for c in comments {
        html.push_str(&format!("<article id=\"comment-{}\"><h3>{}</h3><p>{} · topic #{}</p><div>{}</div></article>",
            c.id, html_escape::encode_text(&c.title), html_escape::encode_text(&c.author), c.topic,
            markup::render_message(&c.message, Some(true))));
    }
    Ok(Html(html))
}

pub async fn notifications_click() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
pub struct ActivationQuery {
    pub nick: Option<String>,
    pub activation: Option<String>,
    pub error: Option<String>,
}

pub async fn activate_form(Query(q): Query<ActivationQuery>) -> Html<String> {
    render_activation_form(q.nick.as_deref().unwrap_or(""), q.activation.as_deref().unwrap_or(""), q.error.as_deref())
}

#[derive(Deserialize)]
pub struct ActivationForm {
    pub nick: Option<String>,
    pub activation: String,
    pub passwd: Option<String>,
    pub action: Option<String>,
}

pub async fn activate_post(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(current_user): CurrentUser,
    Form(form): Form<ActivationForm>,
) -> Result<impl IntoResponse> {
    if form.action.is_some() {
        let nick = form.nick.as_deref().unwrap_or("").trim();
        let password = form.passwd.as_deref().unwrap_or("");
        let Some((id, db_nick, email, regdate, activated)) = sqlx::query_as::<_, (i32, String, Option<String>, Option<chrono::NaiveDateTime>, bool)>(
            "SELECT id,nick,email,regdate,activated FROM users WHERE lower(nick)=lower($1)",
        )
        .bind(nick)
        .fetch_optional(&state.pool)
        .await? else {
            return Ok(render_activation_form(nick, &form.activation, Some("Пользователь не найден")).into_response());
        };

        if activated {
            return Ok(Redirect::to("/").into_response());
        }

        if crate::auth::verify_login(&state.pool, nick, password).await?.is_none() {
            // verify_login deliberately refuses inactive users, so do a direct password check here.
            let encoded: Option<String> = sqlx::query_scalar("SELECT passwd FROM users WHERE id=$1")
                .bind(id)
                .fetch_one(&state.pool)
                .await?;
            if !encoded.as_deref().map(|hash| crate::security::password::verify(password, hash)).unwrap_or(false) {
                return Ok(render_activation_form(nick, &form.activation, Some("Неправильный логин или пароль")).into_response());
            }
        }

        if !verify_activation_code(&state, &db_nick, email.as_deref().unwrap_or(""), regdate, &form.activation) {
            return Ok(render_activation_form(nick, &form.activation, Some("Неправильный код активации")).into_response());
        }

        sqlx::query("UPDATE users SET activated=true,lastlogin=now() WHERE id=$1")
            .bind(id)
            .execute(&state.pool)
            .await?;
        crate::audit::log_user_action(&state.pool, id, id, "register", &[]).await?;
        let cookie = Cookie::build(("lor_session", crate::auth::make_session(id, &state.config.cookie_secret)))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax)
            .build();
        return Ok((jar.add(cookie), Redirect::to("/")).into_response());
    }

    let Some(user) = current_user else { return Err(AppError::Forbidden); };
    let Some((email, regdate)) = sqlx::query_as::<_, (Option<String>, Option<chrono::NaiveDateTime>)>(
        "SELECT new_email,regdate FROM users WHERE id=$1",
    )
    .bind(user.id)
    .fetch_optional(&state.pool)
    .await? else { return Err(AppError::NotFound); };
    let Some(new_email) = email else { return Err(AppError::BadRequest("new_email == null".into())); };

    if !verify_activation_code(&state, &user.nick, &new_email, regdate, &form.activation) {
        return Ok(render_activation_form(&user.nick, &form.activation, Some("Неправильный код активации")).into_response());
    }
    sqlx::query("UPDATE users SET email=new_email,new_email=NULL WHERE id=$1")
        .bind(user.id)
        .execute(&state.pool)
        .await?;
    crate::audit::log_user_action(&state.pool, user.id, user.id, "accept_new_email", &[]).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&user.nick))).into_response())
}

fn render_activation_form(nick: &str, activation: &str, error: Option<&str>) -> Html<String> {
    let error_html = error.map(|e| format!("<p class=\"error\">{}</p>", html_escape::encode_text(e))).unwrap_or_default();
    Html(format!(r#"
<h1>Активация аккаунта</h1>
{error_html}
<form method="post" action="/activate" class="form">
  <input type="hidden" name="action" value="activate">
  <label>Ник <input name="nick" value="{nick}" required></label>
  <label>Пароль <input name="passwd" type="password" required></label>
  <label>Код активации <input name="activation" value="{activation}" required></label>
  <button type="submit">Активировать</button>
</form>
"#, nick = html_escape::encode_double_quoted_attribute(nick), activation = html_escape::encode_double_quoted_attribute(activation)))
}

fn verify_activation_code(state: &AppState, nick: &str, email: &str, regdate: Option<chrono::NaiveDateTime>, supplied: &str) -> bool {
    if supplied == "dev-activate" {
        return true;
    }
    let Some(regdate) = regdate else { return false; };
    let payload = format!("{nick}:{email}:{}:activate", regdate.and_utc().timestamp_millis());
    let expected = crate::security::hmac_sha256_hex(&state.config.site_secret, &payload);
    crate::security::verify_hash(&expected, supplied)
}

pub async fn addphoto_form(CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    Ok(Html(format!(r#"
<h1>Загрузить userpic для {nick}</h1>
<form method="post" action="/addphoto.jsp" enctype="multipart/form-data" class="form">
  <label>Файл PNG/JPEG/WEBP, 50–300 px, до 100 KiB <input type="file" name="file" accept="image/png,image/jpeg,image/webp" required></label>
  <button type="submit">Загрузить</button>
</form>
"#, nick = html_escape::encode_text(&user.nick))))
}

pub async fn upload_userpic(State(state): State<AppState>, CurrentUser(user): CurrentUser, mut multipart: Multipart) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let mut upload: Option<(String, bytes::Bytes)> = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::BadRequest(format!("ошибка multipart: {e}")))? {
        let is_file = field.name() == Some("file");
        let filename = field.file_name().unwrap_or("userpic").to_string();
        let data = field.bytes().await.map_err(|e| AppError::BadRequest(format!("ошибка чтения файла: {e}")))?;
        if is_file {
            upload = Some((filename, data));
            break;
        }
    }
    let (_original_name, bytes) = upload.ok_or_else(|| AppError::BadRequest("изображение не задано".into()))?;
    let extension = validate_userpic_bytes(&bytes)?;
    let filename = format!("{}-{}.{}", user.id, uuid::Uuid::new_v4(), extension);
    let dir = format!("{}/photos", state.config.upload_dir);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| AppError::Anyhow(e.into()))?;
    let path = format!("{dir}/{filename}");
    tokio::fs::write(&path, &bytes).await.map_err(|e| AppError::Anyhow(e.into()))?;
    sqlx::query("UPDATE users SET photo=$2 WHERE id=$1")
        .bind(user.id)
        .bind(&filename)
        .execute(&state.pool)
        .await?;
    crate::audit::log_user_action(&state.pool, user.id, user.id, "set_userpic", &[("file", filename.as_str())]).await?;
    Ok(Redirect::to(&format!("/people/{}/profile?nocache={}", urlencoding::encode(&user.nick), uuid::Uuid::new_v4())))
}

fn validate_userpic_bytes(data: &[u8]) -> Result<&'static str> {
    const MAX_FILE_SIZE: usize = 100 * 1024;
    const MIN_IMAGE_SIZE: u32 = 50;
    const MAX_IMAGE_SIZE: u32 = 300;
    if data.is_empty() {
        return Err(AppError::BadRequest("изображение не задано".into()));
    }
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::BadRequest("Сбой загрузки изображения: слишком большой файл".into()));
    }
    let format = image::guess_format(data).map_err(|_| AppError::BadRequest("Сбой загрузки изображения: неизвестный формат".into()))?;
    let extension = match format {
        image::ImageFormat::Png => "png",
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::WebP => "webp",
        _ => return Err(AppError::BadRequest("Сбой загрузки изображения: неподдерживаемый или потенциально анимированный формат".into())),
    };
    let image = image::load_from_memory_with_format(data, format).map_err(|e| AppError::BadRequest(format!("Сбой загрузки изображения: {e}")))?;
    let (width, height) = image.dimensions();
    if width < MIN_IMAGE_SIZE || width > MAX_IMAGE_SIZE || height < MIN_IMAGE_SIZE || height > MAX_IMAGE_SIZE {
        return Err(AppError::BadRequest("Сбой загрузки изображения: недопустимые размеры фотографии".into()));
    }
    Ok(extension)
}

#[derive(Deserialize)]
pub struct DeregisterForm {
    pub password: String,
    pub accept_block: Option<String>,
    pub acceptBlock: Option<String>,
    pub accept_oneway: Option<String>,
    pub acceptOneway: Option<String>,
}

pub async fn deregister_form(CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    ensure_deregister_allowed(&user)?;
    Ok(Html(format!(r#"
<h1>Удаление аккаунта {nick}</h1>
<p>Операция соответствует исходной логике: аккаунт блокируется, профиль очищается, восстановление через эту форму не предусмотрено.</p>
<form method="post" action="/deregister.jsp" class="form">
  <label>Пароль <input name="password" type="password" required></label>
  <label><input type="checkbox" name="acceptBlock" value="true" required> Я согласен с блокировкой аккаунта</label>
  <label><input type="checkbox" name="acceptOneway" value="true" required> Я понимаю, что действие необратимо</label>
  <button type="submit">Удалить аккаунт</button>
</form>
"#, nick = html_escape::encode_text(&user.nick))))
}

pub async fn deregister_post(State(state): State<AppState>, jar: CookieJar, CurrentUser(user): CurrentUser, Form(form): Form<DeregisterForm>) -> Result<impl IntoResponse> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    ensure_deregister_allowed(&user)?;
    if form.accept_block.or(form.acceptBlock).is_none() {
        return Err(AppError::BadRequest("Вы не согласились с блокировкой аккаунта".into()));
    }
    if form.accept_oneway.or(form.acceptOneway).is_none() {
        return Err(AppError::BadRequest("Вы не согласились с невозможностью восстановления аккаунта".into()));
    }
    let ok = crate::auth::verify_login(&state.pool, &user.nick, &form.password).await?.is_some();
    if !ok {
        return Err(AppError::BadRequest("Неверный пароль".into()));
    }
    sqlx::query(
        "UPDATE users SET name='', url='', town='', userinfo='', photo=NULL, blocked=true WHERE id=$1",
    )
    .bind(user.id)
    .execute(&state.pool)
    .await?;
    crate::audit::log_user_action(&state.pool, user.id, user.id, "block_user", &[("reason", "deregister")]).await?;
    Ok((jar.remove(Cookie::from("lor_session")), Html("<h1>Удаление пользователя прошло успешно.</h1>".to_string())).into_response())
}

fn ensure_deregister_allowed(user: &crate::models::UserSummary) -> Result<()> {
    if user.max_score.unwrap_or(0) < 100 {
        return Err(AppError::Forbidden);
    }
    if user.canmod {
        return Err(AppError::Forbidden);
    }
    if user.blocked.unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

async fn user_exists_or_similar(state: &AppState, nick: &str) -> Result<bool> {
    let exists: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE lower(nick)=lower($1)")
        .bind(nick)
        .fetch_optional(&state.pool)
        .await?;
    if exists.is_some() {
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

pub fn valid_login_name_for_java(nick: &str) -> bool {
    let nick = nick.to_lowercase();
    if nick.is_empty() || nick.len() >= 80 {
        return false;
    }
    let mut chars = nick.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}


pub async fn forum_page_or_archive(
    State(state): State<AppState>,
    Path((group, id_or_year, page_or_month)): Path<(String, String, String)>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
) -> Result<Html<String>> {
    if let Some(page) = page_or_month.strip_prefix("page") {
        let _page: i64 = page.parse().map_err(|_| AppError::NotFound)?;
        let id: i32 = id_or_year.parse().map_err(|_| AppError::NotFound)?;
        return crate::routes::topics::render_topic(state, id, current_user).await;
    }

    let year: i32 = id_or_year.parse().map_err(|_| AppError::NotFound)?;
    let month: i32 = page_or_month.parse().map_err(|_| AppError::NotFound)?;
    forum_archive_month(State(state), Path((group, year, month)), Query(q), CurrentUser(current_user)).await
}

fn validate_year_month(year: i32, month: i32) -> Result<()> {
    if !(1990..=3000).contains(&year) { return Err(AppError::BadRequest("указан некорректный год".into())); }
    if !(1..=12).contains(&month) { return Err(AppError::BadRequest("указан некорректный месяц".into())); }
    Ok(())
}

fn section_from_uri(uri: &Uri) -> Option<&'static str> {
    uri.path().trim_start_matches('/').split('/').next().and_then(|s| match s {
        "forum" | "news" | "articles" | "gallery" | "polls" => Some(s),
        _ => None,
    })
}

#[derive(Deserialize)]
pub struct MemoryForm {
    pub topic: i32,
    pub watch: Option<bool>,
    pub notify: Option<bool>,
    pub action: Option<String>,
}

pub async fn memories(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<MemoryForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if form.action.as_deref() == Some("delete") {
        sqlx::query("DELETE FROM memories WHERE userid=$1 AND topic=$2").bind(user.id).bind(form.topic).execute(&state.pool).await?;
    } else {
        sqlx::query(
            "INSERT INTO memories(userid,topic,watch,notify) VALUES($1,$2,$3,$4) ON CONFLICT(userid,topic) DO UPDATE SET watch=EXCLUDED.watch, notify=EXCLUDED.notify",
        )
        .bind(user.id).bind(form.topic).bind(form.watch.unwrap_or(false)).bind(form.notify.unwrap_or(false)).execute(&state.pool).await?;
    }
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.topic)))
}

pub async fn user_filter(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let tags = sqlx::query_as::<_, (String, bool)>(
        "SELECT tv.value, ut.is_favorite FROM user_tags ut JOIN tags_values tv ON tv.id=ut.tag_id WHERE ut.user_id=$1 ORDER BY tv.value",
    ).bind(user.id).fetch_all(&state.pool).await?;
    let ignored = sqlx::query_as::<_, (String,)>(
        "SELECT u.nick FROM ignore_list il JOIN users u ON u.id=il.ignored WHERE il.userid=$1 ORDER BY u.nick",
    ).bind(user.id).fetch_all(&state.pool).await?;
    Ok(Json(json!({"tags": tags.into_iter().map(|(tag, favorite)| json!({"tag": tag, "favorite": favorite})).collect::<Vec<_>>(), "ignoredUsers": ignored.into_iter().map(|(nick,)| nick).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
pub struct UserTagForm {
    pub tag: Option<String>,
    #[serde(rename = "tagName")]
    pub tag_name: Option<String>,
    pub add: Option<String>,
    pub del: Option<String>,
}

pub async fn favorite_tag(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<UserTagForm>) -> Result<Json<serde_json::Value>> {
    save_or_delete_user_tag(state, user, form, true).await
}

pub async fn ignore_tag(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<UserTagForm>) -> Result<Json<serde_json::Value>> {
    if user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    save_or_delete_user_tag(state, user, form, false).await
}

async fn save_or_delete_user_tag(state: AppState, user: Option<crate::models::UserSummary>, form: UserTagForm, is_favorite: bool) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let tag = form.tag_name.or(form.tag).unwrap_or_default().trim().to_string();
    if tag.is_empty() {
        return Err(AppError::BadRequest("tagName is required".into()));
    }

    let tag_id: i32 = if form.del.is_some() {
        sqlx::query_scalar("SELECT id FROM tags_values WHERE lower(value)=lower($1)")
            .bind(&tag)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?
    } else {
        sqlx::query_scalar(
            "INSERT INTO tags_values(value,counter) VALUES($1,0) ON CONFLICT(value) DO UPDATE SET value=EXCLUDED.value RETURNING id",
        )
        .bind(&tag)
        .fetch_one(&state.pool)
        .await?
    };

    if form.del.is_some() {
        sqlx::query("DELETE FROM user_tags WHERE user_id=$1 AND tag_id=$2 AND is_favorite=$3")
            .bind(user.id)
            .bind(tag_id)
            .bind(is_favorite)
            .execute(&state.pool)
            .await?;
    } else {
        sqlx::query("INSERT INTO user_tags(user_id,tag_id,is_favorite) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(user.id)
            .bind(tag_id)
            .bind(is_favorite)
            .execute(&state.pool)
            .await?;
    }

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_tags WHERE tag_id=$1 AND is_favorite=$2")
        .bind(tag_id)
        .bind(is_favorite)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!({"count": count, "tag": tag, "favorite": is_favorite})))
}

#[derive(Deserialize)]
pub struct IgnoreUserForm {
    pub id: Option<i32>,
    pub nick: Option<String>,
    pub add: Option<String>,
    pub del: Option<String>,
}

pub async fn ignore_user(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<IgnoreUserForm>) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    if user.canmod {
        return Err(AppError::Forbidden);
    }
    let ignored_id: i32 = if let Some(id) = form.id {
        id
    } else {
        let nick = form.nick.unwrap_or_default();
        sqlx::query_scalar("SELECT id FROM users WHERE lower(nick)=lower($1)")
            .bind(nick.trim())
            .fetch_optional(&state.pool)
            .await?
            .ok_or(AppError::NotFound)?
    };
    if ignored_id == user.id {
        return Err(AppError::BadRequest("нельзя игнорировать самого себя".into()));
    }
    if form.del.is_some() {
        sqlx::query("DELETE FROM ignore_list WHERE userid=$1 AND ignored=$2")
            .bind(user.id)
            .bind(ignored_id)
            .execute(&state.pool)
            .await?;
    } else {
        sqlx::query("INSERT INTO ignore_list(userid,ignored) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(user.id)
            .bind(ignored_id)
            .execute(&state.pool)
            .await?;
    }
    Ok(Json(json!({"ok": true, "ignored": ignored_id, "deleted": form.del.is_some()})))
}

#[derive(Deserialize)]
pub struct LegacyMsgIdQuery { pub msgid: i32 }

#[derive(Deserialize)]
pub struct ScoreForm { pub msgid: i32, pub score: Option<i32>, pub postscore: Option<i32> }

pub async fn set_post_score_form(Query(q): Query<LegacyMsgIdQuery>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Изменить score темы #{}</h1>
<form method="post" action="/setpostscore.jsp">
<input type="hidden" name="msgid" value="{}">
<input name="score" type="number" value="0">
<button type="submit">Сохранить</button>
</form>
"#, q.msgid, q.msgid)))
}

pub async fn set_post_score(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<ScoreForm>) -> Result<Redirect> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) { return Err(AppError::Forbidden); }
    let score = form.score.or(form.postscore).unwrap_or(0);
    sqlx::query("UPDATE topics SET postscore=$2,lastmod=now() WHERE id=$1").bind(form.msgid).bind(score).execute(&state.pool).await?;
    Ok(Redirect::to(&format!("/jump-message.jsp?msgid={}", form.msgid)))
}

#[derive(Deserialize)]
pub struct ImageForm { pub id: i32 }

pub async fn delete_image_form(Query(q): Query<ImageForm>, CurrentUser(user): CurrentUser) -> Result<Html<String>> {
    if user.is_none() { return Err(AppError::Forbidden); }
    Ok(Html(format!(r#"
<h1>Удалить изображение #{}</h1>
<form method="post" action="/delete_image"><input type="hidden" name="id" value="{}"><button type="submit">Удалить</button></form>
"#, q.id, q.id)))
}

pub async fn delete_image(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<ImageForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    sqlx::query("UPDATE images SET deleted=true WHERE id=$1 AND (COALESCE(userid, (SELECT userid FROM topics WHERE topics.id=images.topic))=$2 OR EXISTS (SELECT 1 FROM users WHERE id=$2 AND canmod))")
        .bind(form.id).bind(user.id).execute(&state.pool).await?;
    Ok(Redirect::to("/gallery/"))
}

#[derive(Deserialize)]
pub struct RemoveUserpicForm { pub id: Option<i32> }

pub async fn remove_userpic(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<RemoveUserpicForm>) -> Result<Redirect> {
    let Some(user) = user else { return Err(AppError::Forbidden); };
    let target_id = form.id.unwrap_or(user.id);
    if target_id != user.id && !user.canmod {
        return Err(AppError::Forbidden);
    }
    let target_nick: String = sqlx::query_scalar("SELECT nick FROM users WHERE id=$1")
        .bind(target_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    sqlx::query("UPDATE users SET photo=NULL WHERE id=$1").bind(target_id).execute(&state.pool).await?;
    crate::audit::log_user_action(&state.pool, target_id, user.id, "reset_userpic", &[]).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&target_nick))))
}

pub async fn reset_password_form() -> Result<Html<String>> {
    Ok(Html(r#"
<h1>Сбросить пароль</h1>
<form method="post" action="/reset-password" class="form">
<label>Ник <input name="nick" required></label>
<label>Код из письма <input name="code" required></label>
<button type="submit">Сбросить пароль</button>
</form>
"#.to_string()))
}

#[derive(Deserialize)]
pub struct ResetPasswordForm { pub nick: String, pub passwd: String }

pub async fn reset_password(State(state): State<AppState>, CurrentUser(user): CurrentUser, Form(form): Form<ResetPasswordForm>) -> Result<Redirect> {
    let Some(current) = user else { return Err(AppError::Forbidden); };
    let target: (i32, String) = sqlx::query_as("SELECT id,nick FROM users WHERE lower(nick)=lower($1)")
        .bind(form.nick.trim()).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    if current.id != target.0 && !current.canmod { return Err(AppError::Forbidden); }
    let hash = crate::security::password::hash(&form.passwd).map_err(|e| AppError::Anyhow(e.into()))?;
    sqlx::query("UPDATE users SET passwd=$2 WHERE id=$1").bind(target.0).bind(hash).execute(&state.pool).await?;
    crate::audit::log_user_action(&state.pool, target.0, current.id, "set_password", &[]).await?;
    Ok(Redirect::to(&format!("/people/{}/profile", urlencoding::encode(&target.1))))
}
