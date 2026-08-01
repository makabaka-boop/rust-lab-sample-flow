use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::json;
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone)]
pub struct AppError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AppError {}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl AppError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: message.into(),
            details: None,
        }
    }

    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "INVALID_STATE_TRANSITION",
            message: message.into(),
            details: None,
        }
    }

    pub fn not_found(resource: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND",
            message: resource.into(),
            details: None,
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "CONFLICT",
            message: message.into(),
            details: None,
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "BAD_REQUEST",
            message: message.into(),
            details: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error: ErrorBody {
                code: self.code,
                message: self.message,
                details: self.details,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db_error) = &error {
            if db_error.is_unique_violation() {
                return AppError::conflict("资源已存在，唯一字段不能重复");
            }
            if db_error.is_foreign_key_violation() {
                return AppError::validation("关联资源不存在或仍被其他数据引用");
            }
        }
        tracing::error!(?error, "database error");
        AppError::internal("数据库操作失败")
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        AppError::bad_request(format!("JSON 解析失败: {error}"))
    }
}

pub fn json_rejection(error: axum::extract::rejection::JsonRejection) -> AppError {
    AppError::bad_request(format!("请求体必须是合法 JSON: {error}"))
        .with_details(json!({ "expected_content_type": "application/json" }))
}

pub fn path_rejection(error: axum::extract::rejection::PathRejection) -> AppError {
    AppError::validation(format!("路径参数无效: {error}"))
}

pub fn query_rejection(error: axum::extract::rejection::QueryRejection) -> AppError {
    AppError::validation(format!("查询参数无效: {error}"))
}
