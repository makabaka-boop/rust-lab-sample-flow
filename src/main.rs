use std::net::SocketAddr;

use rust_lab_sample_flow::{build_router, init_pool};

const PORT: u16 = 18101;
const DB_DIR: &str = "data";
const DB_FILE: &str = "data/lab_sample_flow.db";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().init();

    // 确保 SQLite 文件所在目录存在。
    std::fs::create_dir_all(DB_DIR)?;

    // 启动时自动初始化数据库表。
    let pool = init_pool(DB_FILE)?;
    let app = build_router(pool);

    let addr = SocketAddr::from(([0, 0, 0, 0], PORT));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("实验样本流转服务已启动: http://{addr}  (SQLite: {DB_FILE})");

    axum::serve(listener, app).await?;
    Ok(())
}
