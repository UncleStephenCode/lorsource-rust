use askama::Template;
use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{Method, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
    routing::{MethodRouter, get},
};

use crate::{
    application::topic::moderation::{
        CTopicModerationService, EnTopicModerationServiceError, StPreparedMove,
    },
    auth::CurrentUser,
    domain::topic::moderation::{
        EnLegacyRequiredBindingError, EnTopicMoveForm, StTopicModerationActor,
        stBindMoveParameters, stBindResolveParameters, stBindUncommitParameters,
    },
    error::AppError,
    infra::{
        postgres::topic_moderation_repository::CTopicModerationPgRepository,
        search_queue::CSearchQueueSender,
    },
    models::UserSummary,
    state::AppState,
};

const I_PARAMETER_BODY_LIMIT: usize = 1024 * 1024;
const S_ALLOW_MOVE: &str = "POST,GET,HEAD,OPTIONS";
const S_ALLOW_UNCOMMIT: &str = "POST,GET,HEAD,OPTIONS";
const S_ALLOW_GET: &str = "GET,HEAD,OPTIONS";
const S_ALLOW_RESOLVE: &str = "GET,HEAD,POST,PUT,PATCH,DELETE,OPTIONS";
const S_METHODS_MOVE: &str = "POST, GET";
const S_METHODS_UNCOMMIT: &str = "POST, GET";
const S_METHODS_GET: &str = "GET";

type TyModerationService =
    CTopicModerationService<CTopicModerationPgRepository, CSearchQueueSender>;
type TyRouteResult = std::result::Result<Response, EnTopicModerationRouteError>;

