//! YantrikDB HTTP Server — axum-based REST API for the companion.
//!
//! Replaces the Python FastAPI service. Same routes, same JSON format.
//! The companion runs in-process — no external LLM/embedding servers needed.

mod routes;

use std::sync::Mutex;

use axum::Router;
use tower_http::cors::CorsLayer;
use yantrik_companion::CompanionService;

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

    build_router_from_state(state)
}

/// Build the axum router from a pre-existing shared state.
/// Use this when you need to share AppState with other threads (e.g. background cognition).
pub fn build_router_from_state(state: std::sync::Arc<AppState>) -> Router {
    Router::new()
        .merge(routes::routes())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
