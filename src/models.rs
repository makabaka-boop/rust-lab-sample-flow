use serde::{Deserialize, Serialize};

/// 样本允许的状态集合（按流转顺序排列）
pub const STATUSES: [&str; 5] = [
    "collected",
    "stored",
    "processing",
    "transferred",
    "archived",
];

/// 异常标记类型
pub const FLAG_TYPES: [&str; 4] = [
    "contamination",      // 样本污染
    "label_missing",      // 标签缺失
    "temperature_abnormal", // 温控异常
    "other",
];

pub fn is_valid_status(s: &str) -> bool {
    STATUSES.contains(&s)
}

pub fn is_valid_flag_type(s: &str) -> bool {
    FLAG_TYPES.contains(&s)
}

/// 状态机：只允许链式向后流转，禁止跳过关键状态、禁止回退。
/// collected -> stored -> processing -> transferred -> archived
pub fn can_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("collected", "stored")
            | ("stored", "processing")
            | ("processing", "transferred")
            | ("transferred", "archived")
    )
}

// ---------- 实体 ----------

#[derive(Debug, Clone, Serialize)]
pub struct Batch {
    pub id: i64,
    pub batch_no: String,
    pub project_name: String,
    pub manager: String,
    pub remark: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Location {
    pub id: i64,
    pub region: String,
    pub freezer_no: String,
    pub shelf: String,
    pub slot: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub id: i64,
    pub sample_no: String,
    pub batch_no: String,
    pub sample_type: String,
    pub status: String,
    pub location: Option<Location>,
    pub updated_at: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OperationLog {
    pub id: i64,
    pub sample_no: String,
    pub operator: String,
    pub action: String,
    pub note: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExceptionFlag {
    pub id: i64,
    pub sample_no: String,
    pub flag_type: String,
    pub description: String,
    pub resolved: bool,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

// ---------- 请求体 ----------

#[derive(Debug, Deserialize)]
pub struct CreateBatchReq {
    pub batch_no: String,
    pub project_name: String,
    pub manager: String,
    #[serde(default)]
    pub remark: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterSampleReq {
    pub sample_no: String,
    pub batch_no: String,
    pub sample_type: String,
    #[serde(default)]
    pub operator: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BulkImportItem {
    pub sample_no: String,
    pub sample_type: String,
    /// 可选初始存放位置；一旦提供，四个字段都必须非空
    #[serde(default)]
    pub location: Option<BulkImportLocation>,
}

#[derive(Debug, Deserialize)]
pub struct BulkImportLocation {
    pub region: String,
    pub freezer_no: String,
    pub shelf: String,
    pub slot: String,
}

#[derive(Debug, Deserialize)]
pub struct BulkImportReq {
    #[serde(default)]
    pub operator: Option<String>,
    pub samples: Vec<BulkImportItem>,
}

#[derive(Debug, Serialize)]
pub struct BulkImportResult {
    pub batch_no: String,
    pub imported: usize,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStatusReq {
    pub status: String,
    pub operator: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChangeLocationReq {
    pub region: String,
    pub freezer_no: String,
    pub shelf: String,
    pub slot: String,
    pub operator: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddExceptionReq {
    pub flag_type: String,
    #[serde(default)]
    pub description: String,
    pub operator: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveExceptionReq {
    pub operator: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SampleFilter {
    pub batch_no: Option<String>,
    pub status: Option<String>,
    pub sample_type: Option<String>,
    pub region: Option<String>,
    pub freezer_no: Option<String>,
    pub shelf: Option<String>,
    pub slot: Option<String>,
}

/// 位置查询参数（四元组必填）
#[derive(Debug, Default, Deserialize)]
pub struct LocationQuery {
    pub region: Option<String>,
    pub freezer_no: Option<String>,
    pub shelf: Option<String>,
    pub slot: Option<String>,
}

/// 一次位置迁入/迁出记录
#[derive(Debug, Clone, Serialize)]
pub struct LocationMovement {
    pub id: i64,
    pub sample_no: String,
    /// 相对所查位置：in=迁入，out=迁出
    pub direction: String,
    /// None 表示首次放入（无来源位置）
    pub from_location: Option<Location>,
    /// None 表示归档释放（无目标位置）
    pub to_location: Option<Location>,
    pub operator: String,
    pub created_at: String,
}

/// 位置详情：当前占用样本（未归档）+ 最近迁入迁出记录
#[derive(Debug, Clone, Serialize)]
pub struct LocationDetail {
    pub location: Location,
    pub current_sample: Option<Sample>,
    pub recent_movements: Vec<LocationMovement>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_chain_is_linear() {
        assert!(can_transition("collected", "stored"));
        assert!(can_transition("stored", "processing"));
        assert!(can_transition("processing", "transferred"));
        assert!(can_transition("transferred", "archived"));
    }

    #[test]
    fn transition_rejects_skipping_key_states() {
        assert!(!can_transition("collected", "archived"));
        assert!(!can_transition("collected", "processing"));
        assert!(!can_transition("stored", "transferred"));
        assert!(!can_transition("processing", "archived"));
    }

    #[test]
    fn transition_rejects_backwards_and_self() {
        assert!(!can_transition("archived", "collected"));
        assert!(!can_transition("stored", "collected"));
        assert!(!can_transition("transferred", "processing"));
        assert!(!can_transition("collected", "collected"));
        // archived 是终态，不允许任何转出
        for to in STATUSES {
            assert!(!can_transition("archived", to));
        }
    }

    #[test]
    fn status_and_flag_type_validation() {
        assert!(is_valid_status("stored"));
        assert!(!is_valid_status("destroyed"));
        assert!(is_valid_flag_type("contamination"));
        assert!(!is_valid_flag_type("unknown_flag"));
    }
}