#[derive(Debug, thiserror::Error)]
enum EnTopicModerationRouteError {
    #[error(transparent)]
    Binding(#[from] EnLegacyRequiredBindingError),
    #[error("invalid request parameters")]
    InvalidParameters,
    #[error(transparent)]
    Service(#[from] EnTopicModerationServiceError),
    #[error(transparent)]
    App(#[from] AppError),
    #[error(transparent)]
    Template(#[from] askama::Error),
}

#[derive(Template)]
#[template(path = "uncommit_topic.html")]
struct StUncommitTemplate {
    csrf_token: String,
    topic_id: i32,
    topic_card_html: String,
}

#[derive(Debug)]
struct StMoveGroupView {
    id: i32,
    label: String,
    selected: bool,
}

#[derive(Template)]
#[template(path = "move_topic.html")]
struct StMoveTemplate {
    csrf_token: String,
    topic_id: i32,
    groups: Vec<StMoveGroupView>,
    author_nick: String,
    author_score: i32,
    author_blocked: bool,
}

#[derive(Template)]
#[template(path = "topic_moderation_forbidden.html")]
struct StForbiddenTemplate<'a> {
    message: &'a str,
}

#[derive(Template)]
#[template(path = "action_done.html")]
struct StActionDoneTemplate {
    message: String,
    big_message: Option<String>,
    link: Option<String>,
}

impl IntoResponse for EnTopicModerationRouteError {
    fn into_response(self) -> Response {
        match self {
            Self::Binding(stError) => AppError::BadRequest(stError.to_string()).into_response(),
            Self::InvalidParameters => {
                AppError::BadRequest("invalid request parameters".to_owned()).into_response()
            }
            Self::Service(EnTopicModerationServiceError::NotFound) => {
                AppError::NotFound.into_response()
            }
            Self::Service(EnTopicModerationServiceError::Forbidden { sReason }) => {
                match (StForbiddenTemplate { message: sReason }).render() {
                    Ok(sBody) => (StatusCode::FORBIDDEN, Html(sBody)).into_response(),
                    Err(stError) => AppError::Template(stError).into_response(),
                }
            }
            Self::Service(EnTopicModerationServiceError::Infrastructure(stError))
            | Self::App(stError) => stError.into_response(),
            Self::Template(stError) => AppError::Template(stError).into_response(),
        }
    }
}

pub fn stResolveRoute() -> MethodRouter<AppState> {
    get(resolve)
        .post(resolve)
        .put(resolve)
        .patch(resolve)
        .delete(resolve)
        .options(options_resolve)
        .fallback(method_not_allowed_resolve)
}

pub fn stMoveRoute() -> MethodRouter<AppState> {
    get(move_form)
        .post(move_submit)
        .options(options_move)
        .fallback(method_not_allowed_move)
}

pub fn stPremoderatedMoveRoute() -> MethodRouter<AppState> {
    get(premoderated_move_form)
        .options(options_get)
        .fallback(method_not_allowed_get)
}

pub fn stUncommitRoute() -> MethodRouter<AppState> {
    get(uncommit_form)
        .post(uncommit_submit)
        .options(options_uncommit)
        .fallback(method_not_allowed_uncommit)
}

fn stMethodResponse(stStatus: StatusCode, sAllow: &'static str) -> Response {
    (
        stStatus,
        [(header::ALLOW, sAllow), (header::CONTENT_LENGTH, "0")],
    )
        .into_response()
}

fn stEmptyResponse(stStatus: StatusCode) -> Response {
    (stStatus, [(header::CONTENT_LENGTH, "0")]).into_response()
}

fn optResolveBindingResponse(
    stMethod: &Method,
    stError: &EnLegacyRequiredBindingError,
) -> Option<Response> {
    // Live Java returns the container's empty 400 for this PUT binding path;
    // ordinary GET binding failures continue through the themed HTML error
    // renderer.  Keep the exception deliberately narrow until another method
    // and error variant is observed against the pinned runtime.
    (*stMethod == Method::PUT
        && matches!(
            stError,
            EnLegacyRequiredBindingError::Missing { sName: "msgid" }
        ))
    .then(|| stEmptyResponse(StatusCode::BAD_REQUEST))
}

async fn options_move() -> Response {
    stMethodResponse(StatusCode::OK, S_ALLOW_MOVE)
}

async fn options_uncommit() -> Response {
    stMethodResponse(StatusCode::OK, S_ALLOW_UNCOMMIT)
}

async fn options_get() -> Response {
    stMethodResponse(StatusCode::OK, S_ALLOW_GET)
}

async fn options_resolve() -> Response {
    stMethodResponse(StatusCode::OK, S_ALLOW_RESOLVE)
}

async fn method_not_allowed_move() -> Response {
    stMethodResponse(StatusCode::METHOD_NOT_ALLOWED, S_METHODS_MOVE)
}

async fn method_not_allowed_uncommit() -> Response {
    stMethodResponse(StatusCode::METHOD_NOT_ALLOWED, S_METHODS_UNCOMMIT)
}

async fn method_not_allowed_get() -> Response {
    stMethodResponse(StatusCode::METHOD_NOT_ALLOWED, S_METHODS_GET)
}

async fn method_not_allowed_resolve() -> Response {
    stMethodResponse(StatusCode::METHOD_NOT_ALLOWED, S_ALLOW_RESOLVE)
}

fn cService(stState: &AppState) -> TyModerationService {
    CTopicModerationService::new(
        CTopicModerationPgRepository::new(stState.pool.clone()),
        CSearchQueueSender::new(
            stState.config.opensearch_url.as_deref(),
            &stState.config.upload_dir,
        ),
    )
}

fn optActor(optUser: &Option<UserSummary>) -> Option<StTopicModerationActor<'_>> {
    optUser.as_ref().map(|stUser| StTopicModerationActor {
        iUserId: stUser.id,
        sNick: &stUser.nick,
        bModerator: stUser.canmod,
    })
}

fn vecQueryParameters(
    stUri: &Uri,
) -> std::result::Result<Vec<(String, String)>, EnTopicModerationRouteError> {
    stUri
        .query()
        .map_or_else(|| Ok(Vec::new()), vecDecodeParameters)
}

fn vecDecodeParameters(
    sEncoded: &str,
) -> std::result::Result<Vec<(String, String)>, EnTopicModerationRouteError> {
    serde_urlencoded::from_str(sEncoded).map_err(|_| EnTopicModerationRouteError::InvalidParameters)
}

async fn vecRequestParameters(
    stRequest: Request,
) -> std::result::Result<Vec<(String, String)>, EnTopicModerationRouteError> {
    let (stParts, stBody) = stRequest.into_parts();
    let mut vecParameters = vecQueryParameters(&stParts.uri)?;
    let vecBody = to_bytes(stBody, I_PARAMETER_BODY_LIMIT)
        .await
        .map_err(|_| EnTopicModerationRouteError::InvalidParameters)?;
    if !vecBody.is_empty() {
        let mut vecBodyParameters: Vec<(String, String)> =
            serde_urlencoded::from_bytes(&vecBody)
                .map_err(|_| EnTopicModerationRouteError::InvalidParameters)?;
        vecParameters.append(&mut vecBodyParameters);
    }
    Ok(vecParameters)
}

async fn uncommit_form(
    State(stState): State<AppState>,
    stUri: Uri,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> TyRouteResult {
    let vecParameters = vecQueryParameters(&stUri)?;
    let stParameters = stBindUncommitParameters(&vecParameters)?;
    cService(&stState)
        .stPrepareUncommit(optActor(&optUser), stParameters.iTopicId)
        .await?;
    let sTopicCard = crate::routes::topics::sPrepareTopicCardHtml(
        &stState,
        stParameters.iTopicId,
        &optUser,
        &sCsrfToken,
        false,
    )
    .await?;
    Ok(Html(
        StUncommitTemplate {
            csrf_token: sCsrfToken,
            topic_id: stParameters.iTopicId,
            topic_card_html: sTopicCard,
        }
        .render()?,
    )
    .into_response())
}

async fn uncommit_submit(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    stRequest: Request,
) -> TyRouteResult {
    let vecParameters = vecRequestParameters(stRequest).await?;
    let stParameters = stBindUncommitParameters(&vecParameters)?;
    let stOutcome = cService(&stState)
        .stUncommit(optActor(&optUser), stParameters.iTopicId)
        .await?;
    Ok(Html(
        StActionDoneTemplate {
            message: stOutcome.sMessage.to_owned(),
            big_message: None,
            link: Some(stOutcome.sCanonicalUrl),
        }
        .render()?,
    )
    .into_response())
}

async fn move_form(
    State(stState): State<AppState>,
    stUri: Uri,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> TyRouteResult {
    stRenderMoveForm(
        &stState,
        &stUri,
        &optUser,
        sCsrfToken,
        EnTopicMoveForm::ForumAndArticles,
    )
    .await
}

async fn premoderated_move_form(
    State(stState): State<AppState>,
    stUri: Uri,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> TyRouteResult {
    stRenderMoveForm(
        &stState,
        &stUri,
        &optUser,
        sCsrfToken,
        EnTopicMoveForm::PremoderatedCompanion,
    )
    .await
}

async fn stRenderMoveForm(
    stState: &AppState,
    stUri: &Uri,
    optUser: &Option<UserSummary>,
    sCsrfToken: String,
    enForm: EnTopicMoveForm,
) -> TyRouteResult {
    let vecParameters = vecQueryParameters(stUri)?;
    let stParameters = stBindUncommitParameters(&vecParameters)?;
    let StPreparedMove { stTopic, vecGroups } = cService(stState)
        .stPrepareMove(optActor(optUser), stParameters.iTopicId, enForm)
        .await?;
    let vecGroups = vecGroups
        .into_iter()
        .map(|stGroup| StMoveGroupView {
            id: stGroup.iId,
            label: stGroup.sFormLabel(),
            selected: stGroup.iId == stTopic.iGroupId,
        })
        .collect();
    Ok(Html(
        StMoveTemplate {
            csrf_token: sCsrfToken,
            topic_id: stTopic.iTopicId,
            groups: vecGroups,
            author_nick: stTopic.sAuthorNick,
            author_score: stTopic.iAuthorScore,
            author_blocked: stTopic.bAuthorBlocked,
        }
        .render()?,
    )
    .into_response())
}

async fn move_submit(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    stRequest: Request,
) -> TyRouteResult {
    let vecParameters = vecRequestParameters(stRequest).await?;
    let stParameters = stBindMoveParameters(&vecParameters)?;
    let stOutcome = cService(&stState)
        .stMove(
            optActor(&optUser),
            stParameters.iTopicId,
            stParameters.iMoveToGroupId,
        )
        .await?;
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, stOutcome.sRedirectUrl)],
    )
        .into_response())
}

async fn resolve(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    stRequest: Request,
) -> TyRouteResult {
    if stRequest.method() == Method::OPTIONS {
        return Ok(stMethodResponse(StatusCode::OK, S_ALLOW_RESOLVE));
    }
    let stMethod = stRequest.method().clone();
    let vecParameters = vecRequestParameters(stRequest).await?;
    let stParameters = match stBindResolveParameters(&vecParameters) {
        Ok(stParameters) => stParameters,
        Err(stError) => {
            if let Some(stResponse) = optResolveBindingResponse(&stMethod, &stError) {
                return Ok(stResponse);
            }
            return Err(stError.into());
        }
    };
    let stOutcome = cService(&stState)
        .stResolve(
            optActor(&optUser),
            stParameters.iTopicId,
            &stParameters.sResolve,
        )
        .await?;
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, stOutcome.sRedirectUrl)],
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_decoder_preserves_query_before_body_binding_order() {
        let stUri: Uri = "/resolve.jsp?msgid=42&resolve=yes".parse().unwrap();
        let vecParameters = vecQueryParameters(&stUri).unwrap();
        assert_eq!(vecParameters[0], ("msgid".to_owned(), "42".to_owned()));
        assert_eq!(vecParameters[1], ("resolve".to_owned(), "yes".to_owned()));
    }

    #[test]
    fn method_routers_expose_the_source_method_surface() {
        let sSource = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/routes/topic_moderation.rs"
        ));
        assert!(sSource.contains(".options(options_move)"));
        assert!(sSource.contains(".fallback(method_not_allowed_move)"));
        assert!(sSource.contains(".options(options_uncommit)"));
        assert!(sSource.contains(".fallback(method_not_allowed_uncommit)"));
        assert!(sSource.contains(".options(options_get)"));
        assert!(sSource.contains(".fallback(method_not_allowed_get)"));
        assert!(sSource.contains("pub fn stResolveRoute()"));
        assert!(sSource.contains(".put(resolve)"));
        assert!(sSource.contains(".patch(resolve)"));
        assert!(sSource.contains(".delete(resolve)"));
        assert!(sSource.contains(".fallback(method_not_allowed_resolve)"));
        assert!(sSource.contains("stRequest.method() == Method::OPTIONS"));
    }

    #[tokio::test]
    async fn explicit_options_and_405_responses_keep_spring_allow_values() {
        for (stResponse, sAllow) in [
            (options_move().await, S_ALLOW_MOVE),
            (options_uncommit().await, S_ALLOW_UNCOMMIT),
            (options_get().await, S_ALLOW_GET),
            (options_resolve().await, S_ALLOW_RESOLVE),
        ] {
            assert_eq!(stResponse.status(), StatusCode::OK);
            assert_eq!(stResponse.headers()[header::ALLOW], sAllow);
        }
        for (stResponse, sAllow) in [
            (method_not_allowed_move().await, S_METHODS_MOVE),
            (method_not_allowed_uncommit().await, S_METHODS_UNCOMMIT),
            (method_not_allowed_get().await, S_METHODS_GET),
            (method_not_allowed_resolve().await, S_ALLOW_RESOLVE),
        ] {
            assert_eq!(stResponse.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert_eq!(stResponse.headers()[header::ALLOW], sAllow);
        }
        let stResponse = stMethodResponse(StatusCode::OK, S_ALLOW_RESOLVE);
        assert_eq!(stResponse.headers()[header::ALLOW], S_ALLOW_RESOLVE);
        assert_eq!(stResponse.headers()[header::CONTENT_LENGTH], "0");
    }

    #[tokio::test]
    async fn put_missing_msgid_uses_the_empty_container_400_only_for_that_binding_path() {
        let stResponse = optResolveBindingResponse(
            &Method::PUT,
            &EnLegacyRequiredBindingError::Missing { sName: "msgid" },
        )
        .expect("PUT missing-msgid binding response");
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
            optResolveBindingResponse(
                &Method::GET,
                &EnLegacyRequiredBindingError::Missing { sName: "msgid" },
            )
            .is_none()
        );
        assert!(
            optResolveBindingResponse(
                &Method::PUT,
                &EnLegacyRequiredBindingError::InvalidInteger { sName: "msgid" },
            )
            .is_none()
        );
    }
}
