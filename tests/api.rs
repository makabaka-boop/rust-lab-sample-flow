use axum::body::Body;
use axum::Router;
use http_body_util::BodyExt;
use rust_lab_sample_flow::build_router;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tower::ServiceExt;

async fn setup() -> (Router, SqlitePool) {
    let database_url = format!("file:test_db_{}?mode=memory&cache=shared", uuid::Uuid::new_v4());
    let pool = rust_lab_sample_flow::init_pool(&database_url).await.unwrap();
    (build_router(pool.clone()), pool)
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (http::StatusCode, Value) {
    let builder = http::Request::builder().method(method).uri(uri);
    let request = match body {
        Some(value) => {
            let bytes = serde_json::to_vec(&value).unwrap();
            builder
                .header("content-type", "application/json")
                .body(Body::from(bytes))
                .unwrap()
        }
        None => builder.body(Body::empty()).unwrap(),
    };

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

#[tokio::test]
async fn validates_batch_inputs() {
    let (app, _pool) = setup().await;
    let (status, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "",
            "project_name": "测序项目",
            "owner": "张老师"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert!(body["error"]["message"].as_str().unwrap().contains("batch_number"));
}

#[tokio::test]
async fn rejects_skipping_state_transition() {
    let (app, _pool) = setup().await;
    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-SKIP",
            "project_name": "状态测试",
            "owner": "李老师"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let (_, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(json!({
            "sample_number": "S-SKIP-001",
            "sample_type": "血液",
            "operator": "alice",
            "description": "登记样本"
        })),
    )
    .await;
    let sample_id = body["id"].as_i64().unwrap();

    let (status, body) = request(
        &app,
        "PUT",
        &format!("/api/samples/{sample_id}/status"),
        Some(json!({
            "status": "archived",
            "operator": "alice",
            "description": "尝试跳过关键状态"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "INVALID_STATE_TRANSITION");
    assert_eq!(body["error"]["details"]["current_status"], "collected");
    assert_eq!(body["error"]["details"]["target_status"], "archived");
    assert_eq!(body["error"]["details"]["allowed_next_status"], "stored");
}

#[tokio::test]
async fn requires_location_when_stored() {
    let (app, _pool) = setup().await;
    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-STORE",
            "project_name": "入库测试",
            "owner": "王老师"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let (_, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(json!({
            "sample_number": "S-STORE-001",
            "sample_type": "组织",
            "operator": "bob"
        })),
    )
    .await;
    let sample_id = body["id"].as_i64().unwrap();

    let (status, body) = request(
        &app,
        "PUT",
        &format!("/api/samples/{sample_id}/status"),
        Some(json!({
            "status": "stored",
            "operator": "bob"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert_eq!(body["error"]["details"]["required_field"], "location");
}

#[tokio::test]
async fn completes_sample_flow_closure() {
    let (app, _pool) = setup().await;
    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-CLOSE",
            "project_name": "闭环测试",
            "owner": "赵老师"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let location = json!({
        "area": "A区",
        "fridge_number": "F-01",
        "shelf": "L-02",
        "slot": "G-03"
    });
    let (_, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(json!({
            "sample_number": "S-CLOSE-001",
            "sample_type": "DNA",
            "location": location,
            "operator": "carol",
            "description": "采集登记"
        })),
    )
    .await;
    let sample_id = body["id"].as_i64().unwrap();
    assert_eq!(body["status"], "collected");

    for status in ["stored", "processing", "transferred", "archived"] {
        let (_, body) = request(
            &app,
            "PUT",
            &format!("/api/samples/{sample_id}/status"),
            Some(json!({
                "status": status,
                "operator": "carol",
                "description": format!("更新到 {status}"),
                "location": location
            })),
        )
        .await;
        assert_eq!(body["status"], status);
    }

    let (status, body) = request(
        &app,
        "GET",
        &format!("/api/samples/{sample_id}/flow"),
        None,
    )
    .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["sample"]["status"], "archived");
    let actions: Vec<&str> = body["operation_logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|log| log["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, vec!["register", "store", "process", "transfer", "archive"]);
}

#[tokio::test]
async fn validates_bulk_import_and_rejects_duplicate() {
    let (app, _pool) = setup().await;
    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-BULK",
            "project_name": "批量导入测试",
            "owner": "周老师"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples/bulk-import"),
        Some(json!({
            "samples": [
                {"sample_number": "S-BULK-001", "sample_type": "RNA"},
                {"sample_number": "S-BULK-001", "sample_type": "RNA"}
            ],
            "operator": "dave"
        })),
    )
    .await;
    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"].as_str().unwrap().contains("重复"));

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples/bulk-import"),
        Some(json!({
            "samples": [
                {"sample_number": "S-BULK-001", "sample_type": "RNA"},
                {"sample_number": "S-BULK-002", "sample_type": "DNA"}
            ],
            "operator": "dave",
            "description": "批量登记"
        })),
    )
    .await;
    assert_eq!(status, http::StatusCode::CREATED);
    assert_eq!(body["count"], 2);
}

#[tokio::test]
async fn marks_and_resolves_anomaly() {
    let (app, _pool) = setup().await;
    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-ANOMALY",
            "project_name": "异常测试",
            "owner": "孙老师"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let (_, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(json!({
            "sample_number": "S-ANOMALY-001",
            "sample_type": "血清",
            "operator": "erin"
        })),
    )
    .await;
    let sample_id = body["id"].as_i64().unwrap();

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/samples/{sample_id}/anomalies"),
        Some(json!({
            "anomaly_type": "temperature_abnormal",
            "description": "运输温度短暂超标",
            "operator": "erin"
        })),
    )
    .await;
    assert_eq!(status, http::StatusCode::CREATED);
    assert_eq!(body["resolved"], false);
    let anomaly_id = body["id"].as_i64().unwrap();

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/anomalies/{anomaly_id}/resolve"),
        Some(json!({
            "operator": "frank",
            "description": "复核后解除"
        })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["resolved"], true);
    assert_eq!(body["resolved_by"], "frank");
}

#[tokio::test]
async fn rejects_invalid_anomaly_type() {
    let (app, _pool) = setup().await;
    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-ANOMALY-TYPE",
            "project_name": "异常类型测试",
            "owner": "吴老师"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let (_, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(json!({
            "sample_number": "S-ANOMALY-TYPE-001",
            "sample_type": "拭子",
            "operator": "grace"
        })),
    )
    .await;
    let sample_id = body["id"].as_i64().unwrap();

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/samples/{sample_id}/anomalies"),
        Some(json!({
            "anomaly_type": "unknown_error",
            "operator": "grace"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert!(body["error"]["details"]["allowed_types"].is_array());
}

#[tokio::test]
async fn filters_samples_by_batch_status_and_location() {
    let (app, _pool) = setup().await;
    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-FILTER",
            "project_name": "筛选测试",
            "owner": "郑老师"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let (_, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples/bulk-import"),
        Some(json!({
            "samples": [
                {"sample_number": "S-F-001", "sample_type": "血液"},
                {"sample_number": "S-F-002", "sample_type": "组织"}
            ],
            "operator": "henry"
        })),
    )
    .await;
    let second_id = body["items"][1]["id"].as_i64().unwrap();

    let location = json!({
        "area": "冷库B",
        "fridge_number": "F-99",
        "shelf": "L-1",
        "slot": "G-1"
    });
    request(
        &app,
        "PUT",
        &format!("/api/samples/{second_id}/status"),
        Some(json!({
            "status": "stored",
            "location": location,
            "operator": "henry"
        })),
    )
    .await;

    let (status, body) = request(
        &app,
        "GET",
        &format!("/api/samples?batch_id={batch_id}&status=stored"),
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["count"], 1);
    assert_eq!(body["items"][0]["sample_number"], "S-F-002");

    let (status, body) = request(
        &app,
        "GET",
        "/api/samples?area=%E5%86%B7%E5%BA%93B&fridge_number=F-99",
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["count"], 1);
    assert_eq!(body["items"][0]["sample_number"], "S-F-002");
}

#[tokio::test]
async fn archived_sample_cannot_change_location() {
    let (app, _pool) = setup().await;
    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-ARCHIVED",
            "project_name": "归档位置测试",
            "owner": "冯老师"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let location = json!({
        "area": "C区",
        "fridge_number": "F-10",
        "shelf": "L-3",
        "slot": "G-2"
    });
    let (_, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(json!({
            "sample_number": "S-ARCHIVED-001",
            "sample_type": "蛋白",
            "location": location,
            "operator": "ivy"
        })),
    )
    .await;
    let sample_id = body["id"].as_i64().unwrap();

    for status in ["stored", "processing", "transferred", "archived"] {
        let (_, body) = request(
            &app,
            "PUT",
            &format!("/api/samples/{sample_id}/status"),
            Some(json!({
                "status": status,
                "operator": "ivy",
                "location": location
            })),
        )
        .await;
        assert_eq!(body["status"], status);
    }

    let (status, body) = request(
        &app,
        "PUT",
        &format!("/api/samples/{sample_id}/location"),
        Some(json!({
            "location": {
                "area": "D区",
                "fridge_number": "F-11",
                "shelf": "L-4",
                "slot": "G-9"
            },
            "operator": "ivy"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "CONFLICT");
    assert!(body["error"]["message"].as_str().unwrap().contains("归档"));
}

#[tokio::test]
async fn batch_samples_endpoint_does_not_leak_other_batches() {
    let (app, _pool) = setup().await;

    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-LEAK-A",
            "project_name": "批次A",
            "owner": "alice"
        })),
    )
    .await;
    let batch_a_id = body["id"].as_i64().unwrap();

    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-LEAK-B",
            "project_name": "批次B",
            "owner": "bob"
        })),
    )
    .await;
    let batch_b_id = body["id"].as_i64().unwrap();

    request(
        &app,
        "POST",
        &format!("/api/batches/{batch_a_id}/samples"),
        Some(json!({
            "sample_number": "S-LEAK-A-001",
            "sample_type": "血液",
            "operator": "alice"
        })),
    )
    .await;

    request(
        &app,
        "POST",
        &format!("/api/batches/{batch_b_id}/samples"),
        Some(json!({
            "sample_number": "S-LEAK-B-001",
            "sample_type": "组织",
            "operator": "bob"
        })),
    )
    .await;

    let (status, body) = request(
        &app,
        "GET",
        &format!("/api/batches/{batch_a_id}/samples"),
        None,
    )
    .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["count"], 1);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["sample_number"], "S-LEAK-A-001");
    assert_eq!(items[0]["batch_id"], batch_a_id);

    let (status, body) = request(
        &app,
        "GET",
        &format!("/api/batches/{batch_a_id}/samples?batch_id={batch_b_id}"),
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["count"], 1);
    assert_eq!(body["items"][0]["sample_number"], "S-LEAK-A-001");
}

#[tokio::test]
async fn bulk_import_succeeds_and_writes_created_logs() {
    let (app, _pool) = setup().await;

    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-BULK-OK",
            "project_name": "批量导入成功测试",
            "owner": "alice"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples/bulk-import"),
        Some(json!({
            "samples": [
                {
                    "sample_number": "S-BULK-OK-001",
                    "sample_type": "血液",
                    "location": {
                        "area": "A区",
                        "fridge_number": "F-01",
                        "shelf": "L-01",
                        "slot": "G-01"
                    }
                },
                {"sample_number": "S-BULK-OK-002", "sample_type": "组织"},
                {"sample_number": "S-BULK-OK-003", "sample_type": "DNA"}
            ],
            "operator": "alice",
            "description": "批量导入"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert_eq!(body["count"], 3);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    for item in items {
        assert_eq!(item["status"], "collected");
        assert_eq!(item["batch_id"], batch_id);
    }

    let (_, list_body) = request(
        &app,
        "GET",
        &format!("/api/batches/{batch_id}/samples"),
        None,
    )
    .await;
    assert_eq!(list_body["count"], 3);

    let first_id = items[0]["id"].as_i64().unwrap();
    let (_, flow_body) = request(
        &app,
        "GET",
        &format!("/api/samples/{first_id}/flow"),
        None,
    )
    .await;
    let logs = flow_body["operation_logs"].as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["action"], "created");
    assert_eq!(logs[0]["operator"], "alice");
    assert_eq!(logs[0]["to_status"], "collected");
}

#[tokio::test]
async fn bulk_import_rolls_back_when_any_sample_is_duplicate() {
    let (app, _pool) = setup().await;

    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-BULK-ROLLBACK",
            "project_name": "批量导入回滚测试",
            "owner": "bob"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(json!({
            "sample_number": "S-EXISTING-001",
            "sample_type": "血液",
            "operator": "bob"
        })),
    )
    .await;

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples/bulk-import"),
        Some(json!({
            "samples": [
                {"sample_number": "S-NEW-001", "sample_type": "RNA"},
                {"sample_number": "S-EXISTING-001", "sample_type": "DNA"},
                {"sample_number": "S-NEW-002", "sample_type": "蛋白"}
            ],
            "operator": "bob"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "CONFLICT");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("S-EXISTING-001"));

    let (_, list_body) = request(
        &app,
        "GET",
        &format!("/api/batches/{batch_id}/samples"),
        None,
    )
    .await;
    assert_eq!(list_body["count"], 1);
    assert_eq!(list_body["items"][0]["sample_number"], "S-EXISTING-001");
}

#[tokio::test]
async fn bulk_import_rolls_back_when_sample_type_empty() {
    let (app, _pool) = setup().await;

    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-BULK-EMPTY",
            "project_name": "类型为空回滚测试",
            "owner": "carol"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples/bulk-import"),
        Some(json!({
            "samples": [
                {"sample_number": "S-EMPTY-001", "sample_type": "血液"},
                {"sample_number": "S-EMPTY-002", "sample_type": ""},
                {"sample_number": "S-EMPTY-003", "sample_type": "DNA"}
            ],
            "operator": "carol"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert!(body["error"]["message"].as_str().unwrap().contains("sample_type"));

    let (_, list_body) = request(
        &app,
        "GET",
        &format!("/api/batches/{batch_id}/samples"),
        None,
    )
    .await;
    assert_eq!(list_body["count"], 0);
}

#[tokio::test]
async fn bulk_import_rolls_back_when_location_invalid() {
    let (app, _pool) = setup().await;

    let (_, body) = request(
        &app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": "B-BULK-LOC",
            "project_name": "位置非法回滚测试",
            "owner": "dave"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples/bulk-import"),
        Some(json!({
            "samples": [
                {"sample_number": "S-LOC-001", "sample_type": "血液"},
                {
                    "sample_number": "S-LOC-002",
                    "sample_type": "DNA",
                    "location": {
                        "area": "A区",
                        "fridge_number": "",
                        "shelf": "L-01",
                        "slot": "G-01"
                    }
                },
                {"sample_number": "S-LOC-003", "sample_type": "RNA"}
            ],
            "operator": "dave"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("fridge_number"));

    let (_, list_body) = request(
        &app,
        "GET",
        &format!("/api/batches/{batch_id}/samples"),
        None,
    )
    .await;
    assert_eq!(list_body["count"], 0);
}

#[tokio::test]
async fn bulk_import_fails_when_batch_not_found() {
    let (app, _pool) = setup().await;

    let (status, body) = request(
        &app,
        "POST",
        "/api/batches/999999/samples/bulk-import",
        Some(json!({
            "samples": [
                {"sample_number": "S-NOBATCH-001", "sample_type": "血液"},
                {"sample_number": "S-NOBATCH-002", "sample_type": "DNA"}
            ],
            "operator": "erin"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
    assert!(body["error"]["message"].as_str().unwrap().contains("批次不存在"));
}

async fn create_batch_with_sample(
    app: &Router,
    batch_number: &str,
    sample_number: &str,
    sample_type: &str,
    location: Option<Value>,
) -> (i64, i64, Option<i64>) {
    let (_, body) = request(
        app,
        "POST",
        "/api/batches",
        Some(json!({
            "batch_number": batch_number,
            "project_name": "位置占用测试",
            "owner": "alice"
        })),
    )
    .await;
    let batch_id = body["id"].as_i64().unwrap();

    let mut payload = json!({
        "sample_number": sample_number,
        "sample_type": sample_type,
        "operator": "alice"
    });
    if let Some(location) = location {
        payload.as_object_mut().unwrap().insert("location".into(), location);
    }

    let (_, body) = request(
        app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(payload),
    )
    .await;
    let sample_id = body["id"].as_i64().unwrap();
    let location_id = body["current_location"]
        .as_object()
        .and_then(|l| l["id"].as_i64());
    (batch_id, sample_id, location_id)
}

#[tokio::test]
async fn rejects_second_unarchived_sample_at_same_location() {
    let (app, _pool) = setup().await;

    let location = json!({
        "area": "占用区",
        "fridge_number": "F-1",
        "shelf": "L-1",
        "slot": "G-1"
    });

    let (batch_id, _first_id, _) = create_batch_with_sample(
        &app,
        "B-OCC-1",
        "S-OCC-001",
        "血液",
        Some(location.clone()),
    )
    .await;

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(json!({
            "sample_number": "S-OCC-002",
            "sample_type": "DNA",
            "location": location,
            "operator": "alice"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "CONFLICT");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("S-OCC-001"));
    assert_eq!(body["error"]["details"]["occupied_by_sample_number"], "S-OCC-001");

    let (_, list_body) = request(
        &app,
        "GET",
        &format!("/api/batches/{batch_id}/samples"),
        None,
    )
    .await;
    assert_eq!(list_body["count"], 1);
}

#[tokio::test]
async fn archived_sample_releases_location_for_reuse() {
    let (app, _pool) = setup().await;

    let location = json!({
        "area": "释放区",
        "fridge_number": "F-2",
        "shelf": "L-2",
        "slot": "G-2"
    });

    let (batch_id, first_id, _) = create_batch_with_sample(
        &app,
        "B-REL-1",
        "S-REL-001",
        "血液",
        Some(location.clone()),
    )
    .await;

    for status in ["stored", "processing", "transferred", "archived"] {
        request(
            &app,
            "PUT",
            &format!("/api/samples/{first_id}/status"),
            Some(json!({
                "status": status,
                "operator": "alice",
                "location": location
            })),
        )
        .await;
    }

    let (_, flow) = request(
        &app,
        "GET",
        &format!("/api/samples/{first_id}/flow"),
        None,
    )
    .await;
    assert_eq!(flow["sample"]["status"], "archived");
    assert!(flow["sample"]["current_location"].is_null());

    let (status, body) = request(
        &app,
        "POST",
        &format!("/api/batches/{batch_id}/samples"),
        Some(json!({
            "sample_number": "S-REL-002",
            "sample_type": "DNA",
            "location": location,
            "operator": "bob"
        })),
    )
    .await;

    assert_eq!(status, http::StatusCode::CREATED);
    assert_eq!(body["sample_number"], "S-REL-002");
}

#[tokio::test]
async fn location_change_logs_old_and_new_locations() {
    let (app, _pool) = setup().await;

    let old_location = json!({
        "area": "变更区",
        "fridge_number": "F-3",
        "shelf": "L-3",
        "slot": "G-3"
    });
    let new_location = json!({
        "area": "变更区",
        "fridge_number": "F-3",
        "shelf": "L-3",
        "slot": "G-4"
    });

    let (_, sample_id, old_location_id) = create_batch_with_sample(
        &app,
        "B-MOVE-1",
        "S-MOVE-001",
        "血液",
        Some(old_location),
    )
    .await;
    let old_location_id = old_location_id.unwrap();

    let (status, _) = request(
        &app,
        "PUT",
        &format!("/api/samples/{sample_id}/location"),
        Some(json!({
            "location": new_location,
            "operator": "carol",
            "description": "转移到新格位"
        })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK);

    let (_, flow) = request(
        &app,
        "GET",
        &format!("/api/samples/{sample_id}/flow"),
        None,
    )
    .await;
    let logs = flow["operation_logs"].as_array().unwrap();
    let move_log = logs
        .iter()
        .find(|log| log["action"] == "change_location")
        .unwrap();
    assert_eq!(move_log["from_location_id"], old_location_id);
    assert!(move_log["to_location_id"].is_number());
    assert_ne!(
        move_log["from_location_id"],
        move_log["to_location_id"]
    );
}

#[tokio::test]
async fn location_detail_returns_current_sample_and_movements() {
    let (app, _pool) = setup().await;

    let location_a = json!({
        "area": "详情区",
        "fridge_number": "F-4",
        "shelf": "L-4",
        "slot": "G-5"
    });
    let location_b = json!({
        "area": "详情区",
        "fridge_number": "F-4",
        "shelf": "L-4",
        "slot": "G-6"
    });

    let (_, sample_id, location_a_id) = create_batch_with_sample(
        &app,
        "B-DETAIL-1",
        "S-DETAIL-001",
        "血液",
        Some(location_a),
    )
    .await;
    let location_a_id = location_a_id.unwrap();

    request(
        &app,
        "PUT",
        &format!("/api/samples/{sample_id}/location"),
        Some(json!({
            "location": location_b,
            "operator": "dave",
            "description": "迁出到新格位"
        })),
    )
    .await;

    let (status, body) = request(
        &app,
        "GET",
        &format!("/api/locations/{location_a_id}"),
        None,
    )
    .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["location"]["id"], location_a_id);
    assert!(body["current_sample"].is_null());
    let movements = body["recent_movements"].as_array().unwrap();
    assert!(!movements.is_empty());
    assert!(movements.len() <= 10);
    let out_movement = movements
        .iter()
        .find(|m| m["direction"] == "out")
        .expect("应包含迁出记录");
    assert_eq!(out_movement["sample_number"], "S-DETAIL-001");
    assert_eq!(out_movement["action"], "change_location");
    assert_eq!(out_movement["from_location"]["id"], location_a_id);
    assert!(out_movement["to_location"].is_object());
}

#[tokio::test]
async fn location_detail_not_found() {
    let (app, _pool) = setup().await;
    let (status, body) = request(&app, "GET", "/api/locations/999999", None).await;
    assert_eq!(status, http::StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn location_detail_excludes_same_location_status_changes() {
    let (app, _pool) = setup().await;

    let location = json!({
        "area": "同位置区",
        "fridge_number": "F-5",
        "shelf": "L-5",
        "slot": "G-5"
    });

    let (_, sample_id, location_id) = create_batch_with_sample(
        &app,
        "B-SAMELOC-1",
        "S-SAMELOC-001",
        "血液",
        Some(location.clone()),
    )
    .await;
    let location_id = location_id.unwrap();

    for status in ["stored", "processing", "transferred"] {
        let (status_code, _) = request(
            &app,
            "PUT",
            &format!("/api/samples/{sample_id}/status"),
            Some(json!({
                "status": status,
                "operator": "alice",
                "location": location
            })),
        )
        .await;
        assert_eq!(status_code, http::StatusCode::OK);
    }

    let (status, body) = request(
        &app,
        "GET",
        &format!("/api/locations/{location_id}"),
        None,
    )
    .await;

    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["location"]["id"], location_id);
    assert_eq!(body["current_sample"]["id"], sample_id);

    let movements = body["recent_movements"].as_array().unwrap();
    for movement in movements {
        let from = &movement["from_location"];
        let to = &movement["to_location"];
        let same_location = from.is_object()
            && to.is_object()
            && from["id"] == to["id"];
        assert!(
            !same_location,
            "同位置状态变更日志不应出现在迁入迁出记录中: {:?}",
            movement
        );
    }

    let actions: Vec<&str> = movements
        .iter()
        .map(|m| m["action"].as_str().unwrap())
        .collect();
    assert!(
        !actions.contains(&"process"),
        "处理状态变更不应算作位置迁移"
    );
    assert!(
        !actions.contains(&"transfer"),
        "同位置转移状态变更不应算作位置迁移"
    );
    assert!(
        actions.iter().any(|a| *a == "register" || *a == "store"),
        "应保留最初迁入该位置的记录"
    );
}
