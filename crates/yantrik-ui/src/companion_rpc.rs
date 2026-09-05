//! Serve the companion to other processes.
//!
//! The shell owns the LLM: it holds the config, the model, the memory and the bond. Every app
//! under `apps/` therefore had its AI actions stubbed, because a separate process had no way to
//! reach any of it.
//!
//! Services in this system already speak JSON-RPC over a socket per service id, and the apps
//! already carry a client for it. So the companion is published on that same bus under the id
//! `companion`, and an app calls it exactly like it calls notes or weather. Anything else that
//! can open the socket — a script, an agent — gets the same three methods.
//!
//! Methods:
//!   companion.ask    { prompt, timeout_ms? }  → { text }
//!   companion.recall { query, limit? }        → { results: [{ rid, text, score, ... }] }
//!   companion.status { }                      → { online }

use std::sync::Arc;
use std::time::Duration;

use yantrik_ipc_contracts::email::ServiceError;
use yantrik_ipc_transport::server::{RpcServer, ServiceHandler};

use crate::bridge::CompanionHandle;

/// An LLM answer can take a while on a local model; a caller may ask for longer.
const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 300_000;

fn bad_request(message: impl Into<String>) -> ServiceError {
    ServiceError { code: -32602, message: message.into() }
}

fn failed(message: impl Into<String>) -> ServiceError {
    ServiceError { code: -32000, message: message.into() }
}

struct CompanionRpc {
    handle: CompanionHandle,
}

impl ServiceHandler for CompanionRpc {
    fn service_id(&self) -> &str {
        "companion"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "companion.ask" => {
                let prompt = params
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if prompt.is_empty() {
                    return Err(bad_request("ask needs a non-empty `prompt`"));
                }
                let timeout_ms = params
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_TIMEOUT_MS)
                    .min(MAX_TIMEOUT_MS);

                tracing::info!(chars = prompt.len(), timeout_ms, "companion.ask");
                let text = self
                    .handle
                    .ask(prompt, Duration::from_millis(timeout_ms))
                    .map_err(failed)?;
                Ok(serde_json::json!({ "text": text }))
            }

            "companion.recall" => {
                let query = params
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if query.is_empty() {
                    return Err(bad_request("recall needs a non-empty `query`"));
                }
                let limit = params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10)
                    .clamp(1, 100) as usize;

                let mut results = self
                    .handle
                    .recall(query, Duration::from_secs(20))
                    .map_err(failed)?;
                results.truncate(limit);

                let results: Vec<serde_json::Value> = results
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "rid": m.rid,
                            "text": m.text,
                            "score": m.score,
                            "memory_type": m.memory_type,
                            "created_at": m.created_at,
                        })
                    })
                    .collect();
                Ok(serde_json::json!({ "results": results }))
            }

            // Cheap enough to call before showing an AI affordance at all.
            "companion.status" => Ok(serde_json::json!({ "online": self.handle.is_online() })),

            other => Err(bad_request(format!("unknown method `{other}`"))),
        }
    }
}

/// Publish the companion on the service bus.
///
/// Runs on its own thread with its own small runtime: the shell's main thread belongs to Slint,
/// and a blocked UI thread is a frozen desktop.
pub fn serve(handle: CompanionHandle) {
    std::thread::Builder::new()
        .name("companion-rpc".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(error = %e, "Companion RPC: no runtime, apps cannot reach the companion");
                    return;
                }
            };
            runtime.block_on(async {
                let address = RpcServer::default_address("companion");
                tracing::info!(address = %address, "Companion RPC listening");
                if let Err(e) = RpcServer::new(&address)
                    .serve(Arc::new(CompanionRpc { handle }))
                    .await
                {
                    tracing::error!(error = %e, "Companion RPC stopped");
                }
            });
        })
        .expect("spawn companion-rpc thread");
}
