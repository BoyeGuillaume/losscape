use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use losscape_core::proto::v0;

use crate::proto_body::Proto;

/// Error returned to clients as a protobuf `Error` with a stable machine-readable code.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// A document was saved based on a version that isn't the latest any more.
    pub fn stale(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "stale", message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }

    pub fn unknown_run() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "unknown_run",
            "no run with this id; call init first",
        )
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_unavailable",
            message,
        )
    }

    /// Too much going on right now: worth retrying soon.
    pub fn busy() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "the server is busy; try again shortly",
        )
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal server error",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Proto(v0::Error {
            code: self.code.to_owned(),
            message: self.message,
        });
        (self.status, body).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(error = %err, "postgres error");
        Self::unavailable("metadata store unavailable")
    }
}

impl From<clickhouse::error::Error> for ApiError {
    fn from(err: clickhouse::error::Error) -> Self {
        tracing::error!(error = %err, "clickhouse error");
        Self::unavailable("metric store unavailable")
    }
}
