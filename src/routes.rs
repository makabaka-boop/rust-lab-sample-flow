use std::sync::{Arc, Mutex, MutexGuard};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use rusqlite::Connection;

use crate::error::ApiError;
use crate::models::*;
use crate::service;

pub type AppState = Arc<Mutex<Connection>>;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        // 批次
        .route("/api/batches", post(create_batch).get(list_batches))
        .route("/api/batches/:batch_no", get(get_batch))
        .route("/api/batches/:batch_no/samples", get(list_batch_samples))
        .route(
            "/api/batches/:batch_no/samples/bulk-import",
            post(bulk_import),
        )
        // 样本
        .route("/api/samples", post(register_sample).get(query_samples))
        .route("/api/samples/:sample_no", get(get_sample))
        .route("/api/samples/:sample_no/status", put(update_status))
        .route("/api/samples/:sample_no/location", put(change_location))
        // 存放位置查询（当前占用样本 + 最近 10 次迁入迁出）
        .route("/api/locations", get(get_location))
        .route(
            "/api/samples/:sample_no/exceptions",
            post(add_exception).get(list_exceptions),
        )
        .route(
            "/api/samples/:sample_no/exceptions/:exception_id/resolve",
            post(resolve_exception),
        )
        .route("/api/samples/:sample_no/logs", get(list_logs))
        // 未匹配路径 / 方法不允许时同样返回统一 JSON 错误
        .fallback(fallback_not_found)
        .method_not_allowed_fallback(fallback_method_not_allowed)
        .with_state(state)
}

async fn fallback_not_found() -> ApiError {
    ApiError::not_found("ROUTE_NOT_FOUND", "请求的路径不存在")
}

async fn fallback_method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "METHOD_NOT_ALLOWED",
        "该路径不支持此请求方法",
    )
}

fn lock(state: &AppState) -> Result<MutexGuard<'_, Connection>, ApiError> {
    state
        .lock()
        .map_err(|_| ApiError::internal("internal state lock poisoned"))
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

// ---------- 批次 ----------

async fn create_batch(
    State(state): State<AppState>,
    payload: Result<Json<CreateBatchReq>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(req) = payload?;
    let conn = lock(&state)?;
    let batch = service::create_batch(&conn, &req)?;
    Ok((StatusCode::CREATED, Json(batch)))
}

async fn list_batches(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let conn = lock(&state)?;
    let batches = service::list_batches(&conn)?;
    Ok(Json(batches))
}

async fn get_batch(
    State(state): State<AppState>,
    Path(batch_no): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let conn = lock(&state)?;
    let batch = service::get_batch(&conn, &batch_no)?;
    Ok(Json(batch))
}

async fn list_batch_samples(
    State(state): State<AppState>,
    Path(batch_no): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let conn = lock(&state)?;
    let samples = service::list_batch_samples(&conn, &batch_no)?;
    Ok(Json(samples))
}

// ---------- 样本 ----------

async fn register_sample(
    State(state): State<AppState>,
    payload: Result<Json<RegisterSampleReq>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(req) = payload?;
    let conn = lock(&state)?;
    let sample = service::register_sample(&conn, &req)?;
    Ok((StatusCode::CREATED, Json(sample)))
}

async fn bulk_import(
    State(state): State<AppState>,
    Path(batch_no): Path<String>,
    payload: Result<Json<BulkImportReq>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(req) = payload?;
    let mut conn = lock(&state)?;
    let result = service::bulk_import(&mut conn, &batch_no, &req)?;
    Ok((StatusCode::CREATED, Json(result)))
}

async fn get_sample(
    State(state): State<AppState>,
    Path(sample_no): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let conn = lock(&state)?;
    let sample = service::get_sample(&conn, &sample_no)?;
    Ok(Json(sample))
}

async fn query_samples(
    State(state): State<AppState>,
    query: Result<Query<SampleFilter>, axum::extract::rejection::QueryRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Query(filter) = query?;
    let conn = lock(&state)?;
    let samples = service::query_samples(&conn, &filter)?;
    Ok(Json(samples))
}

// ---------- 状态与位置 ----------

async fn update_status(
    State(state): State<AppState>,
    Path(sample_no): Path<String>,
    payload: Result<Json<UpdateStatusReq>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(req) = payload?;
    let conn = lock(&state)?;
    let sample = service::update_status(&conn, &sample_no, &req)?;
    Ok(Json(sample))
}

async fn change_location(
    State(state): State<AppState>,
    Path(sample_no): Path<String>,
    payload: Result<Json<ChangeLocationReq>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(req) = payload?;
    let conn = lock(&state)?;
    let sample = service::change_location(&conn, &sample_no, &req)?;
    Ok(Json(sample))
}

// ---------- 存放位置查询 ----------

async fn get_location(
    State(state): State<AppState>,
    query: Result<Query<LocationQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Query(q) = query?;
    let conn = lock(&state)?;
    let detail = service::get_location_detail(&conn, &q)?;
    Ok(Json(detail))
}

// ---------- 异常标记 ----------

async fn add_exception(
    State(state): State<AppState>,
    Path(sample_no): Path<String>,
    payload: Result<Json<AddExceptionReq>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(req) = payload?;
    let conn = lock(&state)?;
    let flag = service::add_exception(&conn, &sample_no, &req)?;
    Ok((StatusCode::CREATED, Json(flag)))
}

