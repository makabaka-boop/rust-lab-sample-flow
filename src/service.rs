use crate::db;
use crate::error::AppError;
use crate::models::{
    Anomaly, Batch, BulkImportRequest, CreateAnomalyRequest, CreateBatchRequest,
    CreateLocationRequest, Location, LocationDetail, LocationFilter, OperationLog,
    RegisterSampleRequest, ResolveAnomalyRequest, Sample, SampleFilter, SampleStatus,
    UpdateLocationRequest, UpdateStatusRequest,
};
use crate::validation::{
    max_length, required, validate_batch, validate_location_input, validate_register_sample,
    validate_status_request,
};
use serde_json::json;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::str::FromStr;

const ALLOWED_ANOMALY_TYPES: [&str; 4] = ["contamination", "label_missing", "temperature_abnormal", "other"];

pub async fn create_batch(pool: &SqlitePool, request: CreateBatchRequest) -> Result<Batch, AppError> {
    validate_batch(&request)?;
    db::create_batch(pool, &request).await
}

pub async fn list_batches(pool: &SqlitePool) -> Result<Vec<Batch>, AppError> {
    db::list_batches(pool).await
}

pub async fn get_batch(pool: &SqlitePool, id: i64) -> Result<Batch, AppError> {
    db::get_batch(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("批次不存在"))
}

pub async fn create_location(
    pool: &SqlitePool,
    request: CreateLocationRequest,
) -> Result<Location, AppError> {
    let input = crate::models::LocationInput {
        area: request.area,
        fridge_number: request.fridge_number,
        shelf: request.shelf,
        slot: request.slot,
    };
    validate_location_input(&input)?;
    db::create_location(pool, &input).await
}

pub async fn list_locations(
    pool: &SqlitePool,
    filter: LocationFilter,
) -> Result<Vec<Location>, AppError> {
    if let Some(area) = &filter.area {
        required(area, "area")?;
    }
    if let Some(fridge_number) = &filter.fridge_number {
        required(fridge_number, "fridge_number")?;
    }
    if let Some(shelf) = &filter.shelf {
        required(shelf, "shelf")?;
    }
    if let Some(slot) = &filter.slot {
        required(slot, "slot")?;
    }
    db::list_locations(pool, &filter).await
}

pub async fn get_location_detail(
    pool: &SqlitePool,
    location_id: i64,
) -> Result<LocationDetail, AppError> {
    db::get_location_detail(pool, location_id)
        .await?
        .ok_or_else(|| AppError::not_found("存放位置不存在"))
}

pub async fn register_sample(
    pool: &SqlitePool,
    batch_id: i64,
    request: RegisterSampleRequest,
) -> Result<Sample, AppError> {
    validate_register_sample(&request)?;
    ensure_batch_exists(pool, batch_id).await?;
    if db::sample_number_exists(pool, &request.sample_number).await? {
        return Err(AppError::conflict("样本编号已存在"));
    }
    if let Some(input) = &request.location {
        let location = db::create_location(pool, input).await?;
        ensure_location_available(pool, location.id, None).await?;
    }
    db::register_sample(pool, batch_id, &request).await
}

async fn ensure_location_available(
    pool: &SqlitePool,
    location_id: i64,
    current_sample_id: Option<i64>,
) -> Result<(), AppError> {
    if let Some(occupant) = db::find_active_sample_at_location(pool, location_id).await? {
        if current_sample_id != Some(occupant.id) {
            return Err(AppError::conflict(format!(
                "该存放位置已被未归档样本 {} 占用，同一位置同一时间只能存放一个未归档样本",
                occupant.sample_number
            ))
            .with_details(json!({
                "location_id": location_id,
                "occupied_by_sample_id": occupant.id,
                "occupied_by_sample_number": occupant.sample_number
            })));
        }
    }
    Ok(())
}

