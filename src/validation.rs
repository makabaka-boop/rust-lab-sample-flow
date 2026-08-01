use crate::error::AppError;
use crate::models::{
    CreateBatchRequest, LocationInput, RegisterSampleRequest, UpdateStatusRequest,
};

pub fn required(value: &str, field: &str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        return Err(AppError::validation(format!("{field} 不能为空")));
    }
    if value.trim() != value {
        return Err(AppError::validation(format!("{field} 首尾不能包含空白字符")));
    }
    Ok(())
}

pub fn max_length(value: &str, limit: usize, field: &str) -> Result<(), AppError> {
    if value.chars().count() > limit {
        return Err(AppError::validation(format!(
            "{field} 长度不能超过 {limit} 个字符"
        )));
    }
    Ok(())
}

pub fn validate_location_input(location: &LocationInput) -> Result<(), AppError> {
    required(&location.area, "location.area")?;
    required(&location.fridge_number, "location.fridge_number")?;
    required(&location.shelf, "location.shelf")?;
    required(&location.slot, "location.slot")?;
    max_length(&location.area, 100, "location.area")?;
    max_length(&location.fridge_number, 100, "location.fridge_number")?;
    max_length(&location.shelf, 100, "location.shelf")?;
    max_length(&location.slot, 100, "location.slot")?;
    Ok(())
}

pub fn validate_batch(request: &CreateBatchRequest) -> Result<(), AppError> {
    required(&request.batch_number, "batch_number")?;
    required(&request.project_name, "project_name")?;
    required(&request.owner, "owner")?;
    max_length(&request.batch_number, 100, "batch_number")?;
    max_length(&request.project_name, 200, "project_name")?;
    max_length(&request.owner, 100, "owner")?;
    if let Some(remark) = &request.remark {
        max_length(remark, 1000, "remark")?;
    }
    Ok(())
}

pub fn validate_register_sample(request: &RegisterSampleRequest) -> Result<(), AppError> {
    required(&request.sample_number, "sample_number")?;
    required(&request.sample_type, "sample_type")?;
    required(&request.operator, "operator")?;
    max_length(&request.sample_number, 100, "sample_number")?;
    max_length(&request.sample_type, 100, "sample_type")?;
    max_length(&request.operator, 100, "operator")?;
    if let Some(description) = &request.description {
        max_length(description, 1000, "description")?;
    }
    if let Some(location) = &request.location {
        validate_location_input(location)?;
    }
    Ok(())
}

pub fn validate_status_request(request: &UpdateStatusRequest) -> Result<(), AppError> {
    required(&request.status, "status")?;
    required(&request.operator, "operator")?;
    max_length(&request.operator, 100, "operator")?;
    if let Some(description) = &request.description {
        max_length(description, 1000, "description")?;
    }
    if let Some(location) = &request.location {
        validate_location_input(location)?;
    }
    Ok(())
}
