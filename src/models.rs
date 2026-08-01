use serde::{Deserialize, Serialize};

/// 样本状态机的五个合法状态。
pub const SAMPLE_STATES: [&str; 5] = [
    "collected",
    "stored",
    "processing",
    "transferred",
    "archived",
];

/// 异常标记的常见类型（不做强制约束，仅作参考文档）。
pub const EXCEPTION_TYPES: [&str; 3] = ["contamination", "label_missing", "temperature"];

// ---------- 存放位置 ----------

/// 存放位置：区域 / 冰箱编号 / 层架 / 格位。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub id: i64,
    pub area: String,
    pub freezer: String,
    pub shelf: String,
    pub slot: String,
}

/// 位置入参（用于登记 / 变更位置）。
#[derive(Debug, Clone, Deserialize)]
pub struct LocationInput {
    pub area: String,
    pub freezer: String,
    pub shelf: String,
    pub slot: String,
}

// ---------- 样本批次 ----------

#[derive(Debug, Serialize)]
pub struct Batch {
    pub id: i64,
    pub batch_no: String,
    pub project_name: String,
    pub owner: String,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateBatchRequest {
    pub batch_no: String,
    pub project_name: String,
    pub owner: String,
    pub note: Option<String>,
}

// ---------- 单个样本 ----------

#[derive(Debug, Serialize)]
pub struct Sample {
    pub id: i64,
    pub sample_no: String,
    pub batch_no: String,
    pub sample_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    pub last_processed_at: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterSampleRequest {
    pub sample_no: String,
    pub sample_type: String,
    pub location: Option<LocationInput>,
}

/// 批量导入的单条样本条目。
#[derive(Debug, Deserialize)]
pub struct BulkImportItem {
    pub sample_no: String,
    pub sample_type: String,
    pub location: Option<LocationInput>,
}

#[derive(Debug, Deserialize)]
pub struct BulkImportRequest {
    pub samples: Vec<BulkImportItem>,
}

#[derive(Debug, Serialize)]
pub struct BulkImportResponse {
    pub imported: usize,
    pub samples: Vec<Sample>,
}

// ---------- 状态更新 / 位置变更 ----------

#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
    pub operator: String,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChangeLocationRequest {
    pub location: LocationInput,
    pub operator: String,
    pub note: Option<String>,
}

// ---------- 操作日志 ----------

#[derive(Debug, Serialize)]
pub struct OperationLog {
    pub id: i64,
    pub sample_no: String,
    pub operator: String,
    pub action: String,
    pub note: Option<String>,
    pub created_at: String,
}

// ---------- 异常标记 ----------

#[derive(Debug, Serialize)]
pub struct Exception {
    pub id: i64,
    pub sample_no: String,
    pub exception_type: String,
    pub description: Option<String>,
    pub status: String, // open / resolved
    pub reported_by: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MarkExceptionRequest {
    pub exception_type: String,
    pub description: Option<String>,
    pub reported_by: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveExceptionRequest {
    pub resolved_by: String,
    pub note: Option<String>,
}

// ---------- 位置查询响应 ----------

#[derive(Debug, Serialize)]
pub struct LocationMovement {
    pub sample_no: String,
    pub direction: String, // in / out
    pub operator: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct LocationView {
    pub location: Location,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_sample: Option<String>,
    pub recent_movements: Vec<LocationMovement>,
}

/// 查询样本时可用的过滤条件。
#[derive(Debug, Default, Deserialize)]
pub struct SampleQuery {
    pub batch_no: Option<String>,
    pub status: Option<String>,
    pub area: Option<String>,
    pub freezer: Option<String>,
    pub shelf: Option<String>,
    pub slot: Option<String>,
    pub sample_type: Option<String>,
}
