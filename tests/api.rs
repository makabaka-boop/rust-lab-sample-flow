use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use rust_lab_sample_flow::{build_router, init_pool};
use serde_json::{json, Value};
use tower::ServiceExt;

/// 每个测试使用独立的临时 SQLite 文件，避免相互干扰。
fn test_app() -> axum::Router {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "lab_test_{}_{}_{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let path = std::env::temp_dir().join(name);
    let pool = init_pool(path.to_str().unwrap()).expect("init pool");
    build_router(pool)
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    let req = match body {
        Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, val)
}

async fn seed_batch(app: &axum::Router, batch_no: &str) {
    let (s, _) = call(
        app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_no": batch_no,
            "project_name": "P",
            "owner": "alice"
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
}

async fn seed_sample(app: &axum::Router, batch_no: &str, sample_no: &str) {
    let (s, _) = call(
        app,
        "POST",
        &format!("/api/batches/{batch_no}/samples"),
        Some(json!({ "sample_no": sample_no, "sample_type": "blood" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
}

async fn set_status(app: &axum::Router, sample_no: &str, status: &str) -> (StatusCode, Value) {
    call(
        app,
        "PATCH",
        &format!("/api/samples/{sample_no}/status"),
        Some(json!({ "status": status, "operator": "bob" })),
    )
    .await
}

// ---------- 状态流转 ----------

#[tokio::test]
async fn full_lifecycle_closed_loop() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_sample(&app, "B1", "S1").await;

    for st in ["stored", "processing", "transferred", "archived"] {
        let (s, body) = set_status(&app, "S1", st).await;
        assert_eq!(s, StatusCode::OK, "status {st} -> {body}");
        assert_eq!(body["status"], st);
    }
}

// 并发/重复建批次：即使绕过前置检查（依赖 UNIQUE 约束）也应返回 409 而非 500。
#[tokio::test]
async fn concurrent_duplicate_batch_returns_conflict() {
    let app = test_app();
    let payload = json!({ "batch_no": "BX", "project_name": "P", "owner": "a" });

    // 并发发起两次相同批次创建。
    let a = call(&app, "POST", "/api/batches", Some(payload.clone()));
    let b = call(&app, "POST", "/api/batches", Some(payload.clone()));
    let ((s1, _), (s2, _)) = tokio::join!(a, b);

    let mut codes = [s1, s2];
    codes.sort();
    assert_eq!(codes[0], StatusCode::CREATED);
    assert_eq!(codes[1], StatusCode::CONFLICT, "第二次应为 409 而非 500");
}

// 归档后样本不再携带位置，且位置检索不应混入已归档样本。
#[tokio::test]
async fn archived_sample_releases_location() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    let loc = json!({ "area": "A9", "freezer": "F9", "shelf": "1", "slot": "1" });
    let (s, _) = call(
        &app,
        "POST",
        "/api/batches/B1/samples",
        Some(json!({ "sample_no": "S1", "sample_type": "blood", "location": loc })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    for st in ["stored", "processing", "transferred", "archived"] {
        let (s, _) = set_status(&app, "S1", st).await;
        assert_eq!(s, StatusCode::OK);
    }

    // 归档后样本 location 字段应被清空。
    let (s, body) = call(&app, "GET", "/api/samples/S1", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.get("location").is_none() || body["location"].is_null());

    // 按位置检索不应再返回已归档样本。
    let (s2, body2) = call(&app, "GET", "/api/samples?area=A9&freezer=F9", None).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(body2["total"], 0);

    // 位置查询接口显示该位置无当前占用样本。
    let (s3, body3) = call(
        &app,
        "GET",
        "/api/locations?area=A9&freezer=F9&shelf=1&slot=1",
        None,
    )
    .await;
    assert_eq!(s3, StatusCode::OK);
    assert!(body3.get("current_sample").is_none() || body3["current_sample"].is_null());
}

#[tokio::test]
async fn cannot_skip_states() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_sample(&app, "B1", "S1").await;

    // collected -> archived 应被拒绝。
    let (s, body) = set_status(&app, "S1", "archived").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_TRANSITION");
}

#[tokio::test]
async fn cannot_go_backwards() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_sample(&app, "B1", "S1").await;
    set_status(&app, "S1", "stored").await;

    // stored -> collected 回退应被拒绝。
    let (s, body) = set_status(&app, "S1", "collected").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "INVALID_TRANSITION");
}

#[tokio::test]
async fn invalid_status_value_rejected() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_sample(&app, "B1", "S1").await;
    let (s, body) = set_status(&app, "S1", "frozen").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

// ---------- 输入校验 ----------

#[tokio::test]
async fn create_batch_requires_fields() {
    let app = test_app();
    let (s, body) = call(
        &app,
        "POST",
        "/api/batches",
        Some(json!({ "batch_no": "  ", "project_name": "P", "owner": "a" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn duplicate_batch_conflict() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    let (s, body) = call(
        &app,
        "POST",
        "/api/batches",
        Some(json!({ "batch_no": "B1", "project_name": "P", "owner": "a" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "CONFLICT");
}

#[tokio::test]
async fn register_sample_unknown_batch() {
    let app = test_app();
    let (s, body) = call(
        &app,
        "POST",
        "/api/batches/NOPE/samples",
        Some(json!({ "sample_no": "S1", "sample_type": "blood" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn register_sample_incomplete_location() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    let (s, body) = call(
        &app,
        "POST",
        "/api/batches/B1/samples",
        Some(json!({
            "sample_no": "S1", "sample_type": "blood",
            "location": { "area": "A", "freezer": "F1", "shelf": "1", "slot": "" }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn operator_only_spaces_rejected() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_sample(&app, "B1", "S1").await;
    let (s, body) = call(
        &app,
        "PATCH",
        "/api/samples/S1/status",
        Some(json!({ "status": "stored", "operator": "   " })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn malformed_json_returns_unified_error() {
    let app = test_app();
    let req = Request::builder()
        .method("POST")
        .uri("/api/batches")
        .header("content-type", "application/json")
        .body(Body::from("{ not json"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

// ---------- 批量导入 ----------

#[tokio::test]
async fn bulk_import_success() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    let (s, body) = call(
        &app,
        "POST",
        "/api/batches/B1/samples/bulk-import",
        Some(json!({ "samples": [
            { "sample_no": "S1", "sample_type": "blood" },
            { "sample_no": "S2", "sample_type": "urine" }
        ]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(body["imported"], 2);

    // 每个样本都应自动写入一条 action=created 的初始操作日志。
    for no in ["S1", "S2"] {
        let (ls, logs) = call(&app, "GET", &format!("/api/samples/{no}/logs"), None).await;
        assert_eq!(ls, StatusCode::OK);
        let arr = logs["logs"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["action"], "created");
    }
}

#[tokio::test]
async fn bulk_import_unknown_batch_not_found() {
    let app = test_app();
    let (s, body) = call(
        &app,
        "POST",
        "/api/batches/NOPE/samples/bulk-import",
        Some(json!({ "samples": [
            { "sample_no": "S1", "sample_type": "blood" }
        ]})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn bulk_import_rolls_back_on_duplicate() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_sample(&app, "B1", "S1").await;

    let (s, body) = call(
        &app,
        "POST",
        "/api/batches/B1/samples/bulk-import",
        Some(json!({ "samples": [
            { "sample_no": "S2", "sample_type": "blood" },
            { "sample_no": "S1", "sample_type": "blood" }
        ]})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "CONFLICT");

    // S2 不应被写入（整批回滚）。
    let (s2, _) = call(&app, "GET", "/api/samples/S2", None).await;
    assert_eq!(s2, StatusCode::NOT_FOUND);
}

// ---------- 异常标记 ----------

#[tokio::test]
async fn exception_blocks_transition_until_resolved() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_sample(&app, "B1", "S1").await;

    let (s, ex) = call(
        &app,
        "POST",
        "/api/samples/S1/exceptions",
        Some(json!({ "exception_type": "contamination", "reported_by": "bob" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let ex_id = ex["id"].as_i64().unwrap();

    // 存在未解除异常，流转被拒。
    let (s2, body2) = set_status(&app, "S1", "stored").await;
    assert_eq!(s2, StatusCode::CONFLICT);
    assert_eq!(body2["code"], "CONFLICT");

    // 解除异常。
    let (s3, _) = call(
        &app,
        "PATCH",
        &format!("/api/exceptions/{ex_id}/resolve"),
        Some(json!({ "resolved_by": "carol" })),
    )
    .await;
    assert_eq!(s3, StatusCode::OK);

    // 解除后可正常流转。
    let (s4, _) = set_status(&app, "S1", "stored").await;
    assert_eq!(s4, StatusCode::OK);
}

#[tokio::test]
async fn resolve_missing_exception_not_found() {
    let app = test_app();
    let (s, body) = call(
        &app,
        "PATCH",
        "/api/exceptions/999/resolve",
        Some(json!({ "resolved_by": "x" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "NOT_FOUND");
}

// ---------- 查询过滤 ----------

#[tokio::test]
async fn query_by_batch_and_status() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_batch(&app, "B2").await;
    seed_sample(&app, "B1", "S1").await;
    seed_sample(&app, "B1", "S2").await;
    seed_sample(&app, "B2", "S3").await;
    set_status(&app, "S1", "stored").await;

    // 按批次过滤。
    let (s, body) = call(&app, "GET", "/api/samples?batch_no=B1", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["total"], 2);

    // 按状态过滤。
    let (s2, body2) = call(&app, "GET", "/api/samples?status=stored", None).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(body2["total"], 1);
    assert_eq!(body2["samples"][0]["sample_no"], "S1");
}

#[tokio::test]
async fn query_by_location() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    let (s, _) = call(
        &app,
        "POST",
        "/api/batches/B1/samples",
        Some(json!({
            "sample_no": "S1", "sample_type": "blood",
            "location": { "area": "A1", "freezer": "F1", "shelf": "1", "slot": "01" }
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s2, body) = call(&app, "GET", "/api/samples?area=A1&freezer=F1", None).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(body["total"], 1);

    // 无匹配位置返回空。
    let (s3, body3) = call(&app, "GET", "/api/samples?area=ZZ", None).await;
    assert_eq!(s3, StatusCode::OK);
    assert_eq!(body3["total"], 0);
}

#[tokio::test]
async fn empty_query_param_rejected() {
    let app = test_app();
    let (s, body) = call(&app, "GET", "/api/samples?status=", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn invalid_status_filter_rejected() {
    let app = test_app();
    let (s, body) = call(&app, "GET", "/api/samples?status=bogus", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_ERROR");
}

// ---------- 位置变更 & 占用 ----------

#[tokio::test]
async fn change_location_and_occupancy_conflict() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_sample(&app, "B1", "S1").await;
    seed_sample(&app, "B1", "S2").await;

    let loc = json!({ "area": "A", "freezer": "F", "shelf": "1", "slot": "1" });

    let (s, _) = call(
        &app,
        "PATCH",
        "/api/samples/S1/location",
        Some(json!({ "location": loc, "operator": "bob" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // S2 想占用同一位置应冲突。
    let (s2, body2) = call(
        &app,
        "PATCH",
        "/api/samples/S2/location",
        Some(json!({ "location": loc, "operator": "bob" })),
    )
    .await;
    assert_eq!(s2, StatusCode::CONFLICT);
    assert_eq!(body2["code"], "CONFLICT");
}

// 位置变更必须记录含旧位置和新位置的操作日志。
#[tokio::test]
async fn change_location_logs_old_and_new() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    let loc1 = json!({ "area": "A", "freezer": "F1", "shelf": "1", "slot": "01" });
    let loc2 = json!({ "area": "B", "freezer": "F2", "shelf": "2", "slot": "02" });

    let (s, _) = call(
        &app,
        "POST",
        "/api/batches/B1/samples",
        Some(json!({ "sample_no": "S1", "sample_type": "blood", "location": loc1 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s2, _) = call(
        &app,
        "PATCH",
        "/api/samples/S1/location",
        Some(json!({ "location": loc2, "operator": "bob" })),
    )
    .await;
    assert_eq!(s2, StatusCode::OK);

    let (ls, logs) = call(&app, "GET", "/api/samples/S1/logs", None).await;
    assert_eq!(ls, StatusCode::OK);
    let arr = logs["logs"].as_array().unwrap();
    let change = arr
        .iter()
        .find(|l| l["action"] == "location_changed")
        .expect("应存在 location_changed 日志");
    let note = change["note"].as_str().unwrap();
    // 旧位置 A/F1/1/01 与新位置 B/F2/2/02 都应出现在日志说明里。
    assert!(note.contains("A/F1/1/01"), "缺少旧位置: {note}");
    assert!(note.contains("B/F2/2/02"), "缺少新位置: {note}");
}

// 手动填写备注时，日志仍要保留旧位置和新位置说明，备注追加在后。
#[tokio::test]
async fn change_location_manual_note_keeps_old_and_new() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    let loc1 = json!({ "area": "A", "freezer": "F1", "shelf": "1", "slot": "01" });
    let loc2 = json!({ "area": "B", "freezer": "F2", "shelf": "2", "slot": "02" });

    let (s, _) = call(
        &app,
        "POST",
        "/api/batches/B1/samples",
        Some(json!({ "sample_no": "S1", "sample_type": "blood", "location": loc1 })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s2, _) = call(
        &app,
        "PATCH",
        "/api/samples/S1/location",
        Some(json!({ "location": loc2, "operator": "bob", "note": "转移到大容量冰箱" })),
    )
    .await;
    assert_eq!(s2, StatusCode::OK);

    let (_, logs) = call(&app, "GET", "/api/samples/S1/logs", None).await;
    let arr = logs["logs"].as_array().unwrap();
    let change = arr
        .iter()
        .find(|l| l["action"] == "location_changed")
        .expect("应存在 location_changed 日志");
    let note = change["note"].as_str().unwrap();
    assert!(note.contains("A/F1/1/01"), "备注盖掉了旧位置: {note}");
    assert!(note.contains("B/F2/2/02"), "备注盖掉了新位置: {note}");
    assert!(note.contains("转移到大容量冰箱"), "手动备注丢失: {note}");
}

// 位置查询返回当前占用样本与最近的迁入/迁出记录。
#[tokio::test]
async fn location_query_returns_occupant_and_movements() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    let loc = json!({ "area": "Q", "freezer": "F", "shelf": "1", "slot": "1" });
    let other = json!({ "area": "Q2", "freezer": "F", "shelf": "1", "slot": "1" });

    // S1 登记到 Q（一次迁入）。
    let (s, _) = call(
        &app,
        "POST",
        "/api/batches/B1/samples",
        Some(json!({ "sample_no": "S1", "sample_type": "blood", "location": loc })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    // S1 迁出 Q 到 Q2（Q 产生一次迁出）。
    let (s2, _) = call(
        &app,
        "PATCH",
        "/api/samples/S1/location",
        Some(json!({ "location": other, "operator": "bob" })),
    )
    .await;
    assert_eq!(s2, StatusCode::OK);

    // 另一个样本 S2 迁入 Q（Q 再产生一次迁入，并成为当前占用者）。
    seed_sample(&app, "B1", "S2").await;
    let (s3, _) = call(
        &app,
        "PATCH",
        "/api/samples/S2/location",
        Some(json!({ "location": loc, "operator": "carol" })),
    )
    .await;
    assert_eq!(s3, StatusCode::OK);

    let (s4, view) = call(&app, "GET", "/api/locations?area=Q&freezer=F&shelf=1&slot=1", None).await;
    assert_eq!(s4, StatusCode::OK);
    assert_eq!(view["current_sample"], "S2");
    let moves = view["recent_movements"].as_array().unwrap();
    // Q 上应有 3 条迁移记录：S1 in、S1 out、S2 in。
    assert_eq!(moves.len(), 3);
    let dirs: Vec<&str> = moves.iter().map(|m| m["direction"].as_str().unwrap()).collect();
    assert!(dirs.contains(&"in") && dirs.contains(&"out"));
}

// ---------- 操作日志 ----------

#[tokio::test]
async fn logs_track_full_flow() {
    let app = test_app();
    seed_batch(&app, "B1").await;
    seed_sample(&app, "B1", "S1").await;
    set_status(&app, "S1", "stored").await;

    let (s, body) = call(&app, "GET", "/api/samples/S1/logs", None).await;
    assert_eq!(s, StatusCode::OK);
    let logs = body["logs"].as_array().unwrap();
    // created + status:stored
    assert!(logs.len() >= 2);
    assert_eq!(logs[0]["action"], "created");
}

// ---------- 路由错误 ----------

#[tokio::test]
async fn unknown_route_returns_unified_404() {
    let app = test_app();
    let (s, body) = call(&app, "GET", "/api/does-not-exist", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn method_not_allowed_returns_unified_error() {
    let app = test_app();
    let (s, body) = call(&app, "DELETE", "/api/batches", None).await;
    assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body["code"], "METHOD_NOT_ALLOWED");
}
