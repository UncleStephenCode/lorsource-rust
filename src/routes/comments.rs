use crate::{
    auth::CurrentUser,
    error::{AppError, Result},
    markup,
    state::AppState,
};
use askama::Template;
use axum::{
    Form,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Deserialize)]
pub struct JumpQuery {
    pub msgid: i32,
    pub page: Option<i32>,
    pub cid: Option<i32>,
}

pub async fn jump_message(
    State(state): State<AppState>,
    Query(q): Query<JumpQuery>,
    CurrentUser(user): CurrentUser,
) -> Result<Response> {
    let topic = crate::routes::topics::get_topic(&state, q.msgid).await?;
    if let Some(cid) = q.cid {
        return crate::routes::topics::resolve_comment_jump(
            &state,
            &topic,
            cid,
            user.as_ref().is_some_and(|stUser| stUser.canmod),
            &user,
        )
        .await;
    }
    let target = match q.page {
        Some(page) => format!("{}/page{page}", topic.topic_url()),
        None => topic.topic_url(),
    };
    Ok((StatusCode::FOUND, [(header::LOCATION, target)]).into_response())
}

#[derive(Deserialize)]
pub struct CommentForm {
    pub topic: i32,
    pub replyto: Option<i32>,
    pub title: Option<String>,
    #[serde(default)]
    pub msg: String,
    pub nick: Option<String>,
    pub password: Option<String>,
    pub preview: Option<String>,
    #[serde(rename = "h-captcha-response")]
    pub captcha_response: Option<String>,
    pub csrf: Option<String>,
}

#[derive(Template)]
#[template(path = "comment_form.html")]
struct CommentFormTemplate {
    topic_id: i32,
    topic_title: String,
    topic_url: String,
    replyto: Option<i32>,
    csrf_token: String,
    format_mode: String,
    format_title: String,
    form_error: Option<String>,
    preview_html: Option<String>,
    form_msg: String,
    form_title: String,
    anonymous_form: bool,
    form_nick: String,
    require_captcha: bool,
    captcha_site_key: String,
    context_html: Option<String>,
}

async fn optCommentFormContextHtml(
    state: &AppState,
    stTopic: &crate::models::TopicDetail,
    optReplyTo: Option<i32>,
    bShowTopic: bool,
) -> Result<Option<String>> {
    if let Some(iReplyTo) = optReplyTo.filter(|iValue| *iValue > 0) {
        let optRow: Option<(
            i32,
            String,
            String,
            String,
            chrono::DateTime<chrono::Utc>,
            String,
        )> = sqlx::query_as(
            r#"SELECT c.topic,c.title,m.message,m.markup::text,c.postdate,u.nick
               FROM comments c JOIN msgbase m ON m.id=c.id JOIN users u ON u.id=c.userid
               WHERE c.id=$1 AND NOT c.deleted"#,
        )
        .bind(iReplyTo)
        .fetch_optional(&state.pool)
        .await?;
        let Some((iReplyTopicId, sTitle, sMessage, sMarkup, dtPostdate, sAuthor)) = optRow else {
            return Err(AppError::NotFound);
        };
        if iReplyTopicId != stTopic.id {
            return Err(AppError::BadRequest("некорректная тема".into()));
        }
        let sTitleHtml = if sTitle.trim().is_empty() {
            String::new()
        } else {
            format!(
                "<div class=title>{}</div>",
                html_escape::encode_text(&sTitle)
            )
        };
        return Ok(Some(format!(
            r#"<div class="comment"><div class="messages"><article class="msg" id="comment-{iReplyTo}"><div class="msg-container"><div class="msg_body">{sTitleHtml}<div class="msg-text">{}</div><div class="sign"><a href="/people/{}/profile">{}</a> (<time data-format="default" datetime="{}">{}</time>)</div></div></div></article></div></div>"#,
            markup::render_message_with_markup(&sMessage, Some(&sMarkup), None),
            urlencoding::encode(&sAuthor),
            html_escape::encode_text(&sAuthor),
            dtPostdate.to_rfc3339(),
            dtPostdate,
        )));
    }
    if bShowTopic {
        return Ok(Some(format!(
            r#"<div class="messages"><article class="msg" id="topic-{}"><div class="msg-container"><div class="msg_body"><header><h1><a href="{}">{}</a></h1></header><div class="msg-text">{}</div><div class="sign"><a href="/people/{}/profile">{}</a> (<time data-format="default" datetime="{}">{}</time>)</div></div></div></article></div>"#,
            stTopic.id,
            stTopic.topic_url(),
            html_escape::encode_text(&stTopic.title),
            markup::render_message_with_markup(&stTopic.message, Some(&stTopic.markup), None),
            urlencoding::encode(&stTopic.author),
            html_escape::encode_text(&stTopic.author),
            stTopic.postdate.to_rfc3339(),
            stTopic.postdate,
        )));
    }
    Ok(None)
}

async fn render_comment_form(
    state: &AppState,
    form: &CommentForm,
    csrf_token: String,
    format_mode: String,
    format_title: String,
    form_error: Option<String>,
    preview_html: Option<String>,
    anonymous_form: bool,
    require_captcha: bool,
    bShowTopicContext: bool,
) -> Result<Html<String>> {
    let topic = crate::routes::topics::get_topic(state, form.topic).await?;
    let topic_url = topic.topic_url();
    let context_html =
        optCommentFormContextHtml(state, &topic, form.replyto, bShowTopicContext).await?;
    Ok(Html(
        CommentFormTemplate {
            topic_id: topic.id,
            topic_title: topic.title,
            topic_url,
            replyto: form.replyto.filter(|id| *id > 0),
            csrf_token,
            format_mode,
            format_title,
            form_error,
            preview_html,
            form_msg: form.msg.clone(),
            form_title: form.title.clone().unwrap_or_default(),
            anonymous_form,
            form_nick: form.nick.clone().unwrap_or_else(|| "anonymous".into()),
            require_captcha,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
            context_html,
        }
        .render()?,
    ))
}

async fn comment_format(state: &AppState, user_id: i32) -> Result<(String, String, String)> {
    let settings_text: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    let mode = crate::profile::ProfileSettings::from_hstore_text(settings_text).format_mode;
    let title = crate::profile::FORMAT_MODES
        .iter()
        .find(|(id, _, _)| *id == mode)
        .map(|(_, title, _)| *title)
        .unwrap_or("Markdown")
        .to_string();
    let markup = match mode.as_str() {
        "markdown" => "MARKDOWN",
        "ntobr" => "BBCODE_ULB",
        "plain" => "PLAIN",
        _ => "BBCODE_TEX",
    };
    Ok((mode, title, markup.to_string()))
}

