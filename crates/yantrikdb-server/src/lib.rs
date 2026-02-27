//! YantrikDB HTTP Server — axum-based REST API for the companion.
//!
//! Replaces the Python FastAPI service. Same routes, same JSON format.
//! The companion runs in-process — no external LLM/embedding servers needed.

mod routes;

use std::sync::Mutex;

use axum::Router;
use tower_http::cors::CorsLayer;
use yantrikdb_companion::CompanionService;

/// Shared application state.
pub struct AppState {
    pub service: Mutex<CompanionService>,
    pub start_time: f64,
}

/// Build the axum router with all companion routes.
pub fn build_router(service: CompanionService) -> Router {
    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    let state = std::sync::Arc::new(AppState {
        service: Mutex::new(service),
        start_time,
    });

    Router::new()
        .merge(routes::routes())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
