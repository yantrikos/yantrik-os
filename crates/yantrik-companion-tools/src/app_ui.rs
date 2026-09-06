//! Reading and driving Yantrik's own app windows, without looking at pixels.
//!
//! The other way to answer "what is on screen" is `screenshot_and_analyze` in `vision.rs`: run
//! `grim`, base64 the PNG, post it to a vision model, and read back a description. That is the
//! right tool for a foreign window — we did not write Chromium and it owes us no account of
//! itself — and the wrong one for our own, which knows the answer exactly and can simply say it.
//!
//! Each app under `apps/` publishes `app.describe` and `app.act` on the socket bus (see
//! `yantrik-app-runtime::control`). These three tools are the other end of that: a survey, a
//! detailed read, and a way to act. No GPU, no round-trip to a vision model, no guessing from a
//! screenshot — and the answer is current rather than as of whenever the picture was taken.
//!
//! An app that has not published a surface simply does not appear here; there is nothing to fall
//! back to and nothing to apologise for. Use the vision tools for those, as before.

use std::time::Duration;

use yantrik_ipc_transport::SyncRpcClient;

use super::{parse_permission, PermissionLevel, Tool, ToolContext, ToolRegistry};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ListAppsTool));
    reg.register(Box::new(DescribeAppTool));
    reg.register(Box::new(AppActionTool));
}

/// A `describe` reads a few properties on the app's UI thread; it should be immediate.
const READ_TIMEOUT: Duration = Duration::from_secs(4);

/// An action may open a file or save one, so it gets more room — but it is still not a place for
/// work an app should have moved to a worker thread.
const ACT_TIMEOUT: Duration = Duration::from_secs(15);

fn client(app: &str, timeout: Duration) -> SyncRpcClient {
    let service = yantrik_ipc_transport::server::RpcServer::default_address(&format!("app-{app}"));
    SyncRpcClient::new(&service).with_timeout(timeout)
}

/// The app ids with a control socket in this session.
///
/// A socket file survives a crash, so this lists candidates. Anything that fails to answer is
/// simply left out rather than reported as an error — a stale socket is not news.
#[cfg(unix)]
fn app_sockets() -> Vec<String> {
    let dir = yantrik_ipc_transport::server::socket_dir();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| {
            name.strip_suffix(".sock")
                .and_then(|stem| stem.strip_prefix("app-"))
                .map(str::to_string)
        })
        .collect();
    ids.sort();
    ids
}

#[cfg(not(unix))]
fn app_sockets() -> Vec<String> {
    Vec::new()
}

fn describe(app: &str) -> Result<serde_json::Value, String> {
    client(app, READ_TIMEOUT)
        .call("app.describe", serde_json::json!({}))
        .map_err(|e| e.message)
}

/// The risk the app itself declared for this action.
///
/// Apps do not have one risk level — reading which note is open and killing a process arrive
/// through the same door — so each action states its own, and it is checked here against the
/// caller's ceiling. An action with no declaration is treated as Standard, the same floor the
/// runtime uses: an unknown risk is never treated as no risk.
fn declared_permission(view: &serde_json::Value, action: &str) -> PermissionLevel {
    view.get("actions")
        .and_then(|v| v.as_array())
        .and_then(|actions| {
            actions.iter().find(|a| a.get("name").and_then(|n| n.as_str()) == Some(action))
        })
        .and_then(|a| a.get("permission").and_then(|p| p.as_str()))
        .map(parse_permission)
        .unwrap_or(PermissionLevel::Standard)
}

// ── Which of our apps are open ──

pub struct ListAppsTool;

impl Tool for ListAppsTool {
    fn name(&self) -> &'static str {
        "list_apps"
    }
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Safe
    }
    fn category(&self) -> &'static str {
        "app"
    }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_apps",
                "description": "List the Yantrik apps that are open, with a one-line summary of \
                                what each is showing. Use this instead of a screenshot when the \
                                question is about our own apps (notes, email, calendar, music, \
                                files...). Follow up with describe_app for detail.",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let ids = app_sockets();
        if ids.is_empty() {
            return "No Yantrik app is open (or none publishes a control surface). \
                    For other windows use list_windows, or screenshot_and_analyze to see them."
                .to_string();
        }

        let mut lines = Vec::new();
        for id in &ids {
            match describe(id) {
                Ok(view) => {
                    let summary = view
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(no summary)");
                    let actions: Vec<&str> = view
                        .get("actions")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter().filter_map(|x| x.get("name").and_then(|n| n.as_str())).collect()
                        })
                        .unwrap_or_default();
                    lines.push(format!(
                        "{id}: {summary}\n  can: {}",
                        if actions.is_empty() { "—".to_string() } else { actions.join(", ") }
                    ));
                }
                // A socket nobody answers is a dead app, not a failure worth reporting.
                Err(_) => continue,
            }
        }

        if lines.is_empty() {
            "No Yantrik app answered. Their sockets exist but the processes are gone.".to_string()
        } else {
            format!("Open Yantrik apps ({}):\n{}", lines.len(), lines.join("\n"))
        }
    }
}