pub async fn add_comment_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CommentFormQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Html<String>> {
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let stResolution =
        crate::application::auth::stResolvePostingIdentity(&state, user.as_ref(), None, None)
            .await?;
    check_comment_posting_allowed(
        &state,
        &stResolution.stIdentity.stUser,
        !stResolution.stIdentity.bAuthorized,
        q.topic,
    )
    .await?;
    if let Some(sError) = optCommentReplyError(&state, q.topic, q.replyto).await? {
        return Err(AppError::BadRequest(sError));
    }
    let (format_mode, format_title, _) = match &user {
        Some(user) => comment_format(&state, user.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    let bRequireCaptcha =
        user.is_none() || crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    render_comment_form(
        &state,
        &CommentForm {
            topic: q.topic,
            replyto: q.replyto,
            title: None,
            msg: String::new(),
            nick: None,
            password: None,
            preview: None,
            captcha_response: None,
            csrf: None,
        },
        csrf_token,
        format_mode,
        format_title,
        None,
        None,
        user.is_none(),
        bRequireCaptcha,
        false,
    )
    .await
}

#[derive(Deserialize)]
pub struct CommentFormQuery {
    pub topic: i32,
    pub replyto: Option<i32>,
}

/// Java redirects comment actions to `topic.getLink + "?cid=" + msgid`
/// (see AddCommentController.scala:132, EditCommentController, DeleteCommentController)
/// rather than through a jump/redirect endpoint. Reuses the topic/comment
/// lookup already needed by `/jump-message.jsp` so both stay consistent.
async fn comment_link(state: &AppState, comment_id: i32) -> Result<String> {
    match locate_topic_or_comment(state, comment_id).await? {
        Some((section, group, topic_id, _)) => {
            Ok(format!("/{section}/{group}/{topic_id}?cid={comment_id}"))
        }
        None => Ok(format!("/jump-message.jsp?msgid={comment_id}")),
    }
}

pub async fn add_comment(
    State(state): State<AppState>,
    headers: HeaderMap,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    Form(form): Form<CommentForm>,
) -> Result<Response> {
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let bSessionAuthorized = user.is_some();
    let bRequireCaptcha =
        !bSessionAuthorized || crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    let (format_mode, format_title, markup) = match user.as_ref() {
        Some(stUser) => comment_format(&state, stUser.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    let mut optError = None;
    if form.preview.is_none()
        && bRequireCaptcha
        && let Err(sError) = crate::application::auth::sValidateCaptcha(
            &state.config,
            &state.http,
            form.captcha_response.as_deref(),
            &sRemoteIp,
        )
        .await
    {
        optError = Some(sError);
    }
    if form.preview.is_none()
        && form.csrf.as_deref().map(str::trim) != Some(csrf_token.trim())
        && optError.is_none()
    {
        optError = Some("Неправильный код защиты CSRF. Возможно сессия устарела".into());
    }
    let stResolution = crate::application::auth::stResolvePostingIdentity(
        &state,
        user.as_ref(),
        form.nick.as_deref(),
        form.password.as_deref(),
    )
    .await?;
    if optError.is_none() {
        optError = stResolution.optError.clone();
    }
    let stIdentity = stResolution.stIdentity;
    if optError.is_none()
        && let Some(sError) = optCommentActorError(
            &state,
            &stIdentity.stUser,
            !stIdentity.bAuthorized,
            &sRemoteIp,
        )
        .await?
    {
        optError = Some(sError);
    }
    if optError.is_none() && form.preview.is_none() {
        let iThreshold =
            iCommentRateThresholdSeconds(&state, &stIdentity.stUser, !stIdentity.bAuthorized)
                .await?;
        optError = state.comment_flood.optCheck(&sRemoteIp, iThreshold).await;
    }
    if optError.is_none()
        && let Err(stError) = check_comment_posting_allowed(
            &state,
            &stIdentity.stUser,
            !stIdentity.bAuthorized,
            form.topic,
        )
        .await
    {
        optError = Some(sCommentFormError(stError)?);
    }
    if optError.is_none() {
        optError = optCommentReplyError(&state, form.topic, form.replyto).await?;
    }
    if optError.is_none() {
        optError = optCommentBodyError(&form.msg, !stIdentity.bAuthorized);
    }
    let optPreview = form
        .preview
        .as_ref()
        .map(|_| markup::render_message_with_markup(&form.msg, Some(&markup), None));
    if form.preview.is_some() || optError.is_some() {
        return Ok(render_comment_form(
            &state,
            &form,
            csrf_token,
            format_mode,
            format_title,
            optError,
            optPreview,
            !bSessionAuthorized,
            bRequireCaptcha,
            false,
        )
        .await?
        .into_response());
    }
    let sUserAgent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|stValue| stValue.to_str().ok());
    let id = insert_comment(
        &state,
        stIdentity.stUser.id,
        !stIdentity.bAuthorized,
        stIdentity.stUser.score.unwrap_or(0) >= 0,
        &form,
        &markup,
        &sRemoteIp,
        sUserAgent,
    )
    .await?;
    let sLocation = comment_link(&state, id).await?;
    Ok((StatusCode::FOUND, [(header::LOCATION, sLocation)]).into_response())
}

pub async fn add_comment_ajax(
    State(state): State<AppState>,
    headers: HeaderMap,
    CurrentUser(user): CurrentUser,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    Form(form): Form<CommentForm>,
) -> Result<axum::Json<serde_json::Value>> {
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let bRequireCaptcha =
        user.is_none() || crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    let (_, _, markup) = match user.as_ref() {
        Some(stUser) => comment_format(&state, stUser.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    let mut vecErrors = Vec::new();
    if form.preview.is_none()
        && bRequireCaptcha
        && let Err(sError) = crate::application::auth::sValidateCaptcha(
            &state.config,
            &state.http,
            form.captcha_response.as_deref(),
            &sRemoteIp,
        )
        .await
    {
        vecErrors.push(sError);
    }
    let stResolution = crate::application::auth::stResolvePostingIdentity(
        &state,
        user.as_ref(),
        form.nick.as_deref(),
        form.password.as_deref(),
    )
    .await?;
    if let Some(sError) = stResolution.optError {
        vecErrors.push(sError);
    }
    let stIdentity = stResolution.stIdentity;
    if let Some(sError) = optCommentActorError(
        &state,
        &stIdentity.stUser,
        !stIdentity.bAuthorized,
        &sRemoteIp,
    )
    .await?
    {
        vecErrors.push(sError);
    }
    if vecErrors.is_empty() && form.preview.is_none() {
        let iThreshold =
            iCommentRateThresholdSeconds(&state, &stIdentity.stUser, !stIdentity.bAuthorized)
                .await?;
        if let Some(sError) = state.comment_flood.optCheck(&sRemoteIp, iThreshold).await {
            vecErrors.push(sError);
        }
    }
    if let Err(stError) = check_comment_posting_allowed(
        &state,
        &stIdentity.stUser,
        !stIdentity.bAuthorized,
        form.topic,
    )
    .await
    {
        vecErrors.push(sCommentFormError(stError)?);
    }
    if let Some(sError) = optCommentReplyError(&state, form.topic, form.replyto).await? {
        vecErrors.push(sError);
    }
    if let Some(sError) = optCommentBodyError(&form.msg, !stIdentity.bAuthorized) {
        vecErrors.push(sError);
    }
    if form.preview.is_some() || !vecErrors.is_empty() {
        return Ok(axum::Json(serde_json::json!({
            "errors": vecErrors,
            "preview": markup::render_message_with_markup(&form.msg, Some(&markup), None),
        })));
    }
    let sUserAgent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|stValue| stValue.to_str().ok());
    let id = insert_comment(
        &state,
        stIdentity.stUser.id,
        !stIdentity.bAuthorized,
        stIdentity.stUser.score.unwrap_or(0) >= 0,
        &form,
        &markup,
        &sRemoteIp,
        sUserAgent,
    )
    .await?;
    let url = comment_link(&state, id).await?;
    Ok(axum::Json(serde_json::json!({"url": url})))
}

fn sCommentFormError(stError: AppError) -> Result<String> {
    match stError {
        AppError::BadRequest(sMessage) | AppError::TooManyRequests(sMessage) => Ok(sMessage),
        AppError::Forbidden => Ok("Это сообщение нельзя комментировать".into()),
        stOther => Err(stOther),
    }
}

fn optCommentBodyError(sMessage: &str, bAnonymous: bool) -> Option<String> {
    if let Some(cInvalid) = sMessage.chars().find(|cValue| {
        !matches!(
            *cValue,
            '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
        )
    }) {
        Some(format!(
            "Недопустимый XML-символ U+{:04X}",
            u32::from(cInvalid)
        ))
    } else if sMessage.trim().is_empty() {
        Some("комментарий не может быть пустым".into())
    } else if sMessage.encode_utf16().count()
        > if bAnonymous {
            COMMENT_MAX_LENGTH_ANONYMOUS
        } else {
            COMMENT_MAX_LENGTH
        }
    {
        Some("Слишком большое сообщение".into())
    } else {
        None
    }
}

async fn optCommentReplyError(
    state: &AppState,
    iTopicId: i32,
    optReplyTo: Option<i32>,
) -> Result<Option<String>> {
    let Some(iReplyTo) = optReplyTo.filter(|iValue| *iValue > 0) else {
        return Ok(None);
    };
    let optReply: Option<(i32, bool)> =
        sqlx::query_as("SELECT topic,deleted FROM comments WHERE id=$1")
            .bind(iReplyTo)
            .fetch_optional(&state.pool)
            .await?;
    let Some((iReplyTopicId, bDeleted)) = optReply else {
        return Err(AppError::NotFound);
    };
    if bDeleted {
        Ok(Some(
            "нельзя комментировать удаленные комментарии".to_owned(),
        ))
    } else if iReplyTopicId != iTopicId {
        Ok(Some("некорректная тема".to_owned()))
    } else {
        Ok(None)
    }
}

pub(crate) async fn optCommentActorError(
    state: &AppState,
    user: &crate::models::UserSummary,
    bAnonymous: bool,
    sRemoteIp: &str,
) -> Result<Option<String>> {
    let optIpBlock: Option<(bool, bool)> = sqlx::query_as(
        r#"SELECT ban_date IS NULL OR ban_date > CURRENT_TIMESTAMP,
                  COALESCE(allow_posting,false)
           FROM b_ips WHERE ip=$1::inet"#,
    )
    .bind(sRemoteIp)
    .fetch_optional(&state.pool)
    .await?;
    if let Some((true, bAllowRegisteredPosting)) = optIpBlock {
        if bAnonymous {
            return Ok(Some(
                "анонимный постинг с этого IP адреса заблокирован".to_owned(),
            ));
        }
        if user.blocked.unwrap_or(false) || user.score.unwrap_or(0) < 50 {
            return Ok(Some(
                "постинг с этого IP адреса ограничен для пользователей с score < 50".to_owned(),
            ));
        }
        if !bAllowRegisteredPosting {
            return Ok(Some("постинг с этого IP адреса заблокирован".to_owned()));
        }
    }

    let bFrozen: bool = sqlx::query_scalar(
        "SELECT COALESCE(frozen_until > CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
    )
    .bind(user.id)
    .fetch_one(&state.pool)
    .await?;
    if user.blocked.unwrap_or(false) || bFrozen {
        return Ok(Some("установлен режим только для чтения".to_owned()));
    }
    Ok(None)
}

async fn iCommentRateThresholdSeconds(
    state: &AppState,
    user: &crate::models::UserSummary,
    bAnonymous: bool,
) -> Result<u64> {
    if bAnonymous {
        return Ok(iCommentThresholdSeconds(true, 0, None, 0));
    }
    let optFrozenUntil: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(user.id)
            .fetch_one(&state.pool)
            .await?;
    let iRecentScoreLoss: i64 = sqlx::query_scalar(
        r#"SELECT COALESCE(abs(sum(di.bonus)),0)::bigint FROM del_info di
           WHERE di.deldate > CURRENT_TIMESTAMP - interval '3 days'
             AND di.msgid IN (
               SELECT id FROM comments WHERE userid=$1
               UNION ALL
               SELECT id FROM topics WHERE userid=$1
             )"#,
    )
    .bind(user.id)
    .fetch_one(&state.pool)
    .await?;
    Ok(iCommentThresholdSeconds(
        false,
        user.score.unwrap_or(0),
        optFrozenUntil,
        iRecentScoreLoss,
    ))
}

fn iCommentThresholdSeconds(
    bAnonymous: bool,
    iScore: i32,
    optFrozenUntil: Option<chrono::DateTime<chrono::Utc>>,
    iRecentScoreLoss: i64,
) -> u64 {
    if bAnonymous {
        return 30;
    }
    let bSlowMode = iScore < 35
        || optFrozenUntil
            .is_some_and(|dtValue| dtValue > chrono::Utc::now() - chrono::Duration::days(3))
        || iRecentScoreLoss >= 30;
    if bSlowMode {
        5 * 60
    } else if iScore >= 100 {
        3
    } else {
        30
    }
}

/// Section.getCommentPostscore: Forum/News are unrestricted by section;
/// Articles/Gallery/Polls require 45; an unknown section follows the
/// original defensive default of 50. Section ids per the Java `Section`.
fn section_comment_postscore(section_id: i32) -> i32 {
    match section_id {
        1 | 2 => -9999,
        3 | 5 | 6 => 45,
        _ => 50,
    }
}

const TOPIC_MAX_WARNINGS: i32 = 2;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct StCommentPostingContext {
    bDeleted: bool,
    bDraft: bool,
    bExpired: bool,
    bSticky: bool,
    iTopicPostscore: i32,
    iRestrictComments: i32,
    iSectionId: i32,
    iCommentCount: i32,
    iOpenWarnings: i32,
    bAllowAnonymous: bool,
    iScoreLoss: i32,
    iTopicAuthorId: i32,
}

pub(crate) async fn stCommentPostingContext(
    state: &AppState,
    topic_id: i32,
) -> Result<StCommentPostingContext> {
    sqlx::query_as(
        r#"SELECT t.deleted AS "bDeleted", t.draft AS "bDraft",
                  NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire AS "bExpired",
                  t.sticky AS "bSticky", COALESCE(t.postscore, -9999) AS "iTopicPostscore",
                  g.restrict_comments AS "iRestrictComments", s.id AS "iSectionId",
                  t.stat1 AS "iCommentCount", t.open_warnings AS "iOpenWarnings",
                  t.allow_anonymous AS "bAllowAnonymous",
                  COALESCE((SELECT sum(-di.bonus) FROM del_info di
                            JOIN comments dc ON dc.id=di.msgid
                            WHERE di.bonus IS NOT NULL AND di.bonus<>0
                              AND dc.userid<>2 AND dc.deleted AND dc.topic=t.id), 0)::int AS "iScoreLoss",
                  t.userid AS "iTopicAuthorId"
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

pub(crate) fn check_comment_posting_context(
    stContext: &StCommentPostingContext,
    user: &crate::models::UserSummary,
    bAnonymous: bool,
    bFrozen: bool,
    bIgnoreFrozen: bool,
) -> Result<()> {
    if stContext.bDeleted {
        return Err(AppError::BadRequest(
            "Нельзя добавлять комментарии к удаленному сообщению".into(),
        ));
    }
    if stContext.bDraft {
        return Err(AppError::BadRequest(
            "Нельзя добавлять комментарии к черновику".into(),
        ));
    }
    if stContext.bExpired {
        return Err(AppError::BadRequest("Сообщение уже устарело".into()));
    }
    if user.blocked.unwrap_or(false) || (!bIgnoreFrozen && bFrozen) {
        return Err(AppError::Forbidden);
    }

    let comment_count_restriction = if !stContext.bSticky {
        if stContext.iCommentCount > 3000 {
            200
        } else if stContext.iCommentCount > 2000 {
            100
        } else if stContext.iCommentCount > 1000 {
            50
        } else {
            -9999
        }
    } else {
        -9999
    };
    let score_loss_postscore = if !stContext.bSticky && !stContext.bExpired {
        if stContext.iScoreLoss >= 150 {
            100
        } else if stContext.iScoreLoss >= 100 {
            50
        } else {
            -9999
        }
    } else {
        -9999
    };
    let open_warnings_postscore = if stContext.iOpenWarnings > TOPIC_MAX_WARNINGS {
        100
    } else {
        -9999
    };

    let postscore = [
        stContext.iTopicPostscore,
        stContext.iRestrictComments,
        section_comment_postscore(stContext.iSectionId),
        comment_count_restriction,
        score_loss_postscore,
        open_warnings_postscore,
        if stContext.bAllowAnonymous {
            -9999
        } else {
            -50
        },
    ]
    .into_iter()
    .max()
    .unwrap_or(-9999);

    const POSTSCORE_UNRESTRICTED: i32 = -9999;
    const POSTSCORE_MOD_AUTHOR: i32 = 9999;
    const POSTSCORE_MODERATORS_ONLY: i32 = 10000;
    const POSTSCORE_NO_COMMENTS: i32 = 10001;
    const POSTSCORE_HIDE_COMMENTS: i32 = 10002;
    const POSTSCORE_REGISTERED_ONLY: i32 = -50;

    if postscore == POSTSCORE_NO_COMMENTS || postscore == POSTSCORE_HIDE_COMMENTS {
        return Err(AppError::Forbidden);
    }
    if postscore == POSTSCORE_UNRESTRICTED {
        return Ok(());
    }
    if bAnonymous {
        return Err(AppError::Forbidden);
    }
    if user.canmod {
        return Ok(());
    }
    if postscore == POSTSCORE_REGISTERED_ONLY {
        return Ok(());
    }
    if postscore == POSTSCORE_MODERATORS_ONLY {
        return Err(AppError::Forbidden);
    }
    let view_by_author = user.id == stContext.iTopicAuthorId;
    if postscore == POSTSCORE_MOD_AUTHOR {
        return if view_by_author {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        };
    }
    if view_by_author || user.score.unwrap_or(0) >= postscore {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

/// TopicPermissionService.isCommentsAllowedByUser + checkCommentsAllowed:
/// combines topic state (deleted/expired/draft), user state
/// (blocked/frozen), and a postscore computed as the *max* across six
/// independent restriction sources, including `allow_anonymous`, matching
/// Java's `getPostscore` calculation.
pub(crate) async fn check_comment_posting_allowed(
    state: &AppState,
    user: &crate::models::UserSummary,
    bAnonymous: bool,
    topic_id: i32,
) -> Result<()> {
    let stContext = stCommentPostingContext(state, topic_id).await?;

    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(user.id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    let bFrozen = frozen_until
        .map(|u| u > chrono::Utc::now())
        .unwrap_or(false);
    check_comment_posting_context(&stContext, user, bAnonymous, bFrozen, false)
}

pub async fn comment_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CommentFormQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Html<String>> {
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let stResolution =
        crate::application::auth::stResolvePostingIdentity(&state, user.as_ref(), None, None)
            .await?;
    check_comment_posting_allowed(
        &state,
        &stResolution.stIdentity.stUser,
        !stResolution.stIdentity.bAuthorized,
        q.topic,
    )
    .await?;
    let (format_mode, format_title, _) = match &user {
        Some(stUser) => comment_format(&state, stUser.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    let bRequireCaptcha =
        user.is_none() || crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    render_comment_form(
        &state,
        &CommentForm {
            topic: q.topic,
            replyto: None,
            title: None,
            msg: String::new(),
            nick: None,
            password: None,
            preview: None,
            captcha_response: None,
            csrf: None,
        },
        csrf_token,
        format_mode,
        format_title,
        None,
        None,
        user.is_none(),
        bRequireCaptcha,
        true,
    )
    .await
}

#[derive(Deserialize)]
pub struct EditCommentQuery {
    pub topic: Option<i32>,
    pub original: Option<i32>,
    pub msgid: Option<i32>,
}

#[derive(Template)]
#[template(path = "edit_comment.html")]
struct EditCommentTemplate {
    comment_id: i32,
    topic_id: i32,
    topic_url: String,
    postdate: chrono::DateTime<chrono::Utc>,
    deadline: chrono::DateTime<chrono::Utc>,
    title: String,
    msg: String,
    format_mode: String,
    format_title: String,
    csrf_token: String,
    form_error: Option<String>,
    preview_html: Option<String>,
    require_captcha: bool,
    captcha_site_key: String,
}

type TyEditableCommentRow = (
    i32,
    i32,
    String,
    String,
    String,
    bool,
    chrono::DateTime<chrono::Utc>,
    bool,
);

async fn stEditableComment(
    state: &AppState,
    user: &crate::models::UserSummary,
    comment_id: i32,
) -> Result<TyEditableCommentRow> {
    let row: TyEditableCommentRow = sqlx::query_as(
        r#"SELECT c.topic, c.userid, c.title, m.message, m.markup::text,
                  c.deleted, c.postdate,
                  EXISTS(SELECT 1 FROM comments r WHERE r.replyto=c.id AND NOT r.deleted)
           FROM comments c JOIN msgbase m ON m.id=c.id WHERE c.id=$1"#,
    )
    .bind(comment_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (topic_id, author_id, _, _, _, deleted, postdate, has_replies) = &row;
    let topic_deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1")
        .bind(topic_id)
        .fetch_one(&state.pool)
        .await?;
    if *deleted || topic_deleted {
        return Err(AppError::BadRequest("тема или комментарий удалены".into()));
    }
    if is_topic_expired(state, *topic_id).await? {
        return Err(AppError::BadRequest("сообщение уже устарело".into()));
    }
    if user.id != *author_id {
        return Err(AppError::Forbidden);
    }
    if *has_replies {
        return Err(AppError::BadRequest(
            "редактирование комментариев с ответами запрещено".into(),
        ));
    }
    if user.score.unwrap_or(0) < COMMENT_EDIT_MIN_SCORE {
        return Err(AppError::Forbidden);
    }
    if row.4 == "PLAIN" && !user.candel {
        return Err(AppError::BadRequest(
            "Вы не можете редактировать тексты данного формата".into(),
        ));
    }
    if chrono::Utc::now() > *postdate + chrono::Duration::minutes(COMMENT_EDIT_WINDOW_MINUTES) {
        return Err(AppError::BadRequest("истек срок редактирования".into()));
    }
    Ok(row)
}

pub async fn edit_comment_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<EditCommentQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Response> {
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let Some(user) = user else {
        return Ok(crate::routes::auth::login_redirect(&format!(
            "/edit_comment?{}",
            serde_urlencoded::to_string([
                ("topic", query.topic.unwrap_or_default()),
                (
                    "original",
                    query.original.or(query.msgid).unwrap_or_default(),
                ),
            ])
            .unwrap_or_default()
        )));
    };
    let comment_id = query
        .original
        .or(query.msgid)
        .ok_or_else(|| AppError::BadRequest("Комментарий не задан".into()))?;
    let row = stEditableComment(&state, &user, comment_id).await?;
    if query.topic.is_some_and(|topic_id| topic_id != row.0) {
        return Err(AppError::BadRequest("тема не совпадает".into()));
    }
    let topic = crate::routes::topics::get_topic(&state, row.0).await?;
    if check_comment_posting_allowed(&state, &user, false, row.0)
        .await
        .is_err()
    {
        return Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}?cid={comment_id}", topic.topic_url()),
            )],
        )
            .into_response());
    }
    let (format_mode, format_title) = crate::routes::topics::markup_form_view(&row.4);
    let require_captcha = crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    Ok(Html(
        EditCommentTemplate {
            comment_id,
            topic_id: row.0,
            topic_url: topic.topic_url(),
            postdate: row.6,
            deadline: row.6 + chrono::Duration::minutes(COMMENT_EDIT_WINDOW_MINUTES),
            title: row.2,
            msg: row.3,
            format_mode,
            format_title,
            csrf_token,
            form_error: None,
            preview_html: None,
            require_captcha,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
        }
        .render()?,
    )
    .into_response())
}

#[derive(Deserialize)]
pub struct EditCommentForm {
    #[serde(alias = "original")]
    pub msgid: i32,
    pub topic: Option<i32>,
    pub msg: String,
    pub title: Option<String>,
    pub preview: Option<String>,
    #[serde(rename = "h-captcha-response")]
    pub captcha_response: Option<String>,
    pub csrf: Option<String>,
}

/// Default upstream config (config.properties.dist): comments are editable
/// only by their author, within 30 minutes of posting, only if they have no
/// replies yet, and only once the author has score >= 45.
/// comment.isModeratorAllowedToEdit defaults to false, so moderators do not
/// get a bypass here in the default configuration.
const COMMENT_EDIT_WINDOW_MINUTES: i64 = 30;
const COMMENT_EDIT_MIN_SCORE: i32 = 45;

/// TopicDao's `expired` column: `!sticky && COALESCE(commitdate,postdate) <
/// now()-sections.expire`. Shared by comment edit/delete/undelete, which
/// all gate on the topic's own expiry, not just the comment's age.
pub(crate) async fn is_topic_expired(state: &AppState, topic_id: i32) -> Result<bool> {
    Ok(sqlx::query_scalar(
        r#"SELECT NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_one(&state.pool)
    .await?)
}

pub async fn edit_comment(
    State(state): State<AppState>,
    headers: HeaderMap,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    Form(form): Form<EditCommentForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let row = stEditableComment(&state, &user, form.msgid).await?;
    let topic_id = row.0;
    if form.topic.is_some_and(|iTopicId| iTopicId != topic_id) {
        return Err(AppError::BadRequest("тема не совпадает".into()));
    }
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let bRequireCaptcha = crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    let mut optError = optCommentBodyError(&form.msg, false);
    if optError.is_none()
        && let Some(sError) = optCommentActorError(&state, &user, false, &sRemoteIp).await?
    {
        optError = Some(sError);
    }
    if form.preview.is_none()
        && form.csrf.as_deref().map(str::trim) != Some(csrf_token.trim())
        && optError.is_none()
    {
        optError = Some("Неправильный код защиты CSRF. Возможно сессия устарела".into());
    }
    if form.preview.is_none()
        && bRequireCaptcha
        && let Err(sError) = crate::application::auth::sValidateCaptcha(
            &state.config,
            &state.http,
            form.captcha_response.as_deref(),
            &sRemoteIp,
        )
        .await
        && optError.is_none()
    {
        optError = Some(sError);
    }
    if optError.is_none()
        && let Err(stError) = check_comment_posting_allowed(&state, &user, false, topic_id).await
    {
        optError = Some(sCommentFormError(stError)?);
    }
    let (format_mode, format_title) = crate::routes::topics::markup_form_view(&row.4);
    let topic = crate::routes::topics::get_topic(&state, topic_id).await?;
    if form.preview.is_some() || optError.is_some() {
        return Ok(Html(
            EditCommentTemplate {
                comment_id: form.msgid,
                topic_id,
                topic_url: topic.topic_url(),
                postdate: row.6,
                deadline: row.6 + chrono::Duration::minutes(COMMENT_EDIT_WINDOW_MINUTES),
                title: form.title.clone().unwrap_or_default(),
                msg: form.msg.clone(),
                format_mode,
                format_title,
                csrf_token,
                form_error: optError,
                preview_html: form
                    .preview
                    .as_ref()
                    .map(|_| markup::render_message_with_markup(&form.msg, Some(&row.4), None)),
                require_captcha: bRequireCaptcha,
                captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
            }
            .render()?,
        )
        .into_response());
    }

    let sNewTitle = form.title.unwrap_or_default();
    let optOldMessage = (row.3 != form.msg).then_some(row.3.as_str());
    let optOldTitle = (row.2 != sNewTitle).then_some(row.2.as_str());
    let setOldMentions = markup::extract_mentions(&row.3)
        .into_iter()
        .map(|sNick| sNick.to_lowercase())
        .collect::<std::collections::HashSet<_>>();
    let vecNewMentions = markup::extract_mentions(&form.msg)
        .into_iter()
        .map(|sNick| sNick.to_lowercase())
        .filter(|sNick| !setOldMentions.contains(sNick))
        .collect::<Vec<_>>();
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE msgbase SET message=$2 WHERE id=$1")
        .bind(form.msgid)
        .bind(&form.msg)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE comments SET title=$2 WHERE id=$1")
        .bind(form.msgid)
        .bind(&sNewTitle)
        .execute(&mut *tx)
        .await?;
    if optOldMessage.is_some() || optOldTitle.is_some() {
        sqlx::query(
            r#"INSERT INTO edit_info(msgid,editor,oldmessage,oldtitle,object_type)
               VALUES($1,$2,$3,$4,'COMMENT'::edit_event_type)"#,
        )
        .bind(form.msgid)
        .bind(user.id)
        .bind(optOldMessage)
        .bind(optOldTitle)
        .execute(&mut *tx)
        .await?;
        let iEditCount: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM edit_info WHERE msgid=$1 AND object_type='COMMENT'::edit_event_type",
        )
        .bind(form.msgid)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE comments SET editor_id=$2,edit_date=now(),edit_count=$3 WHERE id=$1")
            .bind(form.msgid)
            .bind(user.id)
            .bind(iEditCount.min(i64::from(i32::MAX)) as i32)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE topics SET lastmod=now() WHERE id=$1")
            .bind(topic_id)
            .execute(&mut *tx)
            .await?;
    }
    let mut vecNotified = if user.score.unwrap_or(0) >= 0 && !vecNewMentions.is_empty() {
        sqlx::query_scalar(
            r#"SELECT u.id FROM users u
               WHERE lower(u.nick)=ANY($1) AND u.id<>$2
                 AND NOT EXISTS (
                   SELECT 1 FROM ignore_list il WHERE il.userid=u.id AND il.ignored=$2
                 )"#,
        )
        .bind(&vecNewMentions)
        .bind(user.id)
        .fetch_all(&mut *tx)
        .await?
    } else {
        Vec::new()
    };
    for iUserId in &vecNotified {
        sqlx::query(
            "INSERT INTO user_events(userid,type,private,message_id,comment_id) VALUES($1,'REF',false,$2,$3)",
        )
        .bind(iUserId)
        .bind(topic_id)
        .bind(form.msgid)
        .execute(&mut *tx)
        .await?;
    }
    if !vecNotified.is_empty() {
        vecNotified.sort_unstable();
        vecNotified.dedup();
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=ANY($1)")
            .bind(&vecNotified)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    state.realtime.vNotifyEvents(vecNotified.iter().copied());
    crate::search_index::index_comment(&state, form.msgid).await;
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, comment_link(&state, form.msgid).await?)],
    )
        .into_response())
}

pub async fn delete_comment_form(
    State(state): State<AppState>,
    Query(q): Query<JumpQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let row: (
        i32,
        String,
        String,
        i32,
        bool,
        chrono::DateTime<chrono::Utc>,
        bool,
    ) = sqlx::query_as(
        r#"SELECT c.topic,c.title,u.nick,c.userid,c.deleted,c.postdate,
                  EXISTS(SELECT 1 FROM comments r WHERE r.replyto=c.id AND NOT r.deleted)
           FROM comments c JOIN users u ON u.id=c.userid WHERE c.id=$1"#,
    )
    .bind(q.msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    if row.4 {
        return Err(AppError::BadRequest("комментарий уже удален".into()));
    }
    let bTopicDeleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1")
        .bind(row.0)
        .fetch_one(&state.pool)
        .await?;
    if bTopicDeleted {
        return Err(AppError::Forbidden);
    }
    let bDeletable = user.canmod
        || (user.id == row.3
            && !row.6
            && !is_topic_expired(&state, row.0).await?
            && chrono::Utc::now() <= row.5 + chrono::Duration::hours(COMMENT_DELETE_WINDOW_HOURS));
    if !bDeletable {
        return Err(AppError::Forbidden);
    }
    // DeleteCommentController.deleteComments: only a moderator may set
    // `bonus`/`delete_replys` - a plain author sees just the reason field.
    let mod_fields = if user.canmod {
        r#"<label>Штраф (0-20) <input type="number" name="bonus" min="0" max="20" value="0"></label>
  <label><input type="checkbox" name="delete_replys" value="true"> Удалить с ответами</label>"#
    } else {
        ""
    };
    Ok(Html(format!(
        r#"
<h1>Удалить комментарий #{}</h1>
<p>Тема #{} · {} · автор {}</p>
<form method="post" action="/delete_comment.jsp">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <label>Причина <input name="reason"></label>
  {mod_fields}
  <button type="submit">Удалить</button>
</form>
"#,
        q.msgid,
        row.0,
        html_escape::encode_text(&row.1),
        html_escape::encode_text(&row.2),
        q.msgid
    )))
}

pub async fn undelete_comment_form(
    State(state): State<AppState>,
    Query(q): Query<JumpQuery>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    let stPermissionRow: Option<(i32, i32, bool, bool, Option<i32>)> = sqlx::query_as(
        r#"SELECT c.topic,c.userid,c.deleted,t.deleted,di.delby
           FROM comments c JOIN topics t ON t.id=c.topic
           LEFT JOIN del_info di ON di.msgid=c.id WHERE c.id=$1"#,
    )
    .bind(q.msgid)
    .fetch_optional(&state.pool)
    .await?;
    let Some((iTopicId, iAuthorId, bDeleted, bTopicDeleted, optDeletedBy)) = stPermissionRow else {
        return Err(AppError::NotFound);
    };
    if bTopicDeleted
        || !bDeleted
        || is_topic_expired(&state, iTopicId).await?
        || optDeletedBy == Some(iAuthorId)
    {
        return Err(AppError::Forbidden);
    }
    Ok(Html(format!(
        r#"
<h1>Восстановить комментарий #{}</h1>
<form method="post" action="/undelete_comment">
  <input type="hidden" name="csrf" value="{csrf_token}">
  <input type="hidden" name="msgid" value="{}">
  <button type="submit">Восстановить</button>
</form>
"#,
        q.msgid, q.msgid
    )))
}

#[derive(Deserialize)]
pub struct CommentAction {
    pub msgid: i32,
    pub reason: Option<String>,
    pub bonus: Option<i32>,
    pub delete_replys: Option<String>,
}

#[derive(Template)]
#[template(path = "action_done.html")]
struct StCommentActionDoneTemplate {
    message: String,
    big_message: Option<String>,
    link: Option<String>,
}

#[derive(Template)]
#[template(path = "comment_deleted_by_moderator.html")]
struct StCommentDeletedByModeratorTemplate {
    message: String,
    big_message: Option<String>,
    link: String,
    author_nick: String,
    ip: String,
    user_agent_id: i32,
}

pub(crate) fn iDeleteScoreDelta(iPenalty: i32) -> i32 {
    -iPenalty.clamp(0, 20)
}

/// DeleteReasons.replyBonusAndReason: when the root comment's penalty was
/// more than a token amount (>2 points), decay the same penalty down the
/// reply tree - direct children lose 2, grandchildren 1, anything deeper 0.
/// Returned as the same non-positive score delta persisted by Java.
fn reply_bonus_and_reason(drop_score: bool, depth: i32) -> (i32, &'static str) {
    if !drop_score {
        return (0, "7.1 Ответ на некорректное сообщение (авто)");
    }
    match depth {
        0 => (-2, "7.1 Ответ на некорректное сообщение (авто, уровень 0)"),
        1 => (-1, "7.1 Ответ на некорректное сообщение (авто, уровень 1)"),
        _ => (0, "7.1 Ответ на некорректное сообщение (авто, уровень >1)"),
    }
}

async fn effective_delete_bonus(
    state: &AppState,
    author_id: i32,
    requested_bonus: i32,
) -> Result<i32> {
    if requested_bonus == 0 || author_id == ANONYMOUS_USER_ID {
        return Ok(requested_bonus);
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(author_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    Ok(
        if frozen_until
            .map(|u| u > chrono::Utc::now())
            .unwrap_or(false)
        {
            0
        } else {
            requested_bonus
        },
    )
}

/// Matches TopicPermissionService.DeletePeriod: authors may delete their own
/// comment for 3 hours after posting (and only if nobody has replied yet).
/// Moderators bypass this window entirely.
const COMMENT_DELETE_WINDOW_HOURS: i64 = 3;

pub async fn delete_comment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<CommentAction>,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    if form.bonus.is_some_and(|iBonus| !(0..=20).contains(&iBonus)) {
        return Err(AppError::BadRequest("неправильный размер штрафа".into()));
    }
    let row: (i32, i32, bool, chrono::DateTime<chrono::Utc>, bool) = sqlx::query_as(
        r#"SELECT c.topic, c.userid, c.deleted, c.postdate,
                  EXISTS(SELECT 1 FROM comments r WHERE r.replyto=c.id AND NOT r.deleted) AS has_replies
           FROM comments c WHERE c.id=$1"#,
    )
    .bind(form.msgid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (topic_id, author_id, deleted, postdate, has_replies) = row;
    if deleted {
        return Err(AppError::BadRequest("комментарий уже удален".into()));
    }
    let topic_deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1")
        .bind(topic_id)
        .fetch_one(&state.pool)
        .await?;

    // isCommentDeletableNow: moderators bypass the expired check entirely;
    // an author may only delete their own comment while the topic is
    // still "live".
    let deletable = user.canmod || {
        let within_window =
            chrono::Utc::now() <= postdate + chrono::Duration::hours(COMMENT_DELETE_WINDOW_HOURS);
        let topic_expired = is_topic_expired(&state, topic_id).await?;
        user.id == author_id && !has_replies && !topic_deleted && !topic_expired && within_window
    };
    if !deletable {
        return Err(AppError::Forbidden);
    }

    let optModeratorContext: Option<(String, Option<String>, i32)> =
        if user.canmod && user.id != author_id {
            Some(
                sqlx::query_as(
                    r#"SELECT u.nick, host(c.postip), COALESCE(c.ua_id,0)
                       FROM comments c JOIN users u ON u.id=c.userid WHERE c.id=$1"#,
                )
                .bind(form.msgid)
                .fetch_one(&state.pool)
                .await?,
            )
        } else {
            None
        };

    let requested_bonus = if user.canmod && user.id != author_id {
        iDeleteScoreDelta(form.bonus.unwrap_or(0))
    } else {
        0
    };
    let bonus = effective_delete_bonus(&state, author_id, requested_bonus).await?;
    let reason = form.reason.clone().unwrap_or_default();

    // DeleteService.deleteCommentWithReplys: moderator-only cascade that
    // walks the still-live reply subtree, decaying the same penalty by
    // depth (see reply_bonus_and_reason), and skips reply notifications
    // when the topic has expired (matching notifyReplys = !topic.expired).
    let mut vecDeletedReplies: Vec<(i32, i32, i32, &'static str)> = Vec::new();
    let mut bNotifyReplies = false;
    if user.canmod && form.delete_replys.is_some() {
        let drop_score = bonus < -2;
        bNotifyReplies = !is_topic_expired(&state, topic_id).await?;
        let replies: Vec<(i32, i32, i32)> = sqlx::query_as(
            r#"WITH RECURSIVE subtree AS (
                 SELECT id, userid, 0 AS depth FROM comments WHERE replyto=$1 AND NOT deleted
                 UNION ALL
                 SELECT c.id, c.userid, s.depth+1 FROM comments c JOIN subtree s ON c.replyto=s.id WHERE NOT c.deleted
               )
               SELECT id, userid, depth FROM subtree"#,
        )
        .bind(form.msgid)
        .fetch_all(&state.pool)
        .await?;
        for (reply_id, reply_author, depth) in &replies {
            let (reply_bonus, reply_reason) = reply_bonus_and_reason(drop_score, *depth);
            let reply_bonus = effective_delete_bonus(&state, *reply_author, reply_bonus).await?;
            vecDeletedReplies.push((*reply_id, *reply_author, reply_bonus, reply_reason));
        }
    }

    let mut tx = state.pool.begin().await?;
    let stRootUpdate = sqlx::query("UPDATE comments SET deleted=true WHERE id=$1 AND NOT deleted")
        .bind(form.msgid)
        .execute(&mut *tx)
        .await?;
    if stRootUpdate.rows_affected() == 0 {
        return Err(AppError::BadRequest("комментарий уже удален".into()));
    }
    let mut vecCommittedReplies = Vec::new();
    for (reply_id, reply_author, reply_bonus, reply_reason) in vecDeletedReplies {
        let stReplyUpdate =
            sqlx::query("UPDATE comments SET deleted=true WHERE id=$1 AND NOT deleted")
                .bind(reply_id)
                .execute(&mut *tx)
                .await?;
        if stReplyUpdate.rows_affected() > 0 {
            sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES($1,$2,$3,now(),$4) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now(), bonus=EXCLUDED.bonus")
                .bind(reply_id).bind(user.id).bind(reply_reason).bind(reply_bonus).execute(&mut *tx).await?;
            if reply_bonus != 0 {
                sqlx::query("UPDATE users SET score=GREATEST(score+$2,0) WHERE id=$1")
                    .bind(reply_author)
                    .bind(reply_bonus)
                    .execute(&mut *tx)
                    .await?;
            }
            vecCommittedReplies.push((reply_id, reply_author, reply_reason));
        }
    }

    sqlx::query("INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES($1,$2,$3,now(),$4) ON CONFLICT(msgid) DO UPDATE SET delby=EXCLUDED.delby, reason=EXCLUDED.reason, deldate=now(), bonus=EXCLUDED.bonus")
        .bind(form.msgid).bind(user.id).bind(&reason).bind(bonus).execute(&mut *tx).await?;
    if bonus != 0 {
        sqlx::query("UPDATE users SET score=GREATEST(score+$2,0) WHERE id=$1")
            .bind(author_id)
            .bind(bonus)
            .execute(&mut *tx)
            .await?;
    }
    // CommentDao.deleteComment: unlike an insert, deletion has no DB
    // trigger - Java decrements topics.stat1 in app code and clamps stat3
    // so it never exceeds the (now smaller) live comment count.
    sqlx::query("UPDATE topics SET stat1=stat1-$2, lastmod=now() WHERE id=$1")
        .bind(topic_id)
        .bind(1 + vecCommittedReplies.len() as i32)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE topics SET stat3=stat1 WHERE id=$1 AND stat3>stat1")
        .bind(topic_id)
        .execute(&mut *tx)
        .await?;
    let vecDeletedCommentIds = std::iter::once(form.msgid)
        .chain(vecCommittedReplies.iter().map(|stReply| stReply.0))
        .collect::<Vec<_>>();
    vDeleteCommentEventsTx(&mut tx, &vecDeletedCommentIds).await?;
    for (reply_id, reply_author, reply_reason) in &vecCommittedReplies {
        if bNotifyReplies {
            vNotifyDeletedTx(
                &mut tx,
                *reply_author,
                user.id,
                Some(topic_id),
                Some(*reply_id),
                reply_reason,
            )
            .await?;
        }
    }
    vNotifyDeletedTx(
        &mut tx,
        author_id,
        user.id,
        Some(topic_id),
        Some(form.msgid),
        &reason,
    )
    .await?;
    tx.commit().await?;
    for (reply_id, _, _) in vecCommittedReplies {
        crate::search_index::index_comment(&state, reply_id).await;
    }
    crate::search_index::index_comment(&state, form.msgid).await;
    let optNextCommentId: Option<i32> = sqlx::query_scalar(
        "SELECT min(id) FROM comments WHERE topic=$1 AND NOT deleted AND id >= $2",
    )
    .bind(topic_id)
    .bind(form.msgid)
    .fetch_one(&state.pool)
    .await?;
    let sNextLink = if let Some(iNextCommentId) = optNextCommentId {
        comment_link(&state, iNextCommentId).await?
    } else {
        crate::routes::topics::get_topic(&state, topic_id)
            .await?
            .topic_url()
    };
    let sBigMessage = Some(format!(
        "Удаленные комментарии: {}",
        vecDeletedCommentIds
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if let Some((sAuthorNick, sIp, iUserAgentId)) = optModeratorContext {
        Ok(Html(
            StCommentDeletedByModeratorTemplate {
                message: "Удалено успешно".into(),
                big_message: sBigMessage,
                link: sNextLink,
                author_nick: sAuthorNick,
                ip: sIp.unwrap_or_default(),
                user_agent_id: iUserAgentId,
            }
            .render()?,
        ))
    } else {
        Ok(Html(
            StCommentActionDoneTemplate {
                message: "Удалено успешно".into(),
                big_message: sBigMessage,
                link: Some(sNextLink),
            }
            .render()?,
        ))
    }
}

/// UserEventService.insertTopicDeleteNotification/insertCommentDeleteNotification:
/// privately tell the author their content was deleted (with the reason),
/// unless they deleted it themselves, are the anonymous user (id=2), or are
/// currently frozen.
pub(crate) const ANONYMOUS_USER_ID: i32 = 2;

pub(crate) async fn vNotifyDeletedTx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    author_id: i32,
    deleted_by: i32,
    topic_id: Option<i32>,
    comment_id: Option<i32>,
    reason: &str,
) -> Result<()> {
    if author_id == deleted_by || author_id == ANONYMOUS_USER_ID {
        return Ok(());
    }
    let frozen_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT frozen_until FROM users WHERE id=$1")
            .bind(author_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten();
    if frozen_until
        .map(|u| u > chrono::Utc::now())
        .unwrap_or(false)
    {
        return Ok(());
    }
    sqlx::query("INSERT INTO user_events(userid,type,private,message_id,comment_id,message) VALUES($1,'DEL',true,$2,$3,$4)")
        .bind(author_id)
        .bind(topic_id)
        .bind(comment_id)
        .bind(reason)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=$1")
        .bind(author_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// UserEventService.processCommentsDeleted: remove notifications whose target
/// disappeared and recalculate every affected user's cached unread counter in
/// the same transaction as the comment deletion.
async fn vDeleteCommentEventsTx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    vecCommentIds: &[i32],
) -> Result<()> {
    if vecCommentIds.is_empty() {
        return Ok(());
    }
    let vecAffectedUsers: Vec<i32> = sqlx::query_scalar(
        r#"SELECT DISTINCT userid FROM user_events
           WHERE comment_id = ANY($1)
             AND type IN ('REPLY','WATCH','REF','REACTION','WARNING')"#,
    )
    .bind(vecCommentIds)
    .fetch_all(&mut **tx)
    .await?;
    sqlx::query(
        r#"DELETE FROM user_events
           WHERE comment_id = ANY($1)
             AND type IN ('REPLY','WATCH','REF','REACTION','WARNING')"#,
    )
    .bind(vecCommentIds)
    .execute(&mut **tx)
    .await?;
    if !vecAffectedUsers.is_empty() {
        sqlx::query(
            r#"UPDATE users SET unread_events=(
                   SELECT count(*) FROM user_events e
                   WHERE e.unread AND e.userid=users.id
               ) WHERE id = ANY($1)"#,
        )
        .bind(&vecAffectedUsers)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn undelete_comment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<CommentAction>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    if !user.canmod {
        return Err(AppError::Forbidden);
    }
    let row: (i32, bool) = sqlx::query_as("SELECT topic, deleted FROM comments WHERE id=$1")
        .bind(form.msgid)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let (topic_id, deleted) = row;
    if !deleted {
        return Err(AppError::Forbidden);
    }
    let topic_deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1")
        .bind(topic_id)
        .fetch_one(&state.pool)
        .await?;
    if topic_deleted {
        return Err(AppError::Forbidden);
    }
    // isUndeletable: unlike delete, the expired check here applies even to
    // moderators - once a topic has expired, its comments are frozen.
    if is_topic_expired(&state, topic_id).await? {
        return Err(AppError::Forbidden);
    }
    // Mirrors TopicPermissionService.isUndeletable: a comment cannot be
    // undeleted if its own author is the one who deleted it (self-moderation
    // is respected, only another moderator's deletion can be reversed).
    let author_id: i32 = sqlx::query_scalar("SELECT userid FROM comments WHERE id=$1")
        .bind(form.msgid)
        .fetch_one(&state.pool)
        .await?;
    let mut tx = state.pool.begin().await?;
    let stDeleteInfo: Option<(i32, Option<i32>)> =
        sqlx::query_as("SELECT delby, bonus FROM del_info WHERE msgid=$1 FOR UPDATE")
            .bind(form.msgid)
            .fetch_optional(&mut *tx)
            .await?;
    if stDeleteInfo.as_ref().map(|stValue| stValue.0) == Some(author_id) {
        return Err(AppError::Forbidden);
    }

    if let Some(iBonus) = stDeleteInfo
        .and_then(|stValue| stValue.1)
        .filter(|iValue| *iValue != 0)
    {
        sqlx::query("UPDATE users SET score=GREATEST(score-$2,0) WHERE id=$1")
            .bind(author_id)
            .bind(iBonus)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE comments SET deleted=false WHERE id=$1")
        .bind(form.msgid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM del_info WHERE msgid=$1")
        .bind(form.msgid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id=$1")
        .bind(topic_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    crate::search_index::index_comment(&state, form.msgid).await;
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, comment_link(&state, form.msgid).await?)],
    )
        .into_response())
}

const COMMENT_MAX_LENGTH: usize = 8192;
const COMMENT_MAX_LENGTH_ANONYMOUS: usize = 4096;

async fn insert_comment(
    state: &AppState,
    user_id: i32,
    bAnonymous: bool,
    bUserCastAllowed: bool,
    form: &CommentForm,
    markup: &str,
    sRemoteIp: &str,
    optUserAgent: Option<&str>,
) -> Result<i32> {
    if let Some(sError) = optCommentBodyError(&form.msg, bAnonymous) {
        return Err(AppError::BadRequest(sError));
    }
    // The original inline form uses replyto=0 for a top-level comment;
    // PostgreSQL expects NULL because comments.replyto is a foreign key.
    let replyto = form.replyto.filter(|id| *id > 0);
    let mut tx = state.pool.begin().await?;
    if let Some(iReplyTo) = replyto {
        let optReply: Option<(i32, bool)> =
            sqlx::query_as("SELECT topic,deleted FROM comments WHERE id=$1 FOR SHARE")
                .bind(iReplyTo)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((iReplyTopicId, bDeleted)) = optReply else {
            return Err(AppError::NotFound);
        };
        if bDeleted {
            return Err(AppError::BadRequest(
                "нельзя комментировать удаленные комментарии".into(),
            ));
        }
        if iReplyTopicId != form.topic {
            return Err(AppError::BadRequest("некорректная тема".into()));
        }
    }
    let id: i32 = sqlx::query_scalar("SELECT nextval('s_msgid')::int")
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO msgbase(id, message, markup) VALUES($1,$2,$3::markup_type)")
        .bind(id)
        .bind(&form.msg)
        .bind(markup)
        .execute(&mut *tx)
        .await?;
    let optUserAgent = optUserAgent.map(|sValue| {
        let mut iEnd = sValue.len().min(511);
        while !sValue.is_char_boundary(iEnd) {
            iEnd -= 1;
        }
        &sValue[..iEnd]
    });
    sqlx::query(
        "INSERT INTO comments(id, topic, userid, title, postdate, replyto, postip, ua_id) VALUES($1,$2,$3,$4,now(),$5,$6::inet,create_user_agent($7))",
    )
    .bind(id)
    .bind(form.topic)
    .bind(user_id)
    // CommentRequest has no title field for creation; Comment.buildNew
    // always persists an empty title. Do not accept a port-only override.
    .bind("")
    .bind(replyto)
    .bind(sRemoteIp)
    .bind(optUserAgent)
    .execute(&mut *tx)
    .await?;
    // topics.stat1/stat3 and groups.stat3 are now kept in sync by the
    // comins() trigger (see db/migrations/0013) - matches Java's DB-side
    // bookkeeping exactly, instead of a partial manual update here that
    // would double-count once the trigger exists.

    // Matches CommentCreateService.notifyReply / UserEventDao.insertCommentWatchNotification:
    // notify the parent comment's author (REPLY) and topic watchers (WATCH),
    // skipping the commenter themselves and anyone who has the commenter ignored.
    let mut notified: Vec<i32> = Vec::new();

    let mut parent_author: Option<i32> = None;
    if let Some(replyto) = replyto
        && let Some(parent_userid) =
            sqlx::query_scalar::<_, i32>("SELECT userid FROM comments WHERE id=$1")
                .bind(replyto)
                .fetch_optional(&mut *tx)
                .await?
    {
        parent_author = Some(parent_userid);
        if parent_userid != user_id && parent_userid != 2 {
            let ignored: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM ignore_list WHERE userid=$1 AND ignored=$2)",
            )
            .bind(parent_userid)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if !ignored {
                sqlx::query("INSERT INTO user_events(userid,type,private,message_id,comment_id) VALUES($1,'REPLY',false,$2,$3)")
                    .bind(parent_userid)
                    .bind(form.topic)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                notified.push(parent_userid);
            }
        }
    }

    let watchers: Vec<i32> = if let Some(iParentAuthor) = parent_author {
        // For a reply, Java suppresses WATCH when the watcher ignores any
        // author in the new comment's branch, not only the immediate author.
        sqlx::query_scalar(
            r#"SELECT m.userid FROM memories m
               WHERE m.topic=$1 AND m.watch AND m.userid<>$2 AND m.userid<>$3 AND m.userid<>2
                 AND NOT EXISTS (
                   SELECT 1 FROM ignore_list il
                   WHERE il.userid=m.userid
                     AND il.ignored IN (SELECT get_branch_authors($4))
                 )"#,
        )
        .bind(form.topic)
        .bind(user_id)
        .bind(iParentAuthor)
        .bind(id)
        .fetch_all(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar(
            r#"SELECT m.userid FROM memories m
               WHERE m.topic=$1 AND m.watch AND m.userid<>$2 AND m.userid<>2
                 AND NOT EXISTS (
                   SELECT 1 FROM ignore_list il
                   WHERE il.userid=m.userid AND il.ignored=$2
                 )"#,
        )
        .bind(form.topic)
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?
    };
    for watcher in &watchers {
        sqlx::query("INSERT INTO user_events(userid,type,private,message_id,comment_id) VALUES($1,'WATCH',false,$2,$3)")
            .bind(watcher)
            .bind(form.topic)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        notified.push(*watcher);
    }

    // CommentCreateService.notifyMentions: notify each @nick referenced in
    // the raw comment text, skipping the commenter and anyone mentioned who
    // has the commenter on their ignore list.
    let mentioned_nicks = markup::extract_mentions(&form.msg);
    if bUserCastAllowed && !mentioned_nicks.is_empty() {
        let mentioned_ids: Vec<i32> = sqlx::query_scalar(
            r#"SELECT u.id FROM users u
               WHERE lower(u.nick) = ANY($1) AND u.id <> $2
                 AND NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=u.id AND il.ignored=$2)"#,
        )
        .bind(mentioned_nicks.iter().map(|n| n.to_lowercase()).collect::<Vec<_>>())
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        for mentioned_id in &mentioned_ids {
            sqlx::query("INSERT INTO user_events(userid,type,private,message_id,comment_id) VALUES($1,'REF',false,$2,$3)")
                .bind(mentioned_id)
                .bind(form.topic)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            notified.push(*mentioned_id);
        }
    }

    if !notified.is_empty() {
        notified.sort_unstable();
        notified.dedup();
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=ANY($1)")
            .bind(&notified)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    // AddCommentController publishes only after the transaction succeeds and
    // preserves this order: topic subscribers first, notification owners
    // second.
    state.realtime.vNotifyNewComment(form.topic, id);
    state.realtime.vNotifyEvents(notified.iter().copied());
    crate::search_index::index_comment(state, id).await;
    Ok(id)
}

async fn locate_topic_or_comment(
    state: &AppState,
    msgid: i32,
) -> Result<Option<(String, String, i32, Option<i32>)>> {
    let row = sqlx::query_as::<_, (String, String, i32, Option<i32>)>(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section,
                  g.urlname, t.id, NULL::integer AS comment_id
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1
           UNION ALL
           SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section,
                  g.urlname, t.id, c.id AS comment_id
           FROM comments c JOIN topics t ON t.id=c.topic JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE c.id=$1
           LIMIT 1"#,
    )
    .bind(msgid)
    .fetch_optional(&state.pool)
    .await?;
    Ok(row)
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct StDeletedCommentRow {
    group_title: String,
    topic_title: String,
    topic_id: i32,
    reason: Option<String>,
    delete_date: Option<chrono::DateTime<chrono::Utc>>,
    bonus: i32,
    comment_id: i32,
    topic_deleted: bool,
    comment_deleted: bool,
}

#[derive(Debug, Clone)]
struct StDeletedCommentFilterView {
    value: &'static str,
    label: &'static str,
    selected: bool,
    url: String,
}

#[derive(Template)]
#[template(path = "deleted_comments.html")]
struct StDeletedCommentsTemplate {
    title: String,
    comments: Vec<StDeletedCommentRow>,
    filters: Vec<StDeletedCommentFilterView>,
    prev_link: Option<String>,
    next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StDeletedCommentsQuery {
    pub filter: Option<String>,
    pub offset: Option<i64>,
}

fn sDeletedCommentsLink(nick: &str, offset: i64, filter: &str) -> String {
    let mut query = Vec::new();
    if offset > 0 {
        query.push(format!("offset={offset}"));
    }
    if filter != "all" {
        query.push(format!("filter={filter}"));
    }
    let base = format!("/people/{}/deleted-comments", urlencoding::encode(nick));
    if query.is_empty() {
        base
    } else {
        format!("{base}?{}", query.join("&"))
    }
}

pub async fn deleted_comments_by_user(
    State(state): State<AppState>,
    Path(nick): Path<String>,
    Query(query): Query<StDeletedCommentsQuery>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>> {
    if !user.as_ref().map(|u| u.canmod).unwrap_or(false) {
        return Err(AppError::Forbidden);
    }
    const PAGE_SIZE: i64 = 50;
    const MAX_OFFSET: i64 = 300;
    let offset = query.offset.unwrap_or(0);
    if !(0..=MAX_OFFSET).contains(&offset) {
        return Err(AppError::BadRequest("Некорректное значение offset".into()));
    }
    let target = crate::routes::users::get_user(&state, &nick).await?;
    let filter = match query.filter.as_deref() {
        Some("penalty") => "penalty",
        Some("noauto") => "noauto",
        Some("self") => "self",
        _ => "all",
    };
    let filter_clause = match filter {
        "penalty" => "di.bonus IS NOT NULL AND di.bonus<>0",
        "noauto" => "c.deleted AND di.reason IS NOT NULL AND di.reason NOT ILIKE '%(авто%'",
        "self" => "di.delby=c.userid",
        _ => "true",
    };
    let sql = format!(
        r#"SELECT g.title AS group_title,
                  CASE WHEN trim(COALESCE(t.title,''))='' THEN 'Без заглавия' ELSE t.title END AS topic_title,
                  t.id AS topic_id, di.reason,
                  COALESCE(di.deldate, topic_di.deldate) AS delete_date,
                  COALESCE(di.bonus,0)::int AS bonus, c.id AS comment_id,
                  t.deleted AS topic_deleted, c.deleted AS comment_deleted
             FROM groups g JOIN topics t ON g.id=t.groupid
             JOIN comments c ON c.topic=t.id
             LEFT JOIN del_info di ON di.msgid=c.id
             LEFT JOIN del_info topic_di ON topic_di.msgid=t.id
            WHERE c.userid=$1 AND (c.deleted OR t.deleted) AND {filter_clause}
            ORDER BY COALESCE(di.deldate, topic_di.deldate) DESC NULLS LAST, c.id DESC
            LIMIT 50 OFFSET $2"#,
    );
    let comments = sqlx::query_as::<_, StDeletedCommentRow>(sqlx::AssertSqlSafe(sql))
        .bind(target.id)
        .bind(offset)
        .fetch_all(&state.pool)
        .await?;
    let definitions = [
        ("all", "все"),
        ("penalty", "со штрафом"),
        ("noauto", "без авто"),
        ("self", "сам удалил"),
    ];
    let filters = definitions
        .into_iter()
        .map(|(value, label)| StDeletedCommentFilterView {
            value,
            label,
            selected: filter == value,
            url: sDeletedCommentsLink(&target.nick, 0, value),
        })
        .collect();
    let title = definitions
        .iter()
        .find(|(value, _)| *value == filter)
        .map(|(_, label)| {
            if filter == "all" {
                format!("Удаленные комментарии {}", target.nick)
            } else {
                format!("Удаленные комментарии {} ({label})", target.nick)
            }
        })
        .expect("known deleted-comment filter");
    let prev_link = (offset >= PAGE_SIZE)
        .then(|| sDeletedCommentsLink(&target.nick, offset - PAGE_SIZE, filter));
    let next_link = (offset < MAX_OFFSET && comments.len() == PAGE_SIZE as usize)
        .then(|| sDeletedCommentsLink(&target.nick, offset + PAGE_SIZE, filter));
    Ok(Html(
        StDeletedCommentsTemplate {
            title,
            comments,
            filters,
            prev_link,
            next_link,
        }
        .render()?,
    ))
}

#[cfg(test)]
mod deletion_semantics_tests {
    use super::{
        StCommentPostingContext, check_comment_posting_context, iCommentThresholdSeconds,
        iDeleteScoreDelta, optCommentBodyError, reply_bonus_and_reason, sDeletedCommentsLink,
    };

    fn stUser(iId: i32, iScore: i32) -> crate::models::UserSummary {
        crate::models::UserSummary {
            id: iId,
            nick: format!("user{iId}"),
            name: None,
            score: Some(iScore),
            max_score: Some(iScore),
            photo: None,
            town: None,
            regdate: None,
            canmod: false,
            candel: false,
            corrector: false,
            blocked: Some(false),
            userinfo: None,
        }
    }

    fn stPostingContext() -> StCommentPostingContext {
        StCommentPostingContext {
            bDeleted: false,
            bDraft: false,
            bExpired: false,
            bSticky: false,
            iTopicPostscore: -9999,
            iRestrictComments: -9999,
            iSectionId: 2,
            iCommentCount: 0,
            iOpenWarnings: 0,
            bAllowAnonymous: true,
            iScoreLoss: 0,
            iTopicAuthorId: 42,
        }
    }

    #[test]
    fn moderator_penalty_is_persisted_as_java_score_delta() {
        assert_eq!(iDeleteScoreDelta(-1), 0);
        assert_eq!(iDeleteScoreDelta(0), 0);
        assert_eq!(iDeleteScoreDelta(7), -7);
        assert_eq!(iDeleteScoreDelta(99), -20);
        assert_eq!(reply_bonus_and_reason(true, 0).0, -2);
        assert_eq!(reply_bonus_and_reason(true, 1).0, -1);
        assert_eq!(reply_bonus_and_reason(true, 2).0, 0);
    }

    #[test]
    fn comment_flood_thresholds_include_slow_mode() {
        assert_eq!(iCommentThresholdSeconds(true, 500, None, 0), 30);
        assert_eq!(iCommentThresholdSeconds(false, 34, None, 0), 300);
        assert_eq!(iCommentThresholdSeconds(false, 99, None, 0), 30);
        assert_eq!(iCommentThresholdSeconds(false, 100, None, 0), 3);
        assert_eq!(iCommentThresholdSeconds(false, 500, None, 30), 300);
    }

    #[test]
    fn comment_body_validation_uses_java_xml_and_utf16_rules() {
        assert_eq!(
            optCommentBodyError(" \n ", false).as_deref(),
            Some("комментарий не может быть пустым")
        );
        assert!(optCommentBodyError(&"😀".repeat(2048), true).is_none());
        assert_eq!(
            optCommentBodyError(&"😀".repeat(2049), true).as_deref(),
            Some("Слишком большое сообщение")
        );
        assert!(
            optCommentBodyError("text\u{1}", false)
                .as_deref()
                .is_some_and(|sError| sError.contains("U+0001"))
        );
    }

    #[test]
    fn comment_postscore_matches_anonymous_registered_and_score_floors() {
        let mut stContext = stPostingContext();
        let stAnonymous = stUser(2, 0);
        let stRegistered = stUser(7, 44);
        assert!(
            check_comment_posting_context(&stContext, &stAnonymous, true, false, false).is_ok()
        );

        stContext.bAllowAnonymous = false;
        assert!(
            check_comment_posting_context(&stContext, &stAnonymous, true, false, false).is_err()
        );
        assert!(
            check_comment_posting_context(&stContext, &stRegistered, false, false, false).is_ok()
        );

        stContext.iSectionId = 3;
        assert!(
            check_comment_posting_context(&stContext, &stRegistered, false, false, false).is_err()
        );
        assert!(
            check_comment_posting_context(&stContext, &stUser(8, 45), false, false, false).is_ok()
        );
    }

    #[test]
    fn author_readonly_ignores_freeze_but_not_blocking() {
        let stContext = stPostingContext();
        let mut stAuthor = stUser(7, 100);
        assert!(check_comment_posting_context(&stContext, &stAuthor, false, true, false).is_err());
        assert!(check_comment_posting_context(&stContext, &stAuthor, false, true, true).is_ok());
        stAuthor.blocked = Some(true);
        assert!(check_comment_posting_context(&stContext, &stAuthor, false, false, true).is_err());
    }

    #[test]
    fn deleted_comment_pagination_preserves_java_query_contract() {
        assert_eq!(
            sDeletedCommentsLink("some user", 0, "all"),
            "/people/some%20user/deleted-comments"
        );
        assert_eq!(
            sDeletedCommentsLink("user", 50, "penalty"),
            "/people/user/deleted-comments?offset=50&filter=penalty"
        );
    }
}
