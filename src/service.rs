use crate::error::ApiError;
use crate::models::*;
use crate::repo;
use chrono::Utc;
use rusqlite::Connection;
use serde_json::json;

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn require_non_empty(field: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::validation(format!("字段 {field} 不能为空")));
    }
    Ok(())
}

/// 可选操作者：一旦填写就不允许是纯空格
fn validate_optional_operator(operator: &Option<String>) -> Result<(), ApiError> {
    if let Some(op) = operator {
        if op.trim().is_empty() {
            return Err(ApiError::validation("operator 不能为空白字符"));
        }
    }
    Ok(())
}

/// 查询参数：传了就不允许是空值或纯空格
fn reject_empty_filter(field: &str, value: &Option<String>) -> Result<(), ApiError> {
    if let Some(v) = value {
        if v.trim().is_empty() {
            return Err(ApiError::validation(format!("查询参数 {field} 不能为空值")));
        }
    }
    Ok(())
}

// ---------- 批次 ----------

pub fn create_batch(conn: &Connection, req: &CreateBatchReq) -> Result<Batch, ApiError> {
    require_non_empty("batch_no", &req.batch_no)?;
    require_non_empty("project_name", &req.project_name)?;
    require_non_empty("manager", &req.manager)?;
    repo::insert_batch(conn, req, &now())
}

pub fn list_batches(conn: &Connection) -> Result<Vec<Batch>, ApiError> {
    repo::list_batches(conn)
}

pub fn get_batch(conn: &Connection, batch_no: &str) -> Result<Batch, ApiError> {
    repo::get_batch(conn, batch_no)
}

// ---------- 样本登记 ----------

fn validate_sample_fields(sample_no: &str, sample_type: &str) -> Result<(), ApiError> {
    require_non_empty("sample_no", sample_no)?;
    require_non_empty("sample_type", sample_type)?;
    Ok(())
}

fn ensure_batch_exists(conn: &Connection, batch_no: &str) -> Result<(), ApiError> {
    require_non_empty("batch_no", batch_no)?;
    if !repo::batch_exists(conn, batch_no)? {
        return Err(ApiError::not_found(
            "BATCH_NOT_FOUND",
            format!("批次 {batch_no} 不存在"),
        ));
    }
    Ok(())
}

pub fn register_sample(conn: &Connection, req: &RegisterSampleReq) -> Result<Sample, ApiError> {
    validate_sample_fields(&req.sample_no, &req.sample_type)?;
    ensure_batch_exists(conn, &req.batch_no)?;
    validate_optional_operator(&req.operator)?;

    let ts = now();
    let sample = repo::insert_sample(conn, &req.sample_no, &req.batch_no, &req.sample_type, &ts)?;
    let operator = req.operator.as_deref().unwrap_or("system");
    let note = req.note.clone().unwrap_or_else(|| "样本登记".to_string());
    repo::insert_log(conn, &sample.sample_no, operator, "register", &note, &ts)?;
    Ok(sample)
}

/// 批量导入：单个事务，全部成功或全部回滚；
/// 任一编号重复、位置非法、类型为空都会回滚，导入成功为每个样本写入 action=created 的初始日志
pub fn bulk_import(
    conn: &mut Connection,
    batch_no: &str,
    req: &BulkImportReq,
) -> Result<BulkImportResult, ApiError> {
    ensure_batch_exists(conn, batch_no)?;
    validate_optional_operator(&req.operator)?;
    if req.samples.is_empty() {
        return Err(ApiError::validation("samples 列表不能为空"));
    }
    // 导入前统一校验，任何一条不合法直接整批拒绝
    for (i, item) in req.samples.iter().enumerate() {
        validate_sample_fields(&item.sample_no, &item.sample_type)
            .map_err(|e| ApiError::validation(format!("第 {} 条样本: {}", i + 1, e.message)))?;
        if let Some(loc) = &item.location {
            for (field, v) in [
                ("region", &loc.region),
                ("freezer_no", &loc.freezer_no),
                ("shelf", &loc.shelf),
                ("slot", &loc.slot),
            ] {
                if v.trim().is_empty() {
                    return Err(ApiError::validation(format!(
                        "第 {} 条样本: 位置非法，location.{field} 不能为空",
                        i + 1
                    )));
                }
            }
        }
    }

    let operator = req.operator.as_deref().unwrap_or("system").to_string();
    let ts = now();
    let tx = conn.transaction()?;
    let mut samples = Vec::with_capacity(req.samples.len());
    for (i, item) in req.samples.iter().enumerate() {
        repo::insert_sample(&tx, &item.sample_no, batch_no, &item.sample_type, &ts)?;
        if let Some(loc) = &item.location {
            let location = repo::find_or_create_location(
                &tx,
                loc.region.trim(),
                loc.freezer_no.trim(),
                loc.shelf.trim(),
                loc.slot.trim(),
            )?;
            // 占用校验同样在事务内执行：与本批次前面导入的样本冲突也会整批回滚
            ensure_location_available(&tx, &location, None, &format!("第 {} 条样本: ", i + 1))?;
            repo::update_sample_location(&tx, &item.sample_no, location.id, &ts)?;
            repo::insert_movement(&tx, &item.sample_no, None, Some(location.id), &operator, &ts)?;
        }
        repo::insert_log(&tx, &item.sample_no, &operator, "created", "批量导入", &ts)?;
        samples.push(repo::get_sample(&tx, &item.sample_no)?);
    }
    tx.commit()?;

    Ok(BulkImportResult {
        batch_no: batch_no.to_string(),
        imported: samples.len(),
        samples,
    })
}

