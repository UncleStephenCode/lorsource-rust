use askama::Template;
use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
    routing::{MethodRouter, get, post},
};

use crate::{
    application::topic::deletion::{
        CTopicDeletionService, EnTopicDeletionServiceError, StDeleteTopicFormData,
    },
    auth::CurrentUser,
    domain::topic::deletion::{
        EnTopicDeletionBindingError, EnTopicDeletionRestriction, StDeleteTopicCommand,
        StTopicDeletionActor, VEC_TOPIC_DELETE_REASONS, stBindDeleteTopicParameters, stBindTopicId,
    },
    error::AppError,
    infra::{
        postgres::topic_deletion_repository::CTopicDeletionPgRepository,
        search_queue::CSearchQueueSender,
    },
    models::UserSummary,
    state::AppState,
};

const I_PARAMETER_BODY_LIMIT: usize = 1024 * 1024;
const S_ALLOW_DELETE_OPTIONS: &str = "GET,HEAD,POST,OPTIONS";
const S_ALLOW_DELETE_405: &str = "GET, POST";
const S_ALLOW_UNDELETE_OPTIONS: &str = "POST,GET,HEAD,OPTIONS";
const S_ALLOW_UNDELETE_405: &str = "POST, GET";

type TyTopicDeletionService = CTopicDeletionService<CTopicDeletionPgRepository, CSearchQueueSender>;
type TyRouteResult = std::result::Result<Response, EnTopicDeletionRouteError>;

