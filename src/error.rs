use askama::Template;
use axum::{http::StatusCode, response::{Html, IntoResponse, Response}};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
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

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, title) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "Не найдено"),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "Доступ запрещён"),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "Некорректный запрос"),
            AppError::Sqlx(sqlx::Error::RowNotFound) => (StatusCode::NOT_FOUND, "Не найдено"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "Ошибка"),
        };
        let body = ErrorTemplate { code: status.as_u16(), title, message: self.to_string() }
            .render()
            .unwrap_or_else(|_| format!("{} {}", status.as_u16(), title));
        (status, Html(body)).into_response()
    }
}