pub fn get_sample(conn: &Connection, sample_no: &str) -> Result<Sample, ApiError> {
    repo::get_sample(conn, sample_no)
}

pub fn query_samples(conn: &Connection, filter: &SampleFilter) -> Result<Vec<Sample>, ApiError> {
    reject_empty_filter("batch_no", &filter.batch_no)?;
    reject_empty_filter("status", &filter.status)?;
    reject_empty_filter("sample_type", &filter.sample_type)?;
    reject_empty_filter("region", &filter.region)?;
    reject_empty_filter("freezer_no", &filter.freezer_no)?;
    reject_empty_filter("shelf", &filter.shelf)?;
    reject_empty_filter("slot", &filter.slot)?;
    if let Some(status) = &filter.status {
        if !is_valid_status(status) {
            return Err(ApiError::validation(format!(
                "非法状态 {status}，允许值: {}",
                STATUSES.join(", ")
            )));
        }
    }
    repo::query_samples(conn, filter)
}

pub fn list_batch_samples(conn: &Connection, batch_no: &str) -> Result<Vec<Sample>, ApiError> {
    ensure_batch_exists(conn, batch_no)?;
    repo::query_samples(
        conn,
        &SampleFilter {
            batch_no: Some(batch_no.to_string()),
            ..Default::default()
        },
    )
}

// ---------- 状态流转 ----------

pub fn update_status(
    conn: &Connection,
    sample_no: &str,
    req: &UpdateStatusReq,
) -> Result<Sample, ApiError> {
    require_non_empty("operator", &req.operator)?;
    if !is_valid_status(&req.status) {
        return Err(ApiError::validation(format!(
            "非法状态 {}，允许值: {}",
            req.status,
            STATUSES.join(", ")
        )));
    }

    let sample = repo::get_sample(conn, sample_no)?;
    if !can_transition(&sample.status, &req.status) {
        return Err(ApiError::conflict(
            "INVALID_STATE_TRANSITION",
            format!("不允许从 {} 直接流转到 {}", sample.status, req.status),
        )
        .with_details(json!({
            "from": sample.status,
            "to": req.status,
            "rule": "collected -> stored -> processing -> transferred -> archived",
        })));
    }

    let ts = now();
    repo::update_sample_status(conn, sample_no, &req.status, &ts)?;
    // 归档释放位置：写入迁出历史（to 为 NULL 表示释放）并清空样本当前位置
    if req.status == "archived" {
        if let Some(old) = &sample.location {
            repo::insert_movement(conn, sample_no, Some(old.id), None, &req.operator, &ts)?;
            repo::clear_sample_location(conn, sample_no)?;
        }
    }
    let note = format!(
        "状态变更: {} -> {}{}",
        sample.status,
        req.status,
        req.note
            .as_deref()
            .map(|n| format!("，{n}"))
            .unwrap_or_default()
    );
    repo::insert_log(conn, sample_no, &req.operator, "status_update", &note, &ts)?;
    repo::get_sample(conn, sample_no)
}

// ---------- 位置变更 ----------

fn fmt_location(loc: Option<&Location>) -> String {
    match loc {
        Some(l) => format!("{}/{}/{}/{}", l.region, l.freezer_no, l.shelf, l.slot),
        None => "(未设置)".to_string(),
    }
}

/// 位置占用校验：同一位置同一时间只允许一个未归档样本（归档即释放）
fn ensure_location_available(
    conn: &Connection,
    loc: &Location,
    exclude_sample_no: Option<&str>,
    context: &str,
) -> Result<(), ApiError> {
    if let Some(occupant) = repo::find_active_occupant(conn, loc.id, exclude_sample_no)? {
        return Err(ApiError::conflict(
            "LOCATION_OCCUPIED",
            format!(
                "{context}位置 {} 已被样本 {} 占用",
                fmt_location(Some(loc)),
                occupant.sample_no
            ),
        )
        .with_details(json!({
            "location_id": loc.id,
            "occupant": occupant.sample_no,
        })));
    }
    Ok(())
}