#[derive(Debug, thiserror::Error)]
enum EnTopicDeletionRouteError {
    #[error(transparent)]
    Binding(#[from] EnTopicDeletionBindingError),
    #[error("invalid request parameters")]
    InvalidParameters,
    #[error(transparent)]
    Service(#[from] EnTopicDeletionServiceError),
    #[error(transparent)]
    App(#[from] AppError),
    #[error(transparent)]
    Template(#[from] askama::Error),
}

#[derive(Template)]
#[template(path = "delete_topic.html")]
struct StDeleteTopicTemplate {
    csrf_token: String,
    topic_id: i32,
    draft: bool,
    moderator: bool,
    bonus_eligible: bool,
    uncommitted: bool,
    author_score: i32,
    delete_reasons: &'static [&'static str],
}

#[derive(Template)]
#[template(path = "undelete_topic.html")]
struct StUndeleteTopicTemplate {
    csrf_token: String,
    topic_id: i32,
    topic_card_html: String,
}

#[derive(Template)]
#[template(path = "action_done.html")]
struct StActionDoneTemplate {
    message: String,
    big_message: Option<String>,
    link: Option<String>,
}

/// `UserErrorException` and `BadParameterException` are handled by the Java
/// common exception resolver as an escaped, user-visible page with HTTP 500.
/// They must not be collapsed into either a sanitized infrastructure 500 or a
/// 4xx response.
#[derive(Template)]
#[template(path = "topic_edit_user_error.html")]
struct StVisibleLegacyErrorTemplate<'a> {
    exception_class: &'static str,
    message: &'a str,
}

#[derive(Template)]
#[template(path = "error.html")]
struct StVisibleLegacyScriptErrorTemplate<'a> {
    title: &'static str,
    message: &'a str,
    bBadParameter: bool,
    bInternal: bool,
}

impl IntoResponse for EnTopicDeletionRouteError {
    fn into_response(self) -> Response {
        match self {
            Self::Binding(stError) => AppError::BadRequest(stError.to_string()).into_response(),
            Self::InvalidParameters => {
                AppError::BadRequest("invalid request parameters".to_owned()).into_response()
            }
            Self::Service(EnTopicDeletionServiceError::NotFound) => {
                AppError::NotFound.into_response()
            }
            Self::Service(EnTopicDeletionServiceError::NotAuthorized)
            | Self::Service(EnTopicDeletionServiceError::Restricted(
                EnTopicDeletionRestriction::CannotDelete
                | EnTopicDeletionRestriction::CannotUndelete,
            )) => AppError::Forbidden.into_response(),
            Self::Service(EnTopicDeletionServiceError::Restricted(
                EnTopicDeletionRestriction::AlreadyDeleted,
            )) => stVisibleLegacyErrorResponse(
                "ru.org.linux.user.UserErrorException",
                EnTopicDeletionRestriction::AlreadyDeleted.sReason(),
            ),
            Self::Service(EnTopicDeletionServiceError::InvalidPenalty) => {
                // This controller invokes BadParameterException's one-String
                // constructor, so that string is the parameter name rather
                // than the final exception message.
                stVisibleLegacyScriptErrorResponse(
                    "Неправильный формат параметра ``неправильный размер штрафа''",
                )
            }
            Self::Service(EnTopicDeletionServiceError::Infrastructure(stError))
            | Self::App(stError) => stError.into_response(),
            Self::Template(stError) => AppError::Template(stError).into_response(),
        }
    }
}

pub fn stDeleteRoute() -> MethodRouter<AppState> {
    get(delete_form)
        .post(delete_submit)
        .options(options_delete)
        .fallback(method_not_allowed_delete)
}

pub fn stUndeleteRoute() -> MethodRouter<AppState> {
    post(undelete_submit)
        .get(undelete_form)
        .options(options_undelete)
        .fallback(method_not_allowed_undelete)
}

fn stMethodResponse(stStatus: StatusCode, sAllow: &'static str) -> Response {
    (
        stStatus,
        [(header::ALLOW, sAllow), (header::CONTENT_LENGTH, "0")],
    )
        .into_response()
}

async fn options_delete() -> Response {
    stMethodResponse(StatusCode::OK, S_ALLOW_DELETE_OPTIONS)
}

async fn method_not_allowed_delete() -> Response {
    stMethodResponse(StatusCode::METHOD_NOT_ALLOWED, S_ALLOW_DELETE_405)
}

async fn options_undelete() -> Response {
    stMethodResponse(StatusCode::OK, S_ALLOW_UNDELETE_OPTIONS)
}

async fn method_not_allowed_undelete() -> Response {
    stMethodResponse(StatusCode::METHOD_NOT_ALLOWED, S_ALLOW_UNDELETE_405)
}

fn stVisibleLegacyErrorResponse(sExceptionClass: &'static str, sMessage: &str) -> Response {
    match (StVisibleLegacyErrorTemplate {
        exception_class: sExceptionClass,
        message: sMessage,
    })
    .render()
    {
        Ok(sBody) => (StatusCode::INTERNAL_SERVER_ERROR, Html(sBody)).into_response(),
        Err(stError) => AppError::Template(stError).into_response(),
    }
}

fn stVisibleLegacyScriptErrorResponse(sMessage: &str) -> Response {
    match (StVisibleLegacyScriptErrorTemplate {
        title: "ru.org.linux.site.BadParameterException",
        message: sMessage,
        bBadParameter: true,
        bInternal: false,
    })
    .render()
    {
        Ok(sBody) => (StatusCode::INTERNAL_SERVER_ERROR, Html(sBody)).into_response(),
        Err(stError) => AppError::Template(stError).into_response(),
    }
}

fn cService(stState: &AppState) -> TyTopicDeletionService {
    CTopicDeletionService::new(
        CTopicDeletionPgRepository::new(stState.pool.clone()),
        CSearchQueueSender::new(
            stState.config.opensearch_url.as_deref(),
            &stState.config.upload_dir,
        ),
    )
}

fn optActor(optUser: &Option<UserSummary>) -> Option<StTopicDeletionActor<'_>> {
    optUser.as_ref().map(|stUser| StTopicDeletionActor {
        iUserId: stUser.id,
        sNick: &stUser.nick,
        bModerator: stUser.canmod,
        bAdministrator: stUser.candel,
    })
}

fn vecQueryParameters(
    stUri: &Uri,
) -> std::result::Result<Vec<(String, String)>, EnTopicDeletionRouteError> {
    stUri
        .query()
        .map_or_else(|| Ok(Vec::new()), vecDecodeParameters)
}

fn vecDecodeParameters(
    sEncoded: &str,
) -> std::result::Result<Vec<(String, String)>, EnTopicDeletionRouteError> {
    serde_urlencoded::from_str(sEncoded).map_err(|_| EnTopicDeletionRouteError::InvalidParameters)
}