// ── What one app is holding ──

pub struct DescribeAppTool;

impl Tool for DescribeAppTool {
    fn name(&self) -> &'static str {
        "describe_app"
    }
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Safe
    }
    fn category(&self) -> &'static str {
        "app"
    }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "describe_app",
                "description": "Read exactly what a Yantrik app is showing — which note is open, \
                                what is playing, what is selected — as structured state, plus the \
                                actions it accepts. Accurate and current, unlike a screenshot.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": {
                            "type": "string",
                            "description": "App id, e.g. notes, email, calendar, music"
                        }
                    },
                    "required": ["app"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let app = args["app"].as_str().unwrap_or("").trim();
        if app.is_empty() {
            return "Error: `app` is required. Call list_apps to see which are open.".to_string();
        }

        match describe(app) {
            Ok(view) => serde_json::to_string_pretty(&view)
                .unwrap_or_else(|e| format!("Error: could not format the reply: {e}")),
            Err(message) => {
                let open = app_sockets();
                if open.is_empty() {
                    format!("'{app}' is not open, and no Yantrik app is. ({message})")
                } else {
                    format!("'{app}' did not answer ({message}). Open: {}", open.join(", "))
                }
            }
        }
    }
}

// ── Telling one app to do something ──

pub struct AppActionTool;

impl Tool for AppActionTool {
    fn name(&self) -> &'static str {
        "app_action"
    }
    /// Standard, not Safe: these change what the user is looking at and can write their files.
    /// Reading state is free; steering the desktop is not.
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Standard
    }
    fn category(&self) -> &'static str {
        "app"
    }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "app_action",
                "description": "Ask a Yantrik app to do one of the actions it published — open a \
                                note, save, search, switch view. Call describe_app first to see \
                                the action names and their arguments. This drives the app through \
                                its own controls; no clicking or typing is involved.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string", "description": "App id, e.g. notes" },
                        "action": { "type": "string", "description": "Action name from describe_app" },
                        "args": {
                            "type": "object",
                            "description": "Arguments for the action, as the action's schema describes"
                        }
                    },
                    "required": ["app", "action"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let app = args["app"].as_str().unwrap_or("").trim();
        let action = args["action"].as_str().unwrap_or("").trim();
        if app.is_empty() || action.is_empty() {
            return "Error: both `app` and `action` are required.".to_string();
        }
        let action_args = args.get("args").cloned().unwrap_or(serde_json::json!({}));

        // Ask the app what this action costs before doing it. `app_action` itself is Standard —
        // enough to open a note — but an app may publish something that ends a process or deletes
        // a file, and that must meet the configured ceiling on its own terms rather than ride in
        // on the tool's.
        if let Ok(ref view) = describe(app) {
            let needed = declared_permission(view, action);
            if needed > ctx.max_permission {
                return format!(
                    "Permission denied: '{app}.{action}' is declared {needed} but max is {}",
                    ctx.max_permission
                );
            }
        }

        let outcome = client(app, ACT_TIMEOUT).call(
            "app.act",
            serde_json::json!({ "action": action, "args": action_args }),
        );

        match outcome {
            Ok(value) => {
                let result = value.get("result").unwrap_or(&value);
                // The state after the action is what the caller actually wants to know; a bare
                // "ok" would send it straight back for a describe.
                let now = describe(app)
                    .ok()
                    .and_then(|v| v.get("summary").and_then(|s| s.as_str()).map(str::to_string))
                    .unwrap_or_default();
                let result = serde_json::to_string(result).unwrap_or_else(|_| "ok".into());
                if now.is_empty() {
                    format!("{app}.{action} → {result}")
                } else {
                    format!("{app}.{action} → {result}\nNow: {now}")
                }
            }
            // The app's own refusals arrive here and already name what was wrong ("no note is
            // open", "`open_note` needs argument `title`"), so pass them through unedited.
            Err(e) => format!("{app}.{action} failed: {}", e.message),
        }
    }
}
