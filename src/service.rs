use chrono::Utc;
use rusqlite::params;
use serde_json::json;

use crate::db::DbPool;
use crate::error::AppError;
use crate::models::*;
use crate::repo;

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// 将 SQLite 的 UNIQUE 约束冲突映射为业务冲突错误，其余错误按内部错误处理。
fn map_unique_conflict(err: rusqlite::Error, message: &str) -> AppError {
    if let rusqlite::Error::SqliteFailure(e, _) = &err {
        if e.code == rusqlite::ErrorCode::ConstraintViolation {
            return AppError::conflict(message.to_string());
        }
    }
    AppError::from(err)
}

/// 校验必填字符串非空（去除首尾空白后不能为空）。
fn require_non_empty(field: &str, value: &str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        return Err(AppError::validation(format!("{field} 不能为空")));
    }
    Ok(())
}

/// 校验可选过滤字段：如果提供则不能为纯空白。
fn require_filter_non_empty(field: &str, value: &Option<String>) -> Result<(), AppError> {
    if let Some(v) = value {
        if v.trim().is_empty() {
            return Err(AppError::validation(format!("查询参数 {field} 不能为空字符串")));
        }
    }
    Ok(())
}

/// 校验位置四字段完整。
fn validate_location(loc: &LocationInput) -> Result<(), AppError> {
    require_non_empty("location.area", &loc.area)?;
    require_non_empty("location.freezer", &loc.freezer)?;
    require_non_empty("location.shelf", &loc.shelf)?;
    require_non_empty("location.slot", &loc.slot)?;
    Ok(())
}

fn is_valid_state(state: &str) -> bool {
    SAMPLE_STATES.contains(&state)
}

/// 状态流转规则：只能相邻推进，禁止跳跃与回退。
fn state_index(state: &str) -> Option<usize> {
    SAMPLE_STATES.iter().position(|s| *s == state)
}

/// 校验从 from 到 to 的流转是否合法。
fn validate_transition(from: &str, to: &str) -> Result<(), AppError> {
    if !is_valid_state(to) {
        return Err(AppError::validation(format!(
            "非法状态 '{to}'，合法状态为 {:?}",
            SAMPLE_STATES
        )));
    }
    let (fi, ti) = match (state_index(from), state_index(to)) {
        (Some(f), Some(t)) => (f, t),
        _ => return Err(AppError::internal("当前状态数据异常")),
    };
    if ti == fi {
        return Err(AppError::invalid_transition(format!(
            "样本已处于 '{to}' 状态"
        )));
    }
    if ti != fi + 1 {
        return Err(AppError::invalid_transition(format!(
            "禁止从 '{from}' 直接变更为 '{to}'，必须按 {:?} 顺序逐级流转",
            SAMPLE_STATES
        )));
    }
    Ok(())
}

// ---------- 批次 ----------

pub fn create_batch(pool: &DbPool, req: CreateBatchRequest) -> Result<Batch, AppError> {
    require_non_empty("batch_no", &req.batch_no)?;
    require_non_empty("project_name", &req.project_name)?;
    require_non_empty("owner", &req.owner)?;

    let conn = pool.get()?;
    if repo::find_batch_by_no(&conn, &req.batch_no)?.is_some() {
        return Err(AppError::conflict(format!(
            "批次编号 '{}' 已存在",
            req.batch_no
        )));
    }
    let created_at = now();
    conn.execute(
        "INSERT INTO batches (batch_no, project_name, owner, note, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            req.batch_no,
            req.project_name,
            req.owner,
            req.note,
            created_at
        ],
    )
    .map_err(|e| map_unique_conflict(e, &format!("批次编号 '{}' 已存在", req.batch_no)))?;
    repo::find_batch_by_no(&conn, &req.batch_no)?
        .ok_or_else(|| AppError::internal("批次创建后无法读取"))
}

// ---------- 样本登记 ----------

