use crate::{
    application::{
        edit_history::{CEditHistoryService, StPreparedEditHistory},
        topic::CTopicService,
        user::{
            account::CUserAccountService, identity::CUserIdentityService, userpic::CUserpicService,
        },
    },
    auth::CurrentUser,
    error::{AppError, Result},
    infra::postgres::{
        edit_history_repository::CEditHistoryPgRepository, topic_repository::CTopicPgRepository,
        user_account_repository::CUserAccountPgRepository,
        user_identity_repository::CUserIdentityPgRepository,
        userpic_repository::CUserpicPgRepository,
    },
    markup,
    models::{CommentItem, PagerQuery, TopicSummary},
    pagination::Pager,
    state::AppState,
};
use askama::Template;
use axum::{
    Form, Json,
    extract::{ConnectInfo, Multipart, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;

pub async fn error_403() -> AppError {
    AppError::Forbidden
}
pub async fn error_404() -> AppError {
    AppError::NotFound
}

pub async fn exception_resolver() -> Response {
    // ExceptionController.defaultExceptionHandler is reached by the servlet
    // container with RequestDispatcher.ERROR_EXCEPTION set.  A direct client
    // request has no such server-side attribute and Java redirects it home;
    // clients cannot manufacture an exception dispatch in Axum either.
    stLegacyFoundRedirect("/".to_owned())
}

#[cfg(test)]
mod legacy_error_tests {
    use axum::{Router, http::header, routing::any};

    use super::{error_403, error_404, exception_resolver};

    async fn stStartServer() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let cApp = Router::new()
            .route("/ExceptionResolver", any(exception_resolver))
            .route("/errors/403", any(error_403))
            .route("/errors/404", any(error_404));
        let stListener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener");
        let stAddress = stListener.local_addr().expect("listener address");
        let hServer = tokio::spawn(async move {
            axum::serve(stListener, cApp)
                .await
                .expect("legacy error test server");
        });
        (stAddress, hServer)
    }

    #[tokio::test]
    async fn exception_resolver_direct_requests_match_java_redirect_for_all_mapped_methods() {
        let (stAddress, hServer) = stStartServer().await;
        let cClient = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("test client");

        for eMethod in [
            reqwest::Method::GET,
            reqwest::Method::HEAD,
            reqwest::Method::POST,
            reqwest::Method::PUT,
        ] {
            let stResponse = cClient
                .request(eMethod, format!("http://{stAddress}/ExceptionResolver"))
                .send()
                .await
                .expect("ExceptionResolver request");
            assert_eq!(stResponse.status(), reqwest::StatusCode::FOUND);
            assert_eq!(
                stResponse
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|stValue| stValue.to_str().ok()),
                Some("/")
            );
        }

        hServer.abort();
    }

    #[tokio::test]
    async fn legacy_code_pages_keep_status_content_type_and_public_html() {
        let (stAddress, hServer) = stStartServer().await;
        let cClient = reqwest::Client::new();

        for (sPath, stExpected, sMarker) in [
            (
                "/errors/403",
                reqwest::StatusCode::FORBIDDEN,
                "403 Forbidden",
            ),
            ("/errors/404", reqwest::StatusCode::NOT_FOUND, "Error 404"),
        ] {
            let stResponse = cClient
                .get(format!("http://{stAddress}{sPath}"))
                .send()
                .await
                .expect("legacy code page request");
            assert_eq!(stResponse.status(), stExpected);
            assert_eq!(
                stResponse
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|stValue| stValue.to_str().ok()),
                Some("text/html; charset=utf-8")
            );
            let sBody = stResponse.text().await.expect("legacy code page body");
            assert!(sBody.contains("id=\"warning-body\""));
            assert!(sBody.contains(sMarker));
            assert!(!sBody.contains("Exception resolver compatibility endpoint"));
        }

        hServer.abort();
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct LegacyIndexTemplate {
    title: String,
    topics: Vec<TopicSummary>,
    news: Vec<crate::routes::topics::NewsTopicView>,
    main_page: bool,
    tracker_layout: bool,
    navigation: Option<crate::routes::topics::TopicListNavigation>,
    prev_link: Option<String>,
    next_link: Option<String>,
}

#[derive(Deserialize)]
pub struct LegacyGroupQuery {
    pub group: Option<String>,
    pub offset: Option<String>,
}

fn iRequiredLegacyParameter(optValue: Option<&str>, sName: &str) -> Result<i32> {
    let sValue = optValue.ok_or_else(|| {
        AppError::BadParameter(format!("Не задан обязательный параметр `{sName}`"))
    })?;
    sValue
        .parse()
        .map_err(|_| AppError::BadParameter(format!("Некорректное значение параметра `{sName}`")))
}

fn sRequiredLegacyParameter(optValue: Option<String>, sName: &str) -> Result<String> {
    optValue
        .ok_or_else(|| AppError::BadParameter(format!("Не задан обязательный параметр `{sName}`")))
}

fn optLegacyI64Parameter(optValue: Option<&str>, sName: &str) -> Result<Option<i64>> {
    optValue
        .map(|sValue| {
            sValue.parse().map_err(|_| {
                AppError::BadParameter(format!("Некорректное значение параметра `{sName}`"))
            })
        })
        .transpose()
}

pub async fn group_jsp(State(state): State<AppState>, stRequest: Request) -> Result<Response> {
    let stMethod = stRequest.method().clone();
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let q = LegacyGroupQuery {
        group: crate::form::get(&vecParameters, "group").map(ToOwned::to_owned),
        offset: crate::form::get(&vecParameters, "offset").map(ToOwned::to_owned),
    };
    stLegacyUnsafeBindingResult(group_redirect(state, q, false).await, &stMethod)
}

pub async fn group_lastmod_jsp(
    State(state): State<AppState>,
    stRequest: Request,
) -> Result<Response> {
    let stMethod = stRequest.method().clone();
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let q = LegacyGroupQuery {
        group: crate::form::get(&vecParameters, "group").map(ToOwned::to_owned),
        offset: crate::form::get(&vecParameters, "offset").map(ToOwned::to_owned),
    };
    stLegacyUnsafeBindingResult(group_redirect(state, q, true).await, &stMethod)
}

fn stLegacyUnsafeBindingResult<T>(stResult: Result<T>, stMethod: &Method) -> Result<T> {
    match (stMethod, stResult) {
        // On the pinned servlet runtime unsafe methods reach unrestricted
        // redirect mappings, but a required-parameter/binding failure is
        // emitted by the container as an empty 400 instead of the JSP-backed
        // 404 used by an ordinary GET request.
        (
            &Method::PUT | &Method::PATCH | &Method::DELETE,
            Err(AppError::BadParameter(sMessage)),
        ) => Err(AppError::BadRequest(sMessage)),
        (_, stResult) => stResult,
    }
}

async fn group_redirect(state: AppState, q: LegacyGroupQuery, lastmod: bool) -> Result<Response> {
    let iGroupId = iRequiredLegacyParameter(q.group.as_deref(), "group")?;
    let optOffset = optLegacyI64Parameter(q.offset.as_deref(), "offset")?;
    let (section, group): (String, String) = sqlx::query_as(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END,
                  g.urlname
           FROM groups g JOIN sections s ON s.id=g.section WHERE g.id=$1"#,
    )
    .bind(iGroupId)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let mut url = format!("/{section}/{group}");
    let mut params = Vec::new();
    if let Some(offset) = optOffset {
        params.push(format!("offset={offset}"));
    }
    if lastmod {
        params.push("lastmod=true".to_string());
    }
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }
    Ok(stLegacyFoundRedirect(url))
}

#[derive(Deserialize)]
pub struct LegacySectionQuery {
    pub section: Option<String>,
}

pub async fn view_section_jsp(
    State(state): State<AppState>,
    stRequest: Request,
) -> Result<Response> {
    let stMethod = stRequest.method().clone();
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let q = LegacySectionQuery {
        section: crate::form::get(&vecParameters, "section").map(ToOwned::to_owned),
    };
    let iSectionId = stLegacyUnsafeBindingResult(
        iRequiredLegacyParameter(q.section.as_deref(), "section"),
        &stMethod,
    )?;
    let section: String = sqlx::query_scalar(
        r#"SELECT CASE id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(name) END
           FROM sections WHERE id=$1"#,
    )
    .bind(iSectionId)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let target = if section == "forum" {
        "/forum".to_string()
    } else {
        format!("/{section}/")
    };
    Ok(stLegacyFoundRedirect(target))
}

#[derive(Deserialize)]
pub struct ViewNewsQuery {
    pub tag: Option<String>,
}

fn stLegacyFoundRedirect(sLocation: String) -> Response {
    // The legacy Spring controllers use RedirectView's default 302. Axum's
    // Redirect::to is a 303 and is therefore not protocol-compatible here.
    (StatusCode::FOUND, [(header::LOCATION, sLocation)]).into_response()
}

fn sEncodeSpringUriPath(sValue: &str) -> String {
    // Spring's UriTemplate expands this value as a URI path, not as a form or
    // path-segment value. RFC 3986 pchar and '/' remain literal; all other
    // UTF-8 bytes are percent encoded with upper-case hex digits.
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut sEncoded = String::with_capacity(sValue.len());

    for iByte in sValue.bytes() {
        if iByte.is_ascii_alphanumeric()
            || matches!(
                iByte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b':'
                    | b'@'
                    | b'/'
            )
        {
            sEncoded.push(char::from(iByte));
        } else {
            sEncoded.push('%');
            sEncoded.push(char::from(HEX[usize::from(iByte >> 4)]));
            sEncoded.push(char::from(HEX[usize::from(iByte & 0x0f)]));
        }
    }

    sEncoded
}

fn stViewNewsRedirect(stQuery: ViewNewsQuery) -> Result<Response> {
    // TagTopicListController.tagFeedOld is selected only by the Spring
    // `params = "tag"` mapping condition.  The pinned Java runtime rejects a
    // request which does not satisfy that condition with HTTP 400.
    let sTag = stQuery
        .tag
        .ok_or_else(|| AppError::BadRequest("Required parameter 'tag' is missing".to_owned()))?;
    Ok(stLegacyFoundRedirect(format!(
        "/tag/{}",
        sEncodeSpringUriPath(&sTag)
    )))
}

pub async fn view_news_jsp(Query(stQuery): Query<ViewNewsQuery>) -> Result<Response> {
    stViewNewsRedirect(stQuery)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StLegacyViewMessageParameters {
    iMessageId: i32,
    optPage: Option<i32>,
    bLastModified: bool,
    optFilter: Option<String>,
    optOutput: Option<String>,
}

fn optServletNumber<T>(vecParameters: &[(String, String)], sName: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
{
    match crate::form::get(vecParameters, sName) {
        None | Some("") => Ok(None),
        Some(sValue) => sValue.parse().map(Some).map_err(|_| {
            AppError::BadRequest(format!("Failed to convert request parameter '{sName}'"))
        }),
    }
}

fn stLegacyViewMessageParameters(
    vecParameters: &[(String, String)],
) -> Result<StLegacyViewMessageParameters> {
    let sMessageId = crate::form::get(vecParameters, "msgid")
        .ok_or_else(|| AppError::BadRequest("Required request parameter 'msgid'".to_owned()))?;
    let iMessageId = sMessageId.parse().map_err(|_| {
        AppError::BadRequest("Failed to convert request parameter 'msgid'".to_owned())
    })?;
    // Spring converts an empty optional wrapper value to null. A present
    // non-empty `lastmod` must still bind as Long before the controller uses
    // only its presence.
    let optLastModified = optServletNumber::<i64>(vecParameters, "lastmod")?;
    Ok(StLegacyViewMessageParameters {
        iMessageId,
        optPage: optServletNumber(vecParameters, "page")?,
        bLastModified: optLastModified.is_some(),
        optFilter: crate::form::get(vecParameters, "filter").map(ToOwned::to_owned),
        optOutput: crate::form::get(vecParameters, "output").map(ToOwned::to_owned),
    })
}

fn sLegacyViewMessageLocation(
    stTopic: &crate::domain::topic::model::StLegacyTopicRedirect,
    stParameters: &StLegacyViewMessageParameters,
) -> String {
    let mut sLocation = stTopic.sCanonicalUrl();
    if let Some(iPage) = stParameters.optPage {
        sLocation.push_str(&format!("/page{iPage}"));
    }
    let mut vecQuery = Vec::new();
    if stParameters.bLastModified && !stTopic.bExpired {
        vecQuery.push(format!(
            "lastmod={}",
            stTopic.dtLastModified.timestamp_millis()
        ));
    }
    if let Some(sFilter) = stParameters.optFilter.as_deref() {
        vecQuery.push(format!("filter={}", sRedirectViewQueryValue(sFilter)));
    }
    if let Some(sOutput) = stParameters.optOutput.as_deref() {
        vecQuery.push(format!("output={}", sRedirectViewQueryValue(sOutput)));
    }
    if !vecQuery.is_empty() {
        sLocation.push('?');
        sLocation.push_str(&vecQuery.join("&"));
    }
    sLocation
}

fn sRedirectViewQueryValue(sValue: &str) -> String {
    // The Java controller appends decoded parameters directly to the
    // RedirectView URL. A pinned Spring Web MVC 6.2.19 render probe confirms
    // that spaces, Unicode and visible query delimiters such as '&' remain in
    // its logical target String; encoding here would change its query shape.
    // Jetty's wire-header conversion is reproduced separately below.
    sValue.to_owned()
}

fn vecServletRedirectHeaderBytes(sLocation: &str) -> Vec<u8> {
    // RedirectView passes its decoded String directly to
    // HttpServletResponse.sendRedirect.  The pinned Jetty 12 runtime writes
    // that Location value as ISO-8859-1: representable characters keep their
    // single byte and each unrepresentable UTF-16 code unit becomes a space.
    // Iterating UTF-16 rather than Rust scalar values is observable for a
    // supplementary character, which therefore produces two spaces.
    sLocation
        .encode_utf16()
        .map(|iUnit| u8::try_from(iUnit).unwrap_or(b' '))
        .collect()
}

fn stLegacyServletFoundRedirect(sLocation: String) -> Result<Response> {
    let stLocation = HeaderValue::from_bytes(&vecServletRedirectHeaderBytes(&sLocation))
        .map_err(|stError| AppError::Anyhow(anyhow::Error::new(stError)))?;
    let mut stResponse = StatusCode::FOUND.into_response();
    stResponse
        .headers_mut()
        .insert(header::LOCATION, stLocation);
    Ok(stResponse)
}

/// TopicController.getMessageOld is a bare RequestMapping: every method
/// admitted by StrictHttpFirewall reaches the same read-only redirect and
/// sees the Servlet query/form parameter view.
pub async fn legacy_view_message(
    State(stState): State<AppState>,
    stRequest: Request,
) -> Result<Response> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let stParameters = stLegacyViewMessageParameters(&vecParameters)?;
    let cService = CTopicService::new(CTopicPgRepository::new(
        stState.pool.clone(),
        stState.config.stLegacyJdbcTimezone(),
    ));
    let stTopic = cService
        .stLegacyTopicRedirect(stParameters.iMessageId)
        .await?;
    stLegacyServletFoundRedirect(sLegacyViewMessageLocation(&stTopic, &stParameters))
}

#[cfg(test)]
mod legacy_view_message_tests {
    use super::{
        StLegacyViewMessageParameters, sLegacyViewMessageLocation, stLegacyServletFoundRedirect,
        stLegacyViewMessageParameters, vecServletRedirectHeaderBytes,
    };
    use crate::{domain::topic::model::StLegacyTopicRedirect, error::AppError};
    use axum::http::header;
    use chrono::{TimeZone, Utc};

    fn stTopic(bExpired: bool) -> StLegacyTopicRedirect {
        StLegacyTopicRedirect {
            iTopicId: 42,
            sGroupUrlName: "rust".to_owned(),
            sSectionPrefix: "news".to_owned(),
            dtLastModified: Utc.timestamp_millis_opt(1_700_000_000_123).unwrap(),
            bExpired,
        }
    }

    #[test]
    fn binding_is_query_first_and_ignores_unbound_from_history() {
        let vecParameters = vec![
            ("msgid".to_owned(), "42".to_owned()),
            ("page".to_owned(), "3".to_owned()),
            ("lastmod".to_owned(), "1".to_owned()),
            ("filter".to_owned(), "show".to_owned()),
            ("output".to_owned(), "rss".to_owned()),
            ("fromHistory".to_owned(), "not-a-number".to_owned()),
            ("msgid".to_owned(), "99".to_owned()),
        ];
        let stParameters = stLegacyViewMessageParameters(&vecParameters).unwrap();
        assert_eq!(stParameters.iMessageId, 42);
        assert_eq!(
            sLegacyViewMessageLocation(&stTopic(false), &stParameters),
            "/news/rust/42/page3?lastmod=1700000000123&filter=show&output=rss"
        );
    }

    #[test]
    fn redirect_view_preserves_logical_query_values_and_delimiters() {
        let stParameters = StLegacyViewMessageParameters {
            iMessageId: 42,
            optPage: None,
            bLastModified: false,
            optFilter: Some("a b&extra=yes".to_owned()),
            optOutput: Some("атом".to_owned()),
        };
        assert_eq!(
            sLegacyViewMessageLocation(&stTopic(false), &stParameters),
            "/news/rust/42?filter=a b&extra=yes&output=атом"
        );
    }

    #[test]
    fn servlet_redirect_serializes_latin1_and_replaces_utf16_units() {
        let sLocation = "/news/rust/42?filter=a b&extra=yes&output=éатом🚀";
        assert_eq!(
            vecServletRedirectHeaderBytes(sLocation),
            b"/news/rust/42?filter=a b&extra=yes&output=\xE9      "
        );

        let stResponse = stLegacyServletFoundRedirect(sLocation.to_owned())
            .expect("Servlet-compatible redirect header");
        assert_eq!(stResponse.status(), axum::http::StatusCode::FOUND);
        assert_eq!(
            stResponse.headers()[header::LOCATION].as_bytes(),
            b"/news/rust/42?filter=a b&extra=yes&output=\xE9      "
        );
    }

    #[test]
    fn expired_topic_omits_lastmod_and_optional_empty_wrappers_bind_null() {
        let stParameters = stLegacyViewMessageParameters(&[
            ("msgid".to_owned(), "42".to_owned()),
            ("page".to_owned(), String::new()),
            ("lastmod".to_owned(), String::new()),
            ("filter".to_owned(), String::new()),
        ])
        .unwrap();
        assert_eq!(stParameters.optPage, None);
        assert!(!stParameters.bLastModified);
        assert_eq!(
            sLegacyViewMessageLocation(&stTopic(true), &stParameters),
            "/news/rust/42?filter="
        );

        let stWithLastmod = StLegacyViewMessageParameters {
            bLastModified: true,
            ..stParameters
        };
        assert_eq!(
            sLegacyViewMessageLocation(&stTopic(true), &stWithLastmod),
            "/news/rust/42?filter="
        );
    }

    #[test]
    fn required_and_numeric_binding_failures_are_400() {
        for vecParameters in [
            Vec::new(),
            vec![("msgid".to_owned(), String::new())],
            vec![("msgid".to_owned(), "bad".to_owned())],
            vec![
                ("msgid".to_owned(), "42".to_owned()),
                ("page".to_owned(), "bad".to_owned()),
            ],
            vec![
                ("msgid".to_owned(), "42".to_owned()),
                ("lastmod".to_owned(), "bad".to_owned()),
            ],
        ] {
            assert!(matches!(
                stLegacyViewMessageParameters(&vecParameters),
                Err(AppError::BadRequest(_))
            ));
        }
    }
}

#[derive(Deserialize)]
pub struct PreviewForm {
    pub text: Option<String>,
    pub markup: Option<String>,
}

/// MarkupPreviewController.preview: validates the markup id against
/// UserPermissionService.allowedFormats before rendering, and caps input at
/// MaxTextLength - the previous handler accepted any `markup` string
/// (including e.g. "html", which the site no longer allows anyone to pick,
/// see profile.rs's FORMAT_MODES) with no permission check at all.
pub async fn markup_preview(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<PreviewForm>,
) -> Result<Response> {
    // The Java controller binds exactly `text`; comment-form aliases such as
    // `msg` or `message` are not part of this endpoint's API.
    let text = form.text.unwrap_or_default();

    let markup_id = match form.markup {
        Some(sMarkupId) => sMarkupId,
        None => match user.as_ref() {
            Some(stUser) => {
                crate::routes::comments::user_comment_format(&state, stUser.id)
                    .await?
                    .0
            }
            None => crate::profile::DEFAULT_FORMAT_MODE.to_owned(),
        },
    };
    let stored_markup = match optPreviewStoredMarkup(&markup_id, user.is_some()) {
        Some(sStoredMarkup) => sStoredMarkup,
        None => {
            return Ok(stJsonUtf8(json!({"error": "Недопустимый режим разметки"})));
        }
    };

    if text.is_empty() {
        return Ok(stJsonUtf8(json!({"html": ""})));
    }
    // Java String.length counts UTF-16 code units, not Unicode scalar values.
    if bPreviewTextTooLong(&text) {
        return Ok(stJsonUtf8(json!({"error": "Слишком длинный текст"})));
    }
    let stMarkupUsers = state
        .markup
        .stResolveBatch([(&*text, stored_markup)])
        .await?;
    let html = markup::render_message_with_markup_policy_and_users(
        &text,
        Some(stored_markup),
        None,
        true,
        Some(&state.config.public_url),
        Some(&stMarkupUsers),
    );
    Ok(stJsonUtf8(json!({"html": html})))
}

fn optPreviewStoredMarkup(sMarkupId: &str, bAuthorized: bool) -> Option<&'static str> {
    Some(match sMarkupId {
        "markdown" => "MARKDOWN",
        "lorcode" => "BBCODE_TEX",
        // UserPermissionService.allowedFormats deliberately excludes the
        // deprecated LorcodeUlb mode for anonymous preview requests.
        "ntobr" if bAuthorized => "BBCODE_ULB",
        _ => return None,
    })
}