pub fn change_location(
    conn: &Connection,
    sample_no: &str,
    req: &ChangeLocationReq,
) -> Result<Sample, ApiError> {
    require_non_empty("region", &req.region)?;
    require_non_empty("freezer_no", &req.freezer_no)?;
    require_non_empty("shelf", &req.shelf)?;
    require_non_empty("slot", &req.slot)?;
    require_non_empty("operator", &req.operator)?;

    let sample = repo::get_sample(conn, sample_no)?;
    let ts = now();
    // 位置字段先 trim 再使用，避免 "A区 " 被当作新位置绕过占用校验
    let loc = repo::find_or_create_location(
        conn,
        req.region.trim(),
        req.freezer_no.trim(),
        req.shelf.trim(),
        req.slot.trim(),
    )?;
    ensure_location_available(conn, &loc, Some(sample_no), "")?;

    let old_desc = fmt_location(sample.location.as_ref());
    repo::update_sample_location(conn, sample_no, loc.id, &ts)?;
    repo::insert_movement(
        conn,
        sample_no,
        sample.location.as_ref().map(|l| l.id),
        Some(loc.id),
        &req.operator,
        &ts,
    )?;
    let note = format!(
        "位置变更: {} -> {}{}",
        old_desc,
        fmt_location(Some(&loc)),
        req.note
            .as_deref()
            .map(|n| format!("，{n}"))
            .unwrap_or_default()
    );
    repo::insert_log(conn, sample_no, &req.operator, "location_change", &note, &ts)?;
    repo::get_sample(conn, sample_no)
}

// ---------- 位置查询 ----------

fn required_param(field: &str, value: &Option<String>) -> Result<String, ApiError> {
    match value {
        Some(v) if !v.trim().is_empty() => Ok(v.trim().to_string()),
        _ => Err(ApiError::validation(format!(
            "查询参数 {field} 为必填且不能为空值"
        ))),
    }
}

/// 查询位置详情：当前占用的未归档样本 + 最近 10 次迁入迁出记录
pub fn get_location_detail(
    conn: &Connection,
    query: &LocationQuery,
) -> Result<LocationDetail, ApiError> {
    let region = required_param("region", &query.region)?;
    let freezer_no = required_param("freezer_no", &query.freezer_no)?;
    let shelf = required_param("shelf", &query.shelf)?;
    let slot = required_param("slot", &query.slot)?;

    let loc = repo::get_location_by_fields(conn, &region, &freezer_no, &shelf, &slot)?
        .ok_or_else(|| {
            ApiError::not_found(
                "LOCATION_NOT_FOUND",
                format!("位置 {region}/{freezer_no}/{shelf}/{slot} 不存在"),
            )
        })?;
    let current_sample = repo::find_active_occupant(conn, loc.id, None)?;
    let recent_movements = repo::list_movements_for_location(conn, loc.id, 10)?;
    Ok(LocationDetail {
        location: loc,
        current_sample,
        recent_movements,
    })
}

// ---------- 异常标记 ----------

pub fn add_exception(
    conn: &Connection,
    sample_no: &str,
    req: &AddExceptionReq,
) -> Result<ExceptionFlag, ApiError> {
    require_non_empty("operator", &req.operator)?;
    if !is_valid_flag_type(&req.flag_type) {
        return Err(ApiError::validation(format!(
            "非法异常类型 {}，允许值: {}",
            req.flag_type,
            FLAG_TYPES.join(", ")
        )));
    }

    repo::get_sample(conn, sample_no)?;
    let ts = now();
    let flag = repo::insert_exception(conn, sample_no, &req.flag_type, &req.description, &ts)?;
    let note = format!("异常标记: {}，{}", req.flag_type, req.description);
    repo::insert_log(conn, sample_no, &req.operator, "exception_flag", &note, &ts)?;
    Ok(flag)
}

pub fn resolve_exception(
    conn: &Connection,
    sample_no: &str,
    exception_id: i64,
    req: &ResolveExceptionReq,
) -> Result<ExceptionFlag, ApiError> {
    require_non_empty("operator", &req.operator)?;

    repo::get_sample(conn, sample_no)?;
    let flag = repo::get_exception(conn, exception_id)?;
    if flag.sample_no != sample_no {
        return Err(ApiError::not_found(
            "EXCEPTION_NOT_FOUND",
            format!("样本 {sample_no} 下不存在异常标记 {exception_id}"),
        ));
    }
    if flag.resolved {
        return Err(ApiError::conflict(
            "ALREADY_RESOLVED",
            format!("异常标记 {exception_id} 已解除"),
        ));
    }

    let ts = now();
    repo::resolve_exception(conn, exception_id, &ts)?;
    let note = format!(
        "异常解除: #{exception_id} ({}){}",
        flag.flag_type,
        req.note
            .as_deref()
            .map(|n| format!("，{n}"))
            .unwrap_or_default()
    );
    repo::insert_log(conn, sample_no, &req.operator, "exception_resolve", &note, &ts)?;
    repo::get_exception(conn, exception_id)
}

pub fn list_exceptions(
    conn: &Connection,
    sample_no: &str,
) -> Result<Vec<ExceptionFlag>, ApiError> {
    repo::get_sample(conn, sample_no)?;
    repo::list_exceptions(conn, sample_no)
}

// ---------- 流转日志 ----------