pub fn register_sample(
    pool: &DbPool,
    batch_no: &str,
    req: RegisterSampleRequest,
) -> Result<Sample, AppError> {
    require_non_empty("sample_no", &req.sample_no)?;
    require_non_empty("sample_type", &req.sample_type)?;
    if let Some(loc) = &req.location {
        validate_location(loc)?;
    }

    let mut conn = pool.get()?;
    let batch = repo::find_batch_by_no(&conn, batch_no)?
        .ok_or_else(|| AppError::not_found(format!("批次 '{batch_no}' 不存在")))?;
    if repo::find_sample_by_no(&conn, &req.sample_no)?.is_some() {
        return Err(AppError::conflict(format!(
            "样本编号 '{}' 已存在",
            req.sample_no
        )));
    }

    let ts = now();
    let tx = conn.transaction()?;

    let location_id = match &req.location {
        Some(loc) => {
            let lid = repo::find_or_create_location(&tx, loc)?;
            if let Some(occupant) = repo::find_active_occupant(&tx, lid, None)? {
                return Err(AppError::conflict(format!(
                    "目标位置已被样本 '{occupant}' 占用"
                )));
            }
            Some(lid)
        }
        None => None,
    };

    tx.execute(
        "INSERT INTO samples (sample_no, batch_id, sample_type, status, location_id, last_processed_at) \
         VALUES (?1, ?2, ?3, 'collected', ?4, ?5)",
        params![req.sample_no, batch.id, req.sample_type, location_id, ts],
    )
    .map_err(|e| map_unique_conflict(e, &format!("样本编号 '{}' 已存在", req.sample_no)))?;
    let sample_id = tx.last_insert_rowid();
    repo::insert_log(&tx, sample_id, "system", "created", Some("样本登记"), &ts)?;
    if let Some(lid) = location_id {
        repo::insert_movement(&tx, sample_id, lid, "in", "system", &ts)?;
    }
    tx.commit()?;

    repo::find_sample_by_no(&conn, &req.sample_no)?
        .ok_or_else(|| AppError::internal("样本登记后无法读取"))
}

// ---------- 批量导入 ----------

pub fn bulk_import(
    pool: &DbPool,
    batch_no: &str,
    req: BulkImportRequest,
) -> Result<BulkImportResponse, AppError> {
    if req.samples.is_empty() {
        return Err(AppError::validation("samples 不能为空"));
    }
    // 预校验：字段完整性 + 请求内编号 / 位置去重。
    let mut seen_nos = std::collections::HashSet::new();
    let mut seen_locs = std::collections::HashSet::new();
    for (i, item) in req.samples.iter().enumerate() {
        if item.sample_no.trim().is_empty() {
            return Err(AppError::validation(format!("第 {} 条 sample_no 不能为空", i + 1)));
        }
        if item.sample_type.trim().is_empty() {
            return Err(AppError::validation(format!(
                "第 {} 条 sample_type 不能为空",
                i + 1
            )));
        }
        if !seen_nos.insert(item.sample_no.clone()) {
            return Err(AppError::validation(format!(
                "请求内样本编号 '{}' 重复",
                item.sample_no
            )));
        }
        if let Some(loc) = &item.location {
            validate_location(loc)?;
            let key = format!("{}|{}|{}|{}", loc.area, loc.freezer, loc.shelf, loc.slot);
            if !seen_locs.insert(key) {
                return Err(AppError::validation(format!(
                    "请求内位置重复：区域 {} 冰箱 {} 层架 {} 格位 {}",
                    loc.area, loc.freezer, loc.shelf, loc.slot
                )));
            }
        }
    }

    let mut conn = pool.get()?;
    let batch = repo::find_batch_by_no(&conn, batch_no)?
        .ok_or_else(|| AppError::not_found(format!("批次 '{batch_no}' 不存在")))?;

    let ts = now();
    let tx = conn.transaction()?;
    let mut imported_nos = Vec::new();

    for item in &req.samples {
        if repo::find_sample_by_no(&tx, &item.sample_no)?.is_some() {
            // 整批回滚
            return Err(AppError::conflict(format!(
                "样本编号 '{}' 已存在，整批导入已回滚",
                item.sample_no
            )));
        }
        let location_id = match &item.location {
            Some(loc) => {
                let lid = repo::find_or_create_location(&tx, loc)?;
                if let Some(occupant) = repo::find_active_occupant(&tx, lid, None)? {
                    return Err(AppError::conflict(format!(
                        "位置已被样本 '{occupant}' 占用，整批导入已回滚"
                    )));
                }
                Some(lid)
            }
            None => None,
        };
        tx.execute(
            "INSERT INTO samples (sample_no, batch_id, sample_type, status, location_id, last_processed_at) \
             VALUES (?1, ?2, ?3, 'collected', ?4, ?5)",
            params![item.sample_no, batch.id, item.sample_type, location_id, ts],
        )?;
        let sid = tx.last_insert_rowid();
        repo::insert_log(&tx, sid, "system", "created", Some("批量导入"), &ts)?;
        if let Some(lid) = location_id {
            repo::insert_movement(&tx, sid, lid, "in", "system", &ts)?;
        }
        imported_nos.push(item.sample_no.clone());
    }
    tx.commit()?;

    let mut samples = Vec::new();
    for no in &imported_nos {
        if let Some(s) = repo::find_sample_by_no(&conn, no)? {
            samples.push(s);
        }
    }
    Ok(BulkImportResponse {
        imported: samples.len(),
        samples,
    })
}