fn bPreviewTextTooLong(sText: &str) -> bool {
    sText.encode_utf16().count() > 65_536
}

fn stJsonUtf8(stValue: serde_json::Value) -> Response {
    let mut stResponse = Json(stValue).into_response();
    stResponse.headers_mut().insert(
        header::CONTENT_TYPE,
        "application/json;charset=utf-8".parse().unwrap(),
    );
    stResponse
}

#[cfg(test)]
mod markup_preview_contract_tests {
    use axum::{body::to_bytes, http::header};

    use super::{bPreviewTextTooLong, optPreviewStoredMarkup, stJsonUtf8};

    #[test]
    fn allowed_formats_match_user_permission_service() {
        assert_eq!(optPreviewStoredMarkup("markdown", false), Some("MARKDOWN"));
        assert_eq!(optPreviewStoredMarkup("lorcode", false), Some("BBCODE_TEX"));
        assert_eq!(optPreviewStoredMarkup("ntobr", false), None);
        assert_eq!(optPreviewStoredMarkup("ntobr", true), Some("BBCODE_ULB"));
        assert_eq!(optPreviewStoredMarkup("plain", true), None);
    }

    #[test]
    fn text_limit_uses_java_utf16_units() {
        assert!(!bPreviewTextTooLong(&"🚀".repeat(32_768)));
        assert!(bPreviewTextTooLong(&format!("{}a", "🚀".repeat(32_768))));
    }

    #[tokio::test]
    async fn preview_declares_the_java_json_utf8_content_type() {
        let stResponse = stJsonUtf8(serde_json::json!({"html": ""}));
        assert_eq!(
            stResponse
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("application/json;charset=utf-8")
        );
        let vecBody = to_bytes(stResponse.into_body(), 1024)
            .await
            .expect("preview json body");
        assert_eq!(&vecBody[..], br#"{"html":""}"#);
    }
}

pub async fn check_login(
    State(state): State<AppState>,
    stRequest: Request,
) -> Result<Json<serde_json::Value>> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let nick = crate::form::get(&vecParameters, "nick").ok_or_else(|| {
        AppError::BadRequest("Required request parameter 'nick' is missing".into())
    })?;
    let result = if nick.is_empty() {
        "Не задан nick.".to_string()
    } else if !valid_login_name_for_java(nick) {
        "Некорректное имя пользователя.".to_string()
    } else if nick.len() > 19 {
        "Слишком длинное имя пользователя.".to_string()
    } else if crate::routes::auth::user_exists_or_similar(&state, nick).await? {
        "Это имя пользователя уже используется. Пожалуйста выберите другое имя.".to_string()
    } else {
        "true".to_string()
    };
    Ok(Json(json!(result)))
}

/// Matches UserEventApiController.getYandexWidget: `{}` for anonymous,
/// `{"notifications": N}` once authenticated - the previous implementation
/// returned an unrelated widget-manifest shape that no real Yandex.Tableau
/// integration understands.
pub async fn yandex_tableau(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Response> {
    let Some(user) = user else {
        return Ok(Json(json!({})).into_response());
    };
    let count: i32 = sqlx::query_scalar("SELECT unread_events FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    Ok(crate::routes::api::stNoCacheJson(
        json!({"notifications": count}),
    ))
}

/// Matches HelpController.HelpPages exactly - only these 3 real pages
/// exist; anything else 404s (the previous handler rendered a placeholder
/// for any string, which never 404'd).
fn help_page_title(page: &str) -> Option<&'static str> {
    match page {
        "lorcode.md" => Some("Разметка сообщений (LORCODE)"),
        "markdown.md" => Some("Разметка сообщений (Markdown)"),
        "rules.md" => Some("Правила форума"),
        _ => None,
    }
}

#[derive(Template)]
#[template(path = "help.html")]
struct HelpTemplate {
    title: &'static str,
    html: String,
}

pub async fn help_page(
    State(state): State<AppState>,
    Path(page): Path<String>,
) -> Result<Html<String>> {
    let Some(title) = help_page_title(&page) else {
        return Err(AppError::NotFound);
    };
    let path = format!("{}/help/{page}", state.config.static_dir);
    let source = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| AppError::NotFound)?;
    let html = markup::render_message(&source, Some(false));
    Ok(Html(HelpTemplate { title, html }.render()?))
}

const MONTH_NAMES: [&str; 12] = [
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];

pub(crate) fn month_name(month: i32) -> &'static str {
    MONTH_NAMES
        .get((month - 1) as usize)
        .copied()
        .unwrap_or("?")
}

#[derive(Template)]
#[template(path = "archive_index.html")]
pub(crate) struct ArchiveIndexTemplate {
    pub(crate) title: String,
    pub(crate) heading: String,
    pub(crate) back_url: String,
    pub(crate) back_label: String,
    pub(crate) active_url: Option<String>,
    pub(crate) archive_url: String,
    pub(crate) section_id: i32,
    pub(crate) section_urlname: String,
    pub(crate) group_urlname: Option<String>,
    pub(crate) uncommitted_count: i64,
    pub(crate) add_url: Option<String>,
    pub(crate) add_reason: String,
    pub(crate) months: Vec<ArchiveMonthLink>,
}

pub(crate) struct ArchiveMonthLink {
    pub(crate) year: i32,
    pub(crate) month_name: &'static str,
    pub(crate) count: i64,
    pub(crate) url: String,
}

/// ArchiveDao.getArchiveStats is backed by `monthly_stats` in Java.  Compute
/// the same projection live so a newly committed topic is visible without
/// waiting for the ten-minute maintenance job, while retaining the exact
/// original visibility predicate used by `update_monthly_stats()`.
pub(crate) async fn list_archive_year_months(
    state: &AppState,
    section: Option<&str>,
    group: Option<&str>,
) -> Result<Vec<(i32, i32, i64)>> {
    Ok(sqlx::query_as::<_, (i32, i32, i64)>(
        r#"SELECT EXTRACT(YEAR FROM t.postdate)::int AS y, EXTRACT(MONTH FROM t.postdate)::int AS m, count(*) AS c
           FROM topics t
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE ($1::text IS NULL OR CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END = $1)
             AND ($2::text IS NULL OR g.urlname=$2)
             AND (t.moderate OR NOT s.moderate)
             AND NOT t.deleted
           GROUP BY y, m
           ORDER BY y, m"#,
    )
    .bind(section)
    .bind(group)
    .fetch_all(&state.pool)
    .await?)
}

