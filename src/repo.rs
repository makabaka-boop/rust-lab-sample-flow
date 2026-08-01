use crate::error::ApiError;
use crate::models::*;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(err, _)
            if err.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

// ---------- 批次 ----------

pub fn insert_batch(conn: &Connection, req: &CreateBatchReq, now: &str) -> Result<Batch, ApiError> {
    conn.execute(
        "INSERT INTO batches (batch_no, project_name, manager, remark, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![req.batch_no, req.project_name, req.manager, req.remark, now],
    )
    .map_err(|e| {
        if is_unique_violation(&e) {
            ApiError::conflict("DUPLICATE_BATCH", format!("批次编号 {} 已存在", req.batch_no))
        } else {
            ApiError::from(e)
        }
    })?;
    get_batch(conn, &req.batch_no)
}

fn map_batch(row: &rusqlite::Row) -> rusqlite::Result<Batch> {
    Ok(Batch {
        id: row.get(0)?,
        batch_no: row.get(1)?,
        project_name: row.get(2)?,
        manager: row.get(3)?,
        remark: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub fn get_batch(conn: &Connection, batch_no: &str) -> Result<Batch, ApiError> {
    conn.query_row(
        "SELECT id, batch_no, project_name, manager, remark, created_at
         FROM batches WHERE batch_no = ?1",
        params![batch_no],
        map_batch,
    )
    .optional()?
    .ok_or_else(|| {
        ApiError::not_found("BATCH_NOT_FOUND", format!("批次 {batch_no} 不存在"))
    })
}

pub fn list_batches(conn: &Connection) -> Result<Vec<Batch>, ApiError> {
    let mut stmt = conn.prepare(
        "SELECT id, batch_no, project_name, manager, remark, created_at
         FROM batches ORDER BY id",
    )?;
    let rows = stmt.query_map([], map_batch)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn batch_exists(conn: &Connection, batch_no: &str) -> Result<bool, ApiError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM batches WHERE batch_no = ?1",
        params![batch_no],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

// ---------- 存放位置 ----------

fn map_location(row: &rusqlite::Row) -> rusqlite::Result<Location> {
    Ok(Location {
        id: row.get(0)?,
        region: row.get(1)?,
        freezer_no: row.get(2)?,
        shelf: row.get(3)?,
        slot: row.get(4)?,
    })
}

pub fn get_location_by_fields(
    conn: &Connection,
    region: &str,
    freezer_no: &str,
    shelf: &str,
    slot: &str,
) -> Result<Option<Location>, ApiError> {
    let loc = conn
        .query_row(
            "SELECT id, region, freezer_no, shelf, slot FROM locations
             WHERE region = ?1 AND freezer_no = ?2 AND shelf = ?3 AND slot = ?4",
            params![region, freezer_no, shelf, slot],
            map_location,
        )
        .optional()?;
    Ok(loc)
}

/// 按 (region, freezer_no, shelf, slot) 查找，不存在则插入（幂等）
pub fn find_or_create_location(
    conn: &Connection,
    region: &str,
    freezer_no: &str,
    shelf: &str,
    slot: &str,
) -> Result<Location, ApiError> {
    conn.execute(
        "INSERT OR IGNORE INTO locations (region, freezer_no, shelf, slot)
         VALUES (?1, ?2, ?3, ?4)",
        params![region, freezer_no, shelf, slot],
    )?;
    get_location_by_fields(conn, region, freezer_no, shelf, slot)?
        .ok_or_else(|| ApiError::internal("location upsert failed"))
}

/// 位置当前占用者：占用只统计未归档样本，归档即释放。
/// exclude_sample_no 用于排除样本自身（重复设置当前位置不算冲突）。
pub fn find_active_occupant(
    conn: &Connection,
    location_id: i64,
    exclude_sample_no: Option<&str>,
) -> Result<Option<Sample>, ApiError> {
    let sample = conn
        .query_row(
            &format!(
                "{SAMPLE_SELECT} WHERE s.location_id = ?1 AND s.status != 'archived'
                 AND (?2 IS NULL OR s.sample_no != ?2)"
            ),
            params![location_id, exclude_sample_no],
            map_sample,
        )
        .optional()?;
    Ok(sample)
}

// ---------- 位置迁入/迁出记录 ----------

pub fn insert_movement(
    conn: &Connection,
    sample_no: &str,
    from_location_id: Option<i64>,
    to_location_id: Option<i64>,
    operator: &str,
    now: &str,
) -> Result<(), ApiError> {
    conn.execute(
        "INSERT INTO location_movements (sample_no, from_location_id, to_location_id, operator, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![sample_no, from_location_id, to_location_id, operator, now],
    )?;
    Ok(())
}

/// 某位置最近的迁入迁出记录（按时间倒序，limit 条），direction 相对该位置计算
pub fn list_movements_for_location(
    conn: &Connection,
    location_id: i64,
    limit: i64,
) -> Result<Vec<LocationMovement>, ApiError> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.sample_no, m.operator, m.created_at,
                fl.id, fl.region, fl.freezer_no, fl.shelf, fl.slot,
                tl.id, tl.region, tl.freezer_no, tl.shelf, tl.slot,
                m.from_location_id, m.to_location_id
         FROM location_movements m
         LEFT JOIN locations fl ON m.from_location_id = fl.id
         LEFT JOIN locations tl ON m.to_location_id = tl.id
         WHERE m.from_location_id = ?1 OR m.to_location_id = ?1
         ORDER BY m.id DESC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![location_id, limit], |row| {
            let from_location = match row.get::<_, Option<i64>>(4)? {
                Some(id) => Some(Location {
                    id,
                    region: row.get(5)?,
                    freezer_no: row.get(6)?,
                    shelf: row.get(7)?,
                    slot: row.get(8)?,
                }),
                None => None,
            };
            let to_location = match row.get::<_, Option<i64>>(9)? {
                Some(id) => Some(Location {
                    id,
                    region: row.get(10)?,
                    freezer_no: row.get(11)?,
                    shelf: row.get(12)?,
                    slot: row.get(13)?,
                }),
                None => None,
            };
            let from_id: Option<i64> = row.get(14)?;
            let to_id: Option<i64> = row.get(15)?;
            let direction = if to_id == Some(location_id) {
                "in"
            } else if from_id == Some(location_id) {
                "out"
            } else {
                "unknown"
            };
            Ok(LocationMovement {
                id: row.get(0)?,
                sample_no: row.get(1)?,
                direction: direction.to_string(),
                from_location,
                to_location,
                operator: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------- 样本 ----------

const SAMPLE_SELECT: &str = "
    SELECT s.id, s.sample_no, s.batch_no, s.sample_type, s.status,
           s.updated_at, s.created_at,
           l.id, l.region, l.freezer_no, l.shelf, l.slot
    FROM samples s
    LEFT JOIN locations l ON s.location_id = l.id
";

fn map_sample(row: &rusqlite::Row) -> rusqlite::Result<Sample> {
    let location = match row.get::<_, Option<i64>>(7)? {
        Some(id) => Some(Location {
            id,
            region: row.get(8)?,
            freezer_no: row.get(9)?,
            shelf: row.get(10)?,
            slot: row.get(11)?,
        }),
        None => None,
    };
    Ok(Sample {
        id: row.get(0)?,
        sample_no: row.get(1)?,
        batch_no: row.get(2)?,
        sample_type: row.get(3)?,
        status: row.get(4)?,
        updated_at: row.get(5)?,
        created_at: row.get(6)?,
        location,
    })
}

pub fn insert_sample(
    conn: &Connection,
    sample_no: &str,
    batch_no: &str,
    sample_type: &str,
    now: &str,
) -> Result<Sample, ApiError> {
    conn.execute(
        "INSERT INTO samples (sample_no, batch_no, sample_type, status, updated_at, created_at)
         VALUES (?1, ?2, ?3, 'collected', ?4, ?4)",
        params![sample_no, batch_no, sample_type, now],
    )
    .map_err(|e| {
        if is_unique_violation(&e) {
            ApiError::conflict("DUPLICATE_SAMPLE", format!("样本编号 {sample_no} 已存在"))
        } else {
            ApiError::from(e)
        }
    })?;
    get_sample(conn, sample_no)
}

pub fn get_sample(conn: &Connection, sample_no: &str) -> Result<Sample, ApiError> {
    conn.query_row(
        &format!("{SAMPLE_SELECT} WHERE s.sample_no = ?1"),
        params![sample_no],
        map_sample,
    )
    .optional()?
    .ok_or_else(|| {
        ApiError::not_found("SAMPLE_NOT_FOUND", format!("样本 {sample_no} 不存在"))
    })
}

pub fn update_sample_status(
    conn: &Connection,
    sample_no: &str,
    status: &str,
    now: &str,
) -> Result<(), ApiError> {
    conn.execute(
        "UPDATE samples SET status = ?1, updated_at = ?2 WHERE sample_no = ?3",
        params![status, now, sample_no],
    )?;
    Ok(())
}

pub fn update_sample_location(
    conn: &Connection,
    sample_no: &str,
    location_id: i64,
    now: &str,
) -> Result<(), ApiError> {
    conn.execute(
        "UPDATE samples SET location_id = ?1, updated_at = ?2 WHERE sample_no = ?3",
        params![location_id, now, sample_no],
    )?;
    Ok(())
}

/// 清空样本当前位置（归档释放时调用）
pub fn clear_sample_location(conn: &Connection, sample_no: &str) -> Result<(), ApiError> {
    conn.execute(
        "UPDATE samples SET location_id = NULL WHERE sample_no = ?1",
        params![sample_no],
    )?;
    Ok(())
}

pub fn query_samples(conn: &Connection, filter: &SampleFilter) -> Result<Vec<Sample>, ApiError> {
    let mut sql = format!("{SAMPLE_SELECT} WHERE 1 = 1");
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(v) = &filter.batch_no {
        sql.push_str(" AND s.batch_no = ?");
        values.push(Box::new(v.clone()));
    }
    if let Some(v) = &filter.status {
        sql.push_str(" AND s.status = ?");
        values.push(Box::new(v.clone()));
    }
    if let Some(v) = &filter.sample_type {
        sql.push_str(" AND s.sample_type = ?");
        values.push(Box::new(v.clone()));
    }
    if let Some(v) = &filter.region {
        sql.push_str(" AND l.region = ?");
        values.push(Box::new(v.clone()));
    }
    if let Some(v) = &filter.freezer_no {
        sql.push_str(" AND l.freezer_no = ?");
        values.push(Box::new(v.clone()));
    }
    if let Some(v) = &filter.shelf {
        sql.push_str(" AND l.shelf = ?");
        values.push(Box::new(v.clone()));
    }
    if let Some(v) = &filter.slot {
        sql.push_str(" AND l.slot = ?");
        values.push(Box::new(v.clone()));
    }
    sql.push_str(" ORDER BY s.id");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(values), map_sample)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------- 操作日志 ----------

pub fn insert_log(
    conn: &Connection,
    sample_no: &str,
    operator: &str,
    action: &str,
    note: &str,
    now: &str,
) -> Result<(), ApiError> {
    conn.execute(
        "INSERT INTO operation_logs (sample_no, operator, action, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![sample_no, operator, action, note, now],
    )?;
    Ok(())
}

pub fn list_logs(conn: &Connection, sample_no: &str) -> Result<Vec<OperationLog>, ApiError> {
    let mut stmt = conn.prepare(
        "SELECT id, sample_no, operator, action, note, created_at
         FROM operation_logs WHERE sample_no = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![sample_no], |row| {
            Ok(OperationLog {
                id: row.get(0)?,
                sample_no: row.get(1)?,
                operator: row.get(2)?,
                action: row.get(3)?,
                note: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------- 异常标记 ----------

pub fn insert_exception(
    conn: &Connection,
    sample_no: &str,
    flag_type: &str,
    description: &str,
    now: &str,
) -> Result<ExceptionFlag, ApiError> {
    conn.execute(
        "INSERT INTO exception_flags (sample_no, flag_type, description, resolved, created_at)
         VALUES (?1, ?2, ?3, 0, ?4)",
        params![sample_no, flag_type, description, now],
    )?;
    let id = conn.last_insert_rowid();
    get_exception(conn, id)
}

fn map_exception(row: &rusqlite::Row) -> rusqlite::Result<ExceptionFlag> {
    Ok(ExceptionFlag {
        id: row.get(0)?,
        sample_no: row.get(1)?,
        flag_type: row.get(2)?,
        description: row.get(3)?,
        resolved: row.get::<_, i64>(4)? != 0,
        created_at: row.get(5)?,
        resolved_at: row.get(6)?,
    })
}

pub fn get_exception(conn: &Connection, id: i64) -> Result<ExceptionFlag, ApiError> {
    conn.query_row(
        "SELECT id, sample_no, flag_type, description, resolved, created_at, resolved_at
         FROM exception_flags WHERE id = ?1",
        params![id],
        map_exception,
    )
    .optional()?
    .ok_or_else(|| {
        ApiError::not_found("EXCEPTION_NOT_FOUND", format!("异常标记 {id} 不存在"))
    })
}

pub fn resolve_exception(conn: &Connection, id: i64, now: &str) -> Result<(), ApiError> {
    conn.execute(
        "UPDATE exception_flags SET resolved = 1, resolved_at = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    Ok(())
}

pub fn list_exceptions(
    conn: &Connection,
    sample_no: &str,
) -> Result<Vec<ExceptionFlag>, ApiError> {
    let mut stmt = conn.prepare(
        "SELECT id, sample_no, flag_type, description, resolved, created_at, resolved_at
         FROM exception_flags WHERE sample_no = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![sample_no], map_exception)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
