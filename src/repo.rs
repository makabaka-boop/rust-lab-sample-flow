use rusqlite::{params, Connection, OptionalExtension};

use crate::models::*;

/// 查找已存在的位置，不存在则创建，返回位置 id。
pub fn find_or_create_location(conn: &Connection, loc: &LocationInput) -> rusqlite::Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM locations WHERE area=?1 AND freezer=?2 AND shelf=?3 AND slot=?4",
            params![loc.area, loc.freezer, loc.shelf, loc.slot],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO locations (area, freezer, shelf, slot) VALUES (?1, ?2, ?3, ?4)",
        params![loc.area, loc.freezer, loc.shelf, loc.slot],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn load_location(conn: &Connection, id: i64) -> rusqlite::Result<Option<Location>> {
    conn.query_row(
        "SELECT id, area, freezer, shelf, slot FROM locations WHERE id=?1",
        params![id],
        |row| {
            Ok(Location {
                id: row.get(0)?,
                area: row.get(1)?,
                freezer: row.get(2)?,
                shelf: row.get(3)?,
                slot: row.get(4)?,
            })
        },
    )
    .optional()
}

/// 查询某个位置当前是否被非归档样本占用，返回占用样本编号。
pub fn find_active_occupant(
    conn: &Connection,
    location_id: i64,
    exclude_sample_id: Option<i64>,
) -> rusqlite::Result<Option<String>> {
    let exclude = exclude_sample_id.unwrap_or(-1);
    conn.query_row(
        "SELECT sample_no FROM samples \
         WHERE location_id=?1 AND status != 'archived' AND id != ?2 LIMIT 1",
        params![location_id, exclude],
        |row| row.get(0),
    )
    .optional()
}

// ---------- 批次 ----------

pub fn find_batch_by_no(conn: &Connection, batch_no: &str) -> rusqlite::Result<Option<Batch>> {
    conn.query_row(
        "SELECT id, batch_no, project_name, owner, note, created_at FROM batches WHERE batch_no=?1",
        params![batch_no],
        row_to_batch,
    )
    .optional()
}

fn row_to_batch(row: &rusqlite::Row) -> rusqlite::Result<Batch> {
    Ok(Batch {
        id: row.get(0)?,
        batch_no: row.get(1)?,
        project_name: row.get(2)?,
        owner: row.get(3)?,
        note: row.get(4)?,
        created_at: row.get(5)?,
    })
}

// ---------- 样本 ----------

pub fn find_sample_by_no(conn: &Connection, sample_no: &str) -> rusqlite::Result<Option<Sample>> {
    let row = conn
        .query_row(
            "SELECT s.id, s.sample_no, b.batch_no, s.sample_type, s.status, s.location_id, s.last_processed_at \
             FROM samples s JOIN batches b ON s.batch_id = b.id WHERE s.sample_no=?1",
            params![sample_no],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;

    match row {
        None => Ok(None),
        Some((id, sample_no, batch_no, sample_type, status, location_id, last_processed_at)) => {
            let location = match location_id {
                Some(lid) => load_location(conn, lid)?,
                None => None,
            };
            Ok(Some(Sample {
                id,
                sample_no,
                batch_no,
                sample_type,
                status,
                location,
                last_processed_at,
            }))
        }
    }
}

/// 获取样本内部 id。
pub fn sample_id_by_no(conn: &Connection, sample_no: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM samples WHERE sample_no=?1",
        params![sample_no],
        |row| row.get(0),
    )
    .optional()
}