async fn vecRequestParameters(
    stRequest: Request,
) -> std::result::Result<Vec<(String, String)>, EnTopicDeletionRouteError> {
    let (stParts, stBody) = stRequest.into_parts();
    let mut vecParameters = vecQueryParameters(&stParts.uri)?;
    let vecBody = to_bytes(stBody, I_PARAMETER_BODY_LIMIT)
        .await
        .map_err(|_| EnTopicDeletionRouteError::InvalidParameters)?;
    if !vecBody.is_empty() {
        let mut vecBodyParameters: Vec<(String, String)> =
            serde_urlencoded::from_bytes(&vecBody)
                .map_err(|_| EnTopicDeletionRouteError::InvalidParameters)?;
        vecParameters.append(&mut vecBodyParameters);
    }
    Ok(vecParameters)
}

async fn delete_form(
    State(stState): State<AppState>,
    stUri: Uri,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> TyRouteResult {
    // Spring resolves `@RequestParam` before entering `AuthorizedOnly`.
    let vecParameters = vecQueryParameters(&stUri)?;
    let stParameters = stBindTopicId(&vecParameters)?;
    let StDeleteTopicFormData {
        stTopic,
        bBonusEligible,
        bModeratorSession,
        bUncommitted,
    } = cService(&stState)
        .stPrepareDelete(optActor(&optUser), stParameters.iTopicId)
        .await?;

    Ok(Html(
        StDeleteTopicTemplate {
            csrf_token: sCsrfToken,
            topic_id: stTopic.iTopicId,
            draft: stTopic.bDraft,
            moderator: bModeratorSession,
            bonus_eligible: bBonusEligible,
            uncommitted: bUncommitted,
            author_score: stTopic.iAuthorScore,
            delete_reasons: VEC_TOPIC_DELETE_REASONS,
        }
        .render()?,
    )
    .into_response())
}

async fn delete_submit(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    stRequest: Request,
) -> TyRouteResult {
    // Keep raw binding in front of the service's authorization boundary.
    let vecParameters = vecRequestParameters(stRequest).await?;
    let stParameters = stBindDeleteTopicParameters(&vecParameters)?;
    let stOutcome = cService(&stState)
        .stDelete(
            optActor(&optUser),
            StDeleteTopicCommand {
                iTopicId: stParameters.iTopicId,
                sReason: stParameters.sReason,
                iPenalty: stParameters.iPenalty,
            },
        )
        .await?;

    Ok(Html(
        StActionDoneTemplate {
            message: stOutcome.sMessage.to_owned(),
            big_message: None,
            link: stOutcome.optLink,
        }
        .render()?,
    )
    .into_response())
}

async fn undelete_form(
    State(stState): State<AppState>,
    stUri: Uri,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> TyRouteResult {
    let vecParameters = vecQueryParameters(&stUri)?;
    let stParameters = stBindTopicId(&vecParameters)?;
    let stPrepared = cService(&stState)
        .stPrepareUndelete(optActor(&optUser), stParameters.iTopicId)
        .await?;
    let sTopicCard = crate::routes::topics::sPrepareTopicCardHtml(
        &stState,
        stPrepared.stTopic.iTopicId,
        &optUser,
        &sCsrfToken,
        false,
    )
    .await?;

    Ok(Html(
        StUndeleteTopicTemplate {
            csrf_token: sCsrfToken,
            topic_id: stPrepared.stTopic.iTopicId,
            topic_card_html: sTopicCard,
        }
        .render()?,
    )
    .into_response())
}

async fn undelete_submit(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    stRequest: Request,
) -> TyRouteResult {
    let vecParameters = vecRequestParameters(stRequest).await?;
    let stParameters = stBindTopicId(&vecParameters)?;
    let stOutcome = cService(&stState)
        .stUndelete(optActor(&optUser), stParameters.iTopicId)
        .await?;

    Ok(Html(
        StActionDoneTemplate {
            message: stOutcome.sMessage.to_owned(),
            big_message: None,
            link: stOutcome.optLink,
        }
        .render()?,
    )
    .into_response())
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    #[tokio::test]
    async fn options_and_unsupported_methods_keep_the_exact_spring_allow_contract() {
        for (stResponse, stExpectedStatus, sExpectedAllow) in [
            (
                options_delete().await,
                StatusCode::OK,
                "GET,HEAD,POST,OPTIONS",
            ),
            (
                method_not_allowed_delete().await,
                StatusCode::METHOD_NOT_ALLOWED,
                "GET, POST",
            ),
            (
                options_undelete().await,
                StatusCode::OK,
                "POST,GET,HEAD,OPTIONS",
            ),
            (
                method_not_allowed_undelete().await,
                StatusCode::METHOD_NOT_ALLOWED,
                "POST, GET",
            ),
        ] {
            assert_eq!(stResponse.status(), stExpectedStatus);
            assert_eq!(stResponse.headers()[header::ALLOW], sExpectedAllow);
            assert_eq!(stResponse.headers()[header::CONTENT_LENGTH], "0");
            assert!(
                to_bytes(stResponse.into_body(), 1)
                    .await
                    .expect("empty method response")
                    .is_empty()
            );
        }
    }

    #[test]
    fn method_routers_keep_the_live_spring_method_order() {
        let sSource = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/routes/topic_deletion.rs"
        ));
        assert!(sSource.contains(
            "get(delete_form)\n        .post(delete_submit)\n        .options(options_delete)"
        ));
        assert!(sSource.contains(
            "post(undelete_submit)\n        .get(undelete_form)\n        .options(options_undelete)"
        ));
    }

    #[tokio::test]
    async fn legacy_500_messages_are_visible_but_html_escaped() {
        let stResponse = stVisibleLegacyErrorResponse(
            "ru.org.linux.user.UserErrorException",
            "уже <script>alert(1)</script>",
        );
        assert_eq!(stResponse.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("visible legacy error body");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8 template");
        assert!(sBody.contains("уже &#60;script&#62;alert(1)&#60;/script&#62;"));
        assert!(!sBody.contains("<script>alert(1)</script>"));

        let stResponse =
            stVisibleLegacyScriptErrorResponse("Неправильный формат параметра ``<bonus>''");
        assert_eq!(stResponse.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("visible legacy script-error body");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8 template");
        assert!(sBody.contains("Неправильный формат параметра ``&#60;bonus&#62;&#39;&#39;"));
        assert!(sBody.contains("Скрипту, генерирующему страничку"));
        assert!(!sBody.contains("<bonus>"));

        let stResponse =
            EnTopicDeletionRouteError::Service(EnTopicDeletionServiceError::InvalidPenalty)
                .into_response();
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("mapped InvalidPenalty body");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8 template");
        assert!(sBody.contains("Неправильный формат параметра"));
        assert!(sBody.contains("неправильный размер штрафа"));
        assert!(sBody.contains("Скрипту, генерирующему страничку"));
    }

    #[tokio::test]
    async fn route_error_mapping_preserves_status_classes() {
        assert_eq!(
            EnTopicDeletionRouteError::Binding(EnTopicDeletionBindingError::Missing {
                sName: "msgid"
            })
            .into_response()
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            EnTopicDeletionRouteError::Service(EnTopicDeletionServiceError::NotAuthorized)
                .into_response()
                .status(),
            StatusCode::FORBIDDEN
        );
        for enRestriction in [
            EnTopicDeletionRestriction::CannotDelete,
            EnTopicDeletionRestriction::CannotUndelete,
        ] {
            assert_eq!(
                EnTopicDeletionRouteError::Service(EnTopicDeletionServiceError::Restricted(
                    enRestriction
                ))
                .into_response()
                .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            EnTopicDeletionRouteError::Service(EnTopicDeletionServiceError::NotFound)
                .into_response()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            EnTopicDeletionRouteError::Service(EnTopicDeletionServiceError::InvalidPenalty)
                .into_response()
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            EnTopicDeletionRouteError::Service(EnTopicDeletionServiceError::Restricted(
                EnTopicDeletionRestriction::AlreadyDeleted
            ))
            .into_response()
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn query_parameters_win_before_body_duplicates_like_spring_request_parameters() {
        let stUri: Uri = "/delete.jsp?msgid=42&reason=query".parse().unwrap();
        let vecParameters = vecQueryParameters(&stUri).unwrap();
        assert_eq!(vecParameters[0], ("msgid".to_owned(), "42".to_owned()));
        assert_eq!(vecParameters[1], ("reason".to_owned(), "query".to_owned()));
    }
}
