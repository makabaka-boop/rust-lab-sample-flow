use rust_lab_sample_flow::{build_router, init_pool};
use std::env;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:sample_flow.db".to_string());
    let pool = init_pool(&database_url).await?;
    let app = build_router(pool);
    let listener = TcpListener::bind("0.0.0.0:18101").await?;

    tracing::info!("sample flow service listening on http://0.0.0.0:18101");
    tracing::info!("using database: {database_url}");

    axum::serve(listener, app).await?;
    Ok(())
}