/// 根据过滤条件查询样本列表。
pub fn query_samples(conn: &Connection, q: &SampleQuery) -> rusqlite::Result<Vec<Sample>> {
    let mut sql = String::from(
        "SELECT s.sample_no FROM samples s \
         JOIN batches b ON s.batch_id = b.id \
         LEFT JOIN locations l ON s.location_id = l.id WHERE 1=1",
    );
    let mut binds: Vec<String> = Vec::new();

    if let Some(v) = &q.batch_no {
        sql.push_str(" AND b.batch_no = ?");
        binds.push(v.clone());
    }
    if let Some(v) = &q.status {
        sql.push_str(" AND s.status = ?");
        binds.push(v.clone());
    }
    if let Some(v) = &q.sample_type {
        sql.push_str(" AND s.sample_type = ?");
        binds.push(v.clone());
    }
    if let Some(v) = &q.area {
        sql.push_str(" AND l.area = ?");
        binds.push(v.clone());
    }
    if let Some(v) = &q.freezer {
        sql.push_str(" AND l.freezer = ?");
        binds.push(v.clone());
    }
    if let Some(v) = &q.shelf {
        sql.push_str(" AND l.shelf = ?");
        binds.push(v.clone());
    }
    if let Some(v) = &q.slot {
        sql.push_str(" AND l.slot = ?");
        binds.push(v.clone());
    }
    sql.push_str(" ORDER BY s.id ASC");

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> =
        binds.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let nos: Vec<String> = stmt
        .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(nos.len());
    for no in nos {
        if let Some(s) = find_sample_by_no(conn, &no)? {
            out.push(s);
        }
    }
    Ok(out)
}

// ---------- 操作日志 ----------

pub fn insert_log(
    conn: &Connection,
    sample_id: i64,
    operator: &str,
    action: &str,
    note: Option<&str>,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO operation_logs (sample_id, operator, action, note, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![sample_id, operator, action, note, now],
    )?;
    Ok(())
}

pub fn logs_for_sample(conn: &Connection, sample_id: i64) -> rusqlite::Result<Vec<OperationLog>> {
    let sample_no: String =
        conn.query_row("SELECT sample_no FROM samples WHERE id=?1", params![sample_id], |r| {
            r.get(0)
        })?;
    let mut stmt = conn.prepare(
        "SELECT id, operator, action, note, created_at FROM operation_logs \
         WHERE sample_id=?1 ORDER BY id ASC",
    )?;
    let logs = stmt
        .query_map(params![sample_id], |row| {
            Ok(OperationLog {
                id: row.get(0)?,
                sample_no: sample_no.clone(),
                operator: row.get(1)?,
                action: row.get(2)?,
                note: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(logs)
}

// ---------- 位置迁移记录 ----------

pub fn insert_movement(
    conn: &Connection,
    sample_id: i64,
    location_id: i64,
    direction: &str,
    operator: &str,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO location_movements (sample_id, location_id, direction, operator, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![sample_id, location_id, direction, operator, now],
    )?;
    Ok(())
}

pub fn recent_movements(
    conn: &Connection,
    location_id: i64,
) -> rusqlite::Result<Vec<LocationMovement>> {
    let mut stmt = conn.prepare(
        "SELECT s.sample_no, m.direction, m.operator, m.created_at \
         FROM location_movements m JOIN samples s ON m.sample_id = s.id \
         WHERE m.location_id=?1 ORDER BY m.id DESC LIMIT 10",
    )?;
    let out = stmt
        .query_map(params![location_id], |row| {
            Ok(LocationMovement {
                sample_no: row.get(0)?,
                direction: row.get(1)?,
                operator: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(out)
}

// ---------- 异常 ----------

pub fn open_exceptions_count(conn: &Connection, sample_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM exceptions WHERE sample_id=?1 AND status='open'",
        params![sample_id],
        |row| row.get(0),
    )
}

pub fn load_exception(conn: &Connection, id: i64) -> rusqlite::Result<Option<Exception>> {
    conn.query_row(
        "SELECT e.id, s.sample_no, e.exception_type, e.description, e.status, e.reported_by, \
         e.created_at, e.resolved_by, e.resolved_at \
         FROM exceptions e JOIN samples s ON e.sample_id = s.id WHERE e.id=?1",
        params![id],
        |row| {
            Ok(Exception {
                id: row.get(0)?,
                sample_no: row.get(1)?,
                exception_type: row.get(2)?,
                description: row.get(3)?,
                status: row.get(4)?,
                reported_by: row.get(5)?,
                created_at: row.get(6)?,
                resolved_by: row.get(7)?,
                resolved_at: row.get(8)?,
            })
        },
    )
    .optional()
}
