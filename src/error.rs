use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    /// Spring binding/servlet-parameter failures mapped to
    /// `errors/bad-parameter.jsp`.  The original page deliberately uses 404,
    /// which is distinct from the port's explicit HTTP 400 validation errors.
    #[error("bad parameter: {0}")]
    BadParameter(String),
    #[error("too many requests: {0}")]
    TooManyRequests(String),
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("template error: {0}")]
    Template(#[from] askama::Error),
    #[error("internal error: {0}")]
    Anyhow(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Clone)]
pub struct StInternalErrorReport {
    pub sType: String,
    pub sDebug: String,
}

#[derive(Template)]
#[template(path = "error.html")]
struct StCommonErrorTemplate<'a> {
    title: &'a str,
    message: &'a str,
    bBadParameter: bool,
    bInternal: bool,
}

#[derive(Template)]
#[template(path = "error_403.html")]
struct StForbiddenErrorTemplate;

#[derive(Template)]
#[template(path = "error_404.html")]
struct StNotFoundErrorTemplate;

impl AppError {
    fn sInternalType(&self) -> &'static str {
        match self {
            AppError::Sqlx(_) => "sqlx::Error",
            AppError::Io(_) => "std::io::Error",
            AppError::Template(_) => "askama::Error",
            AppError::Anyhow(_) => "anyhow::Error",
            _ => "AppError",
        }
    }

    fn response_parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            AppError::NotFound | AppError::Sqlx(sqlx::Error::RowNotFound) => (
                StatusCode::NOT_FOUND,
                "Не найдено",
                "Запрошенная страница не найдена".to_string(),
            ),
            AppError::Forbidden => (
                StatusCode::FORBIDDEN,
                "Доступ запрещён",
                "Доступ запрещён".to_string(),
            ),
            AppError::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                "Некорректный запрос",
                message.clone(),
            ),
            AppError::BadParameter(message) => (
                StatusCode::NOT_FOUND,
                "Некорректные параметры",
                message.clone(),
            ),
            AppError::TooManyRequests(message) => (
                StatusCode::TOO_MANY_REQUESTS,
                "Попробуйте позже",
                message.clone(),
            ),
            AppError::Sqlx(_) | AppError::Io(_) | AppError::Template(_) | AppError::Anyhow(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Ошибка",
                "Внутренняя ошибка сервера".to_string(),
            ),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let bBadParameter = matches!(&self, AppError::BadParameter(_));
        let (status, title, message) = self.response_parts();
        let optReport =
            (status == StatusCode::INTERNAL_SERVER_ERROR).then(|| StInternalErrorReport {
                sType: self.sInternalType().to_owned(),
                sDebug: format!("{self:?}"),
            });
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            // Request-derived values can reach an error's Debug representation.
            // The sanitized exception reporter remains the diagnostic channel;
            // never duplicate the raw error (and possible credentials) in logs.
            tracing::error!(
                error_type = self.sInternalType(),
                "request failed with an internal error"
            );
        }
        let body = match status {
            _ if bBadParameter => StCommonErrorTemplate {
                title,
                message: &message,
                bBadParameter: true,
                bInternal: false,
            }
            .render(),
            StatusCode::NOT_FOUND => StNotFoundErrorTemplate.render(),
            StatusCode::FORBIDDEN => StForbiddenErrorTemplate.render(),
            _ => StCommonErrorTemplate {
                title,
                message: &message,
                bBadParameter: false,
                bInternal: status == StatusCode::INTERNAL_SERVER_ERROR,
            }
            .render(),
        }
        .unwrap_or_else(|_| format!("{} {}", status.as_u16(), title));
        let mut stResponse = (status, Html(body)).into_response();
        if let Some(stReport) = optReport {
            stResponse.extensions_mut().insert(stReport);
        }
        stResponse
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::header};

    #[tokio::test]
    async fn internal_error_text_is_not_exposed_to_the_client() {
        let error = AppError::Anyhow(anyhow::anyhow!("postgres password=secret"));
        let (status, _, message) = error.response_parts();

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(message, "Внутренняя ошибка сервера");
        assert!(!message.contains("secret"));

        let stResponse = error.into_response();
        assert_eq!(stResponse.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            stResponse
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|stValue| stValue.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("error page body");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8 error page");
        assert!(sBody.contains("К сожалению, произошла исключительная ситуация"));
        assert!(sBody.contains("Администраторы получили об этом сигнал"));
        assert!(!sBody.contains("password=secret"));
        assert!(!sBody.contains("anyhow::Error"));
        assert!(!sBody.contains("stack"));
    }

    #[test]
    fn validation_error_remains_visible_to_the_client() {
        let error = AppError::BadRequest("неверное имя поля".to_string());
        let (status, _, message) = error.response_parts();

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(message, "неверное имя поля");
    }

    #[tokio::test]
    async fn request_derived_validation_message_is_html_escaped() {
        let stResponse =
            AppError::BadRequest("<img src=x onerror=alert(1)>".to_owned()).into_response();
        assert_eq!(stResponse.status(), StatusCode::BAD_REQUEST);
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("validation error body");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8 error page");
        assert!(sBody.contains("onerror=alert(1)"));
        assert!(!sBody.contains("<img src=x onerror=alert(1)>"));
    }

    #[tokio::test]
    async fn spring_binding_error_keeps_bad_parameter_404_page() {
        let stResponse =
            AppError::BadParameter("Не задан параметр group".to_owned()).into_response();
        assert_eq!(stResponse.status(), StatusCode::NOT_FOUND);
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("bad parameter body");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8 error page");
        assert!(sBody.contains("Не задан параметр group"));
        assert!(sBody.contains("Скрипту, генерирующему страничку"));
        assert!(!sBody.contains("good-penguin.png"));
    }

    #[tokio::test]
    async fn not_found_uses_the_original_bilingual_warning_page() {
        let stResponse = AppError::NotFound.into_response();
        assert_eq!(stResponse.status(), StatusCode::NOT_FOUND);
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("404 body");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8 error page");
        assert!(sBody.contains("<title>Error 404</title>"));
        assert!(sBody.contains("id=\"warning-body\""));
        assert!(sBody.contains("/img/good-penguin.png"));
        assert!(sBody.contains("Запрошенный Вами URL не был найден"));
        assert!(sBody.contains("The URL you requested was not found"));
    }

    #[tokio::test]
    async fn forbidden_uses_the_original_warning_page() {
        let stResponse = AppError::Forbidden.into_response();
        assert_eq!(stResponse.status(), StatusCode::FORBIDDEN);
        let vecBody = to_bytes(stResponse.into_body(), 128 * 1024)
            .await
            .expect("403 body");
        let sBody = String::from_utf8(vecBody.to_vec()).expect("UTF-8 error page");
        assert!(sBody.contains("<title>Error 403</title>"));
        assert!(sBody.contains("<h1>403 Forbidden</h1>"));
        // code403.jsp computes a local scriptlet variable for logging but its
        // JSTL expression reads a scoped `message` attribute, which the
        // controller does not set.  Preserve the resulting source behavior.
        assert!(sBody.contains("<p>.</p>"));
        assert!(!sBody.contains("Доступ запрещён"));
    }
}
