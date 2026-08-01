use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::Value;

/// 统一的错误结构：错误码 + 错误信息 + 可选详情。
#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// 应用级错误类型，携带映射到 HTTP 状态码的信息。
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub error: ApiError,
}

impl AppError {
    fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        AppError {
            status,
            error: ApiError {
                code: code.to_string(),
                message: message.into(),
                details: None,
            },
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.error.details = Some(details);
        self
    }

    /// 400 输入校验失败。
    pub fn validation(message: impl Into<String>) -> Self {
        AppError::new(StatusCode::BAD_REQUEST, "VALIDATION_ERROR", message)
    }

    /// 400 非法的状态流转。
    pub fn invalid_transition(message: impl Into<String>) -> Self {
        AppError::new(StatusCode::BAD_REQUEST, "INVALID_TRANSITION", message)
    }

    /// 404 资源不存在。
    pub fn not_found(message: impl Into<String>) -> Self {
        AppError::new(StatusCode::NOT_FOUND, "NOT_FOUND", message)
    }

    /// 409 资源冲突（编号重复 / 位置被占用等）。
    pub fn conflict(message: impl Into<String>) -> Self {
        AppError::new(StatusCode::CONFLICT, "CONFLICT", message)
    }

    /// 500 内部错误。
    pub fn internal(message: impl Into<String>) -> Self {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            message,
        )
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(self.error)).into_response()
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::internal(format!("database error: {err}"))
    }
}

impl From<r2d2::Error> for AppError {
    fn from(err: r2d2::Error) -> Self {
        AppError::internal(format!("connection pool error: {err}"))
    }
}