pub async fn archive_section(
    State(state): State<AppState>,
    uri: Uri,
    CurrentUser(current_user): CurrentUser,
    headers: HeaderMap,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Html<String>> {
    let section = section_from_uri(&uri).unwrap_or("news");
    let section_name = match section {
        "news" => "Новости",
        "forum" => "Форум",
        "gallery" => "Галерея",
        "articles" => "Статьи",
        "polls" => "Опросы",
        _ => "Темы",
    };
    let rows = list_archive_year_months(&state, Some(section), None).await?;
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let navigation = crate::routes::topics::build_topic_list_navigation(
        &state,
        section,
        None,
        &current_user,
        &sRemoteIp,
        crate::search_index::EnActiveTagsForumFilter::All,
    )
    .await?;
    let months = rows
        .into_iter()
        .map(|(y, m, c)| ArchiveMonthLink {
            year: y,
            month_name: month_name(m),
            count: c,
            url: format!("/{section}/archive/{y}/{m}/"),
        })
        .collect();
    Ok(Html(
        ArchiveIndexTemplate {
            title: format!("{section_name} - Архив"),
            heading: section_name.to_string(),
            back_url: format!("/{section}/"),
            back_label: "Лента".to_string(),
            active_url: None,
            archive_url: format!("/{section}/archive/"),
            section_id: navigation.section_id,
            section_urlname: section.to_string(),
            group_urlname: None,
            uncommitted_count: navigation.uncommitted_count,
            add_url: navigation.add_url,
            add_reason: navigation.add_reason,
            months,
        }
        .render()?,
    ))
}

pub async fn archive_section_month(
    State(state): State<AppState>,
    uri: Uri,
    Path((year, month)): Path<(i32, i32)>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
) -> Result<Html<String>> {
    validate_year_month(year, month)?;
    let section = section_from_uri(&uri).unwrap_or("news");
    render_archive(
        state,
        Some(section),
        None,
        Some(year),
        Some(month),
        q,
        current_user,
    )
    .await
}

pub async fn forum_archive_month(
    State(state): State<AppState>,
    Path((group, year, month)): Path<(String, i32, i32)>,
    Query(q): Query<PagerQuery>,
    CurrentUser(current_user): CurrentUser,
) -> Result<Html<String>> {
    validate_year_month(year, month)?;
    render_archive(
        state,
        Some("forum"),
        Some(group),
        Some(year),
        Some(month),
        q,
        current_user,
    )
    .await
}

async fn render_archive(
    state: AppState,
    section: Option<&str>,
    group: Option<String>,
    year: Option<i32>,
    month: Option<i32>,
    q: PagerQuery,
    _current_user: Option<crate::models::UserSummary>,
) -> Result<Html<String>> {
    let pager = Pager::new(q.offset.unwrap_or(0), state.config.page_size);
    let topics = list_archive_topics(
        &state,
        section,
        group.as_deref(),
        year,
        month,
        pager.offset,
        pager.limit,
    )
    .await?;
    let news =
        crate::routes::topics::prepare_news_topics(&state, topics.clone(), group.is_none()).await?;
    let prev_link = pager.prev_offset.map(|offset| format!("?offset={offset}"));
    let next_link = Some(format!("?offset={}", pager.next_offset));
    let title = match (section, group.as_deref(), year, month) {
        (Some(sec), Some(group), Some(y), Some(m)) => {
            format!("Архив: {sec}/{group}, {y:04}-{m:02}")
        }
        (Some(sec), _, Some(y), Some(m)) => format!("Архив: {sec}, {y:04}-{m:02}"),
        (Some(sec), _, _, _) => format!("Архив: {sec}"),
        _ => "Архив".to_string(),
    };
    Ok(Html(
        LegacyIndexTemplate {
            title,
            topics,
            news,
            main_page: false,
            tracker_layout: false,
            navigation: None,
            prev_link,
            next_link,
        }
        .render()?,
    ))
}

async fn list_archive_topics(
    state: &AppState,
    section: Option<&str>,
    group: Option<&str>,
    year: Option<i32>,
    month: Option<i32>,
    offset: i64,
    limit: i64,
) -> Result<Vec<TopicSummary>> {
    Ok(sqlx::query_as::<_, TopicSummary>(
        r#"SELECT t.id, t.title, t.url, t.postdate, t.lastmod, u.id AS author_id, u.nick AS author,
                  g.id AS group_id, g.title AS group_title, g.urlname AS group_urlname,
                  s.id AS section_id, s.name AS section_name,
                  CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END AS section_prefix,
                  t.stat1 AS comments, t.deleted, t.sticky, t.resolved,
                  (SELECT string_agg(tv.value, ',' ORDER BY tv.value)
                     FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                    WHERE tg.msgid=t.id) AS tags
           FROM topics t
           JOIN users u ON u.id=t.userid
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE ($1::text IS NULL OR CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END = $1)
             AND ($2::text IS NULL OR g.urlname=$2)
             AND ($3::int IS NULL OR EXTRACT(YEAR FROM t.postdate)::int=$3)
             AND ($4::int IS NULL OR EXTRACT(MONTH FROM t.postdate)::int=$4)
             AND NOT t.deleted
             AND NOT t.draft
             AND (t.moderate OR NOT s.moderate)
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

#[derive(Template)]
#[template(path = "history.html")]
struct StHistoryTemplate {
    topic_id: i32,
    histories: Vec<StPreparedEditHistory>,
    can_restore: bool,
}

pub async fn topic_history(
    State(state): State<AppState>,
    Path((_group, id)): Path<(String, i32)>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>> {
    let Some(stUser) = user else {
        return Err(AppError::Forbidden);
    };
    let stTopic = crate::routes::topics::get_topic(&state, id).await?;
    crate::routes::topics::check_topic_viewable(&state, id, &Some(stUser.clone())).await?;
    let bExpired = crate::routes::comments::is_topic_expired(&state, id).await?;
    if !stUser.canmod && stUser.id != stTopic.author_id && bExpired {
        return Err(AppError::Forbidden);
    }
    let stRules = crate::routes::topics::load_topic_edit_rules(&state, id).await?;
    let bCanRestore = crate::routes::topics::b_topic_content_editable(&stTopic, &stRules, &stUser);
    let cService = CEditHistoryService::new(CEditHistoryPgRepository::new(state.pool.clone()));
    let vecHistories = cService
        .vecTopicHistory(id, &state.markup, &state.config.public_url)
        .await?;
    Ok(Html(
        StHistoryTemplate {
            topic_id: id,
            histories: vecHistories,
            can_restore: bCanRestore,
        }
        .render()?,
    ))
}

pub async fn comment_history(
    State(state): State<AppState>,
    Path((_group, id, commentid)): Path<(String, i32, i32)>,
    CurrentUser(user): CurrentUser,
) -> Result<Html<String>> {
    let stTopic = crate::routes::topics::get_topic(&state, id).await?;
    crate::routes::topics::check_topic_viewable(&state, id, &user).await?;
    let cService = CEditHistoryService::new(CEditHistoryPgRepository::new(state.pool.clone()));
    let vecHistories = cService
        .vecCommentHistory(
            stTopic.id,
            commentid,
            &state.markup,
            &state.config.public_url,
        )
        .await?;
    Ok(Html(
        StHistoryTemplate {
            topic_id: id,
            histories: vecHistories,
            can_restore: false,
        }
        .render()?,
    ))
}

#[derive(Deserialize)]
pub struct ShowCommentsQuery {
    pub nick: Option<String>,
}

fn sShowCommentsLocation(sCanonicalNick: &str) -> String {
    // ShowCommentsController constructs a relative RedirectView target, but
    // the servlet container exposes the normalized context-root path in the
    // Location header.
    format!(
        "/search.jsp?range=COMMENTS&user={}&sort=DATE",
        urlencoding::encode(sCanonicalNick)
    )
}

pub async fn show_comments_jsp(
    State(stState): State<AppState>,
    Query(stQuery): Query<ShowCommentsQuery>,
) -> Result<Response> {
    let sRequestedNick = sRequiredLegacyParameter(stQuery.nick, "nick")?;
    // Java resolves the user before redirecting. Besides rejecting an unknown
    // nick, this puts the canonical database spelling in Location.
    let stUser = crate::routes::users::get_user_exact(&stState, &sRequestedNick).await?;
    Ok(stLegacyFoundRedirect(sShowCommentsLocation(&stUser.nick)))
}

#[cfg(test)]
mod legacy_list_redirect_tests {
    use axum::{
        http::{Method, StatusCode, header},
        response::IntoResponse,
    };

    use super::{
        ViewNewsQuery, iRequiredLegacyParameter, optLegacyI64Parameter, sEncodeSpringUriPath,
        sRequiredLegacyParameter, sShowCommentsLocation, stLegacyFoundRedirect,
        stLegacyUnsafeBindingResult, stViewNewsRedirect,
    };
    use crate::error::AppError;

    #[test]
    fn view_news_requires_the_original_tag_mapping_condition() {
        let stError = stViewNewsRedirect(ViewNewsQuery { tag: None })
            .expect_err("the tag mapping condition is required");

        assert!(matches!(stError, AppError::BadRequest(_)));
        assert_eq!(stError.into_response().status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn view_news_unrelated_msgid_does_not_satisfy_the_tag_mapping_condition() {
        let stQuery: ViewNewsQuery =
            serde_urlencoded::from_str("msgid=invalid").expect("Servlet query binding");
        let stError = stViewNewsRedirect(stQuery)
            .expect_err("an unrelated parameter must not select tagFeedOld");

        assert!(matches!(stError, AppError::BadRequest(_)));
        assert_eq!(stError.into_response().status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn view_news_encodes_the_tag_and_uses_java_302() {
        let stResponse = stViewNewsRedirect(ViewNewsQuery {
            tag: Some("c++ / rust".to_owned()),
        })
        .expect("legacy tag redirect");

        assert_eq!(stResponse.status(), StatusCode::FOUND);
        assert_eq!(
            stResponse
                .headers()
                .get(header::LOCATION)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("/tag/c++%20/%20rust")
        );
    }

    #[test]
    fn spring_uri_template_path_encoding_preserves_only_path_characters() {
        assert_eq!(
            sEncodeSpringUriPath("a:b@c;d,e=f&g!h$i'j(k)l*m+n/o?p#q[r]s%t u"),
            "a:b@c;d,e=f&g!h$i'j(k)l*m+n/o%3Fp%23q%5Br%5Ds%25t%20u"
        );
        assert_eq!(sEncodeSpringUriPath("тег"), "%D1%82%D0%B5%D0%B3");
    }

    #[test]
    fn show_comments_uses_the_servlet_normalized_canonical_redirect_target() {
        let stResponse = stLegacyFoundRedirect(sShowCommentsLocation("maxcom"));

        assert_eq!(stResponse.status(), StatusCode::FOUND);
        assert_eq!(
            stResponse
                .headers()
                .get(header::LOCATION)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("/search.jsp?range=COMMENTS&user=maxcom&sort=DATE")
        );
    }

    #[test]
    fn legacy_spring_binding_failures_use_bad_parameter_404() {
        for stError in [
            iRequiredLegacyParameter(None, "group").expect_err("missing group"),
            iRequiredLegacyParameter(Some("not-an-id"), "section").expect_err("invalid section"),
            optLegacyI64Parameter(Some("not-an-offset"), "offset").expect_err("invalid offset"),
            sRequiredLegacyParameter(None, "nick").expect_err("missing nick"),
        ] {
            assert!(matches!(stError, AppError::BadParameter(_)));
            assert_eq!(stError.into_response().status(), StatusCode::NOT_FOUND);
        }

        assert_eq!(iRequiredLegacyParameter(Some("42"), "group").unwrap(), 42);
        assert_eq!(
            optLegacyI64Parameter(Some("300"), "offset").unwrap(),
            Some(300)
        );
    }

    #[test]
    fn unsafe_legacy_redirect_binding_failure_uses_the_container_400_contract() {
        for stMethod in [Method::PUT, Method::PATCH, Method::DELETE] {
            let stUnsafeError =
                stLegacyUnsafeBindingResult(iRequiredLegacyParameter(None, "group"), &stMethod)
                    .expect_err("unsafe request without group must fail");
            assert!(matches!(stUnsafeError, AppError::BadRequest(_)));
            assert_eq!(
                stUnsafeError.into_response().status(),
                StatusCode::BAD_REQUEST
            );
        }

        let stGetError =
            stLegacyUnsafeBindingResult(iRequiredLegacyParameter(None, "group"), &Method::GET)
                .expect_err("GET without group must fail");
        assert!(matches!(stGetError, AppError::BadParameter(_)));
        assert_eq!(stGetError.into_response().status(), StatusCode::NOT_FOUND);
    }
}

/// UserEventController's three `/show-replies.jsp` branches (Spring
/// disambiguates them via param presence - `!output`+`!nick`,
/// `!output`+`nick`, `output`):
/// 1. bare -> redirect to /notifications (must be logged in)
/// 2. `?nick=X` (no output) -> moderator-only read view of X's notifications
/// 3. `?output=rss|atom&nick=X` -> real XML feed of X's events
pub async fn show_replies_jsp(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    stRequest: Request,
) -> Result<Response> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let optOutput = crate::form::get(&vecParameters, "output");
    let optNick = crate::form::get(&vecParameters, "nick");
    let optFilter = crate::form::get(&vecParameters, "filter");
    if let Some(output) = optOutput {
        // Only the feed mapping binds output/filter/nick. Parameters belonging
        // solely to the moderator HTML branch (such as offset) are ignored.
        let nick = optNick
            .ok_or_else(|| {
                AppError::BadRequest("Required request parameter 'nick' is missing".to_owned())
            })?
            .to_owned();
        if !valid_login_name_for_java(&nick) {
            return Err(AppError::stBadInput("некорректное имя пользователя"));
        }
        let cIdentity =
            CUserIdentityService::new(CUserIdentityPgRepository::new(state.pool.clone()));
        let Some(stTarget) = cIdentity.optExactIdentity(&nick).await? else {
            return Err(AppError::NotFound);
        };
        let view_by_owner = user
            .as_ref()
            .map(|u| u.nick == stTarget.sNick)
            .unwrap_or(false);
        let db_type = optFilter.and_then(crate::routes::api::filter_db_type);
        let events =
            crate::routes::api::fetch_events(&state, stTarget.iId, db_type, view_by_owner, 200, 0)
                .await?;

        let is_atom = output == "atom";
        let stMarkupUsers = state
            .markup
            .stResolveBatch(
                events
                    .iter()
                    .map(|stEvent| (&*stEvent.message_text, &*stEvent.message_markup)),
            )
            .await?;
        let body = render_replies_feed(&state, &stTarget.sNick, &events, is_atom, &stMarkupUsers);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            (if is_atom {
                "application/atom+xml; charset=utf-8"
            } else {
                "application/rss+xml; charset=utf-8"
            })
            .parse()
            .unwrap(),
        );
        // Java sets `Expires: now + 90s` on this feed endpoint.
        let expires = (chrono::Utc::now() + chrono::Duration::seconds(90)).to_rfc2822();
        headers.insert(axum::http::header::EXPIRES, expires.parse().unwrap());
        return Ok((headers, body).into_response());
    }

    let Some(nick) = optNick.map(ToOwned::to_owned) else {
        if user.is_none() {
            return Err(AppError::Forbidden);
        }
        return Ok(crate::routes::stFoundRedirect("/notifications"));
    };
    if !valid_login_name_for_java(&nick) {
        return Err(AppError::stBadInput("некорректное имя пользователя"));
    }
    let Some(current) = user else {
        return Err(AppError::Forbidden);
    };
    if current.nick == nick {
        return Ok(crate::routes::stFoundRedirect("/notifications"));
    }
    if !current.canmod {
        return Err(AppError::Forbidden);
    }

    let cIdentity = CUserIdentityService::new(CUserIdentityPgRepository::new(state.pool.clone()));
    let Some(stTarget) = cIdentity.optExactIdentity(&nick).await? else {
        return Err(AppError::NotFound);
    };
    let sFilter = sCanonicalUserEventFilter(optFilter.unwrap_or("all"));
    let db_type = crate::routes::api::filter_db_type(sFilter);
    let offset = optServletNumber::<i64>(&vecParameters, "offset")?
        .unwrap_or(0)
        .max(0);
    let stSettings = crate::profile::ProfileSettings::from_hstore_text(
        cIdentity.optProfileSettings(current.id).await?,
    );
    let iPageSize = i64::from(stSettings.topics.max(1));
    let events =
        crate::routes::api::fetch_events(&state, stTarget.iId, db_type, true, iPageSize, offset)
            .await?;
    let vecEventTypes = cIdentity.vecEventTypes(stTarget.iId).await?;
    let html =
        sRenderModeratorNotifications(&nick, sFilter, offset, iPageSize, &vecEventTypes, &events);
    let sTitle = format!("Уведомления пользователя {nick}");
    Ok(Html(crate::routes::sRenderLegacyContent(&sTitle, html)?).into_response())
}

fn sCanonicalUserEventFilter(sFilter: &str) -> &'static str {
    match sFilter {
        "answers" => "answers",
        "favorites" => "favorites",
        "deleted" => "deleted",
        "reference" => "reference",
        "tag" => "tag",
        "reaction" => "reaction",
        "warning" => "warning",
        _ => "all",
    }
}

fn sLegacyNotificationIcon(sEventType: &str, optReaction: Option<&str>) -> String {
    match sEventType {
        "DEL" => "<img src=\"/img/del.png\" alt=\"[X]\" title=\"Сообщение удалено\" width=\"15\" height=\"15\">".to_owned(),
        "REPLY" => "<i class=\"icon-reply icon-reply-color\" title=\"Ответ\"></i>".to_owned(),
        "REF" => "<i class=\"icon-user icon-user-color\" title=\"Упоминание\"></i>".to_owned(),
        "TAG" => "<i class=\"icon-tag icon-tag-color\" title=\"Избранный тег\"></i>".to_owned(),
        "REACTION" => html_escape::encode_text(optReaction.unwrap_or("X")).into_owned(),
        "WARNING" => "<span title=\"Уведомление модератора\">⚠️</span>".to_owned(),
        _ => String::new(),
    }
}

fn sRenderModeratorNotifications(
    sNick: &str,
    sFilter: &str,
    iOffset: i64,
    iPageSize: i64,
    vecEventTypes: &[String],
    vecEvents: &[crate::routes::api::NotificationEvent],
) -> String {
    let sTitle = format!("Уведомления пользователя {sNick}");
    let mut sHtml = format!("<h1>{}</h1>", html_escape::encode_text(&sTitle));

    // UserEventService.getEventTypes returns no filter bar for zero/one
    // distinct event type; otherwise it preserves enum order and prepends ALL.
    if vecEventTypes.len() > 1 {
        sHtml.push_str("<nav>");
        for (sName, sLabel, sDbType) in [
            ("all", "все", ""),
            ("answers", "ответы", "REPLY"),
            ("favorites", "отслеживаемое", "WATCH"),
            ("deleted", "удаленное", "DEL"),
            ("reference", "упоминания", "REF"),
            ("tag", "теги", "TAG"),
            ("reaction", "реакции", "REACTION"),
            ("warning", "предупреждения", "WARNING"),
        ] {
            if !sDbType.is_empty() && !vecEventTypes.iter().any(|sType| sType == sDbType) {
                continue;
            }
            let sClass = if sName == sFilter {
                "btn btn-selected"
            } else {
                "btn btn-default"
            };
            sHtml.push_str(&format!(
                "<a href=\"/show-replies.jsp?nick={}&amp;filter={sName}\" class=\"{sClass}\">{sLabel}</a> ",
                urlencoding::encode(sNick),
            ));
        }
        sHtml.push_str("</nav>");
    }

    sHtml.push_str("<div class=\"forum\"><table width=\"100%\" class=\"message-table\">");
    for stEvent in vecEvents {
        let sSubject = stEvent.sSubjectPlain();
        let sTags = stEvent
            .tags
            .iter()
            .map(|sTag| {
                format!(
                    "<span class=\"tag\">{}</span>",
                    html_escape::encode_text(sTag)
                )
            })
            .collect::<String>();
        let sDetails = match stEvent.event_type.as_str() {
            "DEL" => format!(
                "<br>{} ({})",
                html_escape::encode_text(stEvent.event_message.as_deref().unwrap_or("")),
                stEvent.bonus.unwrap_or(0),
            ),
            "WARNING" if stEvent.closed_warning => format!(
                "<br><s>{}</s>",
                html_escape::encode_text(stEvent.event_message.as_deref().unwrap_or("")),
            ),
            "WARNING" => format!(
                "<br>{}",
                html_escape::encode_text(stEvent.event_message.as_deref().unwrap_or("")),
            ),
            _ => String::new(),
        };
        let sDate = crate::request_timezone::sTimeTag("interval", stEvent.event_date);
        sHtml.push_str(&format!(
            "<tr><td align=\"center\">{icon}</td><td><a href=\"{link}\" class=\"event-unread-{unread}\">{tags}{subject}</a> ({section}){details}{unread_mark}</td><td title=\"\">{date}, {author}</td></tr>",
            icon = sLegacyNotificationIcon(&stEvent.event_type, stEvent.reaction.as_deref()),
            link = html_escape::encode_double_quoted_attribute(&stEvent.link()),
            unread = stEvent.unread,
            tags = sTags,
            subject = html_escape::encode_text(&sSubject),
            section = html_escape::encode_text(&stEvent.section_name),
            details = sDetails,
            unread_mark = if stEvent.unread { " •" } else { "" },
            date = sDate,
            author = crate::routes::api::sNotificationAuthor(
                &stEvent.author_nick,
                stEvent.author_blocked,
            ),
        ));
    }
    sHtml.push_str("</table></div>");

    let sFilterSuffix = if sFilter == "all" {
        String::new()
    } else {
        format!("&amp;filter={sFilter}")
    };
    sHtml.push_str(
        "<div class=\"container\" style=\"margin-bottom:1em\"><div style=\"float:left\">",
    );
    if iOffset > 0 {
        sHtml.push_str(&format!(
            "<a rel=\"prev\" href=\"/show-replies.jsp?nick={}&amp;offset={}{}\">← назад</a>",
            urlencoding::encode(sNick),
            (iOffset - iPageSize).max(0),
            sFilterSuffix,
        ));
    }
    sHtml.push_str("</div><div style=\"float:right\">");
    if vecEvents.len() as i64 == iPageSize {
        sHtml.push_str(&format!(
            "<a rel=\"next\" href=\"/show-replies.jsp?nick={}&amp;offset={}{}\">вперед →</a>",
            urlencoding::encode(sNick),
            iOffset + iPageSize,
            sFilterSuffix,
        ));
    }
    sHtml.push_str("</div></div>");
    sHtml.push_str(&format!(
        "<p><i class=\"icon-rss\"></i> <a href=\"/show-replies.jsp?output=rss&amp;nick={}\">RSS подписка на новые уведомления</a></p>",
        urlencoding::encode(sNick),
    ));
    sHtml
}

#[cfg(test)]
mod show_replies_compatibility_tests {
    use chrono::TimeZone;

    use super::sRenderModeratorNotifications;

    fn stEvent() -> crate::routes::api::NotificationEvent {
        crate::routes::api::NotificationEvent {
            id: 101,
            event_date: chrono::Utc.with_ymd_and_hms(2026, 8, 16, 10, 0, 0).unwrap(),
            subj: "Subject".to_owned(),
            msgid: 42,
            cid: Some(43),
            unread: true,
            event_type: "WARNING".to_owned(),
            section_prefix: "forum".to_owned(),
            section_name: "Форум".to_owned(),
            group_urlname: "linux-org-ru".to_owned(),
            origin_nick: Some("moderator".to_owned()),
            author_nick: "moderator".to_owned(),
            author_blocked: false,
            event_message: Some("rule".to_owned()),
            closed_warning: false,
            bonus: None,
            tags: vec!["rust".to_owned()],
            message_text: "body".to_owned(),
            message_markup: "MARKDOWN".to_owned(),
            reaction: None,
        }
    }

    #[test]
    fn moderator_page_contains_the_java_filter_table_details_and_pager_model() {
        let sHtml = sRenderModeratorNotifications(
            "Target",
            "warning",
            20,
            20,
            &["REPLY".to_owned(), "WARNING".to_owned()],
            &[stEvent()],
        );
        assert!(sHtml.contains("class=\"btn btn-selected\">предупреждения</a>"));
        assert!(sHtml.contains("class=\"message-table\""));
        assert!(sHtml.contains("<span class=\"tag\">rust</span>Subject"));
        assert!(sHtml.contains("<br>rule"));
        assert!(sHtml.contains("<td title=\"\">"));
        assert!(!sHtml.contains("/people/moderator/profile"));
        assert!(sHtml.contains("offset=0&amp;filter=warning"));
        assert!(sHtml.contains("output=rss&amp;nick=Target"));
    }

    #[test]
    fn moderator_page_uses_the_plain_blocked_lor_user_contract() {
        let mut stEvent = stEvent();
        stEvent.author_blocked = true;
        let sHtml = sRenderModeratorNotifications("Target", "all", 0, 20, &[], &[stEvent]);
        assert!(sHtml.contains("<s>moderator</s>"));
        assert!(!sHtml.contains("/people/moderator/profile"));
    }

    #[test]
    fn exact_identity_is_enforced_across_both_show_replies_branches() {
        let sSource = include_str!("legacy.rs");
        let sHandler = sSource
            .split(concat!("pub async fn ", "show_replies_jsp("))
            .nth(1)
            .unwrap()
            .split(concat!("fn ", "sCanonicalUserEventFilter("))
            .next()
            .unwrap();
        assert_eq!(sHandler.matches("optExactIdentity(&nick)").count(), 2);
        assert!(sHandler.contains("Required request parameter 'nick' is missing"));
        assert!(!sHandler.contains("optNick.unwrap_or_default()"));
        assert!(!sHandler.contains("lower(nick)"));
        assert!(!sHandler.contains("eq_ignore_ascii_case"));
    }
}

fn render_replies_feed(
    state: &AppState,
    nick: &str,
    events: &[crate::routes::api::NotificationEvent],
    atom: bool,
    stMarkupUsers: &crate::domain::markup::model::StMarkupUserDirectory,
) -> String {
    let title = format!("Уведомления пользователя {nick}");
    if atom {
        let mut body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>{}</title><link href="{}/show-replies.jsp?nick={}&amp;output=atom" rel="self"/><id>{}/show-replies.jsp?nick={}</id>"#,
            html_escape::encode_text(&title),
            state.config.public_url,
            urlencoding::encode(nick),
            state.config.public_url,
            urlencoding::encode(nick),
        );
        for e in events {
            let link = format!("{}{}", state.config.public_url, e.link());
            let sDescription = sNotificationFeedDescription(e, state, stMarkupUsers);
            let sAuthor = e
                .cid
                .map(|_| {
                    format!(
                        "<author><name>{}</name></author>",
                        html_escape::encode_text(&e.author_nick)
                    )
                })
                .unwrap_or_default();
            body.push_str(&format!(
                "<entry><title>{}</title><link href=\"{}\"/><id>{}</id><updated>{}</updated>{author}{description}</entry>",
                html_escape::encode_text(&html_escape::decode_html_entities(&e.subj)),
                html_escape::encode_double_quoted_attribute(&link),
                e.id,
                e.event_date.to_rfc3339(),
                author = sAuthor,
                description = sDescription
                    .map(|sValue| format!("<summary type=\"html\">{}</summary>", html_escape::encode_text(&sValue)))
                    .unwrap_or_default(),
            ));
        }
        body.push_str("</feed>");
        body
    } else {
        let mut body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>{}</title><link>{}/show-replies.jsp?nick={}</link><description>{}</description>"#,
            html_escape::encode_text(&title),
            state.config.public_url,
            urlencoding::encode(nick),
            html_escape::encode_text(&title),
        );
        for e in events {
            let link = format!("{}{}", state.config.public_url, e.link());
            let sDescription = sNotificationFeedDescription(e, state, stMarkupUsers)
                .map(|sValue| {
                    format!(
                        "<description>{}</description>",
                        html_escape::encode_text(&sValue)
                    )
                })
                .unwrap_or_default();
            let sAuthor = e
                .cid
                .map(|_| {
                    format!(
                        "<author>{}</author>",
                        html_escape::encode_text(&e.author_nick)
                    )
                })
                .unwrap_or_default();
            body.push_str(&format!(
                "<item><title>{}</title><link>{}</link><guid isPermaLink=\"false\">{}</guid><pubDate>{}</pubDate>{author}{description}</item>",
                html_escape::encode_text(&html_escape::decode_html_entities(&e.subj)),
                html_escape::encode_text(&link),
                e.id,
                e.event_date.to_rfc2822(),
                author = sAuthor,
                description = sDescription,
            ));
        }
        body.push_str("</channel></rss>");
        body
    }
}

fn sNotificationFeedDescription(
    stEvent: &crate::routes::api::NotificationEvent,
    stState: &AppState,
    stMarkupUsers: &crate::domain::markup::model::StMarkupUserDirectory,
) -> Option<String> {
    let sRendered = markup::render_message_with_markup_policy_and_users(
        &stEvent.message_text,
        Some(&stEvent.message_markup),
        None,
        false,
        Some(&stState.config.public_url),
        Some(stMarkupUsers),
    );
    let sRendered = sRemoveInvalidXmlChars(&sRendered);
    if stEvent.event_type == "REACTION" {
        Some(format!(
            "@{} поставил {}<br>{sRendered}",
            stEvent.author_nick,
            stEvent.reaction.as_deref().unwrap_or("X")
        ))
    } else if sRendered.is_empty() {
        None
    } else {
        Some(sRendered)
    }
}

fn sRemoveInvalidXmlChars(sValue: &str) -> String {
    sValue
        .chars()
        .filter(|cValue| {
            matches!(*cValue, '\u{9}' | '\u{A}' | '\u{D}')
                || ('\u{20}'..='\u{D7FF}').contains(cValue)
                || ('\u{E000}'..='\u{FFFD}').contains(cValue)
                || ('\u{10000}'..='\u{10FFFF}').contains(cValue)
        })
        .collect()
}

#[derive(Deserialize)]
pub struct StViewDeletedQuery {
    pub id: i32,
}

struct StPreparedDeletedComment {
    stComment: CommentItem,
    optDeleteInfo: Option<(String, String)>,
}

async fn optLoadComment(stState: &AppState, iCommentId: i32) -> Result<Option<CommentItem>> {
    Ok(sqlx::query_as::<_, CommentItem>(
        r#"SELECT c.id, c.topic, c.replyto, c.title, m.message, m.markup::text AS markup,
                  c.postdate, u.id AS author_id, u.nick AS author,
                  COALESCE(u.score,0) AS author_score,
                  COALESCE(u.blocked,false) AS author_blocked,
                  COALESCE(u.passwd,'')='' AS author_anonymous,
                  COALESCE(u.frozen_until > CURRENT_TIMESTAMP,false) AS author_frozen,
                  c.deleted
           FROM comments c JOIN msgbase m ON m.id=c.id JOIN users u ON u.id=c.userid
           WHERE c.id=$1"#,
    )
    .bind(iCommentId)
    .fetch_optional(&stState.pool)
    .await?)
}

async fn optLoadDeleteInfo(
    stState: &AppState,
    iCommentId: i32,
) -> Result<Option<(String, String)>> {
    Ok(sqlx::query_as(
        r#"SELECT u.nick,di.reason FROM del_info di JOIN users u ON u.id=di.delby
           WHERE di.msgid=$1"#,
    )
    .bind(iCommentId)
    .fetch_optional(&stState.pool)
    .await?)
}

fn sRenderDeletedComment(
    stPrepared: &StPreparedDeletedComment,
    sSiteOrigin: &str,
    stMarkupUsers: &crate::domain::markup::model::StMarkupUserDirectory,
) -> String {
    let stComment = &stPrepared.stComment;
    let sDeleteInfo = if stComment.deleted {
        stPrepared
            .optDeleteInfo
            .as_ref()
            .map(|(sNick, sReason)| {
                format!(
                    " {} по причине: {}",
                    html_escape::encode_text(sNick),
                    html_escape::encode_text(sReason)
                )
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    let sDeletedTitle = if stComment.deleted {
        format!("<div class=\"title\"><strong>Сообщение удалено{sDeleteInfo}</strong></div>")
    } else {
        String::new()
    };
    let sTitle = stComment
        .optTitlePlain()
        .map(|sTitlePlain| format!("<h1>{}</h1>", html_escape::encode_text(&sTitlePlain)))
        .unwrap_or_default();
    format!(
        "<article class=\"msg\" id=\"comment-{id}\">{deleted_title}<div class=\"msg-container\"><div class=\"msg_body\"><div class=\"msg-text\">{title}{body}</div><div class=\"sign\"><a href=\"/people/{author_url}/profile\">{author}</a>, {date}</div></div></div></article>",
        id = stComment.id,
        deleted_title = sDeletedTitle,
        title = sTitle,
        body = markup::render_message_with_markup_policy_and_users(
            &stComment.message,
            Some(&stComment.markup),
            None,
            stComment.bNofollowAuthorLinks(),
            Some(sSiteOrigin),
            Some(stMarkupUsers),
        ),
        author_url = urlencoding::encode(&stComment.author),
        author = html_escape::encode_text(&stComment.author),
        date = crate::request_timezone::sTimeTag("default", stComment.postdate),
    )
}

fn bCanViewDeletedComment(
    bCanViewAll: bool,
    iViewerId: i32,
    iAuthorId: i32,
    bViewerFrozen: bool,
    dtDeleted: chrono::DateTime<chrono::Utc>,
    dtNow: chrono::DateTime<chrono::Utc>,
) -> bool {
    bCanViewAll
        || (iViewerId == iAuthorId
            && !bViewerFrozen
            && dtDeleted > dtNow - chrono::Duration::days(14))
}

pub async fn view_deleted(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Query(stQuery): Query<StViewDeletedQuery>,
) -> Result<Html<String>> {
    let stUser = optUser.as_ref().ok_or(AppError::Forbidden)?;
    let stComment = optLoadComment(&stState, stQuery.id)
        .await?
        .filter(|stComment| stComment.deleted)
        .ok_or(AppError::NotFound)?;
    let optDeleteRow: Option<(chrono::DateTime<chrono::Utc>, String, String)> = sqlx::query_as(
        r#"SELECT di.deldate,u.nick,di.reason FROM del_info di JOIN users u ON u.id=di.delby
           WHERE di.msgid=$1"#,
    )
    .bind(stComment.id)
    .fetch_optional(&stState.pool)
    .await?;
    let Some((dtDeleted, sDeletedBy, sDeleteReason)) = optDeleteRow else {
        return Err(AppError::NotFound);
    };

    let bCanViewAll =
        crate::routes::topics::allow_view_all_deleted_comments(&stState, stComment.topic, &optUser)
            .await?;
    let bFrozen: bool = sqlx::query_scalar(
        "SELECT COALESCE(frozen_until>CURRENT_TIMESTAMP,false) FROM users WHERE id=$1",
    )
    .bind(stUser.id)
    .fetch_one(&stState.pool)
    .await?;
    if !bCanViewDeletedComment(
        bCanViewAll,
        stUser.id,
        stComment.author_id,
        bFrozen,
        dtDeleted,
        chrono::Utc::now(),
    ) {
        return Err(AppError::Forbidden);
    }

    let (sTopicUrl, iPostScore): (String, i32) = sqlx::query_as(
        r#"SELECT '/'||(CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery'
                    WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END)
                  ||'/'||g.urlname||'/'||t.id, COALESCE(t.postscore,-9999)
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section
           WHERE t.id=$1"#,
    )
    .bind(stComment.topic)
    .fetch_optional(&stState.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let mut vecChain = Vec::new();
    if iPostScore != 10002 {
        let mut optParentId = stComment.replyto.filter(|iValue| *iValue != 0);
        while let Some(iParentId) = optParentId {
            let stParent = optLoadComment(&stState, iParentId)
                .await?
                .ok_or(AppError::NotFound)?;
            let optDeleteInfo = if stParent.deleted {
                optLoadDeleteInfo(&stState, stParent.id).await?
            } else {
                None
            };
            let bContinue = stParent.deleted
                && optDeleteInfo
                    .as_ref()
                    .is_some_and(|(_, sReason)| sReason.starts_with("7.1 "));
            optParentId = bContinue
                .then_some(stParent.replyto)
                .flatten()
                .filter(|iValue| *iValue != 0);
            vecChain.push(StPreparedDeletedComment {
                stComment: stParent,
                optDeleteInfo,
            });
            if !bContinue {
                break;
            }
        }
        vecChain.reverse();
    }

    let sBackLink = if stUser.canmod {
        format!("{sTopicUrl}?cid={}", stComment.id)
    } else {
        sTopicUrl
    };
    let stMarkupUsers = stState
        .markup
        .stResolveBatch(
            vecChain
                .iter()
                .map(|stPrepared| {
                    (
                        stPrepared.stComment.message.as_str(),
                        stPrepared.stComment.markup.as_str(),
                    )
                })
                .chain(std::iter::once((
                    stComment.message.as_str(),
                    stComment.markup.as_str(),
                ))),
        )
        .await?;
    let mut sHtml = format!(
        "<h1>Просмотр удаленного комментария</h1><nav><a class=\"btn btn-default\" href=\"{}\">Перейти в топик</a></nav><div class=\"messages\">",
        html_escape::encode_double_quoted_attribute(&sBackLink)
    );
    for stParent in &vecChain {
        sHtml.push_str("<h2>Ответ на:</h2>");
        sHtml.push_str(&sRenderDeletedComment(
            stParent,
            &stState.config.public_url,
            &stMarkupUsers,
        ));
    }
    if !vecChain.is_empty() {
        sHtml.push_str("<h2>Удаленный комментарий:</h2>");
    }
    sHtml.push_str(&sRenderDeletedComment(
        &StPreparedDeletedComment {
            stComment,
            optDeleteInfo: Some((sDeletedBy, sDeleteReason)),
        },
        &stState.config.public_url,
        &stMarkupUsers,
    ));
    sHtml.push_str("</div>");
    Ok(Html(crate::routes::sRenderLegacyContent(
        "Просмотр удаленного комментария",
        sHtml,
    )?))
}

#[derive(Deserialize)]
pub struct NotificationsClickForm {
    #[serde(rename = "firstId")]
    pub first_id: i32,
    #[serde(rename = "lastId")]
    pub last_id: i32,
}

async fn topic_link(
    state: &AppState,
    topic_id: i32,
    comment_id: Option<i32>,
    event_type: &str,
) -> Result<String> {
    if event_type == "DEL"
        && let Some(iCommentId) = comment_id
    {
        return Ok(format!(
            "/view-deleted?id={iCommentId}#comment-{iCommentId}"
        ));
    }
    let prefix: Option<(String, String)> = sqlx::query_as(
        r#"SELECT CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' ELSE lower(s.name) END,
                  g.urlname
           FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section WHERE t.id=$1"#,
    )
    .bind(topic_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((section, group)) = prefix else {
        return Ok("/notifications".to_string());
    };
    let anchor = comment_id
        .map(|id| format!("?cid={id}"))
        .unwrap_or_default();
    Ok(format!("/{section}/{group}/{topic_id}{anchor}"))
}

#[derive(Debug)]
struct StNotificationClickEvent {
    user_id: i32,
    unread: bool,
    event_type: String,
    topic_id: Option<i32>,
    comment_id: Option<i32>,
}

type TyNotificationClickRow = (i32, bool, String, Option<i32>, Option<i32>);

const S_RESET_WATCH_NOTIFICATIONS: &str = r#"UPDATE user_events SET unread=false
    WHERE userid=$1 AND unread AND id<=$2
      AND type='WATCH'::event_type AND message_id=$3"#;

fn bValidNotificationClickRange(
    first_id: i32,
    first: &StNotificationClickEvent,
    last_id: i32,
    last: &StNotificationClickEvent,
) -> bool {
    if first_id > last_id || first.unread != last.unread {
        return false;
    }
    match last.event_type.as_str() {
        "WATCH" => first.event_type == "WATCH" && first.topic_id == last.topic_id,
        "REACTION" => {
            first.event_type == "REACTION"
                && first.topic_id == last.topic_id
                && first.comment_id == last.comment_id
        }
        _ => first_id == last_id && first.event_type == last.event_type,
    }
}

#[cfg(test)]
mod notification_click_tests {
    use super::*;

    fn stEvent(sType: &str, iTopicId: i32, optCommentId: Option<i32>) -> StNotificationClickEvent {
        StNotificationClickEvent {
            user_id: 1,
            unread: true,
            event_type: sType.into(),
            topic_id: Some(iTopicId),
            comment_id: optCommentId,
        }
    }

    #[test]
    fn watch_range_requires_same_topic_and_order() {
        let stFirst = stEvent("WATCH", 10, Some(1));
        let stLast = stEvent("WATCH", 10, Some(9));
        assert!(bValidNotificationClickRange(2, &stFirst, 5, &stLast));
        assert!(!bValidNotificationClickRange(5, &stFirst, 2, &stLast));
        assert!(!bValidNotificationClickRange(
            2,
            &stFirst,
            5,
            &stEvent("WATCH", 11, None)
        ));
    }

    #[test]
    fn reaction_range_requires_same_topic_and_comment() {
        let stFirst = stEvent("REACTION", 10, Some(7));
        assert!(bValidNotificationClickRange(
            2,
            &stFirst,
            5,
            &stEvent("REACTION", 10, Some(7))
        ));
        assert!(!bValidNotificationClickRange(
            2,
            &stFirst,
            5,
            &stEvent("REACTION", 10, Some(8))
        ));
    }

    #[test]
    fn ordinary_event_must_be_a_single_matching_event() {
        let stEvent = stEvent("REF", 10, None);
        assert!(bValidNotificationClickRange(2, &stEvent, 2, &stEvent));
        assert!(!bValidNotificationClickRange(2, &stEvent, 3, &stEvent));
    }

    #[test]
    fn watch_reset_never_consumes_an_event_newer_than_the_clicked_group() {
        assert!(S_RESET_WATCH_NOTIFICATIONS.contains("id<=$2"));
        assert!(S_RESET_WATCH_NOTIFICATIONS.contains("message_id=$3"));
    }

    #[test]
    fn deleted_comment_visibility_matches_java_owner_window_and_global_gate() {
        let dtNow = chrono::Utc::now();
        assert!(bCanViewDeletedComment(
            true,
            9,
            10,
            true,
            dtNow - chrono::Duration::days(30),
            dtNow,
        ));
        assert!(bCanViewDeletedComment(
            false,
            9,
            9,
            false,
            dtNow - chrono::Duration::days(13),
            dtNow,
        ));
        assert!(!bCanViewDeletedComment(
            false,
            9,
            9,
            true,
            dtNow - chrono::Duration::days(1),
            dtNow,
        ));
        assert!(!bCanViewDeletedComment(
            false,
            9,
            9,
            false,
            dtNow - chrono::Duration::days(14),
            dtNow,
        ));
        assert!(!bCanViewDeletedComment(
            false,
            9,
            10,
            false,
            dtNow - chrono::Duration::days(1),
            dtNow,
        ));
    }

    #[test]
    fn notification_feed_removes_invalid_xml_characters() {
        assert_eq!(sRemoveInvalidXmlChars("ok\u{0} text\n"), "ok text\n");
    }
}

async fn process_notifications_click(
    state: &AppState,
    user_id: i32,
    form: &NotificationsClickForm,
) -> Result<String> {
    let optFirst: Option<TyNotificationClickRow> = sqlx::query_as(
        "SELECT userid,unread,type::text,message_id,comment_id FROM user_events WHERE id=$1",
    )
    .bind(form.first_id)
    .fetch_optional(&state.pool)
    .await?;
    let optLast: Option<TyNotificationClickRow> = sqlx::query_as(
        "SELECT userid,unread,type::text,message_id,comment_id FROM user_events WHERE id=$1",
    )
    .bind(form.last_id)
    .fetch_optional(&state.pool)
    .await?;

    let (Some(stFirstRow), Some(stLastRow)) = (optFirst, optLast) else {
        return Ok("/notifications".to_string());
    };
    let stFirst = StNotificationClickEvent {
        user_id: stFirstRow.0,
        unread: stFirstRow.1,
        event_type: stFirstRow.2,
        topic_id: stFirstRow.3,
        comment_id: stFirstRow.4,
    };
    let stLast = StNotificationClickEvent {
        user_id: stLastRow.0,
        unread: stLastRow.1,
        event_type: stLastRow.2,
        topic_id: stLastRow.3,
        comment_id: stLastRow.4,
    };
    if user_id != stFirst.user_id || user_id != stLast.user_id {
        return Err(AppError::Forbidden);
    }

    if stLast.unread {
        if !bValidNotificationClickRange(form.first_id, &stFirst, form.last_id, &stLast) {
            return Err(AppError::stBadInput("invalid notification click range"));
        }
        let mut tx = state.pool.begin().await?;
        match stLast.event_type.as_str() {
            "WATCH" => {
                // UserEventDao.resetUnreadEvents scopes a grouped WATCH click
                // to ids at or below the rendered group's last event. A newer
                // concurrent event for the same topic must remain unread.
                sqlx::query(S_RESET_WATCH_NOTIFICATIONS)
                    .bind(user_id)
                    .bind(form.last_id)
                    .bind(stLast.topic_id)
                    .execute(&mut *tx)
                    .await?;
            }
            "REACTION" => {
                sqlx::query("UPDATE user_events SET unread=false WHERE userid=$1 AND unread AND type='REACTION'::event_type AND id BETWEEN $2 AND $3 AND message_id IS NOT DISTINCT FROM $4 AND comment_id IS NOT DISTINCT FROM $5")
                    .bind(user_id).bind(form.first_id).bind(form.last_id).bind(stLast.topic_id).bind(stLast.comment_id).execute(&mut *tx).await?;
            }
            _ => {
                sqlx::query(
                    "UPDATE user_events SET unread=false WHERE userid=$1 AND unread AND id=$2",
                )
                .bind(user_id)
                .bind(form.last_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        sqlx::query("UPDATE users SET unread_events=(SELECT count(*) FROM user_events e WHERE e.unread AND e.userid=users.id) WHERE id=$1")
            .bind(user_id).execute(&mut *tx).await?;
        tx.commit().await?;
        state.realtime.vNotifyEvents([user_id]);
    }

    match stFirst.topic_id {
        Some(iTopicId) => {
            topic_link(state, iTopicId, stFirst.comment_id, &stFirst.event_type).await
        }
        None => Ok("/notifications".to_string()),
    }
}

pub async fn notifications_click(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<NotificationsClickForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let url = process_notifications_click(&state, user.id, &form).await?;
    Ok((StatusCode::FOUND, [(axum::http::header::LOCATION, url)]).into_response())
}

pub async fn notifications_click_ajax(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<NotificationsClickForm>,
) -> Result<Json<serde_json::Value>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let url = process_notifications_click(&state, user.id, &form).await?;
    Ok(Json(json!({"url": url})))
}

#[derive(Deserialize)]
pub struct ActivationQuery {
    pub nick: Option<String>,
    pub activation: Option<String>,
}

pub async fn activate_form(
    Query(q): Query<ActivationQuery>,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let sNick = q
        .nick
        .as_deref()
        .filter(|sValue| valid_login_name_for_java(sValue))
        .unwrap_or("");
    let sActivation = q
        .activation
        .as_deref()
        .filter(|sValue| sValue.chars().all(char::is_alphanumeric))
        .unwrap_or("");
    render_activation_form(sNick, sActivation, None, &csrf_token, optUser.is_some())
}

pub async fn activate_post(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    jar: CookieJar,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    stRequest: Request,
) -> Result<impl IntoResponse> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let activation = crate::form::get(&vecParameters, "activation").ok_or_else(|| {
        AppError::BadRequest("Required request parameter 'activation' is missing".to_owned())
    })?;
    if crate::form::get(&vecParameters, "action").is_some() {
        // The `params = "action"` mapping binds nick/passwd as required
        // strings. Empty values are legal strings, but absent fields are a
        // Spring argument-binding 400 after CSRF.
        let nick = crate::form::get(&vecParameters, "nick").ok_or_else(|| {
            AppError::BadRequest("Required request parameter 'nick' is missing".to_owned())
        })?;
        let password = crate::form::get(&vecParameters, "passwd").ok_or_else(|| {
            AppError::BadRequest("Required request parameter 'passwd' is missing".to_owned())
        })?;
        let cIdentity =
            CUserIdentityService::new(CUserIdentityPgRepository::new(state.pool.clone()));
        let Some(stActivationUser) = cIdentity.optActivationIdentity(nick).await? else {
            return Ok(render_activation_form(
                nick,
                activation,
                Some("Пользователь не найден"),
                &csrf_token,
                false,
            )?
            .into_response());
        };

        if stActivationUser.bActivated {
            return Ok(crate::routes::stFoundRedirect("/"));
        }

        // Resolve and authenticate one exact identity.  Never combine the
        // password of one case-colliding row with another row's activation
        // token/update target.
        match crate::auth::verify_login(&state.pool, &stActivationUser.sNick, password).await? {
            crate::auth::LoginOutcome::NotActivated => {}
            crate::auth::LoginOutcome::Failed => {
                return Ok(render_activation_form(
                    nick,
                    activation,
                    Some("Неправильный логин или пароль"),
                    &csrf_token,
                    false,
                )?
                .into_response());
            }
            crate::auth::LoginOutcome::Blocked => {
                // Java lets the uncaught LockedException reach its global 500
                // exception resolver on this activation branch.
                return Err(AppError::Anyhow(anyhow::anyhow!(
                    "blocked user cannot be activated"
                )));
            }
            crate::auth::LoginOutcome::Success(_) => {
                return Ok(crate::routes::stFoundRedirect("/"));
            }
        }

        if !verify_activation_code(
            &state,
            &stActivationUser.sNick,
            stActivationUser.optEmail.as_deref().unwrap_or(""),
            stActivationUser.optRegistrationDate,
            activation,
        ) {
            return Ok(render_activation_form(
                nick,
                activation,
                Some("Неправильный код активации"),
                &csrf_token,
                false,
            )?
            .into_response());
        }

        sqlx::query("UPDATE users SET activated=true,lastlogin=now() WHERE id=$1")
            .bind(stActivationUser.iId)
            .execute(&state.pool)
            .await?;
        crate::audit::log_user_action(
            &state.pool,
            stActivationUser.iId,
            stActivationUser.iId,
            "register",
            &[],
        )
        .await?;
        let Some(stIdentity) =
            crate::auth::optLoadLoginIdentity(&state.pool, stActivationUser.iId).await?
        else {
            return Err(AppError::Anyhow(anyhow::anyhow!(
                "activated user cannot be loaded for remember-me cookie"
            )));
        };
        let cookie = Cookie::build((
            crate::security::remember_me::COOKIE_NAME,
            crate::auth::sMakeRememberMeCookieValue(&stIdentity, &state.config.site_secret),
        ))
        .path("/")
        .max_age(time::Duration::seconds(
            crate::security::remember_me::VALIDITY_SECONDS,
        ))
        .http_only(true)
        .secure(crate::security::is_secure_request(
            &headers,
            Some(stPeerAddress.ip()),
            &state.config.trusted_proxy_cidrs,
        ))
        .build();
        return Ok((jar.add(cookie), crate::routes::stFoundRedirect("/")).into_response());
    }

    let Some(user) = current_user else {
        return Err(AppError::Forbidden);
    };
    let Some((old_email, pending_email, regdate)) =
        sqlx::query_as::<
            _,
            (
                Option<String>,
                Option<String>,
                Option<chrono::DateTime<chrono::Utc>>,
            ),
        >("SELECT email,new_email,regdate FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_optional(&state.pool)
        .await?
    else {
        return Err(AppError::NotFound);
    };
    let Some(new_email) = pending_email else {
        // RegisterController throws AccessViolationException here; it is
        // mapped to the dedicated 403 page before the common exception view.
        return Err(AppError::Forbidden);
    };

    if !verify_activation_code(&state, &user.nick, &new_email, regdate, activation) {
        return Ok(render_activation_form(
            &user.nick,
            activation,
            Some("Неправильный код активации"),
            &csrf_token,
            true,
        )?
        .into_response());
    }

    let mut tx = state.pool.begin().await?;
    let stUpdate = sqlx::query(
        r#"UPDATE users SET email=$2,new_email=NULL
           WHERE id=$1 AND new_email IS NOT DISTINCT FROM $2"#,
    )
    .bind(user.id)
    .bind(&new_email)
    .execute(&mut *tx)
    .await?;
    if stUpdate.rows_affected() != 1 {
        return Err(AppError::Anyhow(anyhow::anyhow!(
            "pending email changed during activation"
        )));
    }
    let mut vecInfo = vec![("new_email", new_email.as_str())];
    if let Some(ref old_email) = old_email {
        vecInfo.push(("old_email", old_email.as_str()));
    }
    crate::audit::log_user_action_tx(&mut tx, user.id, user.id, "accept_new_email", &vecInfo)
        .await?;
    tx.commit().await?;
    Ok(crate::routes::stFoundRedirect(format!(
        "/people/{}/profile",
        urlencoding::encode(&user.nick)
    )))
}

fn render_activation_form(
    nick: &str,
    activation: &str,
    error: Option<&str>,
    csrf_token: &str,
    b_authenticated: bool,
) -> Result<Html<String>> {
    #[derive(Template)]
    #[template(path = "activate.html")]
    struct StActivateTemplate<'a> {
        sNick: &'a str,
        sActivation: &'a str,
        optError: Option<&'a str>,
        sCsrfToken: &'a str,
        bAuthenticated: bool,
    }

    Ok(Html(
        StActivateTemplate {
            sNick: nick,
            sActivation: activation,
            optError: error,
            sCsrfToken: csrf_token,
            bAuthenticated: b_authenticated,
        }
        .render()?,
    ))
}

#[cfg(test)]
mod activation_template_tests {
    use super::render_activation_form;
    use axum::response::Html;

    #[test]
    fn anonymous_activation_matches_java_form_and_uses_theme_shell() {
        let Html(sHtml) =
            render_activation_form("alice", "ABC123", Some("Ошибка"), "csrf-value", false)
                .expect("activation template");

        assert!(sHtml.contains("<!-- LOR_THEME_HEADER -->"));
        assert!(sHtml.contains("action=\"/activate.jsp\""));
        assert!(sHtml.contains("id=\"activateForm\" class=\"form-horizontal\""));
        assert!(sHtml.contains("name=\"action\" value=\"new\""));
        assert!(sHtml.contains("id=\"field_nick\" value=\"alice\""));
        assert!(sHtml.contains("id=\"field_password\""));
        assert!(sHtml.contains("id=\"field_code\" value=\"ABC123\""));
        assert!(sHtml.contains("<div class=\"error\">Ошибка</div>"));
    }

    #[test]
    fn authenticated_activation_only_asks_for_the_code() {
        let Html(sHtml) = render_activation_form("alice", "ABC123", None, "csrf-value", true)
            .expect("activation template");

        assert!(sHtml.contains("name=\"activation\" required autofocus id=\"field_code\""));
        assert!(!sHtml.contains("name=\"nick\""));
        assert!(!sHtml.contains("name=\"passwd\""));
        assert!(!sHtml.contains("name=\"action\""));
    }

    #[test]
    fn activation_binds_required_servlet_parameters_to_one_exact_identity() {
        let sSource = include_str!("legacy.rs");
        let sHandler = sSource
            .split(concat!("pub async fn ", "activate_post("))
            .nth(1)
            .unwrap()
            .split(concat!("fn ", "render_activation_form("))
            .next()
            .unwrap();
        assert!(sHandler.contains("servlet_request_parameters(stRequest)"));
        for sParameter in ["activation", "nick", "passwd"] {
            assert!(sHandler.contains(&format!(
                "Required request parameter '{sParameter}' is missing"
            )));
        }
        assert!(sHandler.contains("optActivationIdentity(nick)"));
        assert!(sHandler.contains("&stActivationUser.sNick, password"));
        assert!(sHandler.contains("&stActivationUser.sNick,"));
        assert!(sHandler.contains(".bind(stActivationUser.iId)"));
        assert!(!sHandler.contains("lower(nick)"));
        assert!(!sHandler.contains("nick.trim()"));
    }

    #[test]
    fn new_email_activation_cas_and_audit_share_one_transaction() {
        let sSource = include_str!("legacy.rs");
        let sHandler = sSource
            .split(concat!("pub async fn ", "activate_post("))
            .nth(1)
            .unwrap()
            .split(concat!("fn ", "render_activation_form("))
            .next()
            .unwrap();
        let sBranch = sHandler
            .split("let Some(user) = current_user else")
            .nth(1)
            .unwrap();

        assert!(sBranch.contains("SELECT email,new_email,regdate FROM users WHERE id=$1"));
        assert!(sBranch.contains("SET email=$2,new_email=NULL"));
        assert!(sBranch.contains("WHERE id=$1 AND new_email IS NOT DISTINCT FROM $2"));
        assert!(sBranch.contains(".bind(&new_email)"));
        assert!(sBranch.contains(".execute(&mut *tx)"));
        assert!(sBranch.contains("stUpdate.rows_affected() != 1"));
        assert!(sBranch.contains("(\"new_email\", new_email.as_str())"));
        assert!(sBranch.contains("(\"old_email\", old_email.as_str())"));
        assert!(sBranch.contains("log_user_action_tx(&mut tx,"));
        assert!(!sBranch.contains("SET email=new_email"));
        assert!(!sBranch.contains(".execute(&state.pool)"));
        assert!(!sBranch.contains("log_user_action(&state.pool"));

        let iVerify = sBranch.find("if !verify_activation_code").unwrap();
        let iBegin = sBranch.find("state.pool.begin().await?").unwrap();
        let iUpdate = sBranch.find("SET email=$2,new_email=NULL").unwrap();
        let iAudit = sBranch.find("log_user_action_tx(").unwrap();
        let iCommit = sBranch.find("tx.commit().await?").unwrap();
        assert!(iVerify < iBegin);
        assert!(iBegin < iUpdate);
        assert!(iUpdate < iAudit);
        assert!(iAudit < iCommit);
    }
}

fn verify_activation_code(
    state: &AppState,
    nick: &str,
    email: &str,
    regdate: Option<chrono::DateTime<chrono::Utc>>,
    supplied: &str,
) -> bool {
    if state.config.enable_dev_bypasses && supplied == "dev-activate" {
        return true;
    }
    let Some(regdate) = regdate else {
        return false;
    };
    crate::security::secret_tokens::verify_activation_code(
        &state.config.site_secret,
        nick,
        email,
        regdate.timestamp_millis(),
        supplied,
    )
}

pub async fn addphoto_form(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    vCheckLoadUserpic(&state, &user).await?;
    stRenderAddphoto(user.nick, csrf_token, None, StatusCode::OK)
}

pub async fn upload_userpic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    mut multipart: Multipart,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let cService = CUserpicService::new(
        CUserpicPgRepository::new(state.pool.clone()),
        state.config.upload_dir.clone(),
    );
    cService.vCheckUpload(user.id).await?;
    let mut optUpload: Option<bytes::Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|stError| AppError::BadRequest(format!("ошибка multipart: {stError}")))?
    {
        let bFile = field.name() == Some("file");
        let arrData = field
            .bytes()
            .await
            .map_err(|stError| AppError::BadRequest(format!("ошибка чтения файла: {stError}")))?;
        if bFile {
            optUpload = Some(arrData);
            break;
        }
    }
    let Some(arrData) = optUpload else {
        return stRenderAddphoto(
            user.nick,
            csrf_token,
            Some("изображение не задано".to_owned()),
            StatusCode::BAD_REQUEST,
        );
    };
    if arrData.is_empty() {
        // `MultipartFile.isEmpty` is handled before Java's try/catch and
        // therefore keeps the default 200 status while redisplaying the form.
        return stRenderAddphoto(
            user.nick,
            csrf_token,
            Some("изображение не задано".to_owned()),
            StatusCode::OK,
        );
    }

    if let Err(stError) = cService.sInstall(user.id, &arrData).await {
        return match stError {
            AppError::BadRequest(sMessage) => stRenderAddphoto(
                user.nick,
                csrf_token,
                Some(sMessage),
                StatusCode::BAD_REQUEST,
            ),
            stError => Err(stError),
        };
    }

    Ok(crate::routes::admin::stProfileRedirect(&user.nick))
}

#[derive(Template)]
#[template(path = "addphoto.html")]
struct StAddphotoTemplate {
    sNick: String,
    sCsrfToken: String,
    optError: Option<String>,
}

fn stRenderAddphoto(
    sNick: String,
    sCsrfToken: String,
    optError: Option<String>,
    stStatus: StatusCode,
) -> Result<Response> {
    let sBody = StAddphotoTemplate {
        sNick,
        sCsrfToken,
        optError,
    }
    .render()?;
    Ok((stStatus, Html(sBody)).into_response())
}

#[cfg(test)]
mod userpic_http_contract_tests {
    use axum::{body::to_bytes, http::StatusCode};

    use super::stRenderAddphoto;

    async fn sBody(stResponse: axum::response::Response) -> String {
        String::from_utf8(
            to_bytes(stResponse.into_body(), 128 * 1024)
                .await
                .expect("addphoto response body")
                .to_vec(),
        )
        .expect("UTF-8 addphoto response")
    }

    #[tokio::test]
    async fn empty_multipart_file_redisplays_the_themed_form_with_java_200() {
        let stResponse = stRenderAddphoto(
            "JB".to_owned(),
            "csrf".to_owned(),
            Some("изображение не задано".to_owned()),
            StatusCode::OK,
        )
        .expect("render empty upload");
        assert_eq!(stResponse.status(), StatusCode::OK);
        let sHtml = sBody(stResponse).await;
        assert!(sHtml.contains("<!-- LOR_THEME_HEADER -->"));
        assert!(sHtml.contains("Ошибка! изображение не задано"));
        assert!(sHtml.contains("action=\"addphoto.jsp\""));
        assert!(sHtml.contains("name=\"file\""));
    }

    #[tokio::test]
    async fn rejected_image_redisplays_the_same_form_with_java_400() {
        let stResponse = stRenderAddphoto(
            "JB".to_owned(),
            "csrf".to_owned(),
            Some("Invalid image".to_owned()),
            StatusCode::BAD_REQUEST,
        )
        .expect("render invalid upload");
        assert_eq!(stResponse.status(), StatusCode::BAD_REQUEST);
        assert!(sBody(stResponse).await.contains("Ошибка! Invalid image"));
    }
}

/// Exact `EditProfileChecker.checkLoadUserpic` policy. This must run on both
/// GET and POST because hiding the form alone does not protect the mutation.
pub(crate) async fn bCanLoadUserpic(
    stState: &AppState,
    stUser: &crate::models::UserSummary,
) -> Result<bool> {
    CUserpicService::new(
        CUserpicPgRepository::new(stState.pool.clone()),
        stState.config.upload_dir.clone(),
    )
    .bCanUpload(stUser.id)
    .await
}

async fn vCheckLoadUserpic(stState: &AppState, stUser: &crate::models::UserSummary) -> Result<()> {
    if !bCanLoadUserpic(stState, stUser).await? {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

#[derive(Template)]
#[template(path = "deregister.html")]
struct StDeregisterTemplate {
    csrf_token: String,
    captcha_site_key: String,
    errors: Vec<String>,
    accept_block: bool,
    accept_oneway: bool,
}

#[derive(Template)]
#[template(path = "action_done.html")]
struct StDeregisterDoneTemplate {
    message: String,
    big_message: Option<String>,
    link: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeregisterForm {
    pub password: Option<String>,
    #[serde(rename = "acceptBlock")]
    pub accept_block: Option<String>,
    #[serde(rename = "acceptOneway")]
    pub accept_oneway: Option<String>,
    #[serde(rename = "h-captcha-response")]
    pub captcha_response: Option<String>,
}

#[cfg(test)]
mod deregister_binding_tests {
    use super::DeregisterForm;

    #[test]
    fn accepts_only_the_java_bean_checkbox_names() {
        let stJava: DeregisterForm =
            serde_urlencoded::from_str("password=secret&acceptBlock=on&acceptOneway=on").unwrap();
        assert!(stJava.accept_block.is_some());
        assert!(stJava.accept_oneway.is_some());

        let stSnakeCase: DeregisterForm =
            serde_urlencoded::from_str("password=secret&accept_block=on&accept_oneway=on").unwrap();
        assert!(stSnakeCase.accept_block.is_none());
        assert!(stSnakeCase.accept_oneway.is_none());
    }
}

pub async fn deregister_form(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let cService = CUserAccountService::new(CUserAccountPgRepository::new(state.pool.clone()));
    cService.vCheckDeregister(user.id).await?;
    render_deregister_page(&state, csrf_token, Vec::new(), false, false)
}

pub async fn deregister_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    Form(form): Form<DeregisterForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let cService = CUserAccountService::new(CUserAccountPgRepository::new(state.pool.clone()));
    cService.vCheckDeregister(user.id).await?;

    let bAcceptBlock = form.accept_block.is_some();
    let bAcceptOneway = form.accept_oneway.is_some();
    let mut vecErrors = Vec::new();
    if !bAcceptBlock {
        vecErrors.push("Вы не согласились с блокировкой аккаунта".to_owned());
    }
    if !bAcceptOneway {
        vecErrors.push("Вы не согласились с невозможностью восстановления аккаунта".to_owned());
    }
    let bPasswordMatches = cService
        .bPasswordMatches(user.id, form.password.as_deref().unwrap_or(""))
        .await?;
    if !bPasswordMatches {
        vecErrors.push("Неверный пароль".to_owned());
    }
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    if let Err(sError) = crate::application::auth::sValidateCaptcha(
        &state.config,
        &state.http,
        form.captcha_response.as_deref(),
        &sRemoteIp,
    )
    .await
    {
        vecErrors.push(sError);
    }

    if !vecErrors.is_empty() {
        return Ok(render_deregister_page(
            &state,
            csrf_token,
            vecErrors,
            bAcceptBlock,
            bAcceptOneway,
        )?
        .into_response());
    }

    cService.vDeregister(user.id).await?;
    Ok(Html(
        StDeregisterDoneTemplate {
            message: "Удаление пользователя прошло успешно.".to_owned(),
            big_message: None,
            link: None,
        }
        .render()?,
    )
    .into_response())
}

fn render_deregister_page(
    state: &AppState,
    csrf_token: String,
    errors: Vec<String>,
    accept_block: bool,
    accept_oneway: bool,
) -> Result<Html<String>> {
    Ok(Html(
        StDeregisterTemplate {
            csrf_token,
            captcha_site_key: state.config.captcha_public_key.clone().unwrap_or_default(),
            errors,
            accept_block,
            accept_oneway,
        }
        .render()?,
    ))
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

#[derive(Deserialize)]
pub struct ForumPageOrArchiveQuery {
    offset: Option<i64>,
    filter: Option<String>,
}

fn optForumTopicPageMethodResponse(stMethod: &Method) -> Option<Response> {
    if stMethod == Method::OPTIONS {
        return Some((StatusCode::OK, [(header::ALLOW, "GET,HEAD,OPTIONS")]).into_response());
    }
    if stMethod != Method::GET && stMethod != Method::HEAD {
        return Some((StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "GET")]).into_response());
    }
    None
}

pub async fn forum_page_or_archive(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((group, id_or_year, page_or_month)): Path<(String, String, String)>,
    stMethod: Method,
    Query(q): Query<ForumPageOrArchiveQuery>,
    CurrentUser(current_user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<axum::response::Response> {
    if let Some(page) = page_or_month.strip_prefix("page") {
        if let Some(stResponse) = optForumTopicPageMethodResponse(&stMethod) {
            return Ok(stResponse);
        }
        let page: i64 = page.parse().map_err(|_| AppError::NotFound)?;
        let id: i32 = id_or_year.parse().map_err(|_| AppError::NotFound)?;
        let sRemoteIp = crate::security::stClientIp(
            stPeerAddress.ip(),
            &headers,
            &state.config.trusted_proxy_cidrs,
        )
        .to_string();
        return crate::routes::topics::render_topic_page(
            state,
            "forum",
            group,
            id,
            page,
            q.filter,
            headers,
            current_user,
            csrf_token,
            sRemoteIp,
        )
        .await;
    }

    // The calendar branch is a separate bare RequestMapping even though Axum
    // must share one dynamic path with the GET-only page branch.
    if stMethod == Method::OPTIONS {
        return Ok(crate::routes::stSpringUnrestrictedOptionsResponse());
    }

    let year: i32 = id_or_year.parse().map_err(|_| AppError::NotFound)?;
    let month: i32 = page_or_month.parse().map_err(|_| AppError::NotFound)?;
    let stResponse = forum_archive_month(
        State(state),
        Path((group, year, month)),
        Query(PagerQuery { offset: q.offset }),
        CurrentUser(current_user),
    )
    .await?
    .into_response();
    if matches!(stMethod, Method::PUT | Method::PATCH | Method::DELETE) {
        Ok(crate::routes::stSpringJspMethodNotAllowedResponse())
    } else {
        Ok(stResponse)
    }
}

#[cfg(test)]
mod forum_page_or_archive_method_tests {
    use super::*;

    #[test]
    fn page_shape_retains_spring_get_mapping_while_archive_shape_is_any() {
        assert!(optForumTopicPageMethodResponse(&Method::GET).is_none());
        assert!(optForumTopicPageMethodResponse(&Method::HEAD).is_none());

        let stOptions = optForumTopicPageMethodResponse(&Method::OPTIONS).unwrap();
        assert_eq!(stOptions.status(), StatusCode::OK);
        assert_eq!(
            stOptions.headers().get(header::ALLOW).unwrap(),
            "GET,HEAD,OPTIONS"
        );

        for stMethod in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            let stResponse = optForumTopicPageMethodResponse(&stMethod).unwrap();
            assert_eq!(stResponse.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert_eq!(stResponse.headers().get(header::ALLOW).unwrap(), "GET");
        }
    }

    #[test]
    fn archive_calendar_bounds_use_the_servlet_bad_parameter_contract() {
        assert!(matches!(
            validate_year_month(1989, 1),
            Err(AppError::BadParameter(ref sMessage))
                if sMessage == "Bad format of 'year' указан некорректный год"
        ));
        assert!(matches!(
            validate_year_month(2026, 13),
            Err(AppError::BadParameter(ref sMessage))
                if sMessage == "Bad format of 'month' указан некорректный месяц"
        ));
        assert!(validate_year_month(2026, 8).is_ok());
    }
}

fn validate_year_month(year: i32, month: i32) -> Result<()> {
    if !(1990..=3000).contains(&year) {
        return Err(AppError::BadParameter(
            "Bad format of 'year' указан некорректный год".into(),
        ));
    }
    if !(1..=12).contains(&month) {
        return Err(AppError::BadParameter(
            "Bad format of 'month' указан некорректный месяц".into(),
        ));
    }
    Ok(())
}

fn section_from_uri(uri: &Uri) -> Option<&'static str> {
    match uri.path().trim_start_matches('/').split('/').next()? {
        "forum" => Some("forum"),
        "news" => Some("news"),
        "articles" => Some("articles"),
        "gallery" => Some("gallery"),
        "polls" => Some("polls"),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnMemoryAction {
    Add { iTopicId: i32, bWatch: bool },
    Remove { iMemoryId: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnMemoryMapping {
    Add,
    Remove,
}

fn enMemoryMapping(vecParameters: &[(String, String)]) -> Result<EnMemoryMapping> {
    let bAdd = crate::form::get(vecParameters, "add").is_some();
    let bRemove = crate::form::get(vecParameters, "remove").is_some();
    match (bAdd, bRemove) {
        (true, false) => Ok(EnMemoryMapping::Add),
        (false, true) => Ok(EnMemoryMapping::Remove),
        // Spring raises UnsatisfiedServletRequestParameterException while
        // selecting the mapping; its default resolver emits the servlet 400
        // response before CSRF or controller argument binding.
        (false, false) => Err(AppError::BadRequest(
            "required add or remove mapping parameter is missing".to_owned(),
        )),
        (true, true) => Err(AppError::Anyhow(anyhow::anyhow!(
            "ambiguous memories add/remove handler"
        ))),
    }
}

fn iMemoryParameter(vecParameters: &[(String, String)], sName: &str) -> Result<i32> {
    crate::form::get(vecParameters, sName)
        .ok_or_else(|| AppError::BadRequest(format!("missing {sName}")))?
        .parse()
        .map_err(|_| AppError::BadRequest(format!("invalid {sName}")))
}

fn enMemoryAction(
    vecParameters: &[(String, String)],
    enMapping: EnMemoryMapping,
) -> Result<EnMemoryAction> {
    match enMapping {
        EnMemoryMapping::Remove => Ok(EnMemoryAction::Remove {
            iMemoryId: iMemoryParameter(vecParameters, "id")?,
        }),
        EnMemoryMapping::Add => {
            let sWatch = crate::form::get(vecParameters, "watch")
                .ok_or_else(|| AppError::BadRequest("missing watch".to_owned()))?;
            let bWatch = if sWatch.eq_ignore_ascii_case("true") {
                true
            } else if sWatch.eq_ignore_ascii_case("false") {
                false
            } else {
                return Err(AppError::BadRequest("invalid watch".to_owned()));
            };
            Ok(EnMemoryAction::Add {
                iTopicId: iMemoryParameter(vecParameters, "msgid")?,
                bWatch,
            })
        }
    }
}

/// MemoriesController.add/remove: "favorite" (watch=false) and "watch"
/// (watch=true) are independent rows per topic - `add` upserts the row for
/// the requested `watch` value only, `remove` deletes one specific row by
/// its own id (never the whole userid+topic pair), matching the frontend
/// contract in `static/js/lor/memories.js` (`{msgid,watch}` to add,
/// `{id}` to remove, JSON `{id,count}`/bare count responses).
pub async fn memories(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    stRequest: Request,
) -> Result<Json<serde_json::Value>> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    // RequestMapping selection precedes HandlerInterceptor execution. No
    // matching predicate is 400 and both predicates are an ambiguous 500 even
    // if the request has no CSRF field.
    let enMapping = enMemoryMapping(&vecParameters)?;
    // Once a mapping is selected, CSRF runs before @RequestParam conversion.
    if !crate::csrf::bServletCsrfValid(&vecParameters, &sCsrfToken) {
        return Err(AppError::Forbidden);
    }
    let enAction = enMemoryAction(&vecParameters, enMapping)?;

    if let EnMemoryAction::Remove { iMemoryId: id } = enAction {
        let Some(user) = user else {
            return Err(AppError::Forbidden);
        };
        let row: Option<(i32, i32, bool)> =
            sqlx::query_as("SELECT userid, topic, watch FROM memories WHERE id=$1")
                .bind(id)
                .fetch_optional(&state.pool)
                .await?;
        let Some((owner_id, topic_id, watch)) = row else {
            return Ok(Json(serde_json::json!(-1)));
        };
        if owner_id != user.id {
            return Err(AppError::Forbidden);
        }
        sqlx::query("DELETE FROM memories WHERE id=$1")
            .bind(id)
            .execute(&state.pool)
            .await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM memories WHERE topic=$1 AND watch=$2")
                .bind(topic_id)
                .bind(watch)
                .fetch_one(&state.pool)
                .await?;
        return Ok(Json(serde_json::json!(count)));
    }

    let EnMemoryAction::Add {
        iTopicId: msgid,
        bWatch: watch,
    } = enAction
    else {
        unreachable!("remove action returned above")
    };
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let deleted: bool = sqlx::query_scalar("SELECT deleted FROM topics WHERE id=$1")
        .bind(msgid)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    if deleted {
        return Err(AppError::stUserError("Тема удалена"));
    }
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO memories(userid,topic,watch) VALUES($1,$2,$3) ON CONFLICT(userid,topic,watch) DO UPDATE SET topic=EXCLUDED.topic RETURNING id",
    )
    .bind(user.id).bind(msgid).bind(watch)
    .fetch_one(&state.pool)
    .await?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM memories WHERE topic=$1 AND watch=$2")
            .bind(msgid)
            .bind(watch)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(serde_json::json!({"id": id, "count": count})))
}

#[cfg(test)]
mod memories_contract_tests {
    use super::{EnMemoryAction, EnMemoryMapping, enMemoryAction, enMemoryMapping};
    use crate::error::AppError;

    fn vecParameters(vecValues: &[(&str, &str)]) -> Vec<(String, String)> {
        vecValues
            .iter()
            .map(|(sName, sValue)| ((*sName).to_owned(), (*sValue).to_owned()))
            .collect()
    }

    #[test]
    fn mapping_conditions_are_resolved_before_csrf_or_binding() {
        assert!(matches!(enMemoryMapping(&[]), Err(AppError::BadRequest(_))));
        assert!(matches!(
            enMemoryMapping(&vecParameters(&[("add", ""), ("remove", "")])),
            Err(AppError::Anyhow(_))
        ));
    }

    #[test]
    fn selected_mapping_checks_csrf_before_required_binding() {
        let vecAdd = vecParameters(&[("add", ""), ("msgid", "42")]);
        assert_eq!(enMemoryMapping(&vecAdd).unwrap(), EnMemoryMapping::Add);
        assert!(!crate::csrf::bServletCsrfValid(&vecAdd, "token"));
        assert!(matches!(
            enMemoryAction(&vecAdd, EnMemoryMapping::Add),
            Err(AppError::BadRequest(_))
        ));

        let vecValidAdd = vecParameters(&[
            ("add", ""),
            ("msgid", "42"),
            ("watch", "FALSE"),
            ("csrf", "token"),
        ]);
        assert!(crate::csrf::bServletCsrfValid(&vecValidAdd, "token"));
        assert_eq!(
            enMemoryAction(&vecValidAdd, EnMemoryMapping::Add).expect("favorite action"),
            EnMemoryAction::Add {
                iTopicId: 42,
                bWatch: false
            }
        );

        let vecRemove = vecParameters(&[("remove", "")]);
        assert!(matches!(
            enMemoryAction(&vecRemove, EnMemoryMapping::Remove),
            Err(AppError::BadRequest(_))
        ));
        assert_eq!(
            enMemoryAction(
                &vecParameters(&[("remove", ""), ("id", "7")]),
                EnMemoryMapping::Remove
            )
            .expect("remove action"),
            EnMemoryAction::Remove { iMemoryId: 7 }
        );
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct StUserFilterQuery {
    #[serde(rename = "newFavoriteTagName")]
    pub optNewFavoriteTagName: Option<String>,
    #[serde(rename = "newIgnoreTagName")]
    pub optNewIgnoreTagName: Option<String>,
}

#[derive(Debug)]
struct StIgnoredUserRow {
    iId: i32,
    sNick: String,
    optRemark: Option<String>,
}

#[derive(Template)]
#[template(path = "user_filter.html")]
struct StUserFilterTemplate {
    vecIgnoredUsers: Vec<StIgnoredUserRow>,
    vecFavoriteTags: Vec<String>,
    vecIgnoreTags: Vec<String>,
    bModerator: bool,
    optNewFavoriteTagName: Option<String>,
    optNewIgnoreTagName: Option<String>,
    vecFavoriteErrors: Vec<String>,
    vecIgnoreErrors: Vec<String>,
    sCsrfToken: String,
}

async fn stRenderUserFilter(
    stState: &AppState,
    stUser: &crate::models::UserSummary,
    stQuery: StUserFilterQuery,
    vecFavoriteErrors: Vec<String>,
    vecIgnoreErrors: Vec<String>,
    sCsrfToken: String,
) -> Result<Response> {
    let vecIgnoredUsers = sqlx::query_as::<_, (i32, String, Option<String>)>(
        r#"SELECT u.id,u.nick,r.remark_text
             FROM ignore_list il
             JOIN users u ON u.id=il.ignored
             LEFT JOIN user_remarks r ON r.user_id=il.userid AND r.ref_user_id=il.ignored
            WHERE il.userid=$1 ORDER BY u.nick"#,
    )
    .bind(stUser.id)
    .fetch_all(&stState.pool)
    .await?
    .into_iter()
    .map(|(iId, sNick, optRemark)| StIgnoredUserRow {
        iId,
        sNick,
        optRemark,
    })
    .collect();
    let vecFavoriteTags = crate::routes::users::user_tags(stState, stUser.id, true).await?;
    let vecIgnoreTags = if stUser.canmod {
        Vec::new()
    } else {
        crate::routes::users::user_tags(stState, stUser.id, false).await?
    };
    let sHtml = StUserFilterTemplate {
        vecIgnoredUsers,
        vecFavoriteTags,
        vecIgnoreTags,
        bModerator: stUser.canmod,
        optNewFavoriteTagName: stQuery
            .optNewFavoriteTagName
            .filter(|sTag| crate::routes::tags::is_good_tag(sTag)),
        optNewIgnoreTagName: stQuery
            .optNewIgnoreTagName
            .filter(|sTag| crate::routes::tags::is_good_tag(sTag)),
        vecFavoriteErrors,
        vecIgnoreErrors,
        sCsrfToken,
    }
    .render()?;
    let mut stResponse = Html(sHtml).into_response();
    stResponse.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store, no-cache, must-revalidate".parse().unwrap(),
    );
    stResponse
        .headers_mut()
        .insert(header::PRAGMA, "no-cache".parse().unwrap());
    Ok(stResponse)
}

pub async fn user_filter(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Query(stQuery): Query<StUserFilterQuery>,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> Result<Response> {
    let stUser = optUser.ok_or(AppError::Forbidden)?;
    stRenderUserFilter(
        &stState,
        &stUser,
        stQuery,
        Vec::new(),
        Vec::new(),
        sCsrfToken,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnUserFilterMapping {
    Add,
    Remove,
}

fn enUserFilterMapping(vecParameters: &[(String, String)]) -> Result<EnUserFilterMapping> {
    match (
        crate::form::get(vecParameters, "add").is_some(),
        crate::form::get(vecParameters, "del").is_some(),
    ) {
        (true, false) => Ok(EnUserFilterMapping::Add),
        (false, true) => Ok(EnUserFilterMapping::Remove),
        (false, false) => Err(AppError::BadRequest(
            "required add or del mapping parameter is missing".to_owned(),
        )),
        (true, true) => Err(AppError::Anyhow(anyhow::anyhow!(
            "ambiguous user-filter add/delete handler"
        ))),
    }
}

fn sRequiredServletParameter(vecParameters: &[(String, String)], sName: &str) -> Result<String> {
    crate::form::get(vecParameters, sName)
        .map(ToOwned::to_owned)
        .ok_or_else(|| AppError::BadParameter(format!("missing {sName}")))
}

pub async fn favorite_tag(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    stRequest: Request,
) -> Result<Response> {
    user_tag_action(stState, optUser, sCsrfToken, stRequest, true).await
}

pub async fn ignore_tag(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    stRequest: Request,
) -> Result<Response> {
    user_tag_action(stState, optUser, sCsrfToken, stRequest, false).await
}

async fn user_tag_action(
    stState: AppState,
    optUser: Option<crate::models::UserSummary>,
    sCsrfToken: String,
    stRequest: Request,
    bFavorite: bool,
) -> Result<Response> {
    let stHeaders = stRequest.headers().clone();
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    // RequestMapping params selection precedes CSRF, binding and auth. Both
    // predicates are an ambiguous handler error and must never select delete.
    let enMapping = enUserFilterMapping(&vecParameters)?;
    if !crate::csrf::bServletCsrfValid(&vecParameters, &sCsrfToken) {
        return Err(AppError::Forbidden);
    }
    let sRawTag = sRequiredServletParameter(&vecParameters, "tagName")?;
    save_or_delete_user_tag(
        stState,
        optUser,
        stHeaders,
        sRawTag,
        matches!(enMapping, EnUserFilterMapping::Remove),
        bFavorite,
        sCsrfToken,
    )
    .await
}

async fn save_or_delete_user_tag(
    stState: AppState,
    optUser: Option<crate::models::UserSummary>,
    stHeaders: HeaderMap,
    sRawTag: String,
    bDelete: bool,
    bFavorite: bool,
    sCsrfToken: String,
) -> Result<Response> {
    let stUser = optUser.ok_or(AppError::Forbidden)?;
    if !bFavorite && stUser.canmod {
        return Err(AppError::Forbidden);
    }
    let bJson = bAcceptsJson(&stHeaders);

    // favoriteDel/ignoreDel and favoriteTagAddJSON call TagDao.getTagId with
    // the raw request value. TagDao uses exact `value=$tag` matching: case and
    // surrounding whitespace are observable and must never select a different
    // persisted tag. Only the HTML multi-add path normalizes via TagName.
    if bDelete {
        let Some(iTagId) =
            optMutateExactUserTag(&stState, stUser.id, &sRawTag, true, bFavorite, false).await?
        else {
            return Err(AppError::NotFound);
        };
        if bJson {
            let iCount = iUserTagCount(&stState, iTagId, bFavorite).await?;
            return Ok(Json(json!({"count": iCount})).into_response());
        }
        return Ok((StatusCode::FOUND, [(header::LOCATION, "/user-filter")]).into_response());
    }

    if bJson && bFavorite {
        let Some(iTagId) =
            optMutateExactUserTag(&stState, stUser.id, &sRawTag, false, true, true).await?
        else {
            return Ok(Json(json!({"error": "Tag not found"})).into_response());
        };
        let iCount = iUserTagCount(&stState, iTagId, true).await?;
        return Ok(Json(json!({"count": iCount})).into_response());
    }

    let vecErrors = vecAddMultipleUserTags(&stState, stUser.id, &sRawTag, bFavorite).await?;

    // ignoreTagAddJSON first runs addMultiplyTags (which normalizes and may
    // commit several tags), then performs one more exact lookup/add using the
    // original raw string. A failure in that second operation is returned as
    // JSON and does not roll the preceding per-tag transactions back.
    if bJson {
        if !vecErrors.is_empty() {
            return Ok(Json(json!({"error": vecErrors.join("; ")})).into_response());
        }
        let Some(iTagId) =
            optMutateExactUserTag(&stState, stUser.id, &sRawTag, false, false, false).await?
        else {
            return Ok(Json(json!({"error": "Tag not found"})).into_response());
        };
        let iCount = iUserTagCount(&stState, iTagId, false).await?;
        return Ok(Json(json!({"count": iCount})).into_response());
    }

    if vecErrors.is_empty() {
        return Ok((StatusCode::FOUND, [(header::LOCATION, "/user-filter")]).into_response());
    }
    let stQuery = if bFavorite {
        StUserFilterQuery {
            optNewFavoriteTagName: Some(sRawTag),
            optNewIgnoreTagName: None,
        }
    } else {
        StUserFilterQuery {
            optNewFavoriteTagName: None,
            optNewIgnoreTagName: Some(sRawTag),
        }
    };
    let (vecFavoriteErrors, vecIgnoreErrors) = if bFavorite {
        (vecErrors, Vec::new())
    } else {
        (Vec::new(), vecErrors)
    };
    stRenderUserFilter(
        &stState,
        &stUser,
        stQuery,
        vecFavoriteErrors,
        vecIgnoreErrors,
        sCsrfToken,
    )
    .await
}

const S_USER_TAG_ID_EXACT: &str = "SELECT id FROM tags_values WHERE value=$1";
const S_ACTIVE_USER_TAG_ID_EXACT: &str = "SELECT id FROM tags_values WHERE value=$1 AND counter>0";

fn stParseUserTagList(sRawTags: &str) -> (Vec<String>, Vec<String>) {
    let mut vecGoodTags = Vec::new();
    let mut vecErrors = Vec::new();
    for sTag in crate::routes::tags::parse_tags(sRawTags) {
        let iJavaLength = sTag.encode_utf16().count();
        if iJavaLength <= 32 && crate::routes::tags::is_good_tag(&sTag) {
            vecGoodTags.push(sTag);
        } else {
            // parseAndValidateTags supplies the human text as defaultMessage
            // with a null errorCode, but errorsToStringList reads getCode().
            // Spring 6.2.19 resolves that code to the empty object name, so
            // the observable list element is an empty string.
            vecErrors.push(String::new());
        }
    }
    if vecErrors.is_empty() && vecGoodTags.is_empty() {
        vecErrors.push(String::new());
    }
    (vecGoodTags, vecErrors)
}

async fn optMutateExactUserTag(
    stState: &AppState,
    iUserId: i32,
    sTag: &str,
    bDelete: bool,
    bFavorite: bool,
    bSkipZero: bool,
) -> Result<Option<i32>> {
    let mut stTransaction = stState.pool.begin().await?;
    let sLookup = if bSkipZero {
        S_ACTIVE_USER_TAG_ID_EXACT
    } else {
        S_USER_TAG_ID_EXACT
    };
    let optTagId: Option<i32> = sqlx::query_scalar(sLookup)
        .bind(sTag)
        .fetch_optional(&mut *stTransaction)
        .await?;
    let Some(iTagId) = optTagId else {
        return Ok(None);
    };
    if bDelete {
        sqlx::query("DELETE FROM user_tags WHERE user_id=$1 AND tag_id=$2 AND is_favorite=$3")
            .bind(iUserId)
            .bind(iTagId)
            .bind(bFavorite)
            .execute(&mut *stTransaction)
            .await?;
    } else {
        sqlx::query("INSERT INTO user_tags(user_id,tag_id,is_favorite) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(iUserId)
            .bind(iTagId)
            .bind(bFavorite)
            .execute(&mut *stTransaction)
            .await?;
    }
    stTransaction.commit().await?;
    Ok(Some(iTagId))
}

async fn vecAddMultipleUserTags(
    stState: &AppState,
    iUserId: i32,
    sRawTags: &str,
    bFavorite: bool,
) -> Result<Vec<String>> {
    let (vecTags, mut vecErrors) = stParseUserTagList(sRawTags);
    for sTag in vecTags {
        if optMutateExactUserTag(stState, iUserId, &sTag, false, bFavorite, bFavorite)
            .await?
            .is_none()
        {
            vecErrors.push(format!("Tag not found: '{sTag}'"));
        }
    }
    Ok(vecErrors)
}

async fn iUserTagCount(stState: &AppState, iTagId: i32, bFavorite: bool) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM user_tags WHERE tag_id=$1 AND is_favorite=$2")
            .bind(iTagId)
            .bind(bFavorite)
            .fetch_one(&stState.pool)
            .await?,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EnIgnoreUserAction {
    Add { sNick: String },
    Remove { iUserId: i32 },
}

fn enIgnoreUserAction(
    vecParameters: &[(String, String)],
    enMapping: EnUserFilterMapping,
) -> Result<EnIgnoreUserAction> {
    match enMapping {
        EnUserFilterMapping::Add => Ok(EnIgnoreUserAction::Add {
            sNick: sRequiredServletParameter(vecParameters, "nick")?,
        }),
        EnUserFilterMapping::Remove => {
            let sId = sRequiredServletParameter(vecParameters, "id")?;
            Ok(EnIgnoreUserAction::Remove {
                iUserId: sId
                    .parse()
                    .map_err(|_| AppError::BadParameter("invalid id".to_owned()))?,
            })
        }
    }
}

pub async fn ignore_user(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
    stRequest: Request,
) -> Result<Response> {
    let vecParameters = crate::form::servlet_request_parameters(stRequest).await?;
    let enMapping = enUserFilterMapping(&vecParameters)?;
    if !crate::csrf::bServletCsrfValid(&vecParameters, &sCsrfToken) {
        return Err(AppError::Forbidden);
    }
    let enAction = enIgnoreUserAction(&vecParameters, enMapping)?;
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    // UserFilterController.listAdd/listDel: the personal user-ignore list
    // has no moderator restriction at all - only ignore-*tags* is
    // moderator-restricted (moderators must see every tag), see
    // ignore_tag below.
    let (ignored_id, bDelete): (i32, bool) = match enAction {
        EnIgnoreUserAction::Add { sNick } => (
            sqlx::query_scalar("SELECT id FROM users WHERE nick=$1")
                .bind(&sNick)
                .fetch_optional(&state.pool)
                .await?
                .ok_or_else(|| AppError::stBadInput("указанный пользователь не существует"))?,
            false,
        ),
        EnIgnoreUserAction::Remove { iUserId } => {
            let bExists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=$1)")
                    .bind(iUserId)
                    .fetch_one(&state.pool)
                    .await?;
            if !bExists {
                return Err(AppError::NotFound);
            }
            (iUserId, true)
        }
    };
    if ignored_id == user.id {
        return Err(AppError::stBadInput("нельзя игнорировать самого себя"));
    }
    if bDelete {
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
    Ok((StatusCode::FOUND, [(header::LOCATION, "/user-filter")]).into_response())
}

#[cfg(test)]
mod user_filter_dispatch_tests {
    use super::{
        EnIgnoreUserAction, EnUserFilterMapping, S_ACTIVE_USER_TAG_ID_EXACT, S_USER_TAG_ID_EXACT,
        enIgnoreUserAction, enUserFilterMapping, sRequiredServletParameter, stParseUserTagList,
    };
    use crate::error::AppError;

    #[test]
    fn add_delete_parameter_mappings_fail_closed_on_none_or_ambiguity() {
        assert!(matches!(
            enUserFilterMapping(&[]),
            Err(AppError::BadRequest(_))
        ));
        assert_eq!(
            enUserFilterMapping(&[("add".to_owned(), String::new())]).unwrap(),
            EnUserFilterMapping::Add
        );
        assert_eq!(
            enUserFilterMapping(&[("del".to_owned(), String::new())]).unwrap(),
            EnUserFilterMapping::Remove
        );
        assert!(matches!(
            enUserFilterMapping(&[
                ("add".to_owned(), String::new()),
                ("del".to_owned(), String::new()),
            ]),
            Err(AppError::Anyhow(_))
        ));
    }

    #[test]
    fn ignore_user_binds_only_the_selected_mappings_arguments() {
        assert_eq!(
            enIgnoreUserAction(
                &[("nick".to_owned(), "alice".to_owned())],
                EnUserFilterMapping::Add,
            )
            .unwrap(),
            EnIgnoreUserAction::Add {
                sNick: "alice".to_owned()
            }
        );
        assert_eq!(
            enIgnoreUserAction(
                &[("id".to_owned(), "42".to_owned())],
                EnUserFilterMapping::Remove,
            )
            .unwrap(),
            EnIgnoreUserAction::Remove { iUserId: 42 }
        );
        assert!(matches!(
            enIgnoreUserAction(
                &[("id".to_owned(), "not-an-id".to_owned())],
                EnUserFilterMapping::Remove,
            ),
            Err(AppError::BadParameter(_))
        ));
    }

    #[test]
    fn required_arguments_keep_spring_bad_parameter_status_family() {
        assert!(matches!(
            sRequiredServletParameter(&[], "tagName"),
            Err(AppError::BadParameter(_))
        ));
    }

    #[test]
    fn only_html_multi_add_normalizes_and_preserves_java_error_codes() {
        assert_eq!(
            stParseUserTagList(" Linux | rust, linux "),
            (vec!["linux".to_owned(), "rust".to_owned()], Vec::new())
        );
        assert_eq!(stParseUserTagList(""), (Vec::new(), vec![String::new()]));
        assert_eq!(
            stParseUserTagList("linux,<bad>"),
            (vec!["linux".to_owned()], vec![String::new()])
        );
        assert_eq!(
            stParseUserTagList(&"𐐀".repeat(17)),
            (Vec::new(), vec![String::new()])
        );
    }

    #[test]
    fn json_and_delete_lookups_are_exact_and_favorite_add_can_skip_zero() {
        assert_eq!(
            S_USER_TAG_ID_EXACT,
            "SELECT id FROM tags_values WHERE value=$1"
        );
        assert_eq!(
            S_ACTIVE_USER_TAG_ID_EXACT,
            "SELECT id FROM tags_values WHERE value=$1 AND counter>0"
        );
        assert!(!S_USER_TAG_ID_EXACT.contains("lower"));
        assert!(!S_ACTIVE_USER_TAG_ID_EXACT.contains("lower"));
    }
}

fn bAcceptsJson(stHeaders: &HeaderMap) -> bool {
    stHeaders
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|stValue| stValue.to_str().ok())
        .any(|sValue| {
            sValue
                .split(',')
                .any(|sMediaType| sMediaType.trim().starts_with("application/json"))
        })
}

#[derive(Deserialize)]
pub struct StSetPostScoreQuery {
    pub msgid: Option<String>,
}

#[derive(Deserialize)]
pub struct StSetPostScoreForm {
    pub msgid: Option<String>,
    pub postscore: Option<String>,
    pub sticky: Option<String>,
    pub notop: Option<String>,
}

#[derive(Template)]
#[template(path = "set_post_score.html")]
struct StSetPostScoreTemplate {
    csrf_token: String,
    topic_id: i32,
    postscore: i32,
    sticky: bool,
    notop: bool,
    premoderated: bool,
}

#[derive(Template)]
#[template(path = "set_post_score_done.html")]
struct StSetPostScoreDoneTemplate {
    big_message: String,
    link: String,
}

#[derive(Template)]
#[template(path = "set_post_score_user_error.html")]
struct StSetPostScoreUserErrorTemplate {
    message: String,
}

fn bSpringRequestBoolean(optValue: Option<&str>, sName: &str) -> Result<bool> {
    match optValue.map(str::to_ascii_lowercase).as_deref() {
        None | Some("") | Some("false") | Some("off") | Some("no") | Some("0") => Ok(false),
        Some("true") | Some("on") | Some("yes") | Some("1") => Ok(true),
        Some(_) => Err(AppError::BadRequest(format!(
            "Некорректное значение параметра `{sName}`"
        ))),
    }
}

fn iSpringRequiredInt(optValue: Option<&str>, sName: &str) -> Result<i32> {
    optValue
        .ok_or_else(|| AppError::BadRequest(format!("Required parameter '{sName}' is missing")))?
        .parse::<i32>()
        .map_err(|_| AppError::BadRequest(format!("Failed to convert parameter '{sName}'")))
}

fn stTopicOptionsService(
    stState: &AppState,
) -> crate::application::topic::options::CTopicOptionsService<
    crate::infra::postgres::topic_options_repository::CTopicOptionsPgRepository,
    crate::infra::search_queue::CSearchQueueSender,
> {
    crate::application::topic::options::CTopicOptionsService::new(
        crate::infra::postgres::topic_options_repository::CTopicOptionsPgRepository::new(
            stState.pool.clone(),
        ),
        crate::infra::search_queue::CSearchQueueSender::new(
            stState.config.opensearch_url.as_deref(),
            &stState.config.upload_dir,
        ),
    )
}

fn stSetPostScoreUserErrorResponse(sMessage: String) -> Response {
    let sBody = StSetPostScoreUserErrorTemplate { message: sMessage }
        .render()
        .unwrap_or_else(|_| "Внутренняя ошибка сервера".to_owned());
    (StatusCode::INTERNAL_SERVER_ERROR, Html(sBody)).into_response()
}

pub async fn set_post_score_form(
    State(stState): State<AppState>,
    Query(stQuery): Query<StSetPostScoreQuery>,
    CurrentUser(optUser): CurrentUser,
    crate::csrf::CsrfToken(sCsrfToken): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    let iTopicId = iSpringRequiredInt(stQuery.msgid.as_deref(), "msgid")?;
    let stOptions = stTopicOptionsService(&stState)
        .stForm(optUser.as_ref(), iTopicId)
        .await?;
    Ok(Html(
        StSetPostScoreTemplate {
            csrf_token: sCsrfToken,
            topic_id: stOptions.iTopicId,
            postscore: stOptions.iPostScore,
            sticky: stOptions.bSticky,
            notop: stOptions.bNoTop,
            premoderated: stOptions.bPremoderated,
        }
        .render()?,
    ))
}

pub async fn set_post_score(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Form(stForm): Form<StSetPostScoreForm>,
) -> Result<Response> {
    let iTopicId = iSpringRequiredInt(stForm.msgid.as_deref(), "msgid")?;
    let iPostScore = iSpringRequiredInt(stForm.postscore.as_deref(), "postscore")?;
    let bSticky = bSpringRequestBoolean(stForm.sticky.as_deref(), "sticky")?;
    let bNoTop = bSpringRequestBoolean(stForm.notop.as_deref(), "notop")?;
    let stOutcome = match stTopicOptionsService(&stState)
        .stSet(
            optUser.as_ref(),
            crate::domain::topic::options::StSetTopicOptions {
                iTopicId,
                iPostScore,
                bSticky,
                bNoTop,
            },
        )
        .await
    {
        Ok(stOutcome) => stOutcome,
        // UserErrorException is deliberately rendered by Java's common error
        // resolver with HTTP 500, while binding failures above remain the
        // separate Spring HTTP 400 contract.
        Err(AppError::BadRequest(sMessage)) => {
            return Ok(stSetPostScoreUserErrorResponse(sMessage));
        }
        Err(stError) => return Err(stError),
    };
    Ok(Html(
        StSetPostScoreDoneTemplate {
            big_message: stOutcome.sBigMessage,
            link: stOutcome.sCanonicalUrl,
        }
        .render()?,
    )
    .into_response())
}

#[cfg(test)]
mod set_post_score_tests {
    use super::*;

    #[test]
    fn spring_checkbox_values_and_empty_defaults_are_preserved() {
        for optValue in [
            None,
            Some(""),
            Some("false"),
            Some("off"),
            Some("no"),
            Some("0"),
        ] {
            assert!(!bSpringRequestBoolean(optValue, "sticky").unwrap());
        }
        for optValue in [Some("true"), Some("on"), Some("yes"), Some("1"), Some("ON")] {
            assert!(bSpringRequestBoolean(optValue, "sticky").unwrap());
        }
        assert!(matches!(
            bSpringRequestBoolean(Some("invalid"), "sticky"),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn spring_integer_binding_is_a_400_validation_error() {
        for optValue in [None, Some("x"), Some("2147483648")] {
            assert!(matches!(
                iSpringRequiredInt(optValue, "msgid"),
                Err(AppError::BadRequest(_))
            ));
        }
        assert_eq!(iSpringRequiredInt(Some("42"), "msgid").unwrap(), 42);
    }
}

#[derive(Deserialize)]
pub struct ImageForm {
    pub id: i32,
}

#[derive(Template)]
#[template(path = "delete_image.html")]
struct StDeleteImageTemplate {
    csrf_token: String,
    image_id: i32,
    topic_title: String,
    medium_url: String,
    original_url: String,
    medium_width: i32,
    medium_height: i32,
    max_width: i32,
    padding: f64,
    srcset: String,
    linked: bool,
}

pub async fn delete_image_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ImageForm>,
    CurrentUser(user): CurrentUser,
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
) -> Result<Html<String>> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let stForm = state.image_delete.stForm(&user, q.id, &sRemoteIp).await?;
    let stImage = stForm.stImage;
    Ok(Html(
        StDeleteImageTemplate {
            csrf_token,
            image_id: stImage.iId,
            topic_title: crate::domain::title::sTopicTitlePlainForDisplay(
                &stForm.stTarget.sTopicTitle,
            ),
            medium_url: stImage.sMediumUrl,
            original_url: stImage.sOriginalUrl,
            medium_width: stImage.iMediumWidth,
            medium_height: stImage.iMediumHeight,
            max_width: stImage.iWidth.min(2000),
            padding: 100.0 * f64::from(stImage.iMediumHeight) / f64::from(stImage.iMediumWidth),
            srcset: stImage.sSrcSet,
            linked: stForm.stTarget.bSectionImagePost
                || stImage.iWidth >= 1920
                || stImage.iHeight >= 1080,
        }
        .render()?,
    ))
}

pub async fn delete_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    CurrentUser(user): CurrentUser,
    ConnectInfo(stPeerAddress): ConnectInfo<SocketAddr>,
    Form(form): Form<ImageForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    let sRemoteIp = crate::security::stClientIp(
        stPeerAddress.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )
    .to_string();
    let sRedirect = state
        .image_delete
        .sDelete(&user, form.id, &sRemoteIp)
        .await?;
    Ok(stLegacyFoundRedirect(sRedirect))
}

#[derive(Deserialize)]
pub struct RemoveUserpicForm {
    pub id: Option<i32>,
}

pub async fn remove_userpic(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<RemoveUserpicForm>,
) -> Result<Response> {
    let Some(user) = user else {
        return Err(AppError::Forbidden);
    };
    // Java declares `id` as a required @RequestParam; it does not default to
    // the current user when the field is missing.
    let iTargetUserId = form.id.ok_or(AppError::NotFound)?;
    let cService = crate::application::user::CUserModerationService::new(
        crate::infra::postgres::user_moderation_repository::CUserModerationPgRepository::new(
            state.pool.clone(),
        ),
        state.config.scheduler_timezone,
    );
    let sTargetNick = cService.sResetUserpic(&user, iTargetUserId).await?;
    Ok(crate::routes::admin::stProfileRedirect(&sTargetNick))
}

pub async fn reset_password_form(
    crate::csrf::CsrfToken(csrf_token): crate::csrf::CsrfToken,
) -> Result<Html<String>> {
    crate::routes::auth::render_reset_password_form(csrf_token, None)
}

#[cfg(test)]
mod explicit_error_parity_tests {
    #[test]
    fn legacy_handlers_keep_java_exception_families() {
        let sSource = include_str!("legacy.rs");

        let sShowReplies = sSource
            .split(concat!("pub async fn ", "show_replies_jsp("))
            .nth(1)
            .unwrap()
            .split(concat!("fn ", "render_replies_feed("))
            .next()
            .unwrap();
        assert_eq!(
            sShowReplies
                .matches(concat!(
                    "AppError::st",
                    "BadInput(\"некорректное имя пользователя\")"
                ))
                .count(),
            2
        );

        let sNotificationClick = sSource
            .split(concat!("async fn ", "process_notifications_click("))
            .nth(1)
            .unwrap()
            .split(concat!("pub async fn ", "notifications_click("))
            .next()
            .unwrap();
        assert!(sNotificationClick.contains(concat!(
            "AppError::stBadInput(\"invalid notification ",
            "click range\")"
        )));

        let sActivation = sSource
            .split(concat!("pub async fn ", "activate_post("))
            .nth(1)
            .unwrap()
            .split(concat!("fn ", "render_activation_form("))
            .next()
            .unwrap();
        let sNewEmailBranch = sActivation
            .split("let Some(new_email) = pending_email else")
            .nth(1)
            .unwrap()
            .split("if !verify_activation_code")
            .next()
            .unwrap();
        assert!(sNewEmailBranch.contains("return Err(AppError::Forbidden)"));
        assert!(!sNewEmailBranch.contains("AppError::BadRequest"));

        let sMemories = sSource
            .split(concat!("pub async fn ", "memories("))
            .nth(1)
            .unwrap()
            .split(concat!("#[cfg(test)]", "\nmod memories_contract_tests"))
            .next()
            .unwrap();
        assert!(sMemories.contains(concat!("AppError::stUser", "Error(\"Тема удалена\")")));

        let sIgnoreUser = sSource
            .split(concat!("pub async fn ", "ignore_user("))
            .nth(1)
            .unwrap()
            .split(concat!("#[cfg(test)]", "\nmod user_filter_dispatch_tests"))
            .next()
            .unwrap();
        assert_eq!(sIgnoreUser.matches("AppError::stBadInput").count(), 2);
        assert!(!sIgnoreUser.contains("AppError::BadRequest"));
    }
}