pub async fn bulk_import_samples(
    pool: &SqlitePool,
    batch_id: i64,
    request: BulkImportRequest,
) -> Result<Vec<Sample>, AppError> {
    required(&request.operator, "operator")?;
    max_length(&request.operator, 100, "operator")?;
    if let Some(description) = &request.description {
        max_length(description, 1000, "description")?;
    }
    if request.samples.is_empty() {
        return Err(AppError::validation("samples 至少包含一个样本"));
    }
    ensure_batch_exists(pool, batch_id).await?;

    let mut seen_numbers = HashSet::new();
    let mut seen_locations = HashSet::new();
    for item in &request.samples {
        required(&item.sample_number, "sample_number")?;
        required(&item.sample_type, "sample_type")?;
        max_length(&item.sample_number, 100, "sample_number")?;
        max_length(&item.sample_type, 100, "sample_type")?;
        if let Some(location) = &item.location {
            validate_location_input(location)?;
            let location = db::create_location(pool, location).await?;
            if !seen_locations.insert(location.id) {
                return Err(AppError::conflict(
                    "本次批量导入中多个样本分配到同一个存放位置，同一位置同一时间只能存放一个未归档样本",
                ));
            }
            ensure_location_available(pool, location.id, None).await?;
        }
        if !seen_numbers.insert(item.sample_number.trim()) {
            return Err(AppError::validation(format!(
                "批量导入请求中样本编号重复: {}",
                item.sample_number
            )));
        }
        if db::sample_number_exists(pool, &item.sample_number).await? {
            return Err(AppError::conflict(format!(
                "样本编号已存在: {}",
                item.sample_number
            )));
        }
    }

    db::bulk_import_samples(
        pool,
        batch_id,
        &request.samples,
        &request.operator,
        request.description.as_deref(),
    )
    .await
}

pub async fn list_samples(
    pool: &SqlitePool,
    filter: SampleFilter,
) -> Result<Vec<Sample>, AppError> {
    if let Some(status) = &filter.status {
        parse_status(status)?;
    }
    validate_optional_filter_field(filter.area.as_deref(), "area")?;
    validate_optional_filter_field(filter.fridge_number.as_deref(), "fridge_number")?;
    validate_optional_filter_field(filter.shelf.as_deref(), "shelf")?;
    validate_optional_filter_field(filter.slot.as_deref(), "slot")?;
    db::list_samples(pool, &filter).await
}

pub async fn get_sample(pool: &SqlitePool, id: i64) -> Result<Sample, AppError> {
    db::get_sample(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("样本不存在"))
}

pub async fn update_sample_status(
    pool: &SqlitePool,
    sample_id: i64,
    request: UpdateStatusRequest,
) -> Result<Sample, AppError> {
    validate_status_request(&request)?;
    let target_status = parse_status(&request.status)?;
    let sample = get_sample(pool, sample_id).await?;

    if !sample.status.can_transition_to(target_status) {
        return Err(AppError::invalid_state(format!(
            "样本不能从 {} 变更为 {}，只能顺序流转到 {}",
            sample.status,
            target_status,
            sample
                .status
                .next()
                .map(|status| status.to_string())
                .unwrap_or_else(|| "无后续状态".to_string())
        ))
        .with_details(json!({
            "current_status": sample.status,
            "target_status": target_status,
            "allowed_next_status": sample.status.next()
        })));
    }

    if target_status == SampleStatus::Stored
        && request.location.is_none()
        && sample.current_location.is_none()
    {
        return Err(AppError::validation("样本入库必须提供存放位置")
            .with_details(json!({ "required_field": "location" })));
    }

    let old_location_id = sample.current_location.as_ref().map(|location| location.id);
    let location_id = if let Some(input) = &request.location {
        validate_location_input(input)?;
        Some(db::create_location(pool, input).await?.id)
    } else {
        old_location_id
    };

    if let Some(new_location_id) = location_id {
        ensure_location_available(pool, new_location_id, Some(sample_id)).await?;
    }

    let action = match target_status {
        SampleStatus::Stored => "store",
        SampleStatus::Processing => "process",
        SampleStatus::Transferred => "transfer",
        SampleStatus::Archived => "archive",
        SampleStatus::Collected => "status_update",
    };

    db::update_sample_status_and_location(
        pool,
        sample_id,
        target_status,
        location_id,
        old_location_id,
        &request.operator,
        action,
        request.description.as_deref(),
        sample.status,
    )
    .await?;

    get_sample(pool, sample_id).await
}

