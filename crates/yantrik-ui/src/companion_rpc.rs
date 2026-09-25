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
//!     — refused with `ERR_NO_MODEL` when the shell has no model behind it; the canned text the
//!       offline responder would serve never crosses this socket as an answer.
//!   companion.recall { query, limit? }        → { results: [{ rid, text, score, ... }] }
//!   companion.status { }                      → { online }
//!   companion.tools  { }                      → { tools: [{ name, category, permission, ... }] }
//!   companion.tool   { name, args, timeout_ms? } → { result }
//!   companion.submit { kind: ask|tool, ... }   → { ticket, ahead, active, eta_seconds?, eta_basis }
//!   companion.await  { ticket, wait_ms? }      → { state, ahead, partial, result?, error? }
//!   companion.jobs   { }                       → { lanes: [...], typical: [...] }
//!   companion.cancel { ticket }                → { was }
//!
//! `ask` and `tool` above block until the work is finished, which is the wrong shape whenever
//! anything else is already in flight: the worker is one lane, so a one-millisecond tool call
//! arriving mid-generation waits for the whole answer — measured at thirty-six to fifty seconds —
//! and learns nothing while it does.
//!
//! `submit` is the same work, accepted rather than served. It returns in about a millisecond with
//! a ticket and, more usefully, with how many jobs are ahead and how long they have typically
//! taken. A caller can then wait on `await`, get on with something else, or decide not to bother.
//! The blocking pair stay, for callers that genuinely want to wait.
//!
//! The last two matter more than they look. The companion carries 178 tools — browsers, windows,
//! files, containers, mail — and until now the only way to reach any of them was to persuade a
//! language model to pick one during a conversation. That is a slow and unreliable way to make a
//! function call, and it stops working entirely when the model does. `companion.tool` is the same
//! registry, with the same permission ceiling and the same audit trail, called by name.

use std::sync::Arc;
use std::time::Duration;

use yantrik_ipc_contracts::email::ServiceError;
use yantrik_ipc_transport::server::{RpcServer, ServiceHandler};

use crate::bridge::{AskError, CompanionHandle};

/// An LLM answer can take a while on a local model; a caller may ask for longer.
const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 300_000;

fn bad_request(message: impl Into<String>) -> ServiceError {
    ServiceError { code: -32602, message: message.into() }
}

fn failed(message: impl Into<String>) -> ServiceError {
    ServiceError { code: -32000, message: message.into() }
}

