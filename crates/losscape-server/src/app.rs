use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router, middleware};

use crate::state::AppState;
use crate::{auth, ingest, runs};

pub fn router(state: AppState, max_body_bytes: usize) -> Router {
    let api = Router::new()
        .route("/runs/init", post(runs::init))
        .route("/runs/heartbeat", post(runs::heartbeat))
        .route("/runs/finish", post(runs::finish))
        .route("/ingest", post(ingest::ingest))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ))
        // Routes below are public.
        .route("/version", get(version));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .nest("/api/v0", api)
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> (StatusCode, &'static str) {
    let pg = sqlx::query("SELECT 1").execute(state.pg()).await;
    let ch = state.ch().query("SELECT 1").fetch_one::<u8>().await;
    match (pg, ch) {
        (Ok(_), Ok(_)) => (StatusCode::OK, "ok"),
        (Err(_), _) => (StatusCode::SERVICE_UNAVAILABLE, "postgres unavailable"),
        (_, Err(_)) => (StatusCode::SERVICE_UNAVAILABLE, "clickhouse unavailable"),
    }
}

#[derive(serde::Serialize)]
struct VersionInfo {
    version: &'static str,
    protocol: u32,
}

async fn version() -> Json<VersionInfo> {
    Json(VersionInfo {
        version: losscape_core::VERSION,
        protocol: losscape_core::PROTOCOL_VERSION,
    })
}
