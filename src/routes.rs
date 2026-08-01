use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde_json::json;

use crate::db::DbPool;
use crate::error::{ApiError, AppError};
use crate::models::*;
use crate::service;

/// 构建应用路由。
pub fn build_router(pool: DbPool) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/batches", post(create_batch))
        .route("/api/batches/{batch_no}/samples", post(register_sample))
        .route(
            "/api/batches/{batch_no}/samples/bulk-import",
            post(bulk_import),
        )
        .route("/api/samples", get(query_samples))
        .route("/api/samples/{sample_no}", get(get_sample))
        .route("/api/samples/{sample_no}/status", patch(update_status))
        .route("/api/samples/{sample_no}/location", patch(change_location))
        .route("/api/samples/{sample_no}/logs", get(sample_logs))
        .route("/api/samples/{sample_no}/exceptions", post(mark_exception))
        .route(
            "/api/exceptions/{exception_id}/resolve",
            patch(resolve_exception),
        )
        .route("/api/locations", get(get_location))
        .fallback(fallback)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(pool)
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

// 统一处理 JSON 解析错误。
fn map_json_rejection(rej: JsonRejection) -> AppError {
    AppError::validation(format!("请求体 JSON 解析失败: {rej}"))
}

fn map_query_rejection(rej: QueryRejection) -> AppError {
    AppError::validation(format!("查询参数解析失败: {rej}"))
}

fn map_path_rejection(rej: PathRejection) -> AppError {
    AppError::validation(format!("路径参数解析失败: {rej}"))
}

async fn create_batch(
    State(pool): State<DbPool>,
    payload: Result<Json<CreateBatchRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = payload.map_err(map_json_rejection)?;
    let batch = service::create_batch(&pool, req)?;
    Ok((StatusCode::CREATED, Json(batch)))
}

async fn register_sample(
    State(pool): State<DbPool>,
    batch_no: Result<Path<String>, PathRejection>,
    payload: Result<Json<RegisterSampleRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(batch_no) = batch_no.map_err(map_path_rejection)?;
    let Json(req) = payload.map_err(map_json_rejection)?;
    let sample = service::register_sample(&pool, &batch_no, req)?;
    Ok((StatusCode::CREATED, Json(sample)))
}

async fn bulk_import(
    State(pool): State<DbPool>,
    batch_no: Result<Path<String>, PathRejection>,
    payload: Result<Json<BulkImportRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(batch_no) = batch_no.map_err(map_path_rejection)?;
    let Json(req) = payload.map_err(map_json_rejection)?;
    let resp = service::bulk_import(&pool, &batch_no, req)?;
    Ok((StatusCode::CREATED, Json(resp)))
}

async fn query_samples(
    State(pool): State<DbPool>,
    q: Result<Query<SampleQuery>, QueryRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Query(q) = q.map_err(map_query_rejection)?;
    let samples = service::query_samples(&pool, q)?;
    Ok(Json(json!({ "total": samples.len(), "samples": samples })))
}

async fn get_sample(
    State(pool): State<DbPool>,
    sample_no: Result<Path<String>, PathRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(sample_no) = sample_no.map_err(map_path_rejection)?;
    let sample = service::get_sample(&pool, &sample_no)?;
    Ok(Json(sample))
}

async fn update_status(
    State(pool): State<DbPool>,
    sample_no: Result<Path<String>, PathRejection>,
    payload: Result<Json<UpdateStatusRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(sample_no) = sample_no.map_err(map_path_rejection)?;
    let Json(req) = payload.map_err(map_json_rejection)?;
    let sample = service::update_status(&pool, &sample_no, req)?;
    Ok(Json(sample))
}

async fn change_location(
    State(pool): State<DbPool>,
    sample_no: Result<Path<String>, PathRejection>,
    payload: Result<Json<ChangeLocationRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(sample_no) = sample_no.map_err(map_path_rejection)?;
    let Json(req) = payload.map_err(map_json_rejection)?;
    let sample = service::change_location(&pool, &sample_no, req)?;
    Ok(Json(sample))
}

async fn sample_logs(
    State(pool): State<DbPool>,
    sample_no: Result<Path<String>, PathRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(sample_no) = sample_no.map_err(map_path_rejection)?;
    let logs = service::sample_logs(&pool, &sample_no)?;
    Ok(Json(json!({ "sample_no": sample_no, "logs": logs })))
}

async fn mark_exception(
    State(pool): State<DbPool>,
    sample_no: Result<Path<String>, PathRejection>,
    payload: Result<Json<MarkExceptionRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(sample_no) = sample_no.map_err(map_path_rejection)?;
    let Json(req) = payload.map_err(map_json_rejection)?;
    let ex = service::mark_exception(&pool, &sample_no, req)?;
    Ok((StatusCode::CREATED, Json(ex)))
}

async fn resolve_exception(
    State(pool): State<DbPool>,
    exception_id: Result<Path<i64>, PathRejection>,
    payload: Result<Json<ResolveExceptionRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(exception_id) = exception_id.map_err(map_path_rejection)?;
    let Json(req) = payload.map_err(map_json_rejection)?;
    let ex = service::resolve_exception(&pool, exception_id, req)?;
    Ok(Json(ex))
}

async fn get_location(
    State(pool): State<DbPool>,
    q: Result<Query<SampleQuery>, QueryRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Query(q) = q.map_err(map_query_rejection)?;
    let view = service::get_location(&pool, q)?;
    Ok(Json(view))
}

// 404：未匹配任何路由。
async fn fallback() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            code: "NOT_FOUND".to_string(),
            message: "接口不存在".to_string(),
            details: None,
        }),
    )
}

// 405：方法不允许。
async fn method_not_allowed() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(ApiError {
            code: "METHOD_NOT_ALLOWED".to_string(),
            message: "请求方法不被允许".to_string(),
            details: None,
        }),
    )
}