async fn list_exceptions(
    State(state): State<AppState>,
    Path(sample_no): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let conn = lock(&state)?;
    let flags = service::list_exceptions(&conn, &sample_no)?;
    Ok(Json(flags))
}

async fn resolve_exception(
    State(state): State<AppState>,
    path: Result<Path<(String, i64)>, axum::extract::rejection::PathRejection>,
    payload: Result<Json<ResolveExceptionReq>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Path((sample_no, exception_id)) = path?;
    let Json(req) = payload?;
    let conn = lock(&state)?;
    let flag = service::resolve_exception(&conn, &sample_no, exception_id, &req)?;
    Ok(Json(flag))
}

// ---------- 流转日志 ----------

async fn list_logs(
    State(state): State<AppState>,
    Path(sample_no): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let conn = lock(&state)?;
    let logs = service::list_logs(&conn, &sample_no)?;
    Ok(Json(logs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_app() -> Router {
        let conn = db::init_memory();
        router(Arc::new(Mutex::new(conn)))
    }

    async fn error_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes)
            .expect("错误响应必须是合法 JSON");
        (status, body)
    }

    fn json_request(method: &str, uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn unknown_path_returns_unified_404() {
        let resp = test_app()
            .oneshot(json_request("GET", "/api/nope", ""))
            .await
            .unwrap();
        let (status, body) = error_body(resp).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "ROUTE_NOT_FOUND");
        assert!(body["error"]["message"].is_string());
    }

    #[tokio::test]
    async fn wrong_method_returns_unified_405() {
        // /api/batches 只支持 GET/POST，DELETE 应返回 405
        let resp = test_app()
            .oneshot(json_request("DELETE", "/api/batches", ""))
            .await
            .unwrap();
        let (status, body) = error_body(resp).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["error"]["code"], "METHOD_NOT_ALLOWED");
    }

    #[tokio::test]
    async fn invalid_exception_id_format_returns_unified_400() {
        let resp = test_app()
            .oneshot(json_request(
                "POST",
                "/api/samples/S0001/exceptions/abc/resolve",
                r#"{"operator":"bob"}"#,
            ))
            .await
            .unwrap();
        let (status, body) = error_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "INVALID_PATH_PARAM");
    }

    #[tokio::test]
    async fn malformed_query_string_returns_unified_400() {
        // 重复的同名单值参数无法反序列化
        let resp = test_app()
            .oneshot(json_request("GET", "/api/samples?status=stored&status=archived", ""))
            .await
            .unwrap();
        let (status, body) = error_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "INVALID_QUERY_PARAM");
    }

    #[tokio::test]
    async fn empty_query_value_returns_unified_400() {
        let resp = test_app()
            .oneshot(json_request("GET", "/api/samples?status=", ""))
            .await
            .unwrap();
        let (status, body) = error_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    }

    #[tokio::test]
    async fn whitespace_operator_returns_unified_400() {
        let app = test_app();
        // 先建批次
        let resp = app
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/batches",
                r#"{"batch_no":"B001","project_name":"p","manager":"m"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp = app
            .oneshot(json_request(
                "POST",
                "/api/samples",
                r#"{"sample_no":"S001","batch_no":"B001","sample_type":"blood","operator":"   "}"#,
            ))
            .await
            .unwrap();
        let (status, body) = error_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    }

    #[tokio::test]
    async fn location_endpoint_returns_occupant_and_movements() {
        let app = test_app();
        for (method, uri, body) in [
            ("POST", "/api/batches", r#"{"batch_no":"B001","project_name":"p","manager":"m"}"#),
            ("POST", "/api/samples", r#"{"sample_no":"S001","batch_no":"B001","sample_type":"blood","operator":"alice"}"#),
            ("POST", "/api/samples", r#"{"sample_no":"S002","batch_no":"B001","sample_type":"tissue","operator":"alice"}"#),
            ("PUT", "/api/samples/S001/location", r#"{"region":"A区","freezer_no":"F-01","shelf":"1","slot":"A1","operator":"alice"}"#),
        ] {
            let resp = app.clone().oneshot(json_request(method, uri, body)).await.unwrap();
            assert!(resp.status().is_success(), "{method} {uri}");
        }

        // 占用冲突：S002 迁入已被 S001 占用的位置
        let resp = app
            .clone()
            .oneshot(json_request(
                "PUT",
                "/api/samples/S002/location",
                r#"{"region":"A区","freezer_no":"F-01","shelf":"1","slot":"A1","operator":"alice"}"#,
            ))
            .await
            .unwrap();
        let (status, body) = error_body(resp).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "LOCATION_OCCUPIED");

        // 查询位置（A区 百分号编码为 A%E5%8C%BA）：当前样本 + 迁入记录
        let resp = app
            .clone()
            .oneshot(json_request(
                "GET",
                "/api/locations?region=A%E5%8C%BA&freezer_no=F-01&shelf=1&slot=A1",
                "",
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["current_sample"]["sample_no"], "S001");
        assert_eq!(body["recent_movements"].as_array().unwrap().len(), 1);
        assert_eq!(body["recent_movements"][0]["direction"], "in");
        assert!(body["recent_movements"][0]["from_location"].is_null());

        // 缺少四元组参数 → 统一 400
        let resp = app
            .oneshot(json_request("GET", "/api/locations?region=A%E5%8C%BA", ""))
            .await
            .unwrap();
        let (status, body) = error_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    }
}
