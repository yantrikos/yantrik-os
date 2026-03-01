//! HTTP route handlers — companion API + OpenAI-compatible endpoints.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

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
        // OpenAI-compatible endpoints
        .route("/v1/chat/completions", post(oai_chat_completions))
        .route("/v1/models", get(oai_models))
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

// ── OpenAI-compatible types ─────────────────────────────

use axum::response::IntoResponse;

#[derive(Deserialize)]
struct OaiChatRequest {
    #[serde(default)]
    model: String,
    messages: Vec<OaiMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(default = "default_oai_max_tokens")]
    #[allow(dead_code)]
    max_tokens: usize,
    #[serde(default = "default_oai_temperature")]
    #[allow(dead_code)]
    temperature: f64,
}

fn default_oai_max_tokens() -> usize {
    512
}
fn default_oai_temperature() -> f64 {
    0.7
}

#[derive(Deserialize)]
struct OaiMessage {
    role: String,
    content: String,
}

// ── OpenAI-compatible handlers ──────────────────────────

async fn oai_models() -> Json<serde_json::Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Json(serde_json::json!({
        "object": "list",
        "data": [{
            "id": "yantrik",
            "object": "model",
            "created": now,
            "owned_by": "yantrik-os"
        }]
    }))
}

async fn oai_chat_completions(
    State(state): State<AppStateRef>,
    Json(req): Json<OaiChatRequest>,
) -> Result<axum::response::Response, StatusCode> {
    let user_text = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    if user_text.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let model_name = if req.model.is_empty() {
        "yantrik".to_string()
    } else {
        req.model.clone()
    };

    let request_id = format!("chatcmpl-{:x}", rand_id());

    if req.stream {
        oai_stream(state, user_text, model_name, request_id).await
    } else {
        oai_blocking(state, user_text, model_name, request_id).await
    }
}

/// Non-streaming: collect all tokens, return a single completion object.
async fn oai_blocking(
    state: AppStateRef,
    user_text: String,
    model: String,
    id: String,
) -> Result<axum::response::Response, StatusCode> {
    let result = tokio::task::spawn_blocking(move || {
        let mut service = match state.service.lock() {
            Ok(s) => s,
            Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
        };
        let response = service.handle_message(&user_text);
        Ok(response.message)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let content = result?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let body = serde_json::json!({
        "id": id,
        "object": "chat.completion",
        "created": now,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0,
        }
    });

    Ok(Json(body).into_response())
}

/// Streaming: SSE with delta chunks (OpenAI wire format).
async fn oai_stream(
    state: AppStateRef,
    user_text: String,
    model: String,
    id: String,
) -> Result<axum::response::Response, StatusCode> {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    let model_clone = model.clone();
    let id_clone = id.clone();

    tokio::task::spawn_blocking(move || {
        let mut service = match state.service.lock() {
            Ok(s) => s,
            Err(_) => {
                let _ = tx.blocking_send(Ok(Event::default().data(
                    serde_json::json!({
                        "error": { "message": "Service busy", "type": "server_error" }
                    })
                    .to_string(),
                )));
                return;
            }
        };

        service.handle_message_streaming(&user_text, |token| {
            if token == "__DONE__" {
                return;
            }
            let chunk = serde_json::json!({
                "id": id_clone,
                "object": "chat.completion.chunk",
                "model": model_clone,
                "choices": [{
                    "index": 0,
                    "delta": { "content": token },
                    "finish_reason": serde_json::Value::Null,
                }]
            });
            let _ = tx.blocking_send(Ok(Event::default().data(chunk.to_string())));
        });

        // Final chunk with finish_reason + [DONE] sentinel
        let done_chunk = serde_json::json!({
            "id": id_clone,
            "object": "chat.completion.chunk",
            "model": model_clone,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop",
            }]
        });
        let _ = tx.blocking_send(Ok(Event::default().data(done_chunk.to_string())));
        let _ = tx.blocking_send(Ok(Event::default().data("[DONE]".to_string())));
    });

    let stream = ReceiverStream::new(rx);
    let sse = Sse::new(stream).keep_alive(KeepAlive::default());
    Ok(sse.into_response())
}

/// Pseudo-random ID for completion responses.
fn rand_id() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
