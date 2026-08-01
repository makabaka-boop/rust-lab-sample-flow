mod db;
mod error;
mod models;
mod repo;
mod routes;
mod service;

use std::sync::{Arc, Mutex};

const DEFAULT_DB_PATH: &str = "./data/sample_flow.db";
const PORT: u16 = 18101;

#[tokio::main]
async fn main() {
    let db_path = std::env::var("SAMPLE_FLOW_DB").unwrap_or_else(|_| DEFAULT_DB_PATH.to_string());
    let conn = db::init(&db_path).unwrap_or_else(|e| {
        eprintln!("数据库初始化失败: {}", e.message);
        std::process::exit(1);
    });
    eprintln!("SQLite 数据库已初始化: {db_path}");

    let state: routes::AppState = Arc::new(Mutex::new(conn));
    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", PORT))
        .await
        .unwrap_or_else(|e| {
            eprintln!("端口 {PORT} 绑定失败: {e}");
            std::process::exit(1);
        });
    eprintln!("sample-flow 服务已启动: http://0.0.0.0:{PORT}");
    axum::serve(listener, app).await.expect("server error");
}