pub fn list_logs(conn: &Connection, sample_no: &str) -> Result<Vec<OperationLog>, ApiError> {
    repo::get_sample(conn, sample_no)?;
    repo::list_logs(conn, sample_no)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use axum::http::StatusCode;

    fn setup() -> Connection {
        let conn = db::init_memory();
        create_batch(
            &conn,
            &CreateBatchReq {
                batch_no: "B001".into(),
                project_name: "肿瘤标志物研究".into(),
                manager: "张三".into(),
                remark: String::new(),
            },
        )
        .unwrap();
        conn
    }

    fn register(conn: &Connection, sample_no: &str) -> Sample {
        register_sample(
            conn,
            &RegisterSampleReq {
                sample_no: sample_no.into(),
                batch_no: "B001".into(),
                sample_type: "blood".into(),
                operator: Some("alice".into()),
                note: None,
            },
        )
        .unwrap()
    }

    fn set_status(conn: &Connection, sample_no: &str, status: &str) -> Result<Sample, ApiError> {
        update_status(
            conn,
            sample_no,
            &UpdateStatusReq {
                status: status.into(),
                operator: "alice".into(),
                note: None,
            },
        )
    }

    // ---------- 输入校验 ----------

    #[test]
    fn batch_validation_rejects_empty_fields() {
        let conn = db::init_memory();
        let err = create_batch(
            &conn,
            &CreateBatchReq {
                batch_no: "  ".into(),
                project_name: "p".into(),
                manager: "m".into(),
                remark: String::new(),
            },
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "VALIDATION_ERROR");
    }

    #[test]
    fn duplicate_batch_conflicts() {
        let conn = setup();
        let err = create_batch(
            &conn,
            &CreateBatchReq {
                batch_no: "B001".into(),
                project_name: "另一个项目".into(),
                manager: "李四".into(),
                remark: String::new(),
            },
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.code, "DUPLICATE_BATCH");
    }

    #[test]
    fn register_rejects_unknown_batch_and_empty_fields() {
        let conn = setup();
        let err = register_sample(
            &conn,
            &RegisterSampleReq {
                sample_no: "S001".into(),
                batch_no: "NOPE".into(),
                sample_type: "blood".into(),
                operator: None,
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "BATCH_NOT_FOUND");

        let err = register_sample(
            &conn,
            &RegisterSampleReq {
                sample_no: "S001".into(),
                batch_no: "B001".into(),
                sample_type: "".into(),
                operator: None,
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "VALIDATION_ERROR");
    }

    #[test]
    fn update_status_rejects_invalid_status_value() {
        let conn = setup();
        register(&conn, "S001");
        let err = set_status(&conn, "S001", "destroyed").unwrap_err();
        assert_eq!(err.code, "VALIDATION_ERROR");
    }

    #[test]
    fn filter_rejects_invalid_status_value() {
        let conn = setup();
        let err = query_samples(
            &conn,
            &SampleFilter {
                status: Some("destroyed".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "VALIDATION_ERROR");
    }

    // ---------- 状态流转 ----------

    #[test]
    fn full_lifecycle_chain_succeeds() {
        let conn = setup();
        register(&conn, "S001");
        for s in ["stored", "processing", "transferred", "archived"] {
            let sample = set_status(&conn, "S001", s).unwrap();
            assert_eq!(sample.status, s);
        }
        // 1 次登记 + 4 次状态变更
        let logs = list_logs(&conn, "S001").unwrap();
        assert_eq!(logs.len(), 5);
        assert_eq!(logs[0].action, "register");
        assert!(logs[1].note.contains("collected -> stored"));
        assert!(logs[4].note.contains("transferred -> archived"));
    }

    #[test]
    fn skipping_key_states_is_forbidden() {
        let conn = setup();
        register(&conn, "S001");
        // collected 不能直接 archived（需求示例）
        let err = set_status(&conn, "S001", "archived").unwrap_err();
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.code, "INVALID_STATE_TRANSITION");

        // collected 不能直接 processing
        assert!(set_status(&conn, "S001", "processing").is_err());
        // 正常前进一步后仍不能跳步
        set_status(&conn, "S001", "stored").unwrap();
        assert!(set_status(&conn, "S001", "archived").is_err());
        // 不能回退
        assert!(set_status(&conn, "S001", "collected").is_err());
    }

    #[test]
    fn archived_is_terminal() {
        let conn = setup();
        register(&conn, "S001");
        for s in ["stored", "processing", "transferred", "archived"] {
            set_status(&conn, "S001", s).unwrap();
        }
        let err = set_status(&conn, "S001", "stored").unwrap_err();
        assert_eq!(err.code, "INVALID_STATE_TRANSITION");
    }

    // ---------- 批量导入 ----------

    #[test]
    fn bulk_import_is_atomic_on_duplicate() {
        let mut conn = setup();
        register(&conn, "S_DUP");
        let err = bulk_import(
            &mut conn,
            "B001",
            &BulkImportReq {
                operator: Some("alice".into()),
                samples: vec![
                    BulkImportItem {
                        sample_no: "S101".into(),
                        sample_type: "tissue".into(),
                        location: None,
                    },
                    BulkImportItem {
                        sample_no: "S_DUP".into(),
                        sample_type: "blood".into(),
                        location: None,
                    },
                ],
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "DUPLICATE_SAMPLE");
        // 回滚：S101 不应存在，且没有写入任何日志
        assert!(repo::get_sample(&conn, "S101").is_err());
        assert!(repo::list_logs(&conn, "S101").unwrap().is_empty());
    }

    #[test]
    fn bulk_import_rejects_empty_list_and_bad_item() {
        let mut conn = setup();
        let err = bulk_import(
            &mut conn,
            "B001",
            &BulkImportReq {
                operator: None,
                samples: vec![],
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "VALIDATION_ERROR");

        let err = bulk_import(
            &mut conn,
            "B001",
            &BulkImportReq {
                operator: None,
                samples: vec![BulkImportItem {
                    sample_no: "".into(),
                    sample_type: "blood".into(),
                    location: None,
                }],
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "VALIDATION_ERROR");
        assert!(err.message.contains("第 1 条"));
    }

    // ---------- 批量导入（扩展：位置与 created 日志） ----------

    #[test]
    fn bulk_import_all_success_with_locations_and_created_logs() {
        let mut conn = setup();
        let result = bulk_import(
            &mut conn,
            "B001",
            &BulkImportReq {
                operator: Some("alice".into()),
                samples: vec![
                    BulkImportItem {
                        sample_no: "S201".into(),
                        sample_type: "blood".into(),
                        location: Some(BulkImportLocation {
                            region: "A区".into(),
                            freezer_no: "F-01".into(),
                            shelf: "1".into(),
                            slot: "A1".into(),
                        }),
                    },
                    BulkImportItem {
                        sample_no: "S202".into(),
                        sample_type: "tissue".into(),
                        location: None,
                    },
                ],
            },
        )
        .unwrap();
        assert_eq!(result.imported, 2);

        // 位置写入正确
        let s1 = repo::get_sample(&conn, "S201").unwrap();
        assert_eq!(s1.location.as_ref().unwrap().freezer_no, "F-01");
        assert_eq!(s1.location.as_ref().unwrap().slot, "A1");
        assert!(repo::get_sample(&conn, "S202").unwrap().location.is_none());

        // 每个样本都有且仅有一条 action=created 的初始日志
        for no in ["S201", "S202"] {
            let logs = list_logs(&conn, no).unwrap();
            assert_eq!(logs.len(), 1);
            assert_eq!(logs[0].action, "created");
            assert_eq!(logs[0].operator, "alice");
        }
        // 初始状态为 collected
        assert!(result.samples.iter().all(|s| s.status == "collected"));
    }

    #[test]
    fn bulk_import_rejects_unknown_batch() {
        let mut conn = setup();
        let err = bulk_import(
            &mut conn,
            "B404",
            &BulkImportReq {
                operator: Some("alice".into()),
                samples: vec![BulkImportItem {
                    sample_no: "S301".into(),
                    sample_type: "blood".into(),
                    location: None,
                }],
            },
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(err.code, "BATCH_NOT_FOUND");
        assert!(repo::get_sample(&conn, "S301").is_err());
    }

    #[test]
    fn bulk_import_rejects_invalid_location_and_rolls_back() {
        let mut conn = setup();
        let err = bulk_import(
            &mut conn,
            "B001",
            &BulkImportReq {
                operator: Some("alice".into()),
                samples: vec![
                    BulkImportItem {
                        sample_no: "S401".into(),
                        sample_type: "blood".into(),
                        location: None,
                    },
                    BulkImportItem {
                        sample_no: "S402".into(),
                        sample_type: "blood".into(),
                        location: Some(BulkImportLocation {
                            region: "A区".into(),
                            freezer_no: "  ".into(), // 位置非法
                            shelf: "1".into(),
                            slot: "A1".into(),
                        }),
                    },
                ],
            },
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "VALIDATION_ERROR");
        assert!(err.message.contains("第 2 条"));
        assert!(err.message.contains("位置非法"));
        // 整批拒绝：第一条也不应写入
        assert!(repo::get_sample(&conn, "S401").is_err());
        assert!(repo::get_sample(&conn, "S402").is_err());
    }

    // ---------- 异常标记 ----------

    #[test]
    fn exception_flag_and_resolve_flow() {
        let conn = setup();
        register(&conn, "S001");
        let flag = add_exception(
            &conn,
            "S001",
            &AddExceptionReq {
                flag_type: "contamination".into(),
                description: "疑似污染".into(),
                operator: "alice".into(),
            },
        )
        .unwrap();
        assert!(!flag.resolved);

        let resolved = resolve_exception(
            &conn,
            "S001",
            flag.id,
            &ResolveExceptionReq {
                operator: "bob".into(),
                note: Some("复检正常".into()),
            },
        )
        .unwrap();
        assert!(resolved.resolved);
        assert!(resolved.resolved_at.is_some());

        // 重复解除报错
        let err = resolve_exception(
            &conn,
            "S001",
            flag.id,
            &ResolveExceptionReq {
                operator: "bob".into(),
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "ALREADY_RESOLVED");

        // 异常事件应写入流转日志
        let logs = list_logs(&conn, "S001").unwrap();
        let actions: Vec<&str> = logs.iter().map(|l| l.action.as_str()).collect();
        assert_eq!(actions, ["register", "exception_flag", "exception_resolve"]);
    }

    #[test]
    fn exception_validation_and_cross_sample_guard() {
        let conn = setup();
        register(&conn, "S001");
        register(&conn, "S002");

        let err = add_exception(
            &conn,
            "S001",
            &AddExceptionReq {
                flag_type: "fire".into(),
                description: String::new(),
                operator: "alice".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "VALIDATION_ERROR");

        let flag = add_exception(
            &conn,
            "S001",
            &AddExceptionReq {
                flag_type: "label_missing".into(),
                description: "标签脱落".into(),
                operator: "alice".into(),
            },
        )
        .unwrap();
        // 不能用 S002 去解除 S001 的异常
        let err = resolve_exception(
            &conn,
            "S002",
            flag.id,
            &ResolveExceptionReq {
                operator: "bob".into(),
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "EXCEPTION_NOT_FOUND");
    }

    // ---------- 位置与查询过滤 ----------

    fn locate(conn: &Connection, sample_no: &str, region: &str, freezer: &str) {
        change_location(
            conn,
            sample_no,
            &ChangeLocationReq {
                region: region.into(),
                freezer_no: freezer.into(),
                shelf: "1".into(),
                slot: "A1".into(),
                operator: "alice".into(),
                note: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn location_change_records_log_and_updates_sample() {
        let conn = setup();
        register(&conn, "S001");
        assert!(repo::get_sample(&conn, "S001").unwrap().location.is_none());

        locate(&conn, "S001", "A区", "F-01");
        let sample = repo::get_sample(&conn, "S001").unwrap();
        let loc = sample.location.unwrap();
        assert_eq!(loc.region, "A区");
        assert_eq!(loc.freezer_no, "F-01");

        // 相同位置重复设置应复用同一条 location 记录
        locate(&conn, "S001", "A区", "F-01");
        let sample2 = repo::get_sample(&conn, "S001").unwrap();
        assert_eq!(sample2.location.unwrap().id, loc.id);

        let logs = list_logs(&conn, "S001").unwrap();
        assert_eq!(logs.iter().filter(|l| l.action == "location_change").count(), 2);
    }

    #[test]
    fn query_filters_by_status_batch_and_location() {
        let conn = setup();
        create_batch(
            &conn,
            &CreateBatchReq {
                batch_no: "B002".into(),
                project_name: "代谢组学".into(),
                manager: "王五".into(),
                remark: String::new(),
            },
        )
        .unwrap();
        register(&conn, "S001");
        register(&conn, "S002");
        register_sample(
            &conn,
            &RegisterSampleReq {
                sample_no: "S003".into(),
                batch_no: "B002".into(),
                sample_type: "urine".into(),
                operator: None,
                note: None,
            },
        )
        .unwrap();

        set_status(&conn, "S001", "stored").unwrap();
        locate(&conn, "S001", "A区", "F-01");
        locate(&conn, "S002", "B区", "F-02");

        // 按状态
        let stored = query_samples(
            &conn,
            &SampleFilter {
                status: Some("stored".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].sample_no, "S001");

        // 按批次
        let b2 = list_batch_samples(&conn, "B002").unwrap();
        assert_eq!(b2.len(), 1);
        assert_eq!(b2[0].sample_no, "S003");
        let err = list_batch_samples(&conn, "B404").unwrap_err();
        assert_eq!(err.code, "BATCH_NOT_FOUND");

        // 按位置（区域 + 冰箱组合）
        let in_a = query_samples(
            &conn,
            &SampleFilter {
                region: Some("A区".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(in_a.len(), 1);
        assert_eq!(in_a[0].sample_no, "S001");

        let none = query_samples(
            &conn,
            &SampleFilter {
                region: Some("A区".into()),
                freezer_no: Some("F-02".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(none.is_empty());
    }

    // ---------- 位置占用与位置详情 ----------

    fn a1_location_query() -> LocationQuery {
        LocationQuery {
            region: Some("A区".into()),
            freezer_no: Some("F-01".into()),
            shelf: Some("1".into()),
            slot: Some("A1".into()),
        }
    }

    #[test]
    fn occupied_location_rejects_second_active_sample() {
        let conn = setup();
        register(&conn, "S001");
        register(&conn, "S002");
        locate(&conn, "S001", "A区", "F-01"); // 占用 A区/F-01/1/A1

        let err = change_location(
            &conn,
            "S002",
            &ChangeLocationReq {
                region: "A区".into(),
                freezer_no: "F-01".into(),
                shelf: "1".into(),
                slot: "A1".into(),
                operator: "alice".into(),
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.code, "LOCATION_OCCUPIED");
        assert_eq!(err.details.unwrap()["occupant"], "S001");

        // 样本重复设置自己当前的位置不算冲突
        locate(&conn, "S001", "A区", "F-01");
        // 同冰箱不同格位不冲突
        change_location(
            &conn,
            "S002",
            &ChangeLocationReq {
                region: "A区".into(),
                freezer_no: "F-01".into(),
                shelf: "1".into(),
                slot: "A2".into(),
                operator: "alice".into(),
                note: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn archived_sample_releases_location() {
        let conn = setup();
        register(&conn, "S001");
        register(&conn, "S002");
        locate(&conn, "S001", "A区", "F-01");
        for s in ["stored", "processing", "transferred", "archived"] {
            set_status(&conn, "S001", s).unwrap();
        }
        // S001 已归档释放位置，S002 可迁入
        change_location(
            &conn,
            "S002",
            &ChangeLocationReq {
                region: "A区".into(),
                freezer_no: "F-01".into(),
                shelf: "1".into(),
                slot: "A1".into(),
                operator: "alice".into(),
                note: None,
            },
        )
        .unwrap();
        // 位置当前样本为 S002（归档的 S001 不再占用）
        let detail = get_location_detail(&conn, &a1_location_query()).unwrap();
        assert_eq!(detail.current_sample.unwrap().sample_no, "S002");
    }

    #[test]
    fn location_change_log_contains_old_and_new_location() {
        let conn = setup();
        register(&conn, "S001");
        locate(&conn, "S001", "A区", "F-01");
        change_location(
            &conn,
            "S001",
            &ChangeLocationReq {
                region: "B区".into(),
                freezer_no: "F-02".into(),
                shelf: "2".into(),
                slot: "B3".into(),
                operator: "bob".into(),
                note: None,
            },
        )
        .unwrap();

        let logs = list_logs(&conn, "S001").unwrap();
        let moves: Vec<&OperationLog> =
            logs.iter().filter(|l| l.action == "location_change").collect();
        assert_eq!(moves.len(), 2);
        // 首次：旧位置为 (未设置)
        assert!(moves[0].note.contains("(未设置) -> A区/F-01/1/A1"));
        // 再次：同时包含旧位置和新位置
        assert!(moves[1].note.contains("A区/F-01/1/A1 -> B区/F-02/2/B3"));
    }

    #[test]
    fn location_detail_returns_current_sample_and_recent_10_movements() {
        let conn = setup();
        register(&conn, "S001");
        // 11 次迁移：A区/F-01 与 B区/F-02 之间往返，最终停在 A区/F-01
        for i in 0..11 {
            let (region, freezer) = if i % 2 == 0 { ("A区", "F-01") } else { ("B区", "F-02") };
            locate(&conn, "S001", region, freezer);
        }
        let detail = get_location_detail(&conn, &a1_location_query()).unwrap();
        assert_eq!(detail.location.freezer_no, "F-01");
        assert_eq!(detail.current_sample.unwrap().sample_no, "S001");
        // A区/F-01/1/A1 相关记录共 11 条，只返回最近 10 条
        assert_eq!(detail.recent_movements.len(), 10);
        // 倒序：最新一条是迁入
        assert_eq!(detail.recent_movements[0].direction, "in");
        assert_eq!(detail.recent_movements[0].sample_no, "S001");
        assert!(detail.recent_movements.iter().any(|m| m.direction == "out"));
        // 迁出记录带 from_location，迁入记录带 to_location
        let out = detail
            .recent_movements
            .iter()
            .find(|m| m.direction == "out")
            .unwrap();
        assert_eq!(out.from_location.as_ref().unwrap().freezer_no, "F-01");
        assert_eq!(out.to_location.as_ref().unwrap().freezer_no, "F-02");
    }

    #[test]
    fn location_detail_requires_full_tuple_and_existing_location() {
        let conn = setup();
        // 缺少 slot
        let err = get_location_detail(
            &conn,
            &LocationQuery {
                region: Some("A区".into()),
                freezer_no: Some("F-01".into()),
                shelf: Some("1".into()),
                slot: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("slot"));
        // 空白参数
        let err = get_location_detail(
            &conn,
            &LocationQuery {
                region: Some("  ".into()),
                freezer_no: Some("F-01".into()),
                shelf: Some("1".into()),
                slot: Some("A1".into()),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "VALIDATION_ERROR");
        // 位置不存在
        let err = get_location_detail(&conn, &a1_location_query()).unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(err.code, "LOCATION_NOT_FOUND");
    }

    #[test]
    fn bulk_import_rejects_occupied_location_and_rolls_back() {
        let mut conn = setup();
        register(&conn, "S001");
        locate(&conn, "S001", "A区", "F-01");
        let err = bulk_import(
            &mut conn,
            "B001",
            &BulkImportReq {
                operator: Some("alice".into()),
                samples: vec![BulkImportItem {
                    sample_no: "S501".into(),
                    sample_type: "blood".into(),
                    location: Some(BulkImportLocation {
                        region: "A区".into(),
                        freezer_no: "F-01".into(),
                        shelf: "1".into(),
                        slot: "A1".into(),
                    }),
                }],
            },
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.code, "LOCATION_OCCUPIED");
        assert!(repo::get_sample(&conn, "S501").is_err());
    }

    #[test]
    fn bulk_import_rejects_same_location_within_request() {
        let mut conn = setup();
        let loc = || {
            Some(BulkImportLocation {
                region: "A区".into(),
                freezer_no: "F-01".into(),
                shelf: "1".into(),
                slot: "A1".into(),
            })
        };
        let err = bulk_import(
            &mut conn,
            "B001",
            &BulkImportReq {
                operator: Some("alice".into()),
                samples: vec![
                    BulkImportItem {
                        sample_no: "S601".into(),
                        sample_type: "blood".into(),
                        location: loc(),
                    },
                    BulkImportItem {
                        sample_no: "S602".into(),
                        sample_type: "tissue".into(),
                        location: loc(),
                    },
                ],
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "LOCATION_OCCUPIED");
        assert!(err.message.contains("第 2 条"));
        // 整批回滚
        assert!(repo::get_sample(&conn, "S601").is_err());
        assert!(repo::get_sample(&conn, "S602").is_err());
        // 移动记录也随事务回滚
        let err = get_location_detail(&conn, &a1_location_query()).unwrap_err();
        assert_eq!(err.code, "LOCATION_NOT_FOUND");
    }

    #[test]
    fn archive_writes_release_movement_and_clears_location() {
        let conn = setup();
        register(&conn, "S001");
        locate(&conn, "S001", "A区", "F-01");
        for s in ["stored", "processing", "transferred", "archived"] {
            set_status(&conn, "S001", s).unwrap();
        }

        // 归档后样本位置被清空（释放）
        assert!(repo::get_sample(&conn, "S001").unwrap().location.is_none());

        // 原位置的迁出历史存在：最新一条为 out，to_location 为 NULL（释放）
        let detail = get_location_detail(&conn, &a1_location_query()).unwrap();
        assert!(detail.current_sample.is_none());
        let latest = &detail.recent_movements[0];
        assert_eq!(latest.sample_no, "S001");
        assert_eq!(latest.direction, "out");
        assert_eq!(latest.from_location.as_ref().unwrap().freezer_no, "F-01");
        assert!(latest.to_location.is_none());
        assert_eq!(latest.operator, "alice");
    }

    #[test]
    fn location_fields_are_trimmed_before_occupancy_check() {
        let conn = setup();
        register(&conn, "S001");
        register(&conn, "S002");
        locate(&conn, "S001", "A区", "F-01"); // 占用 A区/F-01/1/A1

        // 带空格的位置字段不能绕过占用校验
        let err = change_location(
            &conn,
            "S002",
            &ChangeLocationReq {
                region: " A区 ".into(),
                freezer_no: "F-01 ".into(),
                shelf: " 1".into(),
                slot: "A1".into(),
                operator: "alice".into(),
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.code, "LOCATION_OCCUPIED");

        // trim 后指向同一位置记录：样本迁入带空格字段的位置，复用已有 location 行
        let other = change_location(
            &conn,
            "S002",
            &ChangeLocationReq {
                region: "B区".into(),
                freezer_no: " F-02 ".into(),
                shelf: "2".into(),
                slot: " B3 ".into(),
                operator: "alice".into(),
                note: None,
            },
        )
        .unwrap();
        let loc = other.location.unwrap();
        assert_eq!(loc.region, "B区");
        assert_eq!(loc.freezer_no, "F-02");
        assert_eq!(loc.slot, "B3");
    }

    #[test]
    fn logs_for_missing_sample_return_404() {
        let conn = setup();
        let err = list_logs(&conn, "S404").unwrap_err();
        assert_eq!(err.code, "SAMPLE_NOT_FOUND");
    }

    // ---------- 空白操作者与空查询参数 ----------

    #[test]
    fn register_rejects_whitespace_only_operator() {
        let conn = setup();
        let err = register_sample(
            &conn,
            &RegisterSampleReq {
                sample_no: "S001".into(),
                batch_no: "B001".into(),
                sample_type: "blood".into(),
                operator: Some("   ".into()),
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "VALIDATION_ERROR");
    }

    #[test]
    fn bulk_import_rejects_whitespace_only_operator() {
        let mut conn = setup();
        let err = bulk_import(
            &mut conn,
            "B001",
            &BulkImportReq {
                operator: Some(" \t ".into()),
                samples: vec![BulkImportItem {
                    sample_no: "S101".into(),
                    sample_type: "blood".into(),
                    location: None,
                }],
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "VALIDATION_ERROR");
    }

    #[test]
    fn filter_rejects_empty_and_whitespace_values() {
        let conn = setup();
        for (field, filter) in [
            ("status", SampleFilter { status: Some(String::new()), ..Default::default() }),
            ("region", SampleFilter { region: Some("   ".into()), ..Default::default() }),
            ("batch_no", SampleFilter { batch_no: Some(String::new()), ..Default::default() }),
            ("slot", SampleFilter { slot: Some(" ".into()), ..Default::default() }),
        ] {
            let err = query_samples(&conn, &filter).unwrap_err();
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "field {field}");
            assert_eq!(err.code, "VALIDATION_ERROR", "field {field}");
            assert!(err.message.contains(field), "field {field}");
        }
    }
}
