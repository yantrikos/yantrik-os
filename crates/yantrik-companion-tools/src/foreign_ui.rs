//! Windows we did not write.
//!
//! `app_ui.rs` reads our own apps, which publish their state because we made them. This reads
//! everyone else's, which publish their widget trees over AT-SPI because screen readers need
//! them — GTK, Qt, Chromium and Firefox all do, and have for twenty years.
//!
//! That settles the order of the three ways to know what is on screen:
//!
//!   1. `describe_app` — ours. Exact, free.
//!   2. `describe_window` — theirs, if the toolkit has an accessibility bridge. Exact, free.
//!   3. `analyze_screen` — a screenshot and a vision model. Everything else.
//!
//! I had written that rule as *semantic for ours, visual for theirs*. Having looked, it was too
//! pessimistic. Most of theirs is semantic too, and vision belongs third.
//!
//! `window_action` is the part that changes what an agent can do rather than only what it can
//! know: AT-SPI's action interface is how a screen reader presses a button, so a button in a
//! foreign application can be pressed by name — no synthetic pointer, no coordinates, and it
//! works on a window that is not on top.

use std::time::Duration;

use yantrik_ipc_transport::SyncRpcClient;

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ListWindowsReadableTool));
    reg.register(Box::new(DescribeWindowTool));
    reg.register(Box::new(WindowActionTool));
}

/// Walking a tree costs a D-Bus round trip per node, so a browser window is not instant.
const READ_TIMEOUT: Duration = Duration::from_secs(25);

fn client() -> SyncRpcClient {
    SyncRpcClient::for_service("a11y").with_timeout(READ_TIMEOUT)
}

fn call(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    client().call(method, params).map_err(|e| e.message)
}

/// The same advice in every failure: this is one of three ways to see, and the others still work.
fn unavailable(detail: &str) -> String {
    format!(
        "The accessibility service could not answer: {detail}\n\
         Yantrik's own apps are still readable with list_apps and describe_app, and \
         analyze_screen can photograph anything else."
    )
}

// ── What is open that can be read ──

pub struct ListWindowsReadableTool;

impl Tool for ListWindowsReadableTool {
    fn name(&self) -> &'static str {
        "list_readable_windows"
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
                "name": "list_readable_windows",
                "description": "List windows from OTHER applications — Firefox, LibreOffice, a \
                                terminal, anything built with GTK or Qt — that can be read \
                                without a screenshot. Use describe_window next. For Yantrik's own \
                                apps use list_apps instead.",
                "parameters": { "type": "object", "properties": {} }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let result = match call("a11y.windows", serde_json::json!({})) {
            Ok(v) => v,
            Err(e) => return unavailable(&e),
        };
        let windows = result.get("windows").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        if windows.is_empty() {
            return "No other application is publishing a readable window. \
                    Something without an accessibility bridge will not appear here; use \
                    analyze_screen to look at it instead."
                .to_string();
        }

        let lines: Vec<String> = windows
            .iter()
            .map(|w| {
                format!(
                    "{}: {} — {:?} ({})",
                    w["id"].as_str().unwrap_or("?"),
                    w["app"].as_str().unwrap_or("?"),
                    w["title"].as_str().unwrap_or(""),
                    w["role"].as_str().unwrap_or("window"),
                )
            })
            .collect();
        format!("Readable windows ({}):\n{}", lines.len(), lines.join("\n"))
    }
}

// ── What is in one ──

pub struct DescribeWindowTool;

impl Tool for DescribeWindowTool {
    fn name(&self) -> &'static str {
        "describe_window"
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
                "name": "describe_window",
                "description": "Read what is inside another application's window — its labels, \
                                fields, text and buttons — as structure rather than pixels. \
                                Cheaper and more accurate than a screenshot, and it says which \
                                elements can be acted on. Get the id from list_readable_windows.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "window": { "type": "string", "description": "Window id from list_readable_windows" }
                    },
                    "required": ["window"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let window = args["window"].as_str().unwrap_or("").trim();
        if window.is_empty() {
            return "Error: `window` is required. Call list_readable_windows first.".to_string();
        }
        match call("a11y.describe", serde_json::json!({ "window": window })) {
            // Passed through as JSON rather than prose: unlike a tool that reports what it did,
            // this is data the caller will want to pick elements out of by id.
            Ok(v) => serde_json::to_string_pretty(&v)
                .unwrap_or_else(|e| format!("Error: could not format the reply: {e}")),
            Err(e) => unavailable(&e),
        }
    }
}

// ── Doing something in one ──

pub struct WindowActionTool;

impl Tool for WindowActionTool {
    fn name(&self) -> &'static str {
        "window_action"
    }
    /// Standard, like `app_action`: it changes what the person is looking at, in software we do
    /// not control. The action names come from the application itself, so there is no fixed list
    /// to grade by risk the way our own surfaces are — which is a reason to keep the ceiling here
    /// rather than trust the name.
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
                "name": "window_action",
                "description": "Press a button, activate a field or follow a link in another \
                                application's window, by name — the same way a screen reader \
                                does. No mouse or keyboard is simulated. Call describe_window \
                                first: each element lists the actions it accepts.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "element": { "type": "string", "description": "Element id from describe_window" },
                        "action": { "type": "string", "description": "One of that element's listed actions, e.g. Click" }
                    },
                    "required": ["element", "action"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let element = args["element"].as_str().unwrap_or("").trim();
        let action = args["action"].as_str().unwrap_or("").trim();
        if element.is_empty() || action.is_empty() {
            return "Error: both `element` and `action` are required. \
                    describe_window lists the actions each element accepts."
                .to_string();
        }

        match call(
            "a11y.act",
            serde_json::json!({ "element": element, "action": action }),
        ) {
            Ok(v) => format!("Did `{}` on {element}.", v["did"].as_str().unwrap_or(action)),
            // The service's refusals already name what was wrong and list the real actions, so
            // they are passed through rather than wrapped.
            Err(e) => e,
        }
    }
}