// ---------- 状态更新 ----------

pub fn update_status(
    pool: &DbPool,
    sample_no: &str,
    req: UpdateStatusRequest,
) -> Result<Sample, AppError> {
    require_non_empty("operator", &req.operator)?;
    require_non_empty("status", &req.status)?;

    let mut conn = pool.get()?;
    let sample = repo::find_sample_by_no(&conn, sample_no)?
        .ok_or_else(|| AppError::not_found(format!("样本 '{sample_no}' 不存在")))?;

    validate_transition(&sample.status, &req.status)?;

    // 存在未解除异常时禁止流转。
    if repo::open_exceptions_count(&conn, sample.id)? > 0 {
        return Err(AppError::conflict(
            "样本存在未解除的异常标记，禁止状态流转",
        ));
    }

    let ts = now();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE samples SET status=?1, last_processed_at=?2 WHERE id=?3",
        params![req.status, ts, sample.id],
    )?;
    repo::insert_log(
        &tx,
        sample.id,
        &req.operator,
        &format!("status:{}", req.status),
        req.note.as_deref(),
        &ts,
    )?;
    // 归档时释放位置占用：记录迁出并清空样本当前位置。
    if req.status == "archived" {
        if let Some(loc) = &sample.location {
            repo::insert_movement(&tx, sample.id, loc.id, "out", &req.operator, &ts)?;
            tx.execute(
                "UPDATE samples SET location_id = NULL WHERE id=?1",
                params![sample.id],
            )?;
        }
    }
    tx.commit()?;

    repo::find_sample_by_no(&conn, sample_no)?
        .ok_or_else(|| AppError::internal("状态更新后无法读取"))
}

// ---------- 位置变更 ----------

