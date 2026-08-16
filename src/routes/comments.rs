use crate::{
    application::comment::{
        deletion::{CCommentDeletionService, EnCommentDeletionError, EnCommentDeletionRestriction},
        message_form::{CCommentMessageService, EnCommentMessageServiceError},
    },
    auth::CurrentUser,
    domain::comment::{
        deletion::{
            StCommentDeleteActor, StCommentDeletePreview, StDeleteCommentCommand,
            TrCommentReindexQueue, VEC_DELETE_REASONS,
        },
        message_form::{EnCommentMessageBindingError, stBindCommentMessageParameters},
    },
    error::{AppError, Result},
    infra::{
        postgres::comment_deletion_repository::CCommentDeletionPgRepository,
        postgres::comment_message_repository::CCommentMessagePgRepository,
        search_queue::CSearchQueueSender,
    },
    markup,
    state::AppState,
};
use askama::Template;
use axum::{
    Form,
    body::to_bytes,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
    routing::{MethodRouter, get},
};
use serde::Deserialize;
use std::net::SocketAddr;

const I_COMMENT_MESSAGE_PARAMETER_LIMIT: usize = 1024 * 1024;
const S_ALLOW_COMMENT_MESSAGE: &str = "GET,HEAD,POST,PUT,PATCH,DELETE,OPTIONS";
const S_ALLOW_COMMENT_ACTION_OPTIONS: &str = "GET,HEAD,POST,OPTIONS";
const S_ALLOW_COMMENT_ACTION_405: &str = "GET, POST";
const S_ALLOW_COMMENT_UNDELETE_OPTIONS: &str = "GET,HEAD,POST,OPTIONS";
const S_ALLOW_COMMENT_UNDELETE_405: &str = "GET, POST";

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

#[derive(Template)]
#[template(path = "comment_message.html")]
struct StCommentMessageTemplate {
    topic_id: i32,
    topic_title: String,
    group_title: String,
    section_title: String,
    topic_card_html: String,
    csrf_token: String,
    format_mode: String,
    format_title: String,
    form_msg: String,
    anonymous_form: bool,
    postscore_info_html: String,
    require_captcha: bool,
    captcha_site_key: String,
}