pub async fn update_sample_location(
    pool: &SqlitePool,
    sample_id: i64,
    request: UpdateLocationRequest,
) -> Result<Sample, AppError> {
    required(&request.operator, "operator")?;
    max_length(&request.operator, 100, "operator")?;
    if let Some(description) = &request.description {
        max_length(description, 1000, "description")?;
    }
    validate_location_input(&request.location)?;

    let sample = get_sample(pool, sample_id).await?;
    if sample.status == SampleStatus::Archived {
        return Err(AppError::conflict("已归档样本不能变更存放位置"));
    }

    let location = db::create_location(pool, &request.location).await?;
    let old_location_id = sample.current_location.as_ref().map(|location| location.id);
    ensure_location_available(pool, location.id, Some(sample_id)).await?;
    db::update_sample_location(
        pool,
        sample_id,
        location.id,
        old_location_id,
        &request.operator,
        request.description.as_deref(),
        sample.status,
    )
    .await?;

    get_sample(pool, sample_id).await
}

pub async fn list_sample_flow(
    pool: &SqlitePool,
    sample_id: i64,
) -> Result<(Sample, Vec<OperationLog>, Vec<Anomaly>), AppError> {
    let sample = get_sample(pool, sample_id).await?;
    let logs = db::list_operation_logs(pool, sample_id).await?;
    let anomalies = db::list_anomalies(pool, Some(sample_id), None).await?;
    Ok((sample, logs, anomalies))
}

pub async fn create_anomaly(
    pool: &SqlitePool,
    sample_id: i64,
    request: CreateAnomalyRequest,
) -> Result<Anomaly, AppError> {
    required(&request.anomaly_type, "anomaly_type")?;
    required(&request.operator, "operator")?;
    max_length(&request.anomaly_type, 100, "anomaly_type")?;
    max_length(&request.operator, 100, "operator")?;
    if let Some(description) = &request.description {
        max_length(description, 1000, "description")?;
    }
    if !ALLOWED_ANOMALY_TYPES.contains(&request.anomaly_type.as_str()) {
        return Err(AppError::validation(format!(
            "异常类型必须是: {}",
            ALLOWED_ANOMALY_TYPES.join(", ")
        ))
        .with_details(json!({ "allowed_types": ALLOWED_ANOMALY_TYPES })));
    }
    let _ = get_sample(pool, sample_id).await?;
    db::create_anomaly(
        pool,
        sample_id,
        &request.anomaly_type,
        request.description.as_deref(),
        &request.operator,
    )
    .await
}

pub async fn list_anomalies(
    pool: &SqlitePool,
    sample_id: Option<i64>,
    resolved: Option<bool>,
) -> Result<Vec<Anomaly>, AppError> {
    db::list_anomalies(pool, sample_id, resolved).await
}

pub async fn resolve_anomaly(
    pool: &SqlitePool,
    anomaly_id: i64,
    request: ResolveAnomalyRequest,
) -> Result<Anomaly, AppError> {
    required(&request.operator, "operator")?;
    max_length(&request.operator, 100, "operator")?;
    if let Some(description) = &request.description {
        max_length(description, 1000, "description")?;
    }
    db::resolve_anomaly(
        pool,
        anomaly_id,
        &request.operator,
        request.description.as_deref(),
    )
    .await
}

async fn ensure_batch_exists(pool: &SqlitePool, batch_id: i64) -> Result<(), AppError> {
    get_batch(pool, batch_id).await.map(|_| ())
}

fn parse_status(value: &str) -> Result<SampleStatus, AppError> {
    SampleStatus::from_str(value).map_err(|message| {
        AppError::validation(message).with_details(json!({
            "allowed_statuses": ["collected", "stored", "processing", "transferred", "archived"]
        }))
    })
}

fn validate_optional_filter_field(value: Option<&str>, field: &str) -> Result<(), AppError> {
    if let Some(value) = value {
        required(value, field)?;
        max_length(value, 100, field)?;
    }
    Ok(())
}