pub fn change_location(
    pool: &DbPool,
    sample_no: &str,
    req: ChangeLocationRequest,
) -> Result<Sample, AppError> {
    require_non_empty("operator", &req.operator)?;
    validate_location(&req.location)?;

    let mut conn = pool.get()?;
    let sample = repo::find_sample_by_no(&conn, sample_no)?
        .ok_or_else(|| AppError::not_found(format!("样本 '{sample_no}' 不存在")))?;

    if sample.status == "archived" {
        return Err(AppError::conflict("已归档样本不可再变更位置"));
    }

    let ts = now();
    let tx = conn.transaction()?;
    let new_lid = repo::find_or_create_location(&tx, &req.location)?;
    if let Some(occupant) = repo::find_active_occupant(&tx, new_lid, Some(sample.id))? {
        return Err(AppError::conflict(format!(
            "目标位置已被样本 '{occupant}' 占用"
        )));
    }

    let old_loc_desc = sample
        .location
        .as_ref()
        .map(|l| format!("{}/{}/{}/{}", l.area, l.freezer, l.shelf, l.slot))
        .unwrap_or_else(|| "无".to_string());
    let new_loc_desc = format!(
        "{}/{}/{}/{}",
        req.location.area, req.location.freezer, req.location.shelf, req.location.slot
    );

    if let Some(old) = &sample.location {
        repo::insert_movement(&tx, sample.id, old.id, "out", &req.operator, &ts)?;
    }
    tx.execute(
        "UPDATE samples SET location_id=?1, last_processed_at=?2 WHERE id=?3",
        params![new_lid, ts, sample.id],
    )?;
    repo::insert_movement(&tx, sample.id, new_lid, "in", &req.operator, &ts)?;
    // 自动说明始终保留，手动备注追加在后，不允许盖掉旧/新位置信息。
    let auto = format!("位置变更: {old_loc_desc} -> {new_loc_desc}");
    let note = match req.note.as_deref().map(str::trim) {
        Some(n) if !n.is_empty() => format!("{auto}，{n}"),
        _ => auto,
    };
    repo::insert_log(&tx, sample.id, &req.operator, "location_changed", Some(&note), &ts)?;
    tx.commit()?;

    repo::find_sample_by_no(&conn, sample_no)?
        .ok_or_else(|| AppError::internal("位置变更后无法读取"))
}

// ---------- 异常标记 / 解除 ----------

pub fn mark_exception(
    pool: &DbPool,
    sample_no: &str,
    req: MarkExceptionRequest,
) -> Result<Exception, AppError> {
    require_non_empty("exception_type", &req.exception_type)?;
    require_non_empty("reported_by", &req.reported_by)?;

    let mut conn = pool.get()?;
    let sample = repo::find_sample_by_no(&conn, sample_no)?
        .ok_or_else(|| AppError::not_found(format!("样本 '{sample_no}' 不存在")))?;

    let ts = now();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO exceptions (sample_id, exception_type, description, status, reported_by, created_at) \
         VALUES (?1, ?2, ?3, 'open', ?4, ?5)",
        params![
            sample.id,
            req.exception_type,
            req.description,
            req.reported_by,
            ts
        ],
    )?;
    let ex_id = tx.last_insert_rowid();
    repo::insert_log(
        &tx,
        sample.id,
        &req.reported_by,
        &format!("exception:{}", req.exception_type),
        req.description.as_deref(),
        &ts,
    )?;
    tx.commit()?;

    repo::load_exception(&conn, ex_id)?
        .ok_or_else(|| AppError::internal("异常标记后无法读取"))
}

pub fn resolve_exception(
    pool: &DbPool,
    exception_id: i64,
    req: ResolveExceptionRequest,
) -> Result<Exception, AppError> {
    require_non_empty("resolved_by", &req.resolved_by)?;

    let mut conn = pool.get()?;
    let ex = repo::load_exception(&conn, exception_id)?
        .ok_or_else(|| AppError::not_found(format!("异常 #{exception_id} 不存在")))?;
    if ex.status == "resolved" {
        return Err(AppError::conflict("该异常已解除"));
    }
    let sample_id = repo::sample_id_by_no(&conn, &ex.sample_no)?
        .ok_or_else(|| AppError::internal("异常对应样本缺失"))?;

    let ts = now();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE exceptions SET status='resolved', resolved_by=?1, resolved_at=?2 WHERE id=?3",
        params![req.resolved_by, ts, exception_id],
    )?;
    repo::insert_log(
        &tx,
        sample_id,
        &req.resolved_by,
        "exception_resolved",
        req.note.as_deref(),
        &ts,
    )?;
    tx.commit()?;

    repo::load_exception(&conn, exception_id)?
        .ok_or_else(|| AppError::internal("异常解除后无法读取"))
}

