use crate::error::{path_rejection, query_rejection, AppError};
use crate::extractors::ApiJson;
use crate::models::{
    BulkImportRequest, CreateAnomalyRequest, CreateBatchRequest, CreateLocationRequest,
    LocationFilter, RegisterSampleRequest, ResolveAnomalyRequest, SampleFilter,
    UpdateLocationRequest, UpdateStatusRequest,
};
use crate::service;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;

type AppState = State<SqlitePool>;

pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

pub async fn create_batch(
    State(pool): AppState,
    ApiJson(request): ApiJson<CreateBatchRequest>,
) -> Result<impl IntoResponse, AppError> {
    let batch = service::create_batch(&pool, request).await?;
    Ok((StatusCode::CREATED, Json(batch)))
}

pub async fn list_batches(State(pool): AppState) -> Result<impl IntoResponse, AppError> {
    let batches = service::list_batches(&pool).await?;
    Ok(Json(json!({ "items": batches })))
}

pub async fn get_batch(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(id) = path.map_err(path_rejection)?;
    let batch = service::get_batch(&pool, id).await?;
    Ok(Json(batch))
}

pub async fn create_location(
    State(pool): AppState,
    ApiJson(request): ApiJson<CreateLocationRequest>,
) -> Result<impl IntoResponse, AppError> {
    let location = service::create_location(&pool, request).await?;
    Ok((StatusCode::CREATED, Json(location)))
}

pub async fn list_locations(
    State(pool): AppState,
    result: Result<Query<LocationFilter>, axum::extract::rejection::QueryRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Query(filter) = result.map_err(query_rejection)?;
    let locations = service::list_locations(&pool, filter).await?;
    Ok(Json(json!({ "items": locations })))
}

pub async fn get_location_detail(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(id) = path.map_err(path_rejection)?;
    let detail = service::get_location_detail(&pool, id).await?;
    Ok(Json(detail))
}

pub async fn register_sample(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
    ApiJson(request): ApiJson<RegisterSampleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Path(batch_id) = path.map_err(path_rejection)?;
    let sample = service::register_sample(&pool, batch_id, request).await?;
    Ok((StatusCode::CREATED, Json(sample)))
}

pub async fn bulk_import_samples(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
    ApiJson(request): ApiJson<BulkImportRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Path(batch_id) = path.map_err(path_rejection)?;
    let samples = service::bulk_import_samples(&pool, batch_id, request).await?;
    Ok((StatusCode::CREATED, Json(json!({ "items": samples, "count": samples.len() }))))
}

#[derive(Debug, Deserialize, Default)]
pub struct ListSamplesQuery {
    pub batch_id: Option<i64>,
    pub status: Option<String>,
    pub location_id: Option<i64>,
    pub area: Option<String>,
    pub fridge_number: Option<String>,
    pub shelf: Option<String>,
    pub slot: Option<String>,
}

pub async fn list_samples(
    State(pool): AppState,
    result: Result<Query<ListSamplesQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Query(query) = result.map_err(query_rejection)?;
    let filter = SampleFilter {
        batch_id: query.batch_id,
        status: query.status,
        location_id: query.location_id,
        area: query.area,
        fridge_number: query.fridge_number,
        shelf: query.shelf,
        slot: query.slot,
    };
    let samples = service::list_samples(&pool, filter).await?;
    Ok(Json(json!({ "items": samples, "count": samples.len() })))
}

#[derive(Debug, Deserialize, Default)]
pub struct BatchSamplesQuery {
    pub status: Option<String>,
    pub location_id: Option<i64>,
    pub area: Option<String>,
    pub fridge_number: Option<String>,
    pub shelf: Option<String>,
    pub slot: Option<String>,
}

pub async fn list_samples_by_batch(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
    result: Result<Query<BatchSamplesQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(batch_id) = path.map_err(path_rejection)?;
    let Query(query) = result.map_err(query_rejection)?;
    service::get_batch(&pool, batch_id).await?;
    let filter = SampleFilter {
        batch_id: Some(batch_id),
        status: query.status,
        location_id: query.location_id,
        area: query.area,
        fridge_number: query.fridge_number,
        shelf: query.shelf,
        slot: query.slot,
    };
    let samples = service::list_samples(&pool, filter).await?;
    Ok(Json(json!({ "items": samples, "count": samples.len() })))
}

pub async fn get_sample(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(id) = path.map_err(path_rejection)?;
    let sample = service::get_sample(&pool, id).await?;
    Ok(Json(sample))
}

pub async fn update_sample_status(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
    ApiJson(request): ApiJson<UpdateStatusRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Path(sample_id) = path.map_err(path_rejection)?;
    let sample = service::update_sample_status(&pool, sample_id, request).await?;
    Ok(Json(sample))
}

pub async fn update_sample_location(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
    ApiJson(request): ApiJson<UpdateLocationRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Path(sample_id) = path.map_err(path_rejection)?;
    let sample = service::update_sample_location(&pool, sample_id, request).await?;
    Ok(Json(sample))
}

pub async fn get_sample_flow(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Path(sample_id) = path.map_err(path_rejection)?;
    let (sample, logs, anomalies) = service::list_sample_flow(&pool, sample_id).await?;
    Ok(Json(json!({
        "sample": sample,
        "operation_logs": logs,
        "anomalies": anomalies
    })))
}

pub async fn create_anomaly(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
    ApiJson(request): ApiJson<CreateAnomalyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Path(sample_id) = path.map_err(path_rejection)?;
    let anomaly = service::create_anomaly(&pool, sample_id, request).await?;
    Ok((StatusCode::CREATED, Json(anomaly)))
}

#[derive(Debug, Deserialize, Default)]
pub struct ListAnomaliesQuery {
    pub sample_id: Option<i64>,
    pub resolved: Option<bool>,
}

pub async fn list_anomalies(
    State(pool): AppState,
    result: Result<Query<ListAnomaliesQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Query(query) = result.map_err(query_rejection)?;
    let anomalies = service::list_anomalies(&pool, query.sample_id, query.resolved).await?;
    Ok(Json(json!({ "items": anomalies, "count": anomalies.len() })))
}

pub async fn resolve_anomaly(
    State(pool): AppState,
    path: Result<Path<i64>, axum::extract::rejection::PathRejection>,
    ApiJson(request): ApiJson<ResolveAnomalyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Path(anomaly_id) = path.map_err(path_rejection)?;
    let anomaly = service::resolve_anomaly(&pool, anomaly_id, request).await?;
    Ok(Json(anomaly))
}
