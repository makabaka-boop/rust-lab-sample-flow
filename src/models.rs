use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleStatus {
    Collected,
    Stored,
    Processing,
    Transferred,
    Archived,
}

impl SampleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collected => "collected",
            Self::Stored => "stored",
            Self::Processing => "processing",
            Self::Transferred => "transferred",
            Self::Archived => "archived",
        }
    }

    pub fn next(self) -> Option<SampleStatus> {
        match self {
            Self::Collected => Some(Self::Stored),
            Self::Stored => Some(Self::Processing),
            Self::Processing => Some(Self::Transferred),
            Self::Transferred => Some(Self::Archived),
            Self::Archived => None,
        }
    }

    pub fn can_transition_to(self, target: SampleStatus) -> bool {
        self.next() == Some(target)
    }
}

impl FromStr for SampleStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "collected" => Ok(Self::Collected),
            "stored" => Ok(Self::Stored),
            "processing" => Ok(Self::Processing),
            "transferred" => Ok(Self::Transferred),
            "archived" => Ok(Self::Archived),
            other => Err(format!("未知样本状态: {other}")),
        }
    }
}

impl fmt::Display for SampleStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Batch {
    pub id: i64,
    pub batch_number: String,
    pub project_name: String,
    pub owner: String,
    pub created_at: DateTime<Utc>,
    pub remark: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateBatchRequest {
    pub batch_number: String,
    pub project_name: String,
    pub owner: String,
    pub remark: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Location {
    pub id: i64,
    pub area: String,
    pub fridge_number: String,
    pub shelf: String,
    pub slot: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocationInput {
    pub area: String,
    pub fridge_number: String,
    pub shelf: String,
    pub slot: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateLocationRequest {
    pub area: String,
    pub fridge_number: String,
    pub shelf: String,
    pub slot: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub id: i64,
    pub sample_number: String,
    pub batch_id: i64,
    pub sample_type: String,
    pub status: SampleStatus,
    pub current_location: Option<Location>,
    pub last_processed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterSampleRequest {
    pub sample_number: String,
    pub sample_type: String,
    pub location: Option<LocationInput>,
    pub operator: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BulkImportItem {
    pub sample_number: String,
    pub sample_type: String,
    pub location: Option<LocationInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BulkImportRequest {
    pub samples: Vec<BulkImportItem>,
    pub operator: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
    pub operator: String,
    pub description: Option<String>,
    pub location: Option<LocationInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateLocationRequest {
    pub location: LocationInput,
    pub operator: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OperationLog {
    pub id: i64,
    pub sample_id: i64,
    pub operator: String,
    pub action: String,
    pub description: Option<String>,
    pub from_status: Option<String>,
    pub to_status: Option<String>,
    pub from_location_id: Option<i64>,
    pub to_location_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocationMovementRecord {
    pub log_id: i64,
    pub sample_id: i64,
    pub sample_number: String,
    pub operator: String,
    pub action: String,
    pub description: Option<String>,
    pub direction: String,
    pub from_location: Option<Location>,
    pub to_location: Option<Location>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocationDetail {
    pub location: Location,
    pub current_sample: Option<Sample>,
    pub recent_movements: Vec<LocationMovementRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Anomaly {
    pub id: i64,
    pub sample_id: i64,
    pub anomaly_type: String,
    pub description: Option<String>,
    pub resolved: bool,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAnomalyRequest {
    pub anomaly_type: String,
    pub description: Option<String>,
    pub operator: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveAnomalyRequest {
    pub operator: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SampleFilter {
    pub batch_id: Option<i64>,
    pub status: Option<String>,
    pub location_id: Option<i64>,
    pub area: Option<String>,
    pub fridge_number: Option<String>,
    pub shelf: Option<String>,
    pub slot: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocationFilter {
    pub area: Option<String>,
    pub fridge_number: Option<String>,
    pub shelf: Option<String>,
    pub slot: Option<String>,
}