/// The wire shape of a failed ask.
///
/// No-model gets its own code because it is not a transient failure and its fallback is not an
/// answer: apps recognised neither from a generic error, showed the canned text the offline
/// responder produced as the model's words, and wrote it into documents.
fn ask_failed(e: AskError) -> ServiceError {
    match e {
        AskError::NoModel => ServiceError {
            code: yantrik_ipc_contracts::ERR_NO_MODEL,
            message: "no AI model answered: check Settings \u{2192} AI".to_string(),
        },
        AskError::Failed(reason) => failed(reason),
    }
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
                    .map_err(ask_failed)?;
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

            // Reading the catalogue is instant; the wait is the queue. The worker is
            // single-threaded, so this sits behind whatever answer is in flight, and a caller
            // asking what tools exist should not be told "no" because a reply was mid-sentence.
            "companion.tools" => {
                let timeout_ms = params
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_TIMEOUT_MS)
                    .min(MAX_TIMEOUT_MS);
                self.handle.tools(Duration::from_millis(timeout_ms)).map_err(failed)
            }

            "companion.tool" => {
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() {
                    return Err(bad_request("tool needs a `name`; call companion.tools to see them"));
                }
                let args = params.get("args").cloned().unwrap_or(serde_json::json!({}));
                // Tools shell out to real programs — a package install, a container build — so the
                // default here is generous, and still bounded.
                let timeout_ms = params
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_TIMEOUT_MS)
                    .min(MAX_TIMEOUT_MS);

                tracing::info!(tool = %name, "companion.tool");
                let result = self
                    .handle
                    .tool(name, args, Duration::from_millis(timeout_ms))
                    .map_err(failed)?;
                // Tools return prose, not JSON: they were written to be read by a model. A caller
                // that wants structure should parse it, not have us guess at a shape.
                Ok(serde_json::json!({ "result": result }))
            }

            // ── Accepted, not served ──
            "companion.submit" => {
                let receipt = match params.get("kind").and_then(|v| v.as_str()).unwrap_or("ask") {
                    "ask" => {
                        let prompt = params
                            .get("prompt")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if prompt.is_empty() {
                            return Err(bad_request("submitting an ask needs a `prompt`"));
                        }
                        self.handle.submit_ask(prompt).map_err(failed)?
                    }
                    "tool" => {
                        let name = params
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if name.is_empty() {
                            return Err(bad_request(
                                "submitting a tool needs a `name`; call companion.tools to see them",
                            ));
                        }
                        let args = params.get("args").cloned().unwrap_or(serde_json::json!({}));
                        self.handle.submit_tool(name, args).map_err(failed)?
                    }
                    other => {
                        return Err(bad_request(format!(
                            "unknown kind `{other}`; submit an `ask` or a `tool`"
                        )))
                    }
                };
                Ok(serde_json::to_value(&receipt).unwrap_or(serde_json::json!({})))
            }

            "companion.await" => {
                let ticket = params
                    .get("ticket")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if ticket.is_empty() {
                    return Err(bad_request("await needs a `ticket` from companion.submit"));
                }
                // Zero is a legitimate ask — "where is it right now" — so it is not defaulted
                // away. The board's own ceiling bounds the rest.
                let wait =
                    Duration::from_millis(params.get("wait_ms").and_then(|v| v.as_u64()).unwrap_or(0));
                self.handle.board().wait(&ticket, wait).ok_or_else(|| {
                    failed(format!(
                        "no job called `{ticket}`; it either never existed or finished long enough                          ago to have been forgotten"
                    ))
                })
            }

            // The whole board. Answers "is it worth asking right now" without submitting anything.
            "companion.jobs" => Ok(self.handle.board().overview()),

            "companion.cancel" => {
                let ticket = params.get("ticket").and_then(|v| v.as_str()).unwrap_or("").trim();
                if ticket.is_empty() {
                    return Err(bad_request("cancel needs a `ticket`"));
                }
                match self.handle.board().cancel(ticket) {
                    Some(was) => Ok(serde_json::json!({
                        "cancelled": ticket,
                        "was": was,
                        // Said plainly, because the difference matters: a queued job simply never
                        // runs, while a running one is *asked* to stop and may still finish.
                        "note": match was {
                            crate::jobs::State::Queued => "it had not started, so it will not run",
                            crate::jobs::State::Running =>
                                "it is already running; it will stop at the next token it produces",
                            _ => "it had already finished",
                        },
                    })),
                    None => Err(failed(format!("no job called `{ticket}`"))),
                }
            }

            // Cheap enough to call before showing an AI affordance at all.
            "companion.status" => Ok(serde_json::json!({ "online": self.handle.is_online() })),

            other => Err(bad_request(format!(
                "unknown method `{other}`; this service serves companion.ask, companion.recall,                  companion.status, companion.tools, companion.tool, companion.submit,                  companion.await, companion.jobs, companion.cancel"
            ))),
        }
    }
}

/// The runtime the companion's RPC answers on.
///
/// Multi-threaded, and that is load-bearing: `handle` is synchronous, so a `companion.ask`
/// occupies whatever thread runs it for as long as the model takes — up to ninety seconds. On
/// the single thread this used to be, that one call was the whole runtime: every other app's
/// `companion.status` queued behind it, and apps that call status from their UI thread froze
/// their windows along with it. Four threads is a ceiling on how many blocking calls can be in
/// flight at once, not a fix for the queue itself — `companion.submit` and `companion.await`
/// remain the shape that does not block at all.
fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build()
}