async fn optCommentFormContextHtml(
    state: &AppState,
    stTopic: &crate::models::TopicDetail,
    optReplyTo: Option<i32>,
    bShowTopic: bool,
    optSessionViewer: Option<&crate::models::UserSummary>,
    sCsrfToken: &str,
) -> Result<Option<String>> {
    if let Some(iReplyTo) = optReplyTo.filter(|iValue| *iValue > 0) {
        let stViewer = if let Some(stViewer) = optSessionViewer {
            stViewer.clone()
        } else {
            // CommentCreateService.prepareReplyto uses the HTTP session, not
            // a nick/password posting identity. Resolve the site's anonymous
            // principal so default profile/avatar/ignore behavior stays the
            // same when a public form supplies credentials.
            crate::application::auth::stResolvePostingIdentity(state, None, None, None)
                .await?
                .stIdentity
                .stUser
        };
        let (stTarget, stReply) = stCommentDeletionService(state)
            .optPrepareCommentOnly(iReplyTo, stViewer.id)
            .await?
            .ok_or(AppError::NotFound)?;
        if stTarget.iTopicId != stTopic.id {
            return Ok(None);
        }
        let sCommentHtml = sCommentDeletionPreviewHtml(
            state,
            &stViewer,
            optSessionViewer.is_some(),
            stTopic.id,
            stTarget.bTopicExpired,
            stTarget.bTopicDraft,
            stTarget.bCommentsHidden,
            &stTopic.topic_url(),
            sCsrfToken,
            false,
            false,
            std::slice::from_ref(&stReply),
        )
        .await?;
        return Ok(Some(format!(
            "<div class=\"comment\"><div class=\"messages\">{}</div></div>",
            sCommentHtml,
        )));
    }
    if bShowTopic {
        let sTopicTitlePlain = stTopic.sTitlePlain();
        let stMarkupUsers = state
            .markup
            .stResolveBatch([(&*stTopic.message, &*stTopic.markup)])
            .await?;
        return Ok(Some(format!(
            r#"<div class="messages"><article class="msg" id="topic-{}"><div class="msg-container"><div class="msg_body"><header><h1><a href="{}">{}</a></h1></header><div class="msg-text">{}</div><div class="sign"><a href="/people/{}/profile">{}</a> (<time data-format="default" datetime="{}">{}</time>)</div></div></div></article></div>"#,
            stTopic.id,
            stTopic.topic_url(),
            html_escape::encode_text(&sTopicTitlePlain),
            markup::render_message_with_markup_policy_and_users(
                &stTopic.message,
                Some(&stTopic.markup),
                None,
                stTopic.bNofollowAuthorLinks(),
                Some(&state.config.public_url),
                Some(&stMarkupUsers),
            ),
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
    stPreviewAuthor: &crate::models::UserSummary,
    bPreviewAuthorAnonymous: bool,
    optSessionViewer: Option<&crate::models::UserSummary>,
) -> Result<Html<String>> {
    let topic = crate::routes::topics::get_topic(state, form.topic).await?;
    let topic_url = topic.topic_url();
    let context_html = optCommentFormContextHtml(
        state,
        &topic,
        form.replyto,
        bShowTopicContext,
        optSessionViewer,
        &csrf_token,
    )
    .await?;
    let preview_html = preview_html.map(|sMessageHtml| {
        sPreparedFormCommentHtml(
            0,
            form.title.as_deref().unwrap_or_default(),
            &sMessageHtml,
            &stPreviewAuthor.nick,
            stPreviewAuthor.id,
            stPreviewAuthor.score.unwrap_or(0),
            stPreviewAuthor.max_score.unwrap_or(0),
            stPreviewAuthor.blocked.unwrap_or(false),
            bPreviewAuthorAnonymous,
            chrono::Utc::now(),
            topic.author_id,
            optSessionViewer.is_some(),
            optSessionViewer.is_some_and(|stUser| stUser.canmod),
        )
    });
    Ok(Html(
        CommentFormTemplate {
            topic_id: topic.id,
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

/// Resolves the current profile's comment-format form id, display title and
/// stored database markup id.  Markup preview uses the same session-profile
/// default as comment forms in the original application.
pub(crate) async fn user_comment_format(
    state: &AppState,
    user_id: i32,
) -> Result<(String, String, String)> {
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
    let optPostingError = check_comment_posting_allowed(
        &state,
        &stResolution.stIdentity.stUser,
        !stResolution.stIdentity.bAuthorized,
        q.topic,
    )
    .await
    .err()
    .map(sCommentFormError)
    .transpose()?;
    let optReplyError = optCommentReplyError(&state, q.topic, q.replyto).await?;
    let (format_mode, format_title, _) = match &user {
        Some(user) => user_comment_format(&state, user.id).await?,
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
        optPostingError.or(optReplyError),
        None,
        user.is_none(),
        bRequireCaptcha,
        false,
        &stResolution.stIdentity.stUser,
        !stResolution.stIdentity.bAuthorized,
        user.as_ref(),
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
        Some(stUser) => user_comment_format(&state, stUser.id).await?,
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
    let optPreview = if form.preview.is_some() {
        let stMarkupUsers = state
            .markup
            .stResolveBatch([(&*form.msg, &*markup)])
            .await?;
        Some(markup::render_message_with_markup_policy_and_users(
            &form.msg,
            Some(&markup),
            None,
            false,
            Some(&state.config.public_url),
            Some(&stMarkupUsers),
        ))
    } else {
        None
    };
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
            &stIdentity.stUser,
            !stIdentity.bAuthorized,
            user.as_ref(),
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
        true,
    )
    .await?;
    let sLocation = comment_link(&state, id).await?;
    Ok((StatusCode::FOUND, [(header::LOCATION, sLocation)]).into_response())
}

pub async fn add_comment_ajax(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    stRequest: Request,
) -> Result<Response> {
    let headers = stRequest.headers().clone();
    let vecParameters = vecCommentMessageRequestParameters(stRequest).await?;
    // CSRFHandlerInterceptor reads ServletRequest.getParameter, whose first
    // value is the query parameter before an URL-encoded POST body value.
    if !bCommentMessageCsrfValid(&vecParameters, &csrf_token) {
        return Err(AppError::Forbidden);
    }

    let bMessageMissing = optAjaxCommentParameter(&vecParameters, "msg").is_none();
    let sMessage = optAjaxCommentParameter(&vecParameters, "msg")
        .unwrap_or_default()
        .to_owned();
    let optPreview = optAjaxCommentParameter(&vecParameters, "preview").map(ToOwned::to_owned);
    let optNick = optAjaxCommentParameter(&vecParameters, "nick").map(ToOwned::to_owned);
    let optPassword = optAjaxCommentParameter(&vecParameters, "password").map(ToOwned::to_owned);
    let optCaptchaResponse = optAjaxCommentParameter(&vecParameters, "h-captcha-response");

    let mut vecErrors = Vec::new();
    let mut bCommentModel = false;
    let mut optBoundParameters = None;
    match stBindCommentMessageParameters(&vecParameters) {
        Ok(stParameters) => {
            bCommentModel = true;
            let stOriginalParameters = stParameters.clone();
            match CCommentMessageService::new(CCommentMessagePgRepository::new(state.pool.clone()))
                .stValidate(stParameters)
                .await
            {
                Ok(stValidated) => optBoundParameters = Some(stValidated),
                Err(EnCommentMessageServiceError::Binding(stError)) => {
                    if matches!(stError, EnCommentMessageBindingError::InvalidTopic) {
                        bCommentModel = false;
                    }
                    vecErrors.push(sAjaxCommentBindingError(&stError, &stOriginalParameters));
                    optBoundParameters = Some(stOriginalParameters);
                }
                Err(EnCommentMessageServiceError::Application(stError)) => return Err(stError),
            }
        }
        Err(stError) => {
            vecErrors.extend(vecAjaxCommentInitialBindingErrors(&stError, &vecParameters));
            // A conversion failure on reply/original/nick/msg leaves that
            // bean property unset, but Spring can still build the Comment
            // model from a valid topic and therefore returns preview HTML.
            if let Some(stFallback) = stAjaxCommentFallbackParameters(&vecParameters, &stError) {
                let stOriginalFallback = stFallback.clone();
                match CCommentMessageService::new(CCommentMessagePgRepository::new(
                    state.pool.clone(),
                ))
                .stValidate(stFallback)
                .await
                {
                    Ok(stValidated) => {
                        bCommentModel = true;
                        optBoundParameters = Some(stValidated);
                    }
                    Err(EnCommentMessageServiceError::Binding(stValidationError)) => {
                        if !matches!(
                            stValidationError,
                            EnCommentMessageBindingError::InvalidTopic
                        ) {
                            bCommentModel = true;
                            vecErrors.push(sAjaxCommentBindingError(
                                &stValidationError,
                                &stOriginalFallback,
                            ));
                            optBoundParameters = Some(stOriginalFallback);
                        }
                    }
                    Err(EnCommentMessageServiceError::Application(stError)) => {
                        return Err(stError);
                    }
                }
            }
        }
    }

    let optTopicId = optBoundParameters
        .as_ref()
        .map(|stParameters| stParameters.iTopicId);
    let optReplyToId = optBoundParameters
        .as_ref()
        .and_then(|stParameters| stParameters.optReplyToId);
    let form = CommentForm {
        topic: optTopicId.unwrap_or_default(),
        replyto: optReplyToId,
        title: None,
        msg: sMessage,
        nick: optNick,
        password: optPassword,
        preview: optPreview,
        captcha_response: optCaptchaResponse.map(ToOwned::to_owned),
        csrf: Some(csrf_token),
    };
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let bRequireCaptcha =
        user.is_none() || crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    let (_, _, markup) = match user.as_ref() {
        Some(stUser) => user_comment_format(&state, stUser.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    if form.preview.is_none()
        && vecErrors.is_empty()
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
    // AuthUtil.postingUser returns the existing session without inspecting
    // form credentials once model binding or CAPTCHA has already failed.
    let (optPostingNick, optPostingPassword) = if vecErrors.is_empty() {
        (form.nick.as_deref(), form.password.as_deref())
    } else {
        (None, None)
    };
    let stResolution = crate::application::auth::stResolvePostingIdentity(
        &state,
        user.as_ref(),
        optPostingNick,
        optPostingPassword,
    )
    .await?;
    if let Some(sError) = stResolution.optError {
        vecErrors.push(sError);
    }
    // CommentCreateService.checkPostData distinguishes an absent `msg`
    // property from an explicitly submitted empty string.
    if bMessageMissing {
        vecErrors.push("комментарий не задан".to_owned());
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
    if bCommentModel
        && let Some(iTopicId) = optTopicId
        && let Err(stError) = check_comment_posting_allowed(
            &state,
            &stIdentity.stUser,
            !stIdentity.bAuthorized,
            iTopicId,
        )
        .await
    {
        vecErrors.push(sCommentFormError(stError)?);
    }
    if let Some(sError) = optCommentBodyErrorWithPolicy(&form.msg, !stIdentity.bAuthorized, false) {
        vecErrors.push(sError);
    }
    if form.preview.is_some() || !vecErrors.is_empty() || !bCommentModel {
        let optPreviewHtml = if bCommentModel {
            let stMarkupUsers = state
                .markup
                .stResolveBatch([(&*form.msg, &*markup)])
                .await?;
            Some(markup::render_message_with_markup_policy_and_users(
                &form.msg,
                Some(&markup),
                None,
                false,
                Some(&state.config.public_url),
                Some(&stMarkupUsers),
            ))
        } else {
            None
        };
        return Ok(stAjaxCommentJson(serde_json::json!({
            "errors": vecErrors,
            "preview": optPreviewHtml,
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
        false,
    )
    .await?;
    let url = comment_link(&state, id).await?;
    Ok(stAjaxCommentJson(serde_json::json!({"url": url})))
}

fn optAjaxCommentParameter<'a>(
    vecParameters: &'a [(String, String)],
    sName: &str,
) -> Option<&'a str> {
    vecParameters
        .iter()
        .find_map(|(sKey, sValue)| (sKey == sName).then_some(sValue.as_str()))
}

fn stAjaxCommentFallbackParameters(
    vecParameters: &[(String, String)],
    stError: &EnCommentMessageBindingError,
) -> Option<crate::domain::comment::message_form::StCommentMessageParameters> {
    if matches!(
        stError,
        EnCommentMessageBindingError::MissingTopic | EnCommentMessageBindingError::InvalidTopic
    ) {
        return None;
    }
    let iTopicId = optAjaxCommentParameter(vecParameters, "topic")?
        .split(',')
        .next()
        .unwrap_or_default()
        .parse()
        .ok()?;
    Some(
        crate::domain::comment::message_form::StCommentMessageParameters {
            iTopicId,
            optReplyToId: None,
            optOriginalId: None,
            optNick: None,
            sMessage: optAjaxCommentParameter(vecParameters, "msg")
                .unwrap_or_default()
                .to_owned(),
        },
    )
}

fn vecAjaxCommentInitialBindingErrors(
    stError: &EnCommentMessageBindingError,
    vecParameters: &[(String, String)],
) -> Vec<String> {
    match stError {
        EnCommentMessageBindingError::MissingTopic => vec!["тема не задана".to_owned()],
        EnCommentMessageBindingError::InvalidTopic => vec![
            "Failed to convert model attribute 'topic'".to_owned(),
            "тема не задана".to_owned(),
        ],
        EnCommentMessageBindingError::InvalidReplyTo => {
            vec!["Failed to convert model attribute 'replyto'".to_owned()]
        }
        EnCommentMessageBindingError::InvalidOriginal => {
            vec!["Failed to convert model attribute 'original'".to_owned()]
        }
        EnCommentMessageBindingError::InvalidNick => {
            vec!["Failed to convert model attribute 'nick'".to_owned()]
        }
        EnCommentMessageBindingError::InvalidMessage => {
            let sMessage = optAjaxCommentParameter(vecParameters, "msg").unwrap_or_default();
            vec![
                optCommentBodyErrorWithPolicy(sMessage, false, false)
                    .unwrap_or_else(|| "Validation failed for model attribute 'msg'".to_owned()),
            ]
        }
        stOther => vec![stOther.to_string()],
    }
}

fn sAjaxCommentBindingError(
    stError: &EnCommentMessageBindingError,
    stParameters: &crate::domain::comment::message_form::StCommentMessageParameters,
) -> String {
    match stError {
        EnCommentMessageBindingError::InvalidTopic => {
            format!("Сообщение #{} не существует", stParameters.iTopicId)
        }
        EnCommentMessageBindingError::InvalidReplyTo => format!(
            "Сообщение #{} не существует",
            stParameters.optReplyToId.unwrap_or_default()
        ),
        EnCommentMessageBindingError::InvalidOriginal => format!(
            "Сообщение #{} не существует",
            stParameters.optOriginalId.unwrap_or_default()
        ),
        EnCommentMessageBindingError::InvalidNick => format!(
            "Пользователь \"{}\" не найден",
            stParameters.optNick.as_deref().unwrap_or_default()
        ),
        EnCommentMessageBindingError::TopicDeleted => {
            "нельзя добавлять в удаленные темы".to_owned()
        }
        EnCommentMessageBindingError::TopicExpired => {
            "нельзя добавлять в устаревшие темы".to_owned()
        }
        EnCommentMessageBindingError::ReplyDeleted => {
            "нельзя комментировать удаленные комментарии".to_owned()
        }
        EnCommentMessageBindingError::ReplyTopicMismatch => "некорректная тема".to_owned(),
        stOther => stOther.to_string(),
    }
}

fn stAjaxCommentJson(stValue: serde_json::Value) -> Response {
    let mut stResponse = axum::Json(stValue).into_response();
    stResponse.headers_mut().insert(
        header::CONTENT_TYPE,
        "application/json;charset=utf-8".parse().unwrap(),
    );
    stResponse
}

fn sCommentFormError(stError: AppError) -> Result<String> {
    match stError {
        AppError::BadRequest(sMessage) | AppError::TooManyRequests(sMessage) => Ok(sMessage),
        AppError::Forbidden => Ok("Это сообщение нельзя комментировать".into()),
        stOther => Err(stOther),
    }
}

fn optCommentBodyError(sMessage: &str, bAnonymous: bool) -> Option<String> {
    optCommentBodyErrorWithPolicy(sMessage, bAnonymous, true)
}

fn optCommentBodyErrorWithPolicy(
    sMessage: &str,
    bAnonymous: bool,
    bRejectEmpty: bool,
) -> Option<String> {
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
    } else if bRejectEmpty && sMessage.trim().is_empty() {
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

pub fn stCommentMessageRoute() -> MethodRouter<AppState> {
    get(comment_message)
        .post(comment_message)
        .put(comment_message)
        .patch(comment_message)
        .delete(comment_message)
        .options(options_comment_message)
        .fallback(method_not_allowed_comment_message)
}

pub fn stDeleteCommentRoute() -> MethodRouter<AppState> {
    get(delete_comment_form)
        .post(delete_comment)
        .options(options_comment_action)
        .fallback(method_not_allowed_comment_action)
}

pub fn stUndeleteCommentRoute() -> MethodRouter<AppState> {
    get(undelete_comment_form)
        .post(undelete_comment)
        .options(options_comment_undelete)
        .fallback(method_not_allowed_comment_undelete)
}

fn stEmptyMethodResponse(stStatus: StatusCode, optAllow: Option<&'static str>) -> Response {
    let mut stResponse = (stStatus, [(header::CONTENT_LENGTH, "0")]).into_response();
    if let Some(sAllow) = optAllow {
        stResponse.headers_mut().insert(
            header::ALLOW,
            sAllow.parse().expect("static Allow is valid"),
        );
    }
    stResponse
}

async fn options_comment_message() -> Response {
    stEmptyMethodResponse(StatusCode::OK, Some(S_ALLOW_COMMENT_MESSAGE))
}

async fn method_not_allowed_comment_message() -> Response {
    // TRACE is rejected by the original strict firewall before Spring's
    // handler mapping and therefore has no synthesized Allow header.
    stEmptyMethodResponse(StatusCode::METHOD_NOT_ALLOWED, None)
}

async fn options_comment_action() -> Response {
    stEmptyMethodResponse(StatusCode::OK, Some(S_ALLOW_COMMENT_ACTION_OPTIONS))
}

async fn method_not_allowed_comment_action() -> Response {
    // Spring's 405 resolver lists only the declared controller methods.  Its
    // automatic OPTIONS response has a deliberately different value/order.
    stEmptyMethodResponse(
        StatusCode::METHOD_NOT_ALLOWED,
        Some(S_ALLOW_COMMENT_ACTION_405),
    )
}

async fn options_comment_undelete() -> Response {
    stEmptyMethodResponse(StatusCode::OK, Some(S_ALLOW_COMMENT_UNDELETE_OPTIONS))
}

async fn method_not_allowed_comment_undelete() -> Response {
    stEmptyMethodResponse(
        StatusCode::METHOD_NOT_ALLOWED,
        Some(S_ALLOW_COMMENT_UNDELETE_405),
    )
}

fn vecDecodeCommentMessageParameters(sEncoded: &str) -> Result<Vec<(String, String)>> {
    serde_urlencoded::from_str(sEncoded)
        .map_err(|_| AppError::BadRequest("invalid request parameters".to_owned()))
}

fn vecCommentMessageQuery(stUri: &Uri) -> Result<Vec<(String, String)>> {
    stUri
        .query()
        .map_or_else(|| Ok(Vec::new()), vecDecodeCommentMessageParameters)
}

fn bUrlEncodedForm(oHeaders: &HeaderMap) -> bool {
    oHeaders
        .get(header::CONTENT_TYPE)
        .and_then(|stValue| stValue.to_str().ok())
        .and_then(|sValue| sValue.split(';').next())
        .is_some_and(|sMediaType| {
            sMediaType
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        })
}

async fn vecCommentMessageRequestParameters(stRequest: Request) -> Result<Vec<(String, String)>> {
    let (stParts, stBody) = stRequest.into_parts();
    let mut vecParameters = vecCommentMessageQuery(&stParts.uri)?;
    // Tomcat populates ServletRequest parameters from an URL-encoded body for
    // POST only.  Without a FormContentFilter, PUT/PATCH/DELETE bodies are not
    // model-bound.  Query parameters precede form fields, so first-value
    // binding also gives the query value precedence on duplicate names.
    if stParts.method == Method::POST && bUrlEncodedForm(&stParts.headers) {
        let vecBody = to_bytes(stBody, I_COMMENT_MESSAGE_PARAMETER_LIMIT)
            .await
            .map_err(|_| AppError::BadRequest("invalid request parameters".to_owned()))?;
        if !vecBody.is_empty() {
            let mut vecBodyParameters: Vec<(String, String)> =
                serde_urlencoded::from_bytes(&vecBody)
                    .map_err(|_| AppError::BadRequest("invalid request parameters".to_owned()))?;
            vecParameters.append(&mut vecBodyParameters);
        }
    }
    Ok(vecParameters)
}

fn bCommentMessageCsrfValid(vecParameters: &[(String, String)], sExpectedToken: &str) -> bool {
    vecParameters
        .iter()
        .find_map(|(sKey, sValue)| (sKey == crate::csrf::FIELD_NAME).then_some(sValue.as_str()))
        .is_some_and(|sSubmitted| {
            !sSubmitted.is_empty() && sSubmitted.trim() == sExpectedToken.trim()
        })
}

fn stCommentMessageServiceError(stError: EnCommentMessageServiceError) -> AppError {
    match stError {
        EnCommentMessageServiceError::Binding(stBinding) => {
            AppError::BadRequest(stBinding.to_string())
        }
        EnCommentMessageServiceError::Application(stError) => stError,
    }
}

fn optCommentMessageBindingResponse(
    stMethod: &Method,
    stError: &EnCommentMessageBindingError,
) -> Option<Response> {
    // With the unfiltered servlet request used by Java, the URL-encoded PUT
    // body is not model-bound.  A missing `topic` then terminates in the
    // container's empty 400 response instead of the HTML GET error page.
    (*stMethod == Method::PUT && matches!(stError, EnCommentMessageBindingError::MissingTopic))
        .then(|| stEmptyMethodResponse(StatusCode::BAD_REQUEST, None))
}

pub async fn comment_message(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    stRequest: Request,
) -> Result<Response> {
    let stMethod = stRequest.method().clone();
    let bPost = stMethod == Method::POST;
    let headers = stRequest.headers().clone();
    let vecParameters = vecCommentMessageRequestParameters(stRequest).await?;
    // CSRFHandlerInterceptor runs before Spring model binding and reads the
    // first merged ServletRequest value (query before POST form).  This path
    // is manual in the Axum middleware so we can preserve that exact order.
    if bPost && !bCommentMessageCsrfValid(&vecParameters, &csrf_token) {
        return Err(AppError::Forbidden);
    }
    let stParameters = match stBindCommentMessageParameters(&vecParameters) {
        Ok(stParameters) => stParameters,
        Err(stError) => {
            if let Some(stResponse) = optCommentMessageBindingResponse(&stMethod, &stError) {
                return Ok(stResponse);
            }
            return Err(AppError::BadRequest(stError.to_string()));
        }
    };
    let stParameters =
        CCommentMessageService::new(CCommentMessagePgRepository::new(state.pool.clone()))
            .stValidate(stParameters)
            .await
            .map_err(stCommentMessageServiceError)?;
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let stResolution =
        crate::application::auth::stResolvePostingIdentity(&state, user.as_ref(), None, None)
            .await?;
    if let Err(stError) = check_comment_posting_allowed(
        &state,
        &stResolution.stIdentity.stUser,
        !stResolution.stIdentity.bAuthorized,
        stParameters.iTopicId,
    )
    .await
    {
        // CommentRequestValidator has already handled deleted/expired as a
        // binding failure.  A draft reaches `isCommentsAllowed`, whose false
        // result is raised as AccessViolationException by showFormTopic.
        return Err(match stError {
            AppError::BadRequest(_) => AppError::Forbidden,
            stOther => stOther,
        });
    }
    let (format_mode, format_title, _) = match &user {
        Some(stUser) => user_comment_format(&state, stUser.id).await?,
        None => (
            crate::profile::DEFAULT_FORMAT_MODE.into(),
            "Markdown".into(),
            "MARKDOWN".into(),
        ),
    };
    let bRequireCaptcha =
        user.is_none() || crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    let stTopic = crate::routes::topics::get_topic(&state, stParameters.iTopicId).await?;
    let sTopicCardHtml = crate::routes::topics::sPrepareTopicCardHtml(
        &state,
        stParameters.iTopicId,
        &user,
        &csrf_token,
        false,
    )
    .await?;
    Ok(Html(
        StCommentMessageTemplate {
            topic_id: stTopic.id,
            topic_title: stTopic.sTitlePlain(),
            group_title: stTopic.group_title,
            section_title: stTopic.section_name,
            topic_card_html: sTopicCardHtml,
            csrf_token,
            format_mode,
            format_title,
            form_msg: stParameters.sMessage,
            anonymous_form: user.is_none(),
            // The anonymous session can reach showFormTopic only for the
            // unrestricted sentinel, whose Java postscore text is empty.
            postscore_info_html: String::new(),
            require_captcha: bRequireCaptcha,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
        }
        .render()?,
    )
    .into_response())
}

#[derive(Deserialize)]
pub struct EditCommentQuery {
    pub topic: Option<i32>,
    pub original: Option<i32>,
}

#[derive(Template)]
#[template(path = "edit_comment.html")]
struct EditCommentTemplate {
    comment_id: i32,
    topic_id: i32,
    topic_url: String,
    postdate: chrono::DateTime<chrono::Utc>,
    deadline: chrono::DateTime<chrono::Utc>,
    msg: String,
    format_mode: String,
    format_title: String,
    csrf_token: String,
    form_error: Option<String>,
    comment_html: String,
    require_captcha: bool,
    captcha_site_key: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct StEditableCommentRow {
    iTopicId: i32,
    iAuthorId: i32,
    sTitle: String,
    sMessage: String,
    sMarkup: String,
    bDeleted: bool,
    dtPostdate: chrono::DateTime<chrono::Utc>,
    bHasReplies: bool,
    bTopicDeleted: bool,
    bTopicExpired: bool,
}

async fn stEditableComment(state: &AppState, comment_id: i32) -> Result<StEditableCommentRow> {
    sqlx::query_as(
        r#"SELECT c.topic AS "iTopicId", c.userid AS "iAuthorId",
                  c.title AS "sTitle", m.message AS "sMessage",
                  m.markup::text AS "sMarkup", c.deleted AS "bDeleted",
                  c.postdate AS "dtPostdate",
                  EXISTS(SELECT 1 FROM comments r WHERE r.replyto=c.id AND NOT r.deleted)
                    AS "bHasReplies",
                  t.deleted AS "bTopicDeleted",
                  (NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire)
                    AS "bTopicExpired"
           FROM comments c
           JOIN msgbase m ON m.id=c.id
           JOIN topics t ON t.id=c.topic
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE c.id=$1"#,
    )
    .bind(comment_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)
}

fn optCommentEditEligibilityError(
    stRow: &StEditableCommentRow,
    stUser: &crate::models::UserSummary,
    dtNow: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    // TopicPermissionService records these conditions in BindingResult. GET
    // redirects when any error exists; POST re-renders the form with 200.
    if stRow.bDeleted || stRow.bTopicDeleted {
        return Some("Тема или комментарий удалены".to_owned());
    }
    if stRow.bTopicExpired {
        return Some("Сообщение уже устарело".to_owned());
    }
    if stUser.id != stRow.iAuthorId {
        return Some("У вас недостаточно прав для редактирования этого комментария".to_owned());
    }
    if dtNow > stRow.dtPostdate + chrono::Duration::minutes(COMMENT_EDIT_WINDOW_MINUTES) {
        return Some("Истек срок редактирования".to_owned());
    }
    if stRow.bHasReplies {
        return Some("Редактирование комментариев с ответами запрещено".to_owned());
    }
    if stUser.score.unwrap_or(0) < COMMENT_EDIT_MIN_SCORE {
        return Some("У вас недостаточно прав для редактирования этого комментария".to_owned());
    }
    if stRow.sMarkup == "PLAIN" && !stUser.candel {
        return Some("Вы не можете редактировать тексты данного формата".to_owned());
    }
    None
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
    // `EditCommentController` is wrapped in `AuthorizedOnly`; anonymous
    // requests raise AccessViolationException and render the original 403
    // page instead of being redirected to login.
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let comment_id = query
        .original
        .ok_or_else(|| AppError::BadParameter("Комментарий не задан".into()))?;
    let requested_topic_id = query
        .topic
        .ok_or_else(|| AppError::BadParameter("тема не задана".into()))?;
    let row = stEditableComment(&state, comment_id).await?;
    if requested_topic_id != row.iTopicId {
        return Err(AppError::BadRequest("тема не совпадает".into()));
    }
    let topic = crate::routes::topics::get_topic(&state, row.iTopicId).await?;
    if check_comment_posting_allowed(&state, &user, false, row.iTopicId)
        .await
        .is_err()
        || optCommentEditEligibilityError(&row, &user, chrono::Utc::now()).is_some()
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
    let (format_mode, format_title) = crate::routes::topics::markup_form_view(&row.sMarkup);
    let require_captcha = crate::routes::auth::bIpCaptchaRequired(&state, &sRemoteIp).await?;
    // GET uses prepareCommentOnly in Java (profile userpic, delete/edit
    // metadata, moderator IP/UA and reactions). POST preview below deliberately
    // uses the smaller prepareCommentForEdit projection instead.
    let (stTarget, stPreparedComment) = stCommentDeletionService(&state)
        .optPrepareCommentOnly(comment_id, user.id)
        .await?
        .ok_or(AppError::NotFound)?;
    if stTarget.iTopicId != topic.id {
        return Err(AppError::NotFound);
    }
    let sCommentHtml = sCommentDeletionPreviewHtml(
        &state,
        &user,
        true,
        topic.id,
        stTarget.bTopicExpired,
        stTarget.bTopicDraft,
        stTarget.bCommentsHidden,
        &topic.topic_url(),
        &csrf_token,
        false,
        false,
        std::slice::from_ref(&stPreparedComment),
    )
    .await?;
    Ok(Html(
        EditCommentTemplate {
            comment_id,
            topic_id: row.iTopicId,
            topic_url: topic.topic_url(),
            postdate: row.dtPostdate,
            deadline: row.dtPostdate + chrono::Duration::minutes(COMMENT_EDIT_WINDOW_MINUTES),
            msg: row.sMessage,
            format_mode,
            format_title,
            csrf_token,
            form_error: None,
            comment_html: sCommentHtml,
            require_captcha,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
        }
        .render()?,
    )
    .into_response())
}

#[derive(Deserialize)]
pub struct EditCommentForm {
    pub original: i32,
    pub topic: Option<i32>,
    pub msg: String,
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

fn sCommentTitleAfterEdit() -> &'static str {
    // CommentRequest has no title field and Comment.buildNew initializes the
    // edited value to an empty string in the Java application.
    ""
}

fn optCommentOldTitleForHistory(sOldTitle: &str) -> Option<&str> {
    (!sOldTitle.is_empty()).then_some(sOldTitle)
}

#[cfg(test)]
mod comment_edit_title_contract_tests {
    use super::*;

    fn stEditableFixture(dtPostdate: chrono::DateTime<chrono::Utc>) -> StEditableCommentRow {
        StEditableCommentRow {
            iTopicId: 100,
            iAuthorId: 42,
            sTitle: String::new(),
            sMessage: "old".to_owned(),
            sMarkup: "MARKDOWN".to_owned(),
            bDeleted: false,
            dtPostdate,
            bHasReplies: false,
            bTopicDeleted: false,
            bTopicExpired: false,
        }
    }

    fn stEditorFixture() -> crate::models::UserSummary {
        crate::models::UserSummary {
            id: 42,
            nick: "editor".to_owned(),
            name: None,
            score: Some(45),
            max_score: Some(45),
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

    #[test]
    fn edited_comment_title_is_always_empty() {
        assert_eq!(sCommentTitleAfterEdit(), "");
    }

    #[test]
    fn legacy_title_is_preserved_once_in_edit_history() {
        assert_eq!(
            optCommentOldTitleForHistory("legacy &amp; title"),
            Some("legacy &amp; title")
        );
        assert_eq!(optCommentOldTitleForHistory(""), None);
    }

    #[test]
    fn edit_form_does_not_submit_a_port_only_title() {
        let sTemplate = include_str!("../../templates/edit_comment.html");

        assert!(!sTemplate.contains("name=\"title\""));
    }

    #[test]
    fn edit_binding_accepts_only_the_original_comment_parameter_name() {
        let stOriginal: EditCommentForm =
            serde_urlencoded::from_str("original=77&topic=42&msg=body")
                .expect("Java edit-comment form");
        assert_eq!(stOriginal.original, 77);
        assert!(
            serde_urlencoded::from_str::<EditCommentForm>("msgid=77&topic=42&msg=body").is_err()
        );
    }

    #[test]
    fn edit_date_is_written_as_the_legacy_jdbc_wall_clock() {
        let sSource = include_str!("comments.rs");
        let sHandler = sSource
            .split(concat!("pub async fn ", "edit_comment("))
            .nth(1)
            .unwrap()
            .split(concat!("pub async fn ", "delete_comment_form("))
            .next()
            .unwrap();
        assert!(sHandler.contains("edit_date=(CURRENT_TIMESTAMP AT TIME ZONE $4::text)"));
        assert!(sHandler.contains("legacy_timestamp::sLegacyJdbcTimezone("));
        assert!(sHandler.contains("state.config.stLegacyJdbcTimezone()"));
        assert!(!sHandler.contains(concat!("edit_date=", "now()")));
    }

    #[test]
    fn anonymous_preview_author_link_depends_on_the_http_session() {
        let dtNow = chrono::Utc::now();
        let sPublic = sPreparedFormCommentHtml(
            0,
            "",
            "body",
            "anonymous",
            2,
            0,
            0,
            false,
            true,
            dtNow,
            1,
            false,
            false,
        );
        assert!(sPublic.contains(">anonymous "));
        assert!(!sPublic.contains("/people/anonymous/profile"));

        let sAuthorized = sPreparedFormCommentHtml(
            0,
            "",
            "body",
            "anonymous",
            2,
            0,
            0,
            false,
            true,
            dtNow,
            1,
            true,
            false,
        );
        assert!(sAuthorized.contains("/people/anonymous/profile"));
    }

    #[test]
    fn edit_eligibility_is_a_form_outcome_not_an_http_bad_request() {
        let dtNow = chrono::Utc::now();
        let mut stRow = stEditableFixture(dtNow - chrono::Duration::minutes(5));
        let stUser = stEditorFixture();
        assert_eq!(optCommentEditEligibilityError(&stRow, &stUser, dtNow), None);

        stRow.bHasReplies = true;
        assert_eq!(
            optCommentEditEligibilityError(&stRow, &stUser, dtNow).as_deref(),
            Some("Редактирование комментариев с ответами запрещено")
        );

        stRow.bHasReplies = false;
        stRow.bTopicDeleted = true;
        assert_eq!(
            optCommentEditEligibilityError(&stRow, &stUser, dtNow).as_deref(),
            Some("Тема или комментарий удалены")
        );
    }

    #[test]
    fn deadline_and_legacy_markup_match_java_default_policy() {
        let dtNow = chrono::Utc::now();
        let stUser = stEditorFixture();
        let stExpired =
            stEditableFixture(dtNow - chrono::Duration::minutes(COMMENT_EDIT_WINDOW_MINUTES + 1));
        assert_eq!(
            optCommentEditEligibilityError(&stExpired, &stUser, dtNow).as_deref(),
            Some("Истек срок редактирования")
        );

        let mut stLegacy = stEditableFixture(dtNow);
        stLegacy.sMarkup = "PLAIN".to_owned();
        assert_eq!(
            optCommentEditEligibilityError(&stLegacy, &stUser, dtNow).as_deref(),
            Some("Вы не можете редактировать тексты данного формата")
        );
    }
}

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
    let row = stEditableComment(&state, form.original).await?;
    let topic_id = row.iTopicId;
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
    if optError.is_none() {
        optError = optCommentEditEligibilityError(&row, &user, chrono::Utc::now());
    }
    let (format_mode, format_title) = crate::routes::topics::markup_form_view(&row.sMarkup);
    let topic = crate::routes::topics::get_topic(&state, topic_id).await?;
    if form.preview.is_some() || optError.is_some() {
        let (sPreviewMessage, sPreviewTitle) = if form.preview.is_some() {
            (form.msg.as_str(), sCommentTitleAfterEdit())
        } else {
            (row.sMessage.as_str(), row.sTitle.as_str())
        };
        let stMarkupUsers = state
            .markup
            .stResolveBatch([(sPreviewMessage, &*row.sMarkup)])
            .await?;
        let sMessageHtml = markup::render_message_with_markup_policy_and_users(
            sPreviewMessage,
            Some(&row.sMarkup),
            None,
            false,
            Some(&state.config.public_url),
            Some(&stMarkupUsers),
        );
        let sCommentHtml = sPreparedFormCommentHtml(
            form.original,
            sPreviewTitle,
            &sMessageHtml,
            &user.nick,
            user.id,
            user.score.unwrap_or(0),
            user.max_score.unwrap_or(0),
            user.blocked.unwrap_or(false),
            false,
            row.dtPostdate,
            topic.author_id,
            true,
            user.canmod,
        );
        return Ok(Html(
            EditCommentTemplate {
                comment_id: form.original,
                topic_id,
                topic_url: topic.topic_url(),
                postdate: row.dtPostdate,
                deadline: row.dtPostdate + chrono::Duration::minutes(COMMENT_EDIT_WINDOW_MINUTES),
                msg: form.msg.clone(),
                format_mode,
                format_title,
                csrf_token,
                form_error: optError,
                comment_html: sCommentHtml,
                require_captcha: bRequireCaptcha,
                captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
            }
            .render()?,
        )
        .into_response());
    }

    let sNewTitle = sCommentTitleAfterEdit();
    let optOldMessage = (row.sMessage != form.msg).then_some(row.sMessage.as_str());
    let optOldTitle = optCommentOldTitleForHistory(&row.sTitle);
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE msgbase SET message=$2 WHERE id=$1")
        .bind(form.original)
        .bind(&form.msg)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE comments SET title=$2 WHERE id=$1")
        .bind(form.original)
        .bind(sNewTitle)
        .execute(&mut *tx)
        .await?;
    if optOldMessage.is_some() || optOldTitle.is_some() {
        sqlx::query(
            r#"INSERT INTO edit_info(msgid,editor,oldmessage,oldtitle,object_type)
               VALUES($1,$2,$3,$4,'COMMENT'::edit_event_type)"#,
        )
        .bind(form.original)
        .bind(user.id)
        .bind(optOldMessage)
        .bind(optOldTitle)
        .execute(&mut *tx)
        .await?;
        let iEditCount: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM edit_info WHERE msgid=$1 AND object_type='COMMENT'::edit_event_type",
        )
        .bind(form.original)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE comments SET editor_id=$2,\
             edit_date=(CURRENT_TIMESTAMP AT TIME ZONE $4::text),\
             edit_count=$3 WHERE id=$1",
        )
        .bind(form.original)
        .bind(user.id)
        .bind(iEditCount.min(i64::from(i32::MAX)) as i32)
        .bind(
            crate::infra::postgres::legacy_timestamp::sLegacyJdbcTimezone(
                state.config.stLegacyJdbcTimezone(),
            ),
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE topics SET lastmod=now() WHERE id=$1")
            .bind(topic_id)
            .execute(&mut *tx)
            .await?;
    }
    // Current Java reads `oldUserRefs` after it has already updated msgbase,
    // so old/new mention sets are equal and edit-time REF events are never
    // added. Preserve that observable behavior rather than fixing the source
    // bug only in the port.
    tx.commit().await?;
    // EditCommentController sends the committed message to SearchQueueSender
    // and lets a queue failure reach the HTTP caller. Unlike comment create,
    // the original edit controller does not publish a realtime event here.
    CSearchQueueSender::new(
        state.config.opensearch_url.as_deref(),
        &state.config.upload_dir,
    )
    .vUpdateComments(&[form.original])
    .await?;
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, comment_link(&state, form.original).await?)],
    )
        .into_response())
}

pub async fn delete_comment_form(
    State(stState): State<AppState>,
    Query(q): Query<JumpQuery>,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Response> {
    let Some(stUser) = optUser else {
        return Err(AppError::Forbidden);
    };
    let stActor = stCommentDeleteActor(&stUser);
    let stData = match stCommentDeletionService(&stState)
        .stDeleteForm(stActor, q.msgid)
        .await
    {
        Ok(stData) => stData,
        Err(EnCommentDeletionError::Restricted(stRestriction)) => {
            return stCommentDeletionRestrictionResponse(stRestriction);
        }
        Err(EnCommentDeletionError::Application(stError)) => return Err(stError),
    };
    let sPreviewHtml = sCommentDeletionPreviewHtml(
        &stState,
        &stUser,
        true,
        stData.stTarget.iTopicId,
        stData.stTarget.bTopicExpired,
        stData.stTarget.bTopicDraft,
        stData.stTarget.bCommentsHidden,
        &stData.stTarget.sCanonicalTopicUrl,
        &csrf_token,
        true,
        true,
        &stData.vecPreview,
    )
    .await?;
    Ok(Html(
        StDeleteCommentTemplate {
            csrf_token,
            comment_id: q.msgid,
            moderator: stUser.canmod,
            show_bonus: stUser.canmod && !stData.stTarget.bTopicExpired,
            author_score: stData.stTarget.iAuthorScore,
            delete_reasons: VEC_DELETE_REASONS,
            preview_html: sPreviewHtml,
        }
        .render()?,
    )
    .into_response())
}

pub async fn undelete_comment_form(
    State(stState): State<AppState>,
    Query(q): Query<JumpQuery>,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Response> {
    let Some(stUser) = optUser else {
        return Err(AppError::Forbidden);
    };
    let stData = match stCommentDeletionService(&stState)
        .stUndeleteForm(stCommentDeleteActor(&stUser), q.msgid)
        .await
    {
        Ok(stData) => stData,
        Err(EnCommentDeletionError::Restricted(stRestriction)) => {
            return stCommentDeletionRestrictionResponse(stRestriction);
        }
        Err(EnCommentDeletionError::Application(stError)) => return Err(stError),
    };
    let sPreviewHtml = sCommentDeletionPreviewHtml(
        &stState,
        &stUser,
        true,
        stData.stTarget.iTopicId,
        stData.stTarget.bTopicExpired,
        stData.stTarget.bTopicDraft,
        stData.stTarget.bCommentsHidden,
        &stData.stTarget.sCanonicalTopicUrl,
        &csrf_token,
        false,
        false,
        &stData.vecPreview,
    )
    .await?;
    Ok(Html(
        StUndeleteCommentTemplate {
            csrf_token,
            comment_id: q.msgid,
            preview_html: sPreviewHtml,
        }
        .render()?,
    )
    .into_response())
}

#[derive(Deserialize)]
pub struct CommentAction {
    pub msgid: Option<String>,
    pub reason: Option<String>,
    pub bonus: Option<String>,
    pub delete_replys: Option<String>,
}

#[derive(Template)]
#[template(path = "delete_comment.html")]
struct StDeleteCommentTemplate {
    csrf_token: String,
    comment_id: i32,
    moderator: bool,
    show_bonus: bool,
    author_score: i32,
    delete_reasons: &'static [&'static str],
    preview_html: String,
}

#[derive(Template)]
#[template(path = "undelete_comment.html")]
struct StUndeleteCommentTemplate {
    csrf_token: String,
    comment_id: i32,
    preview_html: String,
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

#[derive(Template)]
#[template(path = "comment_deletion_error.html")]
struct StCommentDeletionErrorTemplate<'a> {
    message: &'a str,
}

fn stCommentDeletionRestrictionResponse(
    stRestriction: EnCommentDeletionRestriction,
) -> Result<Response> {
    Ok((
        StatusCode::FORBIDDEN,
        Html(
            StCommentDeletionErrorTemplate {
                message: stRestriction.sMessage(),
            }
            .render()?,
        ),
    )
        .into_response())
}

fn stCommentDeleteActor(stUser: &crate::models::UserSummary) -> StCommentDeleteActor {
    StCommentDeleteActor {
        iUserId: stUser.id,
        bModerator: stUser.canmod,
    }
}

fn stCommentDeletionService(
    stState: &AppState,
) -> CCommentDeletionService<CCommentDeletionPgRepository, CSearchQueueSender> {
    CCommentDeletionService::new(
        CCommentDeletionPgRepository::new(
            stState.pool.clone(),
            stState.config.stLegacyJdbcTimezone(),
        ),
        CSearchQueueSender::new(
            stState.config.opensearch_url.as_deref(),
            &stState.config.upload_dir,
        ),
    )
}

fn iSpringRequiredCommentInt(optValue: Option<&str>, sName: &str) -> Result<i32> {
    optValue
        .ok_or_else(|| AppError::BadRequest(format!("Required parameter '{sName}' is missing")))?
        .parse::<i32>()
        .map_err(|_| AppError::BadRequest(format!("Failed to convert parameter '{sName}'")))
}

fn iSpringDefaultCommentInt(optValue: Option<&str>, sName: &str, iDefault: i32) -> Result<i32> {
    match optValue.map(str::trim) {
        None | Some("") => Ok(iDefault),
        Some(sValue) => sValue
            .parse::<i32>()
            .map_err(|_| AppError::BadRequest(format!("Failed to convert parameter '{sName}'"))),
    }
}

fn bSpringDefaultCommentBoolean(optValue: Option<&str>, sName: &str) -> Result<bool> {
    let Some(sValue) = optValue.map(str::trim).filter(|sValue| !sValue.is_empty()) else {
        return Ok(false);
    };
    match sValue.to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(AppError::BadRequest(format!(
            "Failed to convert parameter '{sName}'"
        ))),
    }
}

fn sCommentStarsHtml(iScore: i32, iMaxScore: i32, bRegistered: bool) -> String {
    if !bRegistered {
        return String::new();
    }
    let iNormalizedScore = iScore.clamp(0, 599);
    let iNormalizedMaxScore = iMaxScore.max(iScore).clamp(0, 599);
    format!(
        "<span class=\"stars\">{}{}</span>",
        "★".repeat((iNormalizedScore / 100) as usize),
        "☆".repeat((iNormalizedMaxScore / 100 - iNormalizedScore / 100) as usize)
    )
}

struct StCommentPreviewReactions {
    sHtml: String,
    bShowMenuLink: bool,
}

fn stCommentPreviewReactions(
    iTopicId: i32,
    iCommentId: i32,
    vecReactionUsers: &[(String, i32, String, i32)],
    iViewerId: i32,
    bAllowInteract: bool,
    sCsrfToken: &str,
) -> StCommentPreviewReactions {
    struct StButton {
        sEmoji: String,
        iCount: usize,
        bClicked: bool,
        sTooltip: String,
    }
    let mut vecButtons = Vec::new();
    for (sEmoji, sDescription) in crate::routes::api::REACTIONS {
        let mut vecUsers = vecReactionUsers
            .iter()
            .filter(|(sReaction, ..)| sReaction == sEmoji)
            .collect::<Vec<_>>();
        vecUsers.sort_by_key(|stUser| std::cmp::Reverse(stUser.3));
        let vecTop = vecUsers
            .iter()
            .take(3)
            .map(|(_, _, sNick, _)| sNick.as_str())
            .collect::<Vec<_>>();
        vecButtons.push(StButton {
            sEmoji: (*sEmoji).to_owned(),
            iCount: vecUsers.len(),
            bClicked: vecUsers
                .iter()
                .any(|(_, iUserId, ..)| *iUserId == iViewerId),
            sTooltip: format!(
                "Реакция \"{sDescription}\": {}{}",
                vecTop.join(" "),
                if vecUsers.len() > 3 { "..." } else { "" }
            ),
        });
    }
    vecButtons.sort_by_key(|stButton| stButton.sEmoji.encode_utf16().collect::<Vec<_>>());
    let bHasReactions = vecButtons.iter().any(|stButton| stButton.iCount > 0);
    let sOuterClass = if bHasReactions {
        "reactions"
    } else {
        "reactions zero-reactions"
    };
    let sDisabled = if bAllowInteract { "" } else { " disabled" };
    let mut sHtml = format!(
        "<div class=\"{sOuterClass}\"><form class=\"reactions-form\" action=\"/reactions\" method=\"post\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"topic\" value=\"{iTopicId}\"><input type=\"hidden\" name=\"comment\" value=\"{iCommentId}\">",
        html_escape::encode_double_quoted_attribute(sCsrfToken),
    );
    for stButton in vecButtons.iter().filter(|stButton| stButton.iCount > 0) {
        let sValue = format!("{}-{}", stButton.sEmoji, !stButton.bClicked);
        let sClickedClass = if stButton.bClicked {
            " btn-primary"
        } else {
            ""
        };
        sHtml.push_str(&format!(
            "<button name=\"reaction\" value=\"{}\" class=\"reaction{sClickedClass}\" title=\"{}\"{sDisabled}>{} <span class=\"reaction-count\">{}</span></button>",
            html_escape::encode_double_quoted_attribute(&sValue),
            html_escape::encode_double_quoted_attribute(&stButton.sTooltip),
            html_escape::encode_text(&stButton.sEmoji),
            stButton.iCount,
        ));
    }
    if bHasReactions {
        sHtml.push_str(&format!(
            "<a class=\"reaction reaction-show-list\" href=\"/reactions?topic={iTopicId}&amp;comment={iCommentId}\">?</a>"
        ));
    }
    if bAllowInteract && vecButtons.iter().any(|stButton| stButton.iCount == 0) {
        if bHasReactions {
            sHtml.push_str(&format!(
                "<a class=\"reaction reaction-show\" href=\"/reactions?topic={iTopicId}&amp;comment={iCommentId}\">»</a><span class=\"zero-reactions\">"
            ));
        }
        for stButton in vecButtons.iter().filter(|stButton| stButton.iCount == 0) {
            sHtml.push_str(&format!(
                "<button name=\"reaction\" value=\"{}-true\" class=\"reaction\" title=\"{}\">{} <span class=\"reaction-count\">0</span></button>",
                html_escape::encode_double_quoted_attribute(&stButton.sEmoji),
                html_escape::encode_double_quoted_attribute(&stButton.sTooltip),
                html_escape::encode_text(&stButton.sEmoji),
            ));
        }
        if bHasReactions {
            sHtml.push_str("</span>");
        }
    }
    sHtml.push_str("</form></div>");
    StCommentPreviewReactions {
        sHtml,
        bShowMenuLink: !bHasReactions && bAllowInteract,
    }
}

pub(crate) fn sCommentPreviewWarnings(sWarningsJson: &str, sCsrfToken: &str) -> String {
    let Ok(vecWarnings) = serde_json::from_str::<Vec<serde_json::Value>>(sWarningsJson) else {
        return String::new();
    };
    if vecWarnings.is_empty() {
        return String::new();
    }
    let mut sHtml = "<div class=\"infoblock\">".to_owned();
    for stWarning in vecWarnings {
        let iId = stWarning["id"].as_i64().unwrap_or_default();
        let sPostdate = stWarning["postdate"].as_str().unwrap_or_default();
        let sAuthor = stWarning["author"].as_str().unwrap_or_default();
        let sMessage = stWarning["message"].as_str().unwrap_or_default();
        let sWarningType = stWarning["warning_type"].as_str().unwrap_or_default();
        let sPreparedMessage = if sWarningType.is_empty() {
            sMessage.to_owned()
        } else {
            let sWarningTypeName =
                crate::domain::warning::model::EnWarningType::optFromId(sWarningType)
                    .map(|enType| enType.sName())
                    .unwrap_or(sWarningType);
            format!("[{sWarningTypeName}] {sMessage}")
        };
        let bAuthorBlocked = stWarning["author_blocked"].as_bool().unwrap_or(false);
        let optClosedBy = stWarning["closed_by"].as_str();
        let bClosedByBlocked = stWarning["closed_by_blocked"].as_bool().unwrap_or(false);
        let sAuthorHtml = format!(
            "{}<a href=\"/people/{}/profile\">{}</a>{}",
            if bAuthorBlocked { "<s>" } else { "" },
            urlencoding::encode(sAuthor),
            html_escape::encode_text(sAuthor),
            if bAuthorBlocked { "</s>" } else { "" },
        );
        let sWarningBody = format!(
            "<time data-format=\"default\" datetime=\"{}\">{}</time> {}: {}",
            html_escape::encode_double_quoted_attribute(sPostdate),
            html_escape::encode_text(sPostdate),
            sAuthorHtml,
            html_escape::encode_text(&sPreparedMessage),
        );
        sHtml.push_str("<div style=\"margin-bottom: 0.5em\">⚠️ ");
        if let Some(sClosedBy) = optClosedBy {
            let sClosedByHtml = format!(
                "{}<a href=\"/people/{}/profile\">{}</a>{}",
                if bClosedByBlocked { "<s>" } else { "" },
                urlencoding::encode(sClosedBy),
                html_escape::encode_text(sClosedBy),
                if bClosedByBlocked { "</s>" } else { "" },
            );
            sHtml.push_str(&format!("<s>{sWarningBody}</s> (закрыт {sClosedByHtml})",));
        } else {
            sHtml.push_str(&sWarningBody);
            sHtml.push_str(&format!(
                "&nbsp;<form class=\"clear-warning-form\" action=\"clear-warning\" method=\"POST\" style=\"display: inline-block\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"id\" value=\"{iId}\"><button type=\"submit\" class=\"btn btn-small btn-default\">закрыть</button></form>",
                html_escape::encode_double_quoted_attribute(sCsrfToken),
            ));
        }
        sHtml.push_str("</div>");
    }
    sHtml.push_str("</div>");
    sHtml
}

fn stCommentPreviewUserpic(
    pathUploadRoot: &std::path::Path,
    sAvatarMode: &str,
    bAuthorAnonymous: bool,
    optPhoto: Option<&str>,
    optEmail: Option<&str>,
) -> (String, i32, i32) {
    let stUserpic = crate::profile::stResolveUserpic(
        pathUploadRoot,
        sAvatarMode,
        false,
        bAuthorAnonymous,
        optPhoto,
        optEmail,
    );
    (stUserpic.sUrl, stUserpic.iWidth, stUserpic.iHeight)
}

struct StPreparedCommentDom<'a> {
    iCommentId: i32,
    sHeader: &'a str,
    sUserpic: &'a str,
    sBodyClass: &'a str,
    sTitle: &'a str,
    sMessage: &'a str,
    sAuthorOpen: &'a str,
    sAuthorLinkOpen: &'a str,
    sAuthorNick: &'a str,
    sAuthorLinkClose: &'a str,
    sAuthorClose: &'a str,
    sStars: &'a str,
    sScore: &'a str,
    sPostdateRfc3339: &'a str,
    sPostdate: &'a str,
    sTopicAuthor: &'a str,
    sRemark: &'a str,
    sModeratorIp: &'a str,
    sEditSummary: &'a str,
    sModeratorUserAgent: &'a str,
    sMenu: &'a str,
    sWarnings: &'a str,
    sReactions: &'a str,
}

impl StPreparedCommentDom<'_> {
    /// The element order is the one emitted by `WEB-INF/tags/comment.tag`:
    /// title, container/userpic, body/text, sign, menu, warnings, reactions.
    fn sHtml(&self) -> String {
        format!(
            "<article class=\"msg\" id=\"comment-{}\">{}<div class=\"msg-container\">{}<div class=\"msg_body{}\"><div class=\"msg-text\">{}{}</div><div class=\"sign\">{}{}{}{}{} {}{}<br class=\"visible-phone\"> <span class=\"hideon-phone\">(</span><time data-format=\"default\" datetime=\"{}\">{}</time><span class=\"hideon-phone\">)</span>{}{}{}{}{}</div>{}{}{}</div></div></article>",
            self.iCommentId,
            self.sHeader,
            self.sUserpic,
            self.sBodyClass,
            self.sTitle,
            self.sMessage,
            self.sAuthorOpen,
            self.sAuthorLinkOpen,
            self.sAuthorNick,
            self.sAuthorLinkClose,
            self.sAuthorClose,
            self.sStars,
            self.sScore,
            self.sPostdateRfc3339,
            self.sPostdate,
            self.sTopicAuthor,
            self.sRemark,
            self.sModeratorIp,
            self.sEditSummary,
            self.sModeratorUserAgent,
            self.sMenu,
            self.sWarnings,
            self.sReactions,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn sPreparedFormCommentHtml(
    iCommentId: i32,
    sStoredTitle: &str,
    sMessageHtml: &str,
    sAuthor: &str,
    iAuthorId: i32,
    iAuthorScore: i32,
    iAuthorMaxScore: i32,
    bAuthorBlocked: bool,
    bAuthorAnonymous: bool,
    dtPostdate: chrono::DateTime<chrono::Utc>,
    iTopicAuthorId: i32,
    bSessionAuthorized: bool,
    bViewerModerator: bool,
) -> String {
    let sTitle = crate::domain::title::optCommentTitlePlainForDisplay(sStoredTitle)
        .map(|sTitle| format!("<h1>{}</h1>", html_escape::encode_text(&sTitle)))
        .unwrap_or_default();
    let sAuthorUrl = urlencoding::encode(sAuthor).into_owned();
    let sAuthorNick = html_escape::encode_text(sAuthor).into_owned();
    let bLinkAuthor = !bAuthorAnonymous || bSessionAuthorized;
    let sAuthorLinkOpen = if bLinkAuthor {
        format!("<a itemprop=\"creator\" href=\"/people/{sAuthorUrl}/profile\">")
    } else {
        String::new()
    };
    let sStars = sCommentStarsHtml(iAuthorScore, iAuthorMaxScore, !bAuthorAnonymous);
    let sScore = if bViewerModerator && !bAuthorAnonymous {
        format!(" (Score:&nbsp;{iAuthorScore} MaxScore:&nbsp;{iAuthorMaxScore})")
    } else {
        String::new()
    };
    let sTopicAuthor = if iAuthorId == iTopicAuthorId && !bAuthorAnonymous {
        " <span class=\"user-tag\">автор топика</span>"
    } else {
        ""
    };
    let sPostdateRfc3339 = dtPostdate.to_rfc3339();
    let sPostdate = dtPostdate.to_string();
    StPreparedCommentDom {
        iCommentId,
        sHeader: "",
        sUserpic: "",
        sBodyClass: "",
        sTitle: &sTitle,
        sMessage: sMessageHtml,
        sAuthorOpen: if bAuthorBlocked { "<s>" } else { "" },
        sAuthorLinkOpen: &sAuthorLinkOpen,
        sAuthorNick: &sAuthorNick,
        sAuthorLinkClose: if bLinkAuthor { "</a>" } else { "" },
        sAuthorClose: if bAuthorBlocked { "</s>" } else { "" },
        sStars: &sStars,
        sScore: &sScore,
        sPostdateRfc3339: &sPostdateRfc3339,
        sPostdate: &sPostdate,
        sTopicAuthor,
        sRemark: "",
        sModeratorIp: "",
        sEditSummary: "",
        sModeratorUserAgent: "",
        sMenu: "",
        sWarnings: "",
        sReactions: "",
    }
    .sHtml()
}

async fn sCommentDeletionPreviewHtml(
    stState: &AppState,
    stViewer: &crate::models::UserSummary,
    bViewerAuthorized: bool,
    iTopicId: i32,
    bTopicExpired: bool,
    bTopicDraft: bool,
    bCommentsHidden: bool,
    sTopicUrl: &str,
    sCsrfToken: &str,
    bShowMenu: bool,
    bFullThreadContext: bool,
    vecPreview: &[StCommentDeletePreview],
) -> Result<String> {
    let optSettings: Option<String> =
        sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
            .bind(stViewer.id)
            .fetch_optional(&stState.pool)
            .await?;
    let stViewerProfile = crate::profile::ProfileSettings::from_hstore_text(optSettings);
    let stMarkupUsers = stState
        .markup
        .stResolveBatch(
            vecPreview
                .iter()
                .map(|stComment| (&*stComment.sMessage, &*stComment.sMarkup)),
        )
        .await?;
    let bViewerFrozen: bool = sqlx::query_scalar(
        "SELECT COALESCE(frozen_until>CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
    )
    .bind(stViewer.id)
    .fetch_one(&stState.pool)
    .await?;
    let setIgnoredUsers =
        sqlx::query_scalar::<_, i32>("SELECT ignored FROM ignore_list WHERE userid=$1")
            .bind(stViewer.id)
            .fetch_all(&stState.pool)
            .await?
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
    let mut setReactionAuthorIds = std::collections::HashSet::new();
    let mut mapRawReactions = std::collections::HashMap::new();
    for stComment in vecPreview {
        let mapReactions = serde_json::from_str::<std::collections::HashMap<String, String>>(
            &stComment.sReactionsJson,
        )
        .unwrap_or_default();
        for sUserId in mapReactions.keys() {
            if let Ok(iUserId) = sUserId.parse::<i32>() {
                setReactionAuthorIds.insert(iUserId);
            }
        }
        mapRawReactions.insert(stComment.iCommentId, mapReactions);
    }
    let vecReactionAuthorIds = setReactionAuthorIds.into_iter().collect::<Vec<_>>();
    let mapReactionAuthors = if vecReactionAuthorIds.is_empty() {
        std::collections::HashMap::new()
    } else {
        sqlx::query_as::<_, (i32, String, i32)>(
            "SELECT id,nick,COALESCE(score,0) FROM users WHERE id=ANY($1)",
        )
        .bind(&vecReactionAuthorIds)
        .fetch_all(&stState.pool)
        .await?
        .into_iter()
        .map(|(iUserId, sNick, iScore)| (iUserId, (sNick, iScore)))
        .collect::<std::collections::HashMap<_, _>>()
    };
    let setPreviewIds = vecPreview
        .iter()
        .map(|stComment| stComment.iCommentId)
        .collect::<std::collections::HashSet<_>>();
    let mut mapAnswerIds: std::collections::HashMap<i32, Vec<i32>> =
        std::collections::HashMap::new();
    for stComment in vecPreview {
        if let Some(iReplyTo) = stComment.optReplyTo {
            mapAnswerIds
                .entry(iReplyTo)
                .or_default()
                .push(stComment.iCommentId);
        }
    }
    let mut sHtml = String::new();
    for stComment in vecPreview {
        let bNofollow = !crate::domain::topic::link_policy::StAuthorLinkState {
            iScore: stComment.iAuthorScore,
            bBlocked: stComment.bAuthorBlocked,
            bAnonymous: stComment.bAuthorAnonymous,
            bFrozen: stComment.bAuthorFrozen,
        }
        .bFollowAuthorLinks();
        let sMessageHtml = markup::render_message_with_markup_policy_and_users(
            &stComment.sMessage,
            Some(&stComment.sMarkup),
            None,
            bNofollow,
            Some(&stState.config.public_url),
            Some(&stMarkupUsers),
        );
        let sTitleHtml = crate::domain::title::optCommentTitlePlainForDisplay(&stComment.sTitle)
            .map(|sTitle| format!("<h1>{}</h1>", html_escape::encode_text(&sTitle)))
            .unwrap_or_default();
        let optUserpic = stViewerProfile.photos.then(|| {
            stCommentPreviewUserpic(
                std::path::Path::new(&stState.config.upload_dir),
                &stViewerProfile.avatar,
                stComment.bAuthorAnonymous,
                stComment.optPhoto.as_deref(),
                stComment.optEmail.as_deref(),
            )
        });
        let (sUserpicHtml, sBodyClass) = optUserpic
            .map(|(sUrl, iWidth, iHeight)| {
                (
                    format!(
                        "<div class=\"userpic\"><img class=\"photo\" src=\"{}\" alt=\"\" width={iWidth} height={iHeight} ></div>",
                        html_escape::encode_double_quoted_attribute(&sUrl),
                    ),
                    " message-w-userpic",
                )
            })
            .unwrap_or_default();
        let sStars = sCommentStarsHtml(
            stComment.iAuthorScore,
            stComment.iAuthorMaxScore,
            !stComment.bAuthorAnonymous,
        );
        let sScore = if stViewer.canmod && !stComment.bAuthorAnonymous {
            format!(
                " (Score:&nbsp;{} MaxScore:&nbsp;{})",
                stComment.iAuthorScore, stComment.iAuthorMaxScore
            )
        } else {
            String::new()
        };
        let sDeletedHeader = if stComment.bDeleted {
            let sDeleteInfo = match (
                stComment.optDeletedByNick.as_deref(),
                stComment.optDeleteReason.as_deref(),
            ) {
                (Some(sDeletedBy), Some(sReason)) => format!(
                    " {} по причине: {}",
                    html_escape::encode_text(sDeletedBy),
                    html_escape::encode_text(sReason),
                ),
                _ => String::new(),
            };
            let bUndeletable = stViewer.canmod
                && !bTopicExpired
                && stComment.optDeletedById != Some(stComment.iAuthorId);
            let sUndeleteLink = if bUndeletable {
                format!(
                    "&emsp;[<a href=\"/undelete_comment?msgid={}\">Восстановить</a>]",
                    stComment.iCommentId,
                )
            } else {
                String::new()
            };
            format!("<strong>Сообщение удалено{sDeleteInfo}</strong>{sUndeleteLink}<br>")
        } else {
            String::new()
        };
        let sReplyHeader = if bFullThreadContext {
            if let Some(iReplyTo) = stComment.optReplyTo {
                if stComment.bReplyDeleted {
                    "Ответ на: удаленный комментарий".to_owned()
                } else {
                    let sReplyTitle = stComment
                        .optReplyTitle
                        .as_deref()
                        .and_then(crate::domain::title::optCommentTitlePlainForDisplay)
                        .unwrap_or_else(|| "комментарий".to_owned());
                    let sReplyAuthor = stComment.optReplyAuthor.as_deref().unwrap_or_default();
                    let optReplyPostdate = stComment.optReplyPostdate;
                    format!(
                        "Ответ на: <a href=\"{}?cid={iReplyTo}\" data-samepage=\"{}\">{}</a> от {}{}",
                        html_escape::encode_double_quoted_attribute(sTopicUrl),
                        setPreviewIds.contains(&iReplyTo),
                        html_escape::encode_text(&sReplyTitle),
                        html_escape::encode_text(sReplyAuthor),
                        optReplyPostdate
                            .map(|dtPostdate| format!(
                                " <time data-format=\"default\" datetime=\"{}\">{}</time>",
                                dtPostdate.to_rfc3339(),
                                dtPostdate
                            ))
                            .unwrap_or_default(),
                    )
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        let sHeader = if sDeletedHeader.is_empty() && sReplyHeader.is_empty() {
            String::new()
        } else {
            format!("<div class=\"title\">{sDeletedHeader}{sReplyHeader}</div>")
        };
        let sAuthorOpen = if stComment.bAuthorBlocked { "<s>" } else { "" };
        let sAuthorClose = if stComment.bAuthorBlocked { "</s>" } else { "" };
        let sTopicAuthor =
            if stComment.iAuthorId == stComment.iTopicAuthorId && !stComment.bAuthorAnonymous {
                " <span class=\"user-tag\">автор топика</span>"
            } else {
                ""
            };
        let sRemark = if bFullThreadContext {
            stComment
                .optRemark
                .as_deref()
                .map(|sRemark| format!(" <span>{}</span>", html_escape::encode_text(sRemark)))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let (sModeratorIp, sModeratorUserAgent) = if stViewer.canmod {
            let sIp = if stComment.sPostIp.is_empty() {
                String::new()
            } else {
                format!(
                    " <a href=\"sameip.jsp?ip={}\">{}</a>",
                    urlencoding::encode(&stComment.sPostIp),
                    html_escape::encode_text(&stComment.sPostIp),
                )
            };
            let sUserAgent = stComment
                .optUserAgent
                .as_deref()
                .map(|sUserAgent| {
                    format!(
                        "<br><span class=\"sign_more\">{}&nbsp;<a href=\"sameip.jsp?ua={}&amp;ip={}&amp;mask=0\">🔍</a></span>",
                        html_escape::encode_text(sUserAgent),
                        stComment.iUserAgentId,
                        urlencoding::encode(&stComment.sPostIp),
                    )
                })
                .unwrap_or_default();
            (sIp, sUserAgent)
        } else {
            (String::new(), String::new())
        };
        let sEditSummary = if stComment.iEditCount > 0 {
            match (&stComment.optEditorNick, stComment.optEditDate) {
                (Some(sEditorNick), Some(dtEditDate)) => format!(
                    "<span class=\"sign_more\"><br>Последнее исправление: {} <time data-format=\"default\" datetime=\"{}\">{}</time> (всего <a href=\"{}/{}/history\">исправлений: {}</a>)</span>",
                    html_escape::encode_text(sEditorNick),
                    dtEditDate.to_rfc3339(),
                    dtEditDate,
                    html_escape::encode_double_quoted_attribute(sTopicUrl),
                    stComment.iCommentId,
                    stComment.iEditCount,
                ),
                _ => String::new(),
            }
        } else {
            String::new()
        };
        let vecReactionUsers = mapRawReactions
            .get(&stComment.iCommentId)
            .into_iter()
            .flat_map(|mapReactions| mapReactions.iter())
            .filter_map(|(sUserId, sEmoji)| {
                let iUserId = sUserId.parse::<i32>().ok()?;
                if setIgnoredUsers.contains(&iUserId) {
                    return None;
                }
                let (sNick, iScore) = mapReactionAuthors.get(&iUserId)?;
                Some((sEmoji.clone(), iUserId, sNick.clone(), *iScore))
            })
            .collect::<Vec<_>>();
        let bReactionAllowed = bViewerAuthorized
            && stViewer.id != stComment.iAuthorId
            && !bViewerFrozen
            && !bTopicExpired
            && !bCommentsHidden
            && !stComment.bDeleted;
        let stReactions = stCommentPreviewReactions(
            iTopicId,
            stComment.iCommentId,
            &vecReactionUsers,
            stViewer.id,
            bReactionAllowed,
            sCsrfToken,
        );
        let vecAnswers = mapAnswerIds
            .get(&stComment.iCommentId)
            .cloned()
            .unwrap_or_default();
        let sMenuHtml = if bShowMenu && !stComment.bDeleted {
            let mut vecItems = Vec::new();
            if stReactions.bShowMenuLink {
                vecItems.push(format!(
                    "<li><a class=\"reaction-show\" href=\"/reactions?topic={iTopicId}&amp;comment={}\">Реакции</a></li>",
                    stComment.iCommentId
                ));
            }
            let bCanEdit = stViewer.id == stComment.iAuthorId
                && stViewer.score.unwrap_or(0) >= 45
                && (!matches!(stComment.sMarkup.as_str(), "PLAIN") || stViewer.candel)
                && !bTopicExpired
                && vecAnswers.is_empty()
                && chrono::Utc::now() <= stComment.dtPostdate + chrono::Duration::minutes(30);
            if bCanEdit {
                vecItems.push(format!(
                    "<li><a href=\"/edit_comment?original={}&amp;topic={iTopicId}\">Править</a></li>",
                    stComment.iCommentId
                ));
            }
            vecItems.push(format!(
                "<li><a href=\"/delete_comment.jsp?msgid={}\">Удалить</a></li>",
                stComment.iCommentId
            ));
            if vecAnswers.len() > 1 {
                vecItems.push(format!(
                    "<li><a href=\"{}/thread/{}#comments\">Показать ответы</a></li>",
                    html_escape::encode_double_quoted_attribute(sTopicUrl),
                    stComment.iCommentId,
                ));
            } else if let Some(iAnswerId) = vecAnswers.first() {
                vecItems.push(format!(
                    "<li><a href=\"{}?cid={iAnswerId}\" data-samepage=\"true\">Показать ответ</a></li>",
                    html_escape::encode_double_quoted_attribute(sTopicUrl),
                ));
            }
            if stViewer.score.unwrap_or(0) >= 50 && !bViewerFrozen && !bTopicExpired && !bTopicDraft
            {
                vecItems.push(format!(
                    "<li><a href=\"/post-warning?topic={iTopicId}&amp;comment={}\">Уведомить модераторов</a></li>",
                    stComment.iCommentId
                ));
            }
            vecItems.push(format!(
                "<li><a href=\"{}?cid={}\">Ссылка</a></li>",
                html_escape::encode_double_quoted_attribute(sTopicUrl),
                stComment.iCommentId,
            ));
            format!("<div class=\"reply\"><ul>{}</ul></div>", vecItems.join(""))
        } else {
            String::new()
        };
        let sWarnings = if stViewer.canmod && !bTopicExpired && bFullThreadContext {
            sCommentPreviewWarnings(&stComment.sWarningsJson, sCsrfToken)
        } else {
            String::new()
        };
        let sAuthorUrl = urlencoding::encode(&stComment.sAuthorNick).into_owned();
        let sAuthorNick = html_escape::encode_text(&stComment.sAuthorNick).into_owned();
        let bLinkAuthor = !stComment.bAuthorAnonymous || bViewerAuthorized;
        let sAuthorLinkOpen = if bLinkAuthor {
            format!("<a itemprop=\"creator\" href=\"/people/{sAuthorUrl}/profile\">")
        } else {
            String::new()
        };
        let sPostdateRfc3339 = stComment.dtPostdate.to_rfc3339();
        let sPostdate = stComment.dtPostdate.to_string();
        sHtml.push_str(
            &StPreparedCommentDom {
                iCommentId: stComment.iCommentId,
                sHeader: &sHeader,
                sUserpic: &sUserpicHtml,
                sBodyClass,
                sTitle: &sTitleHtml,
                sMessage: &sMessageHtml,
                sAuthorOpen,
                sAuthorLinkOpen: &sAuthorLinkOpen,
                sAuthorNick: &sAuthorNick,
                sAuthorLinkClose: if bLinkAuthor { "</a>" } else { "" },
                sAuthorClose,
                sStars: &sStars,
                sScore: &sScore,
                sPostdateRfc3339: &sPostdateRfc3339,
                sPostdate: &sPostdate,
                sTopicAuthor,
                sRemark: &sRemark,
                sModeratorIp: &sModeratorIp,
                sEditSummary: &sEditSummary,
                sModeratorUserAgent: &sModeratorUserAgent,
                sMenu: &sMenuHtml,
                sWarnings: &sWarnings,
                sReactions: &stReactions.sHtml,
            }
            .sHtml(),
        );
    }
    Ok(sHtml)
}

pub async fn delete_comment(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Form(form): Form<CommentAction>,
) -> Result<Response> {
    // Spring resolves @RequestParam values before AuthorizedOnly enters its
    // closure, so malformed/missing binding is a 400 even for an anonymous
    // request. Range validation itself remains inside AuthorizedOnly.
    let iCommentId = iSpringRequiredCommentInt(form.msgid.as_deref(), "msgid")?;
    let sReason = form
        .reason
        .ok_or_else(|| AppError::BadRequest("Required parameter 'reason' is missing".to_owned()))?;
    let iPenalty = iSpringDefaultCommentInt(form.bonus.as_deref(), "bonus", 0)?;
    let bDeleteReplies =
        bSpringDefaultCommentBoolean(form.delete_replys.as_deref(), "delete_replys")?;
    let Some(stUser) = optUser else {
        return Err(AppError::Forbidden);
    };
    let stOutcome = match stCommentDeletionService(&stState)
        .stDelete(
            stCommentDeleteActor(&stUser),
            StDeleteCommentCommand {
                iCommentId,
                sReason,
                iPenalty,
                bDeleteReplies,
            },
        )
        .await
    {
        Ok(stOutcome) => stOutcome,
        Err(EnCommentDeletionError::Restricted(stRestriction)) => {
            return stCommentDeletionRestrictionResponse(stRestriction);
        }
        Err(EnCommentDeletionError::Application(stError)) => return Err(stError),
    };
    let sNextLink = if let Some(iNextCommentId) = stOutcome.optNextCommentId {
        format!(
            "{}?cid={iNextCommentId}",
            stOutcome.stTarget.sCanonicalTopicUrl
        )
    } else {
        stOutcome.stTarget.sCanonicalTopicUrl.clone()
    };
    let sMessage = if stOutcome.vecDeletedIds.is_empty() {
        "Сообщение уже удалено"
    } else {
        "Удалено успешно"
    };
    let optBigMessage = (!stOutcome.vecDeletedIds.is_empty()).then(|| {
        format!(
            "Удаленные комментарии: {}",
            stOutcome
                .vecDeletedIds
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    if stUser.canmod && stUser.id != stOutcome.stTarget.iAuthorId {
        Ok(Html(
            StCommentDeletedByModeratorTemplate {
                message: sMessage.into(),
                big_message: optBigMessage,
                link: sNextLink,
                author_nick: stOutcome.stTarget.sAuthorNick,
                ip: stOutcome.stTarget.sPostIp,
                user_agent_id: stOutcome.stTarget.iUserAgentId,
            }
            .render()?,
        )
        .into_response())
    } else {
        Ok(Html(
            StCommentActionDoneTemplate {
                message: sMessage.into(),
                big_message: optBigMessage,
                link: Some(sNextLink),
            }
            .render()?,
        )
        .into_response())
    }
}

/// Legacy anonymous account used by topic/comment presentation and event
/// eligibility checks throughout the compatibility routes.
pub(crate) const ANONYMOUS_USER_ID: i32 = 2;

pub async fn undelete_comment(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Form(form): Form<CommentAction>,
) -> Result<Response> {
    let iCommentId = iSpringRequiredCommentInt(form.msgid.as_deref(), "msgid")?;
    let Some(stUser) = optUser else {
        return Err(AppError::Forbidden);
    };
    let stTarget = match stCommentDeletionService(&stState)
        .vUndelete(stCommentDeleteActor(&stUser), iCommentId)
        .await
    {
        Ok(stTarget) => stTarget,
        Err(EnCommentDeletionError::Restricted(stRestriction)) => {
            return stCommentDeletionRestrictionResponse(stRestriction);
        }
        Err(EnCommentDeletionError::Application(stError)) => return Err(stError),
    };
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{}?cid={iCommentId}", stTarget.sCanonicalTopicUrl),
        )],
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
    bRejectEmpty: bool,
) -> Result<i32> {
    if let Some(sError) = optCommentBodyErrorWithPolicy(&form.msg, bAnonymous, bRejectEmpty) {
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

    // CommentCreateService.notifyMentions: resolve only references produced
    // by MessageTextService for the stored markup mode, skipping the author
    // and anyone who has the author on their ignore list.
    let mentioned_nicks = markup::extract_mentions(&form.msg, markup);
    if bUserCastAllowed && !mentioned_nicks.is_empty() {
        let mentioned_ids: Vec<i32> = sqlx::query_scalar(
            r#"SELECT u.id FROM users u
               WHERE u.nick = ANY($1) AND u.id <> $2
                 AND ($3 OR NOT COALESCE(u.blocked,false))
                 AND NOT EXISTS (SELECT 1 FROM ignore_list il WHERE il.userid=u.id AND il.ignored=$2)"#,
        )
        .bind(&mentioned_nicks)
        .bind(user_id)
        .bind(markup::mentions_include_blocked_users(markup))
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
    // SearchQueueSender.updateComment runs after CommentCreateService's
    // transaction commits.  A durable-send failure is visible to the caller,
    // but cannot roll the already committed comment back.  Realtime events
    // are published only after that send succeeds, in the controller's exact
    // queue -> NewComment -> RefreshEvents order.
    CSearchQueueSender::new(
        state.config.opensearch_url.as_deref(),
        &state.config.upload_dir,
    )
    .vUpdateComments(&[id])
    .await?;
    state.realtime.vNotifyNewComment(form.topic, id);
    state.realtime.vNotifyEvents(notified.iter().copied());
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

impl StDeletedCommentRow {
    fn sTopicTitlePlain(&self) -> String {
        crate::domain::title::sTopicTitlePlainForDisplay(&self.topic_title)
    }
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
        return Err(AppError::stUserError("Некорректное значение offset"));
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
        StCommentPostingContext, StPreparedCommentDom, bSpringDefaultCommentBoolean,
        check_comment_posting_context, iCommentThresholdSeconds, iSpringDefaultCommentInt,
        iSpringRequiredCommentInt, optCommentBodyError, sCommentPreviewWarnings,
        sDeletedCommentsLink, stCommentPreviewUserpic,
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
    fn spring_comment_action_binding_accepts_only_original_boolean_vocabulary() {
        assert!(!bSpringDefaultCommentBoolean(None, "x").unwrap());
        assert!(!bSpringDefaultCommentBoolean(Some(""), "x").unwrap());
        for sValue in ["true", "TRUE", "on", "ON", "yes", "YES", "1"] {
            assert!(bSpringDefaultCommentBoolean(Some(sValue), "x").unwrap());
        }
        for sValue in ["false", "FALSE", "off", "OFF", "no", "NO", "0"] {
            assert!(!bSpringDefaultCommentBoolean(Some(sValue), "x").unwrap());
        }
        assert!(bSpringDefaultCommentBoolean(Some("garbage"), "x").is_err());
        assert_eq!(iSpringDefaultCommentInt(None, "bonus", 0).unwrap(), 0);
        assert_eq!(iSpringDefaultCommentInt(Some("7"), "bonus", 0).unwrap(), 7);
        assert!(iSpringRequiredCommentInt(None, "msgid").is_err());
        assert!(iSpringRequiredCommentInt(Some("not-an-int"), "msgid").is_err());
    }

    #[test]
    fn deletion_preview_preserves_prepared_comment_dom_and_menu_switch() {
        let sMenu = "<div class=\"reply\"><ul><li>DELETE</li></ul></div>";
        let sFixture = StPreparedCommentDom {
            iCommentId: 7,
            sHeader: "<div class=\"title\">HEADER</div>",
            sUserpic: "<div class=\"userpic\">USERPIC</div>",
            sBodyClass: " message-w-userpic",
            sTitle: "<h1>TITLE</h1>",
            sMessage: "MESSAGE",
            sAuthorOpen: "",
            sAuthorLinkOpen: "<a itemprop=\"creator\" href=\"/people/author/profile\">",
            sAuthorNick: "AUTHOR",
            sAuthorLinkClose: "</a>",
            sAuthorClose: "",
            sStars: "STARS",
            sScore: "SCORE",
            sPostdateRfc3339: "2026-08-15T00:00:00+00:00",
            sPostdate: "DATE",
            sTopicAuthor: "TOPIC-AUTHOR",
            sRemark: "REMARK",
            sModeratorIp: "IP",
            sEditSummary: "EDIT",
            sModeratorUserAgent: "UA",
            sMenu,
            sWarnings: "WARNINGS",
            sReactions: "REACTIONS",
        }
        .sHtml();
        let mut iPrevious = 0;
        for sToken in [
            "<article class=\"msg\" id=\"comment-7\">",
            "<div class=\"title\">HEADER</div>",
            "<div class=\"msg-container\">",
            "<div class=\"userpic\">USERPIC</div>",
            "<div class=\"msg_body message-w-userpic\">",
            "<div class=\"msg-text\"><h1>TITLE</h1>MESSAGE</div>",
            "<div class=\"sign\">",
            "IP",
            "EDIT",
            "UA",
            sMenu,
            "WARNINGS",
            "REACTIONS",
            "</article>",
        ] {
            let iFound = sFixture[iPrevious..]
                .find(sToken)
                .unwrap_or_else(|| panic!("missing prepared-comment DOM token: {sToken}"))
                + iPrevious;
            iPrevious = iFound + sToken.len();
        }
        assert!(!sFixture.contains("Ответить"));

        let sWithoutMenu = StPreparedCommentDom {
            sMenu: "",
            ..StPreparedCommentDom {
                iCommentId: 8,
                sHeader: "",
                sUserpic: "",
                sBodyClass: "",
                sTitle: "",
                sMessage: "MESSAGE",
                sAuthorOpen: "",
                sAuthorLinkOpen: "<a itemprop=\"creator\" href=\"/people/author/profile\">",
                sAuthorNick: "AUTHOR",
                sAuthorLinkClose: "</a>",
                sAuthorClose: "",
                sStars: "",
                sScore: "",
                sPostdateRfc3339: "2026-08-15T00:00:00+00:00",
                sPostdate: "DATE",
                sTopicAuthor: "",
                sRemark: "",
                sModeratorIp: "",
                sEditSummary: "",
                sModeratorUserAgent: "",
                sMenu: "unreachable",
                sWarnings: "",
                sReactions: "REACTIONS",
            }
        }
        .sHtml();
        assert!(!sWithoutMenu.contains("class=\"reply\""));
        assert!(sWithoutMenu.contains("REACTIONS</div></div></article>"));

        let sOrdinaryCommentTemplate = include_str!("../../templates/topic.html");
        for sHook in [
            "class=\"msg",
            "class=\"msg-container\"",
            "class=\"msg_body",
            "class=\"msg-text\"",
            "class=\"sign\"",
            "class=\"reply\"",
            "reactions_html|safe",
        ] {
            assert!(
                sOrdinaryCommentTemplate.contains(sHook),
                "ordinary and deletion previews must share DOM hook {sHook}"
            );
        }
    }

    #[test]
    fn reply_form_context_uses_the_full_prepared_comment_pipeline() {
        let sSource = include_str!("comments.rs");
        let sContext = sSource
            .split_once("async fn optCommentFormContextHtml(")
            .expect("reply context helper")
            .1
            .split_once("async fn render_comment_form(")
            .expect("end of reply context helper")
            .0;
        assert!(sContext.contains(".optPrepareCommentOnly(iReplyTo, stViewer.id)"));
        assert!(sContext.contains("sCommentDeletionPreviewHtml("));
        assert!(sContext.contains("std::slice::from_ref(&stReply)"));
        assert!(sSource.contains("let bReactionAllowed = bViewerAuthorized"));
        assert!(!sSource.contains(concat!("struct StCommentForm", "ReplySource")));
    }

    #[test]
    fn edit_get_and_post_preview_use_their_distinct_java_projections() {
        let sSource = include_str!("comments.rs");
        let sGet = sSource
            .split(concat!("pub async fn ", "edit_comment_form("))
            .nth(1)
            .unwrap()
            .split("pub struct EditCommentForm")
            .next()
            .unwrap();
        assert!(sGet.contains(".optPrepareCommentOnly(comment_id, user.id)"));
        assert!(sGet.contains("sCommentDeletionPreviewHtml("));

        let sPost = sSource
            .split(concat!("pub async fn ", "edit_comment("))
            .nth(1)
            .unwrap()
            .split(concat!("pub async fn ", "delete_comment_form("))
            .next()
            .unwrap();
        assert!(sPost.contains("sPreparedFormCommentHtml("));
        assert!(!sPost.contains(".optPrepareCommentOnly("));
    }

    #[test]
    fn deletion_preview_preserves_uploaded_userpic_aspect_ratio() {
        let iNonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let pathRoot = std::env::temp_dir().join(format!(
            "lorsource-comment-userpic-{}-{iNonce}",
            std::process::id()
        ));
        let pathPhotos = pathRoot.join("photos");
        std::fs::create_dir_all(&pathPhotos).expect("create isolated photo fixture");
        image::RgbImage::from_pixel(400, 200, image::Rgb([1, 2, 3]))
            .save(pathPhotos.join("rect.png"))
            .expect("save userpic fixture");

        let stUserpic = stCommentPreviewUserpic(&pathRoot, "empty", false, Some("rect.png"), None);

        assert_eq!(stUserpic, ("/photos/rect.png".to_owned(), 150, 75));
        std::fs::remove_dir_all(pathRoot).expect("remove isolated photo fixture");
    }

    #[test]
    fn deletion_preview_prefixes_warning_with_localized_type() {
        let sHtml = sCommentPreviewWarnings(
            r#"[{"id":7,"postdate":"2026-08-15T00:00:00Z","message":"<rule>","warning_type":"rule","author":"moderator","author_blocked":false,"closed_by":null}]"#,
            "csrf",
        );

        assert!(sHtml.contains("[Нарушение правил] &lt;rule&gt;"));
        assert!(!sHtml.contains("[rule]"));
    }

    #[test]
    fn prepared_warning_uses_user_tag_blocked_markup_for_closed_by() {
        let sHtml = sCommentPreviewWarnings(
            r#"[{"id":7,"postdate":"2026-08-15T00:00:00Z","message":"rule","warning_type":"rule","author":"moderator","author_blocked":true,"closed_by":"blocked","closed_by_blocked":true}]"#,
            "csrf",
        );

        assert!(sHtml.contains("<s><a href=\"/people/moderator/profile\">moderator</a></s>"));
        assert!(sHtml.contains("(закрыт <s><a href=\"/people/blocked/profile\">blocked</a></s>)"));
    }

    #[test]
    fn deletion_confirmation_templates_keep_java_form_fields_and_order() {
        let sDelete = include_str!("../../templates/delete_comment.html");
        let iDeleteForm = sDelete
            .find("<form method=POST action=\"delete_comment.jsp\"")
            .unwrap();
        let iDeletePreview = sDelete.find("<div class=\"messages\">").unwrap();
        assert!(iDeleteForm < iDeletePreview);
        for sField in [
            "name=\"csrf\"",
            "name=reason",
            "name=bonus",
            "name=\"delete_replys\"",
            "name=msgid",
        ] {
            assert!(sDelete.contains(sField), "missing delete field {sField}");
        }

        let sUndelete = include_str!("../../templates/undelete_comment.html");
        let iUndeletePreview = sUndelete.find("<div class=\"messages\">").unwrap();
        let iUndeleteForm = sUndelete
            .find("<form method=POST action=\"undelete_comment\"")
            .unwrap();
        assert!(iUndeletePreview < iUndeleteForm);
        assert!(sUndelete.contains("name=\"csrf\""));
        assert!(sUndelete.contains("name=msgid"));
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

        let sSource = include_str!("comments.rs");
        let sEditHandler = sSource
            .split(concat!("pub async fn ", "edit_comment_form("))
            .nth(1)
            .unwrap()
            .split(concat!(
                "#[derive(Deserialize)]",
                "\npub struct EditCommentForm"
            ))
            .next()
            .unwrap();
        assert!(sEditHandler.contains("AppError::BadParameter(\"Комментарий не задан\".into())"));
        let sDeletedHandler = sSource
            .split(concat!("pub async fn ", "deleted_comments_by_user("))
            .nth(1)
            .unwrap()
            .split(concat!("#[cfg(test)]", "\nmod tests"))
            .next()
            .unwrap();
        assert!(
            sDeletedHandler.contains("AppError::stUserError(\"Некорректное значение offset\")")
        );
    }
}

#[cfg(test)]
mod comment_message_method_tests {
    use super::*;
    use axum::body::{Body, to_bytes};

    fn stRequest(stMethod: Method, sUri: &str, sBody: &str) -> Request {
        Request::builder()
            .method(stMethod)
            .uri(sUri)
            .header(
                header::CONTENT_TYPE,
                "application/x-www-form-urlencoded; charset=UTF-8",
            )
            .body(Body::from(sBody.to_owned()))
            .unwrap()
    }

    #[tokio::test]
    async fn post_merges_form_after_query_and_preserves_first_value_binding() {
        let vecParameters = vecCommentMessageRequestParameters(stRequest(
            Method::POST,
            "/comment-message.jsp?topic=42&msg=query",
            "topic=99&msg=body",
        ))
        .await
        .unwrap();
        assert_eq!(
            vecParameters,
            [
                ("topic".to_owned(), "42".to_owned()),
                ("msg".to_owned(), "query".to_owned()),
                ("topic".to_owned(), "99".to_owned()),
                ("msg".to_owned(), "body".to_owned()),
            ]
        );
        let stBound = stBindCommentMessageParameters(&vecParameters).unwrap();
        assert_eq!(stBound.iTopicId, 42);
        assert_eq!(stBound.sMessage, "query");
    }

    #[test]
    fn post_csrf_uses_first_merged_value_before_model_binding() {
        let vecQueryTokenFirst = [
            ("csrf".to_owned(), " token ".to_owned()),
            ("csrf".to_owned(), "wrong".to_owned()),
        ];
        assert!(bCommentMessageCsrfValid(&vecQueryTokenFirst, "token"));

        let vecBadQueryFirst = [
            ("csrf".to_owned(), String::new()),
            ("csrf".to_owned(), "token".to_owned()),
        ];
        assert!(!bCommentMessageCsrfValid(&vecBadQueryFirst, "token"));
        assert!(!bCommentMessageCsrfValid(&[], "token"));
    }

    #[tokio::test]
    async fn ajax_comment_csrf_uses_query_before_conflicting_form_value() {
        let vecBadQuery = vecCommentMessageRequestParameters(stRequest(
            Method::POST,
            "/add_comment_ajax?csrf=wrong",
            "csrf=token&topic=42&msg=test",
        ))
        .await
        .unwrap();
        assert!(!bCommentMessageCsrfValid(&vecBadQuery, "token"));

        let vecGoodQuery = vecCommentMessageRequestParameters(stRequest(
            Method::POST,
            "/add_comment_ajax?csrf=token",
            "csrf=wrong&topic=42&msg=test",
        ))
        .await
        .unwrap();
        assert!(bCommentMessageCsrfValid(&vecGoodQuery, "token"));
    }

    #[tokio::test]
    async fn ajax_binding_errors_remain_json_utf8_with_nullable_preview() {
        let vecErrors = vecAjaxCommentInitialBindingErrors(
            &EnCommentMessageBindingError::MissingTopic,
            &[("msg".to_owned(), "test".to_owned())],
        );
        assert_eq!(vecErrors, ["тема не задана"]);
        let stResponse = stAjaxCommentJson(serde_json::json!({
            "errors": vecErrors,
            "preview": Option::<String>::None,
        }));
        assert_eq!(stResponse.status(), StatusCode::OK);
        assert_eq!(
            stResponse.headers()[header::CONTENT_TYPE],
            "application/json;charset=utf-8"
        );
        let vecBody = to_bytes(stResponse.into_body(), 4096).await.unwrap();
        let stBody: serde_json::Value = serde_json::from_slice(&vecBody).unwrap();
        assert_eq!(stBody["errors"], serde_json::json!(["тема не задана"]));
        assert!(stBody["preview"].is_null());
    }

    #[test]
    fn ajax_binder_accepts_topic_suffix_and_never_defaults_malformed_actions() {
        let stFallback = stAjaxCommentFallbackParameters(
            &[
                ("topic".to_owned(), "42,suffix".to_owned()),
                ("replyto".to_owned(), "bad".to_owned()),
                ("msg".to_owned(), "body".to_owned()),
            ],
            &EnCommentMessageBindingError::InvalidReplyTo,
        )
        .expect("valid topic survives a malformed reply");
        assert_eq!(stFallback.iTopicId, 42);
        assert_eq!(stFallback.optReplyToId, None);
        assert_eq!(stFallback.sMessage, "body");
        assert!(
            stAjaxCommentFallbackParameters(
                &[("topic".to_owned(), "bad".to_owned())],
                &EnCommentMessageBindingError::InvalidTopic,
            )
            .is_none()
        );
        assert!(optCommentBodyErrorWithPolicy("", true, false).is_none());
    }

    #[tokio::test]
    async fn non_post_methods_ignore_form_body_like_unfiltered_servlet_requests() {
        for stMethod in [Method::GET, Method::PUT, Method::PATCH, Method::DELETE] {
            let vecParameters = vecCommentMessageRequestParameters(stRequest(
                stMethod,
                "/comment-message.jsp?topic=42",
                "topic=99&msg=body",
            ))
            .await
            .unwrap();
            assert_eq!(vecParameters, [("topic".to_owned(), "42".to_owned())]);
        }
    }

    #[tokio::test]
    async fn urlencoded_post_body_has_a_strict_consumption_limit() {
        let sBody = format!(
            "topic=42&msg={}",
            "x".repeat(I_COMMENT_MESSAGE_PARAMETER_LIMIT)
        );
        assert!(
            vecCommentMessageRequestParameters(stRequest(
                Method::POST,
                "/comment-message.jsp",
                &sBody,
            ))
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn options_and_405_keep_distinct_spring_allow_contracts() {
        let stCommentOptions = options_comment_message().await;
        assert_eq!(stCommentOptions.status(), StatusCode::OK);
        assert_eq!(
            stCommentOptions.headers()[header::ALLOW],
            S_ALLOW_COMMENT_MESSAGE
        );
        assert_eq!(stCommentOptions.headers()[header::CONTENT_LENGTH], "0");

        let stActionOptions = options_comment_action().await;
        assert_eq!(stActionOptions.status(), StatusCode::OK);
        assert_eq!(
            stActionOptions.headers()[header::ALLOW],
            S_ALLOW_COMMENT_ACTION_OPTIONS
        );
        assert_eq!(stActionOptions.headers()[header::CONTENT_LENGTH], "0");

        let stAction405 = method_not_allowed_comment_action().await;
        assert_eq!(stAction405.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            stAction405.headers()[header::ALLOW],
            S_ALLOW_COMMENT_ACTION_405
        );

        let stUndeleteOptions = options_comment_undelete().await;
        assert_eq!(S_ALLOW_COMMENT_UNDELETE_OPTIONS, "GET,HEAD,POST,OPTIONS");
        assert_eq!(S_ALLOW_COMMENT_UNDELETE_405, "GET, POST");
        assert_eq!(stUndeleteOptions.status(), StatusCode::OK);
        assert_eq!(
            stUndeleteOptions.headers()[header::ALLOW],
            S_ALLOW_COMMENT_UNDELETE_OPTIONS
        );
        assert_eq!(stUndeleteOptions.headers()[header::CONTENT_LENGTH], "0");

        let stUndelete405 = method_not_allowed_comment_undelete().await;
        assert_eq!(stUndelete405.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            stUndelete405.headers()[header::ALLOW],
            S_ALLOW_COMMENT_UNDELETE_405
        );

        let stTrace405 = method_not_allowed_comment_message().await;
        assert_eq!(stTrace405.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(!stTrace405.headers().contains_key(header::ALLOW));
    }

    #[tokio::test]
    async fn put_missing_topic_uses_the_empty_container_400_only_for_that_binding_path() {
        let stResponse = optCommentMessageBindingResponse(
            &Method::PUT,
            &EnCommentMessageBindingError::MissingTopic,
        )
        .expect("PUT missing-topic binding response");
        assert_eq!(stResponse.status(), StatusCode::BAD_REQUEST);
        assert_eq!(stResponse.headers()[header::CONTENT_LENGTH], "0");
        assert!(!stResponse.headers().contains_key(header::CONTENT_TYPE));
        assert!(
            to_bytes(stResponse.into_body(), 1)
                .await
                .expect("empty PUT binding response")
                .is_empty()
        );

        assert!(
            optCommentMessageBindingResponse(
                &Method::GET,
                &EnCommentMessageBindingError::MissingTopic,
            )
            .is_none()
        );
        assert!(
            optCommentMessageBindingResponse(
                &Method::PUT,
                &EnCommentMessageBindingError::InvalidTopic,
            )
            .is_none()
        );
    }

    #[test]
    fn method_router_source_exposes_all_java_methods_but_not_trace() {
        let sSource = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/routes/comments.rs"
        ));
        let iStart = sSource.find("pub fn stCommentMessageRoute()").unwrap();
        let iEnd = sSource[iStart..]
            .find("pub fn stDeleteCommentRoute()")
            .map(|iOffset| iStart + iOffset)
            .unwrap();
        let sRouter = &sSource[iStart..iEnd];
        for sMethod in ["get", ".post", ".put", ".patch", ".delete", ".options"] {
            assert!(sRouter.contains(sMethod), "missing method {sMethod}");
        }
        assert!(!sRouter.contains(".trace"));
        assert!(sSource.contains(
            "get(undelete_comment_form)\n        .post(undelete_comment)\n        .options(options_comment_undelete)"
        ));
    }

    #[test]
    fn dedicated_template_uses_full_topic_card_and_original_form_dom() {
        let sTemplate = include_str!("../../templates/comment_message.html");
        for sFragment in [
            "{% block title %}{{ topic_title }} - {{ group_title }} - {{ section_title }}{% endblock %}",
            "$script('/js/add-form.js')",
            "<div class=messages>",
            "{{ topic_card_html|safe }}",
            "<h2><a name=rep>Добавить сообщение:</a></h2>",
            "href=\"/help/rules.md\"",
            "id=\"commentForm\" action=\"/add_comment.jsp\" method=\"post\"",
            "name=\"topic\"",
            "name=\"msg\" autofocus",
        ] {
            assert!(
                sTemplate.contains(sFragment),
                "missing DOM fragment {sFragment}"
            );
        }
        assert!(!sTemplate.contains("Отменить"));
    }
}
