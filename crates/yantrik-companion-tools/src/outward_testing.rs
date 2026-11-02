//! A fake desktop shell for tests of everything that asks the person (`outward`): a socket that
//! answers the approvals calls the real shell answers, and a log of every call.
//!
//! Compiled for this crate's tests and, through the `test-support` feature, for the companion's
//! (its tools ask through `outward` too). Nothing in a production build.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::app_ui::Desk;

/// A shell on a socket: a card that is answered `answer` on the second poll (or needs no card
/// at all when `first` is `not_needed`), and a log of every call.
pub fn shell(tag: &str, first: &'static str, answer: &'static str) -> (Desk, Arc<Mutex<Vec<serde_json::Value>>>) {
    let dir = std::env::temp_dir().join(format!("yantrik-outward-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let listener = UnixListener::bind(dir.join("app-shell.sock")).unwrap();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let log = heard.clone();
    let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let (log, polls) = (log.clone(), polls.clone());
            std::thread::spawn(move || {
                let mut writer = stream.try_clone().unwrap();
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { return };
                    let asked: serde_json::Value = serde_json::from_str(&line).unwrap();
                    let params = asked["params"].clone();
                    log.lock().unwrap().push(params.clone());
                    let result = match params["action"].as_str() {
                        Some("request_approval") if first == "not_needed" => {
                            serde_json::json!({"result": {"status": "not_needed", "mode": "bypass_all"}})
                        }
                        Some("request_approval") if first == "no_id" => serde_json::json!({"result": {"status": "pending"}}),
                        Some("request_approval") if first == "too_long" => serde_json::json!({"error": {"code": -32602,
                            "message": "this request is too long to put in front of a person in full, so nothing was asked: `body` is 900 characters"}}),
                        Some("request_approval") => serde_json::json!({"result": {"request_id": "appr-1", "status": "pending"}}),
                        Some("approval_status") => {
                            let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            serde_json::json!({"result": {"status": if n == 0 { "pending" } else { answer }}})
                        }
                        Some("consume_approval") => serde_json::json!({"result": {"consumed": true}}),
                        Some("record_unasked_action") => serde_json::json!({"result": {"recorded": true}}),
                        _ => serde_json::json!({"error": {"code": -32602, "message": "unknown"}}),
                    };
                    let reply = if result.get("error").is_some() {
                        serde_json::json!({"jsonrpc": "2.0", "id": asked["id"], "error": result["error"]})
                    } else {
                        serde_json::json!({"jsonrpc": "2.0", "id": asked["id"], "result": result})
                    };
                    if writer.write_all(format!("{reply}\n").as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    (Desk { dir: Some(dir), wait: Duration::from_millis(600), poll: Duration::from_millis(20) }, heard)
}

/// The calls a fake shell has heard, by action name.
pub fn actions(heard: &Arc<Mutex<Vec<serde_json::Value>>>) -> Vec<String> {
    heard.lock().unwrap().iter().map(|p| p["action"].as_str().unwrap_or("").to_string()).collect()
}

static OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);
static SERIAL: Mutex<()> = Mutex::new(());

/// While this lives, [`crate::outward`] asks the fake shell in `desk` instead of the session's,
/// and no other test can install one: tools reach the shell through the session desk, which
/// nothing else lets a test redirect.
pub struct Installed {
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Installed {
    fn drop(&mut self) {
        *OVERRIDE.lock().unwrap() = None;
    }
}

/// Point the session desk at `desk`'s directory (see [`Installed`]).
pub fn install(desk: &Desk) -> Installed {
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    *OVERRIDE.lock().unwrap() = desk.dir.clone();
    Installed { _serial: serial }
}

/// The directory a test installed, if one did.
pub(crate) fn installed_dir() -> Option<PathBuf> {
    OVERRIDE.lock().ok().and_then(|d| d.clone())
}

/// A fake shell like [`shell`], installed as the session's, for tests of tools that ask through
/// `outward`. Hold the returned guard for the length of the test.
pub fn installed(tag: &str, first: &'static str, answer: &'static str) -> (Installed, Arc<Mutex<Vec<serde_json::Value>>>) {
    let (desk, heard) = shell(tag, first, answer);
    (install(&desk), heard)
}

/// The arguments of the first card raised (`request_approval`): what the person was shown.
pub fn first_card(heard: &Arc<Mutex<Vec<serde_json::Value>>>) -> serde_json::Value {
    let calls = heard.lock().unwrap();
    let ask = calls.iter().find(|c| c["action"] == "request_approval").expect("no card was raised");
    ask["args"].clone()
}

/// The text a card argument carries, unquoted.
pub fn shown(card: &serde_json::Value, key: &str) -> String {
    serde_json::from_str::<String>(card["args_json"][key].as_str().unwrap_or("\"\"")).unwrap_or_default()
}

/// A tool context over an in-memory database, for running a tool's `execute` in a test.
pub fn with_ctx<R>(f: impl FnOnce(&yantrik_companion_core::tools::ToolContext<'_>) -> R) -> R {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = yantrik_companion_core::tools::ToolContext {
        db: &db,
        max_permission: crate::PermissionLevel::Dangerous,
        registry_metadata: None,
        task_manager: None,
        incognito: true,
        agent_spawner: None,
    };
    f(&ctx)
}