/// Publish the companion on the service bus.
///
/// Runs on its own thread with its own small runtime: the shell's main thread belongs to Slint,
/// and a blocked UI thread is a frozen desktop.
pub fn serve(handle: CompanionHandle) {
    std::thread::Builder::new()
        .name("companion-rpc".into())
        .spawn(move || {
            let runtime = match runtime() {
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

#[cfg(test)]
mod error_tests {
    use super::*;

    /// The no-model case has to travel under its own code: it is the one thing a caller can
    /// recognise without parsing a message, and a generic failure code is what let the offline
    /// responder's canned text pass for an answer on the app side.
    #[test]
    fn no_model_travels_under_its_own_code() {
        let e = ask_failed(AskError::NoModel);
        assert_eq!(e.code, yantrik_ipc_contracts::ERR_NO_MODEL);
        assert!(!e.message.is_empty(), "the code alone is for machines; the message is for logs");
    }

    #[test]
    fn any_other_failure_keeps_the_generic_code_and_its_reason() {
        let e = ask_failed(AskError::Failed("companion timed out".into()));
        assert_eq!(e.code, -32000);
        assert_eq!(e.message, "companion timed out");
    }
}

#[cfg(all(test, unix))]
mod serve_tests {
    use super::*;
    use crossbeam_channel::{Receiver, Sender};
    use yantrik_ipc_transport::SyncRpcClient;

    /// A handler that holds one method open until released, and answers everything else at
    /// once — the shape of a ninety-second `ask` with a `companion.status` arriving mid-answer.
    struct Gated {
        entered: Sender<()>,
        release: Receiver<()>,
    }

    impl ServiceHandler for Gated {
        fn service_id(&self) -> &str {
            "companion"
        }
        fn handle(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, ServiceError> {
            if method == "slow" {
                let _ = self.entered.send(());
                let _ = self.release.recv_timeout(Duration::from_secs(30));
            }
            Ok(serde_json::json!({ "method": method }))
        }
    }

    /// What this pins: while one app's call sits in the handler, another app's call is still
    /// answered. On the single-threaded runtime this service used to run on, the blocked call
    /// was the whole runtime — `companion.status` from a UI thread waited out the full ninety
    /// seconds, and the window asking froze with it.
    #[test]
    fn a_slow_call_in_flight_does_not_hold_up_another_apps_call() {
        let dir = std::env::temp_dir().join(format!("yantrik-companion-rpc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("companion.sock");
        let address = path.to_string_lossy().to_string();

        let (entered_tx, entered_rx) = crossbeam_channel::bounded(1);
        let (release_tx, release_rx) = crossbeam_channel::bounded(1);
        let handler = Arc::new(Gated { entered: entered_tx, release: release_rx });

        let serve_address = address.clone();
        std::thread::spawn(move || {
            let rt = runtime().expect("the runtime the service actually uses");
            let _ = rt.block_on(RpcServer::new(&serve_address).serve(handler));
        });

        // Wait for the socket as state, not by sleeping a guessed while.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::os::unix::net::UnixStream::connect(&path).is_err() {
            assert!(std::time::Instant::now() < deadline, "the server never came up");
            std::thread::sleep(Duration::from_millis(50));
        }

        // One caller sits in the slow method...
        let slow_address = address.clone();
        let slow = std::thread::spawn(move || {
            SyncRpcClient::new(&slow_address)
                .with_timeout(Duration::from_secs(30))
                .call("slow", serde_json::json!({}))
                .expect("the slow call answers once released")
        });
        entered_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the slow call reached the handler");

        // ...and a second must still be answered while the first is held.
        let fast = SyncRpcClient::new(&address)
            .with_timeout(Duration::from_secs(10))
            .call("fast", serde_json::json!({}))
            .expect("a fast call must be answered while a slow one is in flight");
        assert_eq!(fast["method"], "fast");

        let _ = release_tx.send(());
        let _ = slow.join();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
