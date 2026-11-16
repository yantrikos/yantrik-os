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

use yantrik_ipc_transport::gate;
use yantrik_ipc_transport::SyncRpcClient;

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};
use crate::app_reach::published;
pub(crate) use crate::app_ask::{act_asking, await_answer, raise_card, Desk};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ListAppsTool));
    reg.register(Box::new(DescribeAppTool));
    reg.register(Box::new(AppActionTool));
    reg.register(Box::new(crate::app_await::AwaitAppTool));
}

/// A `describe` reads a few properties on the app's UI thread; it should be immediate.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(4);

/// An action may open a file or save one, so it gets more room — but it is still not a place for
/// work an app should have moved to a worker thread.
pub(crate) const ACT_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn client(app: &str, timeout: Duration) -> SyncRpcClient {
    let service = yantrik_ipc_transport::server::RpcServer::default_address(&format!("app-{app}"));
    SyncRpcClient::new(&service).with_timeout(timeout)
}

/// The app ids with a control socket in this session.
///
/// A socket file survives a crash, so this lists candidates. Anything that fails to answer is
/// simply left out rather than reported as an error — a stale socket is not news.
///
/// One app, one entry: an app answers to every name it is known by (`containers` is also
/// `container-manager`) and the other names are symlinks to its socket, so listing them would
/// offer one open window twice under two names — and a survey that names the same thing twice is
/// read as two things.
#[cfg(unix)]
fn app_sockets() -> Vec<String> {
    let dir = yantrik_ipc_transport::server::socket_dir();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_type().is_ok_and(|kind| kind.is_symlink()))
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

pub(crate) fn describe(app: &str) -> Result<serde_json::Value, String> {
    client(app, READ_TIMEOUT)
        .call("app.describe", serde_json::json!({}))
        .map_err(|e| e.message)
}

