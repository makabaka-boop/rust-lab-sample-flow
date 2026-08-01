use crate::handlers;
use axum::routing::{get, post, put};
use axum::Router;
use sqlx::SqlitePool;

pub fn build_router(pool: SqlitePool) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/api/batches", post(handlers::create_batch).get(handlers::list_batches))
        .route("/api/batches/:id", get(handlers::get_batch))
        .route(
            "/api/batches/:id/samples",
            post(handlers::register_sample).get(handlers::list_samples_by_batch),
        )
        .route(
            "/api/batches/:id/samples/bulk-import",
            post(handlers::bulk_import_samples),
        )
        .route(
            "/api/locations",
            post(handlers::create_location).get(handlers::list_locations),
        )
        .route("/api/locations/:id", get(handlers::get_location_detail))
        .route("/api/samples", get(handlers::list_samples))
        .route("/api/samples/:id", get(handlers::get_sample))
        .route("/api/samples/:id/status", put(handlers::update_sample_status))
        .route("/api/samples/:id/location", put(handlers::update_sample_location))
        .route("/api/samples/:id/flow", get(handlers::get_sample_flow))
        .route(
            "/api/samples/:id/anomalies",
            post(handlers::create_anomaly).get(handlers::list_anomalies),
        )
        .route("/api/anomalies", get(handlers::list_anomalies))
        .route("/api/anomalies/:id/resolve", post(handlers::resolve_anomaly))
        .with_state(pool)
}
