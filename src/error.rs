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

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate<'a> {
    code: u16,
    title: &'a str,
    message: String,
}

impl AppError {
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
        let (status, title, message) = self.response_parts();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = ?self, "request failed with an internal error");
        }
        let body = ErrorTemplate {
            code: status.as_u16(),
            title,
            message,
        }
        .render()
        .unwrap_or_else(|_| format!("{} {}", status.as_u16(), title));
        (status, Html(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_error_text_is_not_exposed_to_the_client() {
        let error = AppError::Anyhow(anyhow::anyhow!("postgres password=secret"));
        let (status, _, message) = error.response_parts();

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(message, "Внутренняя ошибка сервера");
        assert!(!message.contains("secret"));
    }

    #[test]
    fn validation_error_remains_visible_to_the_client() {
        let error = AppError::BadRequest("неверное имя поля".to_string());
        let (status, _, message) = error.response_parts();

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(message, "неверное имя поля");
    }
}