/// Whether this assistant's own cap (`tools.max_permission`) lets it use the action at all, or
/// the sentence saying why not.
///
/// Apps do not have one risk level — reading which note is open and killing a process arrive
/// through the same door — so each action states its own, and it is compared here against the
/// cap on the gate's ladder (`gate::permits`), not a copy of it. An action with no declaration is
/// `standard`, the grade `Action::new` gives it; a grade the ladder does not have is refused, as
/// the app's own dispatch would refuse it — an unknown risk is never treated as no risk.
///
/// A courtesy, not the boundary: the grade can move while the app runs (`control::regrade`), and
/// the app's dispatch decides on the grade it publishes at the moment the call arrives, with the
/// machine's ceiling and the person's mode besides. This only saves a call the cap already rules
/// out.
pub(crate) fn beyond_cap(view: &serde_json::Value, app: &str, action: &str, cap: PermissionLevel) -> Option<String> {
    let graded = published(view, action)
        .and_then(|a| a.get("permission").and_then(|p| p.as_str()))
        .unwrap_or("standard");
    match gate::permits(&cap.to_string(), graded) {
        Some(true) => None,
        Some(false) => Some(format!(
            "Permission denied: '{app}.{action}' is declared {graded} but max is {cap}"
        )),
        None => Some(format!(
            "Permission denied: '{app}.{action}' is graded `{graded}`, which is not a level this \
             OS defines ({}), so it was not run.",
            gate::LADDER.join(" < ")
        )),
    }
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
                                question is about our own apps (notes, email, calendar, weather, \
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
                            "description": "App id, e.g. notes, email, calendar, weather"
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

        // A Yantrik app that is not running is started with no window first (#978 step 1).
        if let Err(said) = crate::app_start::ensure(app, false) {
            return said;
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
                                its own controls; no clicking or typing is involved. The reply \
                                says whether the action was accepted and whether the work is \
                                finished — those are different things.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string", "description": "App id, e.g. notes" },
                        "action": { "type": "string", "description": "Action name from describe_app" },
                        "args": {
                            "type": "object",
                            "description": "Arguments for the action, as the action's schema describes"
                        },
                        "expect_revision": {
                            "type": "string",
                            "description": "The `revision` from the describe_app you based this \
                                            on. Pass it whenever your decision depended on what \
                                            the app was showing: the action is then refused if \
                                            the user changed something in the meantime, instead \
                                            of landing on the wrong note."
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
        let expect = args.get("expect_revision").and_then(|v| v.as_str()).unwrap_or("").trim();

        // One road for every model call that acts on an app (`app_act::gated_act`): what the call names is
        // refused first, then the app is started, its own declaration is read, the person's cap is held,
        // and a card goes up when the app says to. `await_app`'s `then_act` takes the same road.
        match crate::app_act::gated_act(ctx, app, action, &action_args, expect) {
            Ok(value) => describe_outcome(app, action, &value),
            // The app's own refusals arrive here and already name what was wrong ("no note is
            // open", "`open_note` needs argument `title`", "STALE: this app is at revision …"),
            // and so does what became of a card; pass them through unedited.
            Err(sentence) => sentence,
        }
    }
}

/// Turn the app's reply into something a model cannot misread as "finished".
///
/// The reply already carries the state after the action, so there is no second `describe` here:
/// the app read it on its own UI thread in the same turn of the event loop that dispatched, which
/// is both cheaper and the only version of that read nothing could have changed underneath.
pub(crate) fn describe_outcome(app: &str, action: &str, value: &serde_json::Value) -> String {
    let result = value.get("result").unwrap_or(value);
    let result = serde_json::to_string(result).unwrap_or_else(|_| "ok".into());
    let settled = value.get("settled").and_then(|v| v.as_bool()).unwrap_or(true);

    let mut out = if settled {
        format!("{app}.{action} → {result}")
    } else {
        // The distinction the whole return type exists for. Without this sentence a model reads
        // an accepted dispatch as a completed build.
        let id = value.get("action_id").and_then(|v| v.as_str()).unwrap_or("");
        format!(
            "{app}.{action} accepted (not finished) → {result}\n\
             This action only starts the work{}. Check back before reporting a result.",
            if id.is_empty() { String::new() } else { format!("; it is {id}") }
        )
    };

    if let Some(summary) = value.get("summary").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        out.push_str(&format!("\nNow: {summary}"));
    }
    // Handed back so a follow-up action can be guarded on the state this one left behind.
    if let Some(revision) = value.get("revision").and_then(|v| v.as_str()) {
        out.push_str(&format!("\nrevision: {revision}"));
    }
    out
}


#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn a_dispatched_action_is_not_reported_as_a_finished_one() {
        // What a build looks like coming back. A model reading "notes.build → {job:83}" would
        // tell the user the build succeeded, having been told only that it started.
        let reply = serde_json::json!({
            "accepted": true,
            "action_id": "app-builder#7",
            "settled": false,
            "result": { "job": 83 },
            "summary": "Builder — compiling",
            "revision": "0123456789abcdef",
        });

        let text = describe_outcome("builder", "build", &reply);
        assert!(text.contains("not finished"), "{text}");
        assert!(text.contains("app-builder#7"), "the caller needs a name to wait on: {text}");
        assert!(text.contains("Now: Builder — compiling"));
    }

    #[test]
    fn a_finished_action_reads_plainly() {
        let reply = serde_json::json!({
            "accepted": true,
            "action_id": "app-notes#1",
            "settled": true,
            "result": { "opened": true },
            "summary": "Notes — Kernel asks",
            "revision": "fedcba9876543210",
        });

        let text = describe_outcome("notes", "open_note", &reply);
        assert!(!text.contains("not finished"), "{text}");
        assert!(text.contains("Now: Notes — Kernel asks"));
        // The revision comes back so a follow-up can be guarded on it without another read.
        assert!(text.contains("revision: fedcba9876543210"), "{text}");
    }

    #[test]
    fn the_cap_is_the_gates_ladder() {
        let view = serde_json::json!({"actions": [
            {"name": "kill_process", "permission": "dangerous"},
            {"name": "open_note", "permission": "standard"},
            {"name": "odd", "permission": "spicy"},
        ]});
        assert!(beyond_cap(&view, "x", "open_note", PermissionLevel::Standard).is_none());
        let err = beyond_cap(&view, "x", "kill_process", PermissionLevel::Standard).unwrap();
        assert!(err.starts_with("Permission denied") && err.contains("dangerous"), "{err}");
        let err = beyond_cap(&view, "x", "odd", PermissionLevel::Dangerous).unwrap();
        assert!(err.contains("not a level this OS defines"), "an unknown grade is refused, not read as sensitive: {err}");
        assert!(beyond_cap(&view, "x", "unlisted", PermissionLevel::Standard).is_none(), "no declaration is standard");
    }
}
