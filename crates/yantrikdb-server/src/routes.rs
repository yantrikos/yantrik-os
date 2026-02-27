//! HTTP route handlers — 1:1 mapping from the Python FastAPI service.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;

type AppStateRef = Arc<AppState>;

// ── Request / Response types ─────────────────────────────

#[derive(Deserialize)]
pub struct ChatRequest {
    message: String,
}

#[derive(Serialize)]
pub struct ChatResponse {
    response: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    proactive_messages: Vec<serde_json::Value>,
    metadata: serde_json::Value,
}

#[derive(Serialize)]
pub struct HealthResponse {
    ok: bool,
}

#[derive(Serialize)]
pub struct StatusResponse {
    status: String,
    uptime_seconds: f64,
    memory_count: i64,
    pending_urges: usize,
    last_interaction_ago_seconds: f64,
    session_active: bool,
    session_turns: usize,
}

// ── Routes ───────────────────────────────────────────────

pub fn routes() -> Router<AppStateRef> {
    Router::new()
        .route("/health", get(health))
        .route("/chat", post(chat))
        .route("/status", get(status))
        .route("/urges", get(get_urges))
        .route("/urges/{urge_id}/suppress", post(suppress_urge))
        .route("/personality", get(get_personality))
        .route("/history", get(get_history))
}

// ── Handlers ─────────────────────────────────────────────

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

async fn chat(
    State(state): State<AppStateRef>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    let mut service = state
        .service
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = service.handle_message(&req.message);

    // Check for proactive messages
    let proactive = service
        .take_proactive_message()
        .map(|m| {
            vec![serde_json::json!({
                "text": m.text,
                "urge_ids": m.urge_ids,
                "generated_at": m.generated_at,
            })]
        })
        .unwrap_or_default();

    Ok(Json(ChatResponse {
        response: response.message,
        proactive_messages: proactive,
        metadata: serde_json::json!({
            "memories_recalled": response.memories_recalled,
            "urges_delivered": response.urges_delivered,
            "tool_calls_made": response.tool_calls_made,
        }),
    }))
}

async fn status(State(state): State<AppStateRef>) -> Result<Json<StatusResponse>, StatusCode> {
    let service = state
        .service
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    let memory_count = service
        .db
        .stats(None)
        .map(|s| s.active_memories)
        .unwrap_or(0);

    let pending_urges = service.urge_queue.count_pending(service.db.conn());
    let idle = service.idle_seconds();
    let state_snapshot = service.build_state();

    Ok(Json(StatusResponse {
        status: "running".to_string(),
        uptime_seconds: now - state.start_time,
        memory_count,
        pending_urges,
        last_interaction_ago_seconds: idle,
        session_active: state_snapshot.session_active,
        session_turns: state_snapshot.conversation_turn_count,
    }))
}

async fn get_urges(State(state): State<AppStateRef>) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let service = state
        .service
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let urges = service.urge_queue.get_pending(service.db.conn(), 20);
    let result: Vec<serde_json::Value> = urges
        .iter()
        .map(|u| {
            serde_json::json!({
                "urge_id": u.urge_id,
                "instinct_name": u.instinct_name,
                "reason": u.reason,
                "urgency": u.urgency,
                "suggested_message": u.suggested_message,
                "created_at": u.created_at,
                "boost_count": u.boost_count,
            })
        })
        .collect();

    Ok(Json(result))
}

async fn suppress_urge(
    State(state): State<AppStateRef>,
    axum::extract::Path(urge_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let service = state
        .service
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let success = service.urge_queue.suppress(service.db.conn(), &urge_id);
    Ok(Json(serde_json::json!({ "suppressed": success })))
}

async fn get_personality(
    State(state): State<AppStateRef>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let service = state
        .service
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match service.db.get_personality() {
        Ok(p) => Ok(Json(serde_json::to_value(p).unwrap_or_default())),
        Err(_) => Ok(Json(serde_json::json!({}))),
    }
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

async fn get_history(
    State(state): State<AppStateRef>,
    axum::extract::Query(query): axum::extract::Query<HistoryQuery>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let service = state
        .service
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let history = service.history();
    let result: Vec<serde_json::Value> = history
        .iter()
        .rev()
        .take(query.limit)
        .rev()
        .map(|msg| {
            serde_json::json!({
                "role": msg.role,
                "content": msg.content,
            })
        })
        .collect();

    Ok(Json(result))
}
