use crate::error::AppError;
use crate::models::{
    Anomaly, Batch, BulkImportItem, CreateBatchRequest, Location, LocationFilter, LocationInput,
    OperationLog, RegisterSampleRequest, Sample, SampleFilter, SampleStatus,
};
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{FromRow, QueryBuilder, SqlitePool};
use std::str::FromStr;

pub async fn init_pool(database_url: &str) -> Result<SqlitePool, AppError> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    init_schema(&pool).await?;
    Ok(pool)
}

async fn init_schema(pool: &SqlitePool) -> Result<(), AppError> {
    sqlx::query("PRAGMA journal_mode = WAL").execute(pool).await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS batches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            batch_number TEXT NOT NULL UNIQUE,
            project_name TEXT NOT NULL,
            owner TEXT NOT NULL,
            created_at TEXT NOT NULL,
            remark TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS locations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            area TEXT NOT NULL,
            fridge_number TEXT NOT NULL,
            shelf TEXT NOT NULL,
            slot TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(area, fridge_number, shelf, slot)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS samples (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sample_number TEXT NOT NULL UNIQUE,
            batch_id INTEGER NOT NULL,
            sample_type TEXT NOT NULL,
            status TEXT NOT NULL,
            current_location_id INTEGER,
            last_processed_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY(batch_id) REFERENCES batches(id),
            FOREIGN KEY(current_location_id) REFERENCES locations(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS operation_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sample_id INTEGER NOT NULL,
            operator TEXT NOT NULL,
            action TEXT NOT NULL,
            description TEXT,
            from_status TEXT,
            to_status TEXT,
            from_location_id INTEGER,
            to_location_id INTEGER,
            created_at TEXT NOT NULL,
            FOREIGN KEY(sample_id) REFERENCES samples(id) ON DELETE CASCADE,
            FOREIGN KEY(from_location_id) REFERENCES locations(id),
            FOREIGN KEY(to_location_id) REFERENCES locations(id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS anomalies (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sample_id INTEGER NOT NULL,
            anomaly_type TEXT NOT NULL,
            description TEXT,
            resolved INTEGER NOT NULL DEFAULT 0,
            resolved_by TEXT,
            resolved_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(sample_id) REFERENCES samples(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    let _ = sqlx::query("ALTER TABLE operation_logs ADD COLUMN from_location_id INTEGER")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE operation_logs ADD COLUMN to_location_id INTEGER")
        .execute(pool)
        .await;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_samples_batch_id ON samples(batch_id)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_samples_status ON samples(status)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_samples_location_id ON samples(current_location_id)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_logs_sample_id ON operation_logs(sample_id)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_anomalies_sample_id ON anomalies(sample_id)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_logs_locations ON operation_logs(from_location_id, to_location_id)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_unique_active_location
        ON samples(current_location_id)
        WHERE current_location_id IS NOT NULL AND status != 'archived'
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn create_batch(pool: &SqlitePool, request: &CreateBatchRequest) -> Result<Batch, AppError> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO batches (batch_number, project_name, owner, created_at, remark)
        VALUES (?, ?, ?, ?, ?)
        "#,
    )
    .bind(&request.batch_number)
    .bind(&request.project_name)
    .bind(&request.owner)
    .bind(now)
    .bind(&request.remark)
    .execute(&mut *tx)
    .await?;

    let id: (i64,) = sqlx::query_as("SELECT last_insert_rowid()")
        .fetch_one(&mut *tx)
        .await?;
    let batch = sqlx::query_as::<_, Batch>(
        "SELECT id, batch_number, project_name, owner, created_at, remark FROM batches WHERE id = ?",
    )
    .bind(id.0)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(batch)
}

pub async fn list_batches(pool: &SqlitePool) -> Result<Vec<Batch>, AppError> {
    sqlx::query_as::<_, Batch>(
        "SELECT id, batch_number, project_name, owner, created_at, remark FROM batches ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(AppError::from)
}

pub async fn get_batch(pool: &SqlitePool, id: i64) -> Result<Option<Batch>, AppError> {
    sqlx::query_as::<_, Batch>(
        "SELECT id, batch_number, project_name, owner, created_at, remark FROM batches WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

pub async fn create_location(pool: &SqlitePool, input: &LocationInput) -> Result<Location, AppError> {
    let now = Utc::now();
    sqlx::query(
        "INSERT OR IGNORE INTO locations (area, fridge_number, shelf, slot, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&input.area)
    .bind(&input.fridge_number)
    .bind(&input.shelf)
    .bind(&input.slot)
    .bind(now)
    .execute(pool)
    .await?;

    sqlx::query_as::<_, Location>(
        r#"
        SELECT id, area, fridge_number, shelf, slot, created_at
        FROM locations
        WHERE area = ? AND fridge_number = ? AND shelf = ? AND slot = ?
        "#,
    )
    .bind(&input.area)
    .bind(&input.fridge_number)
    .bind(&input.shelf)
    .bind(&input.slot)
    .fetch_one(pool)
    .await
    .map_err(AppError::from)
}

pub async fn list_locations(
    pool: &SqlitePool,
    filter: &LocationFilter,
) -> Result<Vec<Location>, AppError> {
    let mut query = QueryBuilder::new(
        "SELECT id, area, fridge_number, shelf, slot, created_at FROM locations WHERE 1 = 1",
    );

    if let Some(area) = &filter.area {
        query.push(" AND area = ").push_bind(area);
    }
    if let Some(fridge_number) = &filter.fridge_number {
        query.push(" AND fridge_number = ").push_bind(fridge_number);
    }
    if let Some(shelf) = &filter.shelf {
        query.push(" AND shelf = ").push_bind(shelf);
    }
    if let Some(slot) = &filter.slot {
        query.push(" AND slot = ").push_bind(slot);
    }
    query.push(" ORDER BY id DESC");

    query.build_query_as::<Location>().fetch_all(pool).await.map_err(AppError::from)
}

pub async fn get_location(pool: &SqlitePool, id: i64) -> Result<Option<Location>, AppError> {
    sqlx::query_as::<_, Location>(
        "SELECT id, area, fridge_number, shelf, slot, created_at FROM locations WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

#[derive(Debug, FromRow)]
struct SampleJoinRow {
    id: i64,
    sample_number: String,
    batch_id: i64,
    sample_type: String,
    status: String,
    last_processed_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    location_id: Option<i64>,
    area: Option<String>,
    fridge_number: Option<String>,
    shelf: Option<String>,
    slot: Option<String>,
    location_created_at: Option<DateTime<Utc>>,
}

impl TryFrom<SampleJoinRow> for Sample {
    type Error = AppError;

    fn try_from(row: SampleJoinRow) -> Result<Self, Self::Error> {
        let status = SampleStatus::from_str(&row.status)
            .map_err(AppError::invalid_state)?;
        let current_location = row.location_id.map(|id| Location {
            id,
            area: row.area.unwrap_or_default(),
            fridge_number: row.fridge_number.unwrap_or_default(),
            shelf: row.shelf.unwrap_or_default(),
            slot: row.slot.unwrap_or_default(),
            created_at: row.location_created_at.unwrap_or_else(Utc::now),
        });
        Ok(Self {
            id: row.id,
            sample_number: row.sample_number,
            batch_id: row.batch_id,
            sample_type: row.sample_type,
            status,
            current_location,
            last_processed_at: row.last_processed_at,
            created_at: row.created_at,
        })
    }
}

const SAMPLE_SELECT: &str = r#"
SELECT
    s.id,
    s.sample_number,
    s.batch_id,
    s.sample_type,
    s.status,
    s.last_processed_at,
    s.created_at,
    l.id AS location_id,
    l.area,
    l.fridge_number,
    l.shelf,
    l.slot,
    l.created_at AS location_created_at
FROM samples s
LEFT JOIN locations l ON s.current_location_id = l.id
"#;

pub async fn register_sample(
    pool: &SqlitePool,
    batch_id: i64,
    request: &RegisterSampleRequest,
) -> Result<Sample, AppError> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    let location_id = if let Some(input) = &request.location {
        Some(ensure_location_in_tx(&mut tx, input, now).await?)
    } else {
        None
    };

    sqlx::query(
        r#"
        INSERT INTO samples (
            sample_number, batch_id, sample_type, status,
            current_location_id, last_processed_at, created_at
        )
        VALUES (?, ?, ?, 'collected', ?, ?, ?)
        "#,
    )
    .bind(&request.sample_number)
    .bind(batch_id)
    .bind(&request.sample_type)
    .bind(location_id)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let id: (i64,) = sqlx::query_as("SELECT last_insert_rowid()")
        .fetch_one(&mut *tx)
        .await?;
    let sample_id = id.0;

    sqlx::query(
        r#"
        INSERT INTO operation_logs (
            sample_id, operator, action, description, from_status, to_status,
            from_location_id, to_location_id, created_at
        )
        VALUES (?, ?, 'register', ?, NULL, 'collected', NULL, ?, ?)
        "#,
    )
    .bind(sample_id)
    .bind(&request.operator)
    .bind(&request.description)
    .bind(location_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let sample = get_sample(pool, sample_id).await?
        .ok_or_else(|| AppError::internal("样本创建后无法读取"))?;
    Ok(sample)
}

async fn ensure_location_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    input: &LocationInput,
    now: DateTime<Utc>,
) -> Result<i64, AppError> {
    sqlx::query(
        "INSERT OR IGNORE INTO locations (area, fridge_number, shelf, slot, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&input.area)
    .bind(&input.fridge_number)
    .bind(&input.shelf)
    .bind(&input.slot)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    let id: (i64,) = sqlx::query_as(
        r#"
        SELECT id FROM locations
        WHERE area = ? AND fridge_number = ? AND shelf = ? AND slot = ?
        "#,
    )
    .bind(&input.area)
    .bind(&input.fridge_number)
    .bind(&input.shelf)
    .bind(&input.slot)
    .fetch_one(&mut **tx)
    .await?;

    Ok(id.0)
}

pub async fn bulk_import_samples(
    pool: &SqlitePool,
    batch_id: i64,
    items: &[BulkImportItem],
    operator: &str,
    description: Option<&str>,
) -> Result<Vec<Sample>, AppError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();
    let mut sample_ids = Vec::with_capacity(items.len());

    for item in items {
        let location_id = if let Some(input) = &item.location {
            Some(ensure_location_in_tx(&mut tx, input, now).await?)
        } else {
            None
        };

        sqlx::query(
            r#"
            INSERT INTO samples (
                sample_number, batch_id, sample_type, status,
                current_location_id, last_processed_at, created_at
            )
            VALUES (?, ?, ?, 'collected', ?, ?, ?)
            "#,
        )
        .bind(&item.sample_number)
        .bind(batch_id)
        .bind(&item.sample_type)
        .bind(location_id)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        let id: (i64,) = sqlx::query_as("SELECT last_insert_rowid()")
            .fetch_one(&mut *tx)
            .await?;
        let inserted_id = id.0;

        sqlx::query(
            r#"
            INSERT INTO operation_logs (
                sample_id, operator, action, description, from_status, to_status,
                from_location_id, to_location_id, created_at
            )
            VALUES (?, ?, 'created', ?, NULL, 'collected', NULL, ?, ?)
            "#,
        )
        .bind(inserted_id)
        .bind(operator)
        .bind(description)
        .bind(location_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sample_ids.push(inserted_id);
    }

    tx.commit().await?;

    let mut samples = Vec::with_capacity(sample_ids.len());
    for id in sample_ids {
        if let Some(sample) = get_sample(pool, id).await? {
            samples.push(sample);
        }
    }
    Ok(samples)
}

pub async fn sample_number_exists(
    pool: &SqlitePool,
    sample_number: &str,
) -> Result<bool, AppError> {
    let result: Option<(i64,)> = sqlx::query_as("SELECT id FROM samples WHERE sample_number = ?")
        .bind(sample_number)
        .fetch_optional(pool)
        .await?;
    Ok(result.is_some())
}

pub async fn list_samples(
    pool: &SqlitePool,
    filter: &SampleFilter,
) -> Result<Vec<Sample>, AppError> {
    let mut query = QueryBuilder::new(SAMPLE_SELECT);
    query.push(" WHERE 1 = 1");

    if let Some(batch_id) = filter.batch_id {
        query.push(" AND s.batch_id = ").push_bind(batch_id);
    }
    if let Some(status) = &filter.status {
        query.push(" AND s.status = ").push_bind(status);
    }
    if let Some(location_id) = filter.location_id {
        query.push(" AND s.current_location_id = ").push_bind(location_id);
    }
    if let Some(area) = &filter.area {
        query.push(" AND l.area = ").push_bind(area);
    }
    if let Some(fridge_number) = &filter.fridge_number {
        query.push(" AND l.fridge_number = ").push_bind(fridge_number);
    }
    if let Some(shelf) = &filter.shelf {
        query.push(" AND l.shelf = ").push_bind(shelf);
    }
    if let Some(slot) = &filter.slot {
        query.push(" AND l.slot = ").push_bind(slot);
    }

    query.push(" ORDER BY s.id DESC");

    let rows = query.build_query_as::<SampleJoinRow>().fetch_all(pool).await?;
    rows.into_iter().map(Sample::try_from).collect()
}

pub async fn get_sample(pool: &SqlitePool, id: i64) -> Result<Option<Sample>, AppError> {
    let row = sqlx::query_as::<_, SampleJoinRow>(&format!(
        "{SAMPLE_SELECT} WHERE s.id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;

    row.map(Sample::try_from).transpose()
}

pub async fn update_sample_status_and_location(
    pool: &SqlitePool,
    sample_id: i64,
    status: SampleStatus,
    location_id: Option<i64>,
    old_location_id: Option<i64>,
    operator: &str,
    action: &str,
    description: Option<&str>,
    from_status: SampleStatus,
) -> Result<(), AppError> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    let new_location_id = if status == SampleStatus::Archived {
        None
    } else {
        location_id
    };

    sqlx::query(
        "UPDATE samples SET status = ?, current_location_id = ?, last_processed_at = ? WHERE id = ?",
    )
    .bind(status.as_str())
    .bind(new_location_id)
    .bind(now)
    .bind(sample_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO operation_logs (
            sample_id, operator, action, description, from_status, to_status,
            from_location_id, to_location_id, created_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(sample_id)
    .bind(operator)
    .bind(action)
    .bind(description)
    .bind(from_status.as_str())
    .bind(status.as_str())
    .bind(old_location_id)
    .bind(new_location_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn update_sample_location(
    pool: &SqlitePool,
    sample_id: i64,
    location_id: i64,
    old_location_id: Option<i64>,
    operator: &str,
    description: Option<&str>,
    current_status: SampleStatus,
) -> Result<(), AppError> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    sqlx::query(
        "UPDATE samples SET current_location_id = ?, last_processed_at = ? WHERE id = ?",
    )
    .bind(location_id)
    .bind(now)
    .bind(sample_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO operation_logs (
            sample_id, operator, action, description, from_status, to_status,
            from_location_id, to_location_id, created_at
        )
        VALUES (?, ?, 'change_location', ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(sample_id)
    .bind(operator)
    .bind(description)
    .bind(current_status.as_str())
    .bind(current_status.as_str())
    .bind(old_location_id)
    .bind(location_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn list_operation_logs(
    pool: &SqlitePool,
    sample_id: i64,
) -> Result<Vec<OperationLog>, AppError> {
    sqlx::query_as::<_, OperationLog>(
        r#"
        SELECT id, sample_id, operator, action, description, from_status, to_status,
               from_location_id, to_location_id, created_at
        FROM operation_logs
        WHERE sample_id = ?
        ORDER BY id ASC
        "#,
    )
    .bind(sample_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::from)
}

pub async fn create_anomaly(
    pool: &SqlitePool,
    sample_id: i64,
    anomaly_type: &str,
    description: Option<&str>,
    operator: &str,
) -> Result<Anomaly, AppError> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO anomalies (
            sample_id, anomaly_type, description, resolved,
            resolved_by, resolved_at, created_at, updated_at
        )
        VALUES (?, ?, ?, 0, NULL, NULL, ?, ?)
        "#,
    )
    .bind(sample_id)
    .bind(anomaly_type)
    .bind(description)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let id: (i64,) = sqlx::query_as("SELECT last_insert_rowid()")
        .fetch_one(&mut *tx)
        .await?;
    let anomaly_id = id.0;

    sqlx::query(
        r#"
        INSERT INTO operation_logs (
            sample_id, operator, action, description, from_status, to_status,
            from_location_id, to_location_id, created_at
        )
        VALUES (?, ?, 'mark_anomaly', ?, NULL, NULL, NULL, NULL, ?)
        "#,
    )
    .bind(sample_id)
    .bind(operator)
    .bind(description)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let anomaly = sqlx::query_as::<_, Anomaly>(
        r#"
        SELECT id, sample_id, anomaly_type, description, resolved,
               resolved_by, resolved_at, created_at, updated_at
        FROM anomalies WHERE id = ?
        "#,
    )
    .bind(anomaly_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(anomaly)
}

pub async fn list_anomalies(
    pool: &SqlitePool,
    sample_id: Option<i64>,
    resolved: Option<bool>,
) -> Result<Vec<Anomaly>, AppError> {
    let mut query = QueryBuilder::new(
        r#"
        SELECT id, sample_id, anomaly_type, description, resolved,
               resolved_by, resolved_at, created_at, updated_at
        FROM anomalies WHERE 1 = 1
        "#,
    );

    if let Some(sample_id) = sample_id {
        query.push(" AND sample_id = ").push_bind(sample_id);
    }
    if let Some(resolved) = resolved {
        query.push(" AND resolved = ").push_bind(resolved as i64);
    }
    query.push(" ORDER BY id DESC");

    query.build_query_as::<Anomaly>().fetch_all(pool).await.map_err(AppError::from)
}

pub async fn get_anomaly(pool: &SqlitePool, id: i64) -> Result<Option<Anomaly>, AppError> {
    sqlx::query_as::<_, Anomaly>(
        r#"
        SELECT id, sample_id, anomaly_type, description, resolved,
               resolved_by, resolved_at, created_at, updated_at
        FROM anomalies WHERE id = ?
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)
}

pub async fn resolve_anomaly(
    pool: &SqlitePool,
    anomaly_id: i64,
    operator: &str,
    description: Option<&str>,
) -> Result<Anomaly, AppError> {
    let now = Utc::now();
    let anomaly = get_anomaly(pool, anomaly_id)
        .await?
        .ok_or_else(|| AppError::not_found("异常标记不存在"))?;

    if anomaly.resolved {
        return Err(AppError::conflict("异常标记已经解除，不能重复解除"));
    }

    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        UPDATE anomalies
        SET resolved = 1, resolved_by = ?, resolved_at = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(operator)
    .bind(now)
    .bind(now)
    .bind(anomaly_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO operation_logs (
            sample_id, operator, action, description, from_status, to_status,
            from_location_id, to_location_id, created_at
        )
        VALUES (?, ?, 'resolve_anomaly', ?, NULL, NULL, NULL, NULL, ?)
        "#,
    )
    .bind(anomaly.sample_id)
    .bind(operator)
    .bind(description)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let updated = sqlx::query_as::<_, Anomaly>(
        r#"
        SELECT id, sample_id, anomaly_type, description, resolved,
               resolved_by, resolved_at, created_at, updated_at
        FROM anomalies WHERE id = ?
        "#,
    )
    .bind(anomaly_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(updated)
}

#[derive(Debug, FromRow)]
struct MovementRow {
    log_id: i64,
    sample_id: i64,
    sample_number: String,
    operator: String,
    action: String,
    description: Option<String>,
    direction: String,
    created_at: DateTime<Utc>,
    from_id: Option<i64>,
    from_area: Option<String>,
    from_fridge: Option<String>,
    from_shelf: Option<String>,
    from_slot: Option<String>,
    from_created_at: Option<DateTime<Utc>>,
    to_id: Option<i64>,
    to_area: Option<String>,
    to_fridge: Option<String>,
    to_shelf: Option<String>,
    to_slot: Option<String>,
    to_created_at: Option<DateTime<Utc>>,
}

fn row_to_location(
    id: Option<i64>,
    area: Option<String>,
    fridge: Option<String>,
    shelf: Option<String>,
    slot: Option<String>,
    created_at: Option<DateTime<Utc>>,
) -> Option<Location> {
    id.map(|id| Location {
        id,
        area: area.unwrap_or_default(),
        fridge_number: fridge.unwrap_or_default(),
        shelf: shelf.unwrap_or_default(),
        slot: slot.unwrap_or_default(),
        created_at: created_at.unwrap_or_else(Utc::now),
    })
}

pub async fn get_location_detail(
    pool: &SqlitePool,
    location_id: i64,
) -> Result<Option<crate::models::LocationDetail>, AppError> {
    let location = match get_location(pool, location_id).await? {
        Some(location) => location,
        None => return Ok(None),
    };

    let current_sample = sqlx::query_as::<_, SampleJoinRow>(&format!(
        "{SAMPLE_SELECT} WHERE s.current_location_id = ? AND s.status != 'archived'"
    ))
    .bind(location_id)
    .fetch_optional(pool)
    .await?;
    let current_sample = current_sample.map(Sample::try_from).transpose()?;

    let rows = sqlx::query_as::<_, MovementRow>(
        r#"
        SELECT
            ol.id AS log_id,
            ol.sample_id,
            sm.sample_number,
            ol.operator,
            ol.action,
            ol.description,
            CASE
                WHEN ol.to_location_id = ? AND (ol.from_location_id IS NULL OR ol.from_location_id != ?) THEN 'in'
                WHEN ol.from_location_id = ? AND (ol.to_location_id IS NULL OR ol.to_location_id != ?) THEN 'out'
                ELSE 'related'
            END AS direction,
            ol.created_at,
            fl.id AS from_id,
            fl.area AS from_area,
            fl.fridge_number AS from_fridge,
            fl.shelf AS from_shelf,
            fl.slot AS from_slot,
            fl.created_at AS from_created_at,
            tl.id AS to_id,
            tl.area AS to_area,
            tl.fridge_number AS to_fridge,
            tl.shelf AS to_shelf,
            tl.slot AS to_slot,
            tl.created_at AS to_created_at
        FROM operation_logs ol
        INNER JOIN samples sm ON sm.id = ol.sample_id
        LEFT JOIN locations fl ON fl.id = ol.from_location_id
        LEFT JOIN locations tl ON tl.id = ol.to_location_id
        WHERE
            (ol.to_location_id = ? AND (ol.from_location_id IS NULL OR ol.from_location_id != ?))
            OR
            (ol.from_location_id = ? AND (ol.to_location_id IS NULL OR ol.to_location_id != ?))
        ORDER BY ol.id DESC
        LIMIT 10
        "#,
    )
    .bind(location_id)
    .bind(location_id)
    .bind(location_id)
    .bind(location_id)
    .bind(location_id)
    .bind(location_id)
    .bind(location_id)
    .bind(location_id)
    .fetch_all(pool)
    .await?;

    let mut recent_movements = Vec::with_capacity(rows.len());
    for row in rows {
        let from_location = row_to_location(
            row.from_id,
            row.from_area,
            row.from_fridge,
            row.from_shelf,
            row.from_slot,
            row.from_created_at,
        );
        let to_location = row_to_location(
            row.to_id,
            row.to_area,
            row.to_fridge,
            row.to_shelf,
            row.to_slot,
            row.to_created_at,
        );
        recent_movements.push(crate::models::LocationMovementRecord {
            log_id: row.log_id,
            sample_id: row.sample_id,
            sample_number: row.sample_number,
            operator: row.operator,
            action: row.action,
            description: row.description,
            direction: row.direction,
            from_location,
            to_location,
            created_at: row.created_at,
        });
    }

    Ok(Some(crate::models::LocationDetail {
        location,
        current_sample,
        recent_movements,
    }))
}

pub async fn find_active_sample_at_location(
    pool: &SqlitePool,
    location_id: i64,
) -> Result<Option<Sample>, AppError> {
    let row = sqlx::query_as::<_, SampleJoinRow>(&format!(
        "{SAMPLE_SELECT} WHERE s.current_location_id = ? AND s.status != 'archived'"
    ))
    .bind(location_id)
    .fetch_optional(pool)
    .await?;
    row.map(Sample::try_from).transpose()
}
