use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,

    #[error("etag mismatch")]
    EtagMismatch { current: String },

    #[error("invalid path")]
    InvalidPath,

    #[error("unauthorized")]
    Unauthorized,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("db: {0}")]
    Db(#[from] sqlx::Error),

    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, json!({"error": "not_found"})),
            AppError::EtagMismatch { current } => (
                StatusCode::PRECONDITION_FAILED,
                json!({"error": "etag_mismatch", "current_etag": current}),
            ),
            AppError::InvalidPath => (
                StatusCode::BAD_REQUEST,
                json!({"error": "invalid_path"}),
            ),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                json!({"error": "unauthorized"}),
            ),
            AppError::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                json!({"error": "bad_request", "message": msg}),
            ),
            AppError::Io(e) => {
                tracing::error!(error = ?e, "io error");
                (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "io"}))
            }
            AppError::Db(e) => {
                tracing::error!(error = ?e, "db error");
                (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "db"}))
            }
            AppError::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "internal"}))
            }
        };
        (status, Json(body)).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