// ---------- 查询 ----------

pub fn get_sample(pool: &DbPool, sample_no: &str) -> Result<Sample, AppError> {
    let conn = pool.get()?;
    repo::find_sample_by_no(&conn, sample_no)?
        .ok_or_else(|| AppError::not_found(format!("样本 '{sample_no}' 不存在")))
}

pub fn query_samples(pool: &DbPool, q: SampleQuery) -> Result<Vec<Sample>, AppError> {
    require_filter_non_empty("batch_no", &q.batch_no)?;
    require_filter_non_empty("status", &q.status)?;
    require_filter_non_empty("sample_type", &q.sample_type)?;
    require_filter_non_empty("area", &q.area)?;
    require_filter_non_empty("freezer", &q.freezer)?;
    require_filter_non_empty("shelf", &q.shelf)?;
    require_filter_non_empty("slot", &q.slot)?;

    if let Some(st) = &q.status {
        if !is_valid_state(st) {
            return Err(AppError::validation(format!(
                "非法状态过滤 '{st}'，合法状态为 {:?}",
                SAMPLE_STATES
            )));
        }
    }

    let conn = pool.get()?;
    // 若指定批次，先确认批次存在，便于返回 404 而非空列表。
    if let Some(bn) = &q.batch_no {
        if repo::find_batch_by_no(&conn, bn)?.is_none() {
            return Err(AppError::not_found(format!("批次 '{bn}' 不存在")));
        }
    }
    Ok(repo::query_samples(&conn, &q)?)
}

pub fn sample_logs(pool: &DbPool, sample_no: &str) -> Result<Vec<OperationLog>, AppError> {
    let conn = pool.get()?;
    let sample = repo::find_sample_by_no(&conn, sample_no)?
        .ok_or_else(|| AppError::not_found(format!("样本 '{sample_no}' 不存在")))?;
    Ok(repo::logs_for_sample(&conn, sample.id)?)
}

pub fn get_location(pool: &DbPool, q: SampleQuery) -> Result<LocationView, AppError> {
    // 位置检索需要四字段齐全。
    let loc = LocationInput {
        area: q.area.clone().ok_or_else(|| AppError::validation("缺少 area"))?,
        freezer: q
            .freezer
            .clone()
            .ok_or_else(|| AppError::validation("缺少 freezer"))?,
        shelf: q.shelf.clone().ok_or_else(|| AppError::validation("缺少 shelf"))?,
        slot: q.slot.clone().ok_or_else(|| AppError::validation("缺少 slot"))?,
    };
    validate_location(&loc)?;

    let conn = pool.get()?;
    let existing: Option<i64> = {
        use rusqlite::OptionalExtension;
        conn.query_row(
            "SELECT id FROM locations WHERE area=?1 AND freezer=?2 AND shelf=?3 AND slot=?4",
            params![loc.area, loc.freezer, loc.shelf, loc.slot],
            |row| row.get(0),
        )
        .optional()?
    };
    let location_id = existing.ok_or_else(|| {
        AppError::not_found("该位置尚无记录".to_string())
            .with_details(json!({ "location": {
                "area": loc.area, "freezer": loc.freezer, "shelf": loc.shelf, "slot": loc.slot
            }}))
    })?;

    let location = repo::load_location(&conn, location_id)?
        .ok_or_else(|| AppError::internal("位置读取失败"))?;
    let current_sample = repo::find_active_occupant(&conn, location_id, None)?;
    let recent_movements = repo::recent_movements(&conn, location_id)?;
    Ok(LocationView {
        location,
        current_sample,
        recent_movements,
    })
}
