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

use std::time::{Duration, Instant};

use yantrik_ipc_transport::gate;
use yantrik_ipc_transport::SyncRpcClient;

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ListAppsTool));
    reg.register(Box::new(DescribeAppTool));
    reg.register(Box::new(AppActionTool));
    reg.register(Box::new(AwaitAppTool));
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

fn describe(app: &str) -> Result<serde_json::Value, String> {
    client(app, READ_TIMEOUT)
        .call("app.describe", serde_json::json!({}))
        .map_err(|e| e.message)
}

/// One action as the app publishes it in `describe`, if it publishes one by that name.
fn published<'a>(view: &'a serde_json::Value, action: &str) -> Option<&'a serde_json::Value> {
    view.get("actions")
        .and_then(|v| v.as_array())
        .and_then(|actions| {
            actions.iter().find(|a| a.get("name").and_then(|n| n.as_str()) == Some(action))
        })
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
fn beyond_cap(view: &serde_json::Value, app: &str, action: &str, cap: PermissionLevel) -> Option<String> {
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

// ── Asking the person, when the app says to ──
//
// Since the dispatch enforces the person's mode on every door (#116, #152), an app answers a
// `sensitive` act in `ask` mode with `GRANT:` unless the call carries a grant. Every other mind
// has a way through that — the MCP bridge and `yos act` put the card up, wait for the person, and
// act again with their Allow — and the built-in companion had none: its `app_action` simply
// failed where any other mind would have asked (#154, item 3). So it takes the same three steps.
// Nothing here decides anything: `request_approval` puts a card up, `approval_status` reads an
// answer somebody else gave, and the app's own dispatch spends the grant when the action runs.

/// What the card says is asking.
const REQUESTER: &str = "companion";

/// How long a card waits for a person, as `yos act` waits. The tool runs on the bridge's worker,
/// never on the thread that paints, so a wait costs this tool call and nothing else.
const APPROVAL_WAIT: Duration = Duration::from_secs(120);
const APPROVAL_POLL: Duration = Duration::from_secs(1);

/// A request to the shell is one hop to its UI thread and back.
const SHELL_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the desktop's sockets are and how long to wait for a person. The session's own in use;
/// a test hands in a scratch directory and a shorter wait. `pub(crate)` because `window.rs`
/// drives the shell's own window actions through the same desk and the same asking.
pub(crate) struct Desk {
    pub(crate) dir: Option<std::path::PathBuf>,
    pub(crate) wait: Duration,
    pub(crate) poll: Duration,
}

impl Desk {
    pub(crate) fn session() -> Desk {
        Desk { dir: None, wait: APPROVAL_WAIT, poll: APPROVAL_POLL }
    }

    fn client(&self, service: &str, timeout: Duration) -> SyncRpcClient {
        let address = match &self.dir {
            Some(dir) => dir.join(format!("{service}.sock")).to_string_lossy().to_string(),
            None => yantrik_ipc_transport::server::RpcServer::default_address(service),
        };
        SyncRpcClient::new(&address).with_timeout(timeout)
    }
}

/// `app.act`, and when the app refuses for want of a grant, the person's Allow and `app.act` again.
///
/// `Err` is always a sentence for the model to read and relay: the app's own refusal passed
/// through, or what became of the card — denied, unanswered, not needed after all.
pub(crate) fn act_asking(
    desk: &Desk,
    app: &str,
    action: &str,
    args: &serde_json::Value,
    expect: &str,
    purpose: &str,
) -> Result<serde_json::Value, String> {
    let mut call = serde_json::json!({ "action": action, "args": args });
    if !expect.is_empty() {
        call["expect_revision"] = serde_json::Value::String(expect.to_string());
    }
    let refusal = match desk.client(&format!("app-{app}"), ACT_TIMEOUT).call("app.act", call.clone()) {
        Ok(reply) => return Ok(reply),
        Err(e) if !e.message.starts_with("GRANT:") => return Err(format!("{app}.{action} failed: {}", e.message)),
        Err(e) => e.message,
    };

    // Every GRANT sentence names the grade, and the shell re-reads the published one anyway.
    let grade = refusal
        .split("graded `")
        .nth(1)
        .and_then(|rest| rest.split('`').next())
        .filter(|g| gate::grade(g).is_some())
        .unwrap_or("sensitive");
    let asked = desk
        .client("app-shell", SHELL_TIMEOUT)
        .call(
            "app.act",
            serde_json::json!({ "action": "request_approval", "args": {
                "app": app, "action": action, "grade": grade, "args_json": args,
                "purpose": purpose, "requester": REQUESTER,
            }}),
        )
        .map_err(|e| {
            format!("{app}.{action} needs the person's Allow and they could not be asked: {}", e.message)
        })?;
    let answer = asked.get("result").cloned().unwrap_or_default();
    if answer["status"] == "not_needed" {
        return Err(format!(
            "{app} refused {app}.{action} without a grant, but the desktop says its {} mode runs it \
             unasked. Nothing was run; the two should agree, so try once more in a moment. The app \
             said: {refusal}",
            answer["mode"].as_str().unwrap_or("current")
        ));
    }
    let Some(request_id) = answer["request_id"].as_str().map(str::to_string) else {
        return Err(format!(
            "the desktop was asked to put a card up for {app}.{action} and did not say which \
             request it was, so there is nothing to wait on. Nothing was run."
        ));
    };

    let deadline = Instant::now() + desk.wait;
    let mut status = "pending".to_string();
    while Instant::now() < deadline {
        std::thread::sleep(desk.poll.min(deadline.saturating_duration_since(Instant::now())));
        // A poll that fails is not an answer: the card is still up, so ask again.
        let Ok(poll) = desk.client("app-shell", SHELL_TIMEOUT).call(
            "app.act",
            serde_json::json!({ "action": "approval_status", "args": { "request_id": request_id } }),
        ) else {
            continue;
        };
        status = poll["result"]["status"].as_str().unwrap_or("pending").to_string();
        if status != "pending" {
            break;
        }
    }

    match status.as_str() {
        "granted" => {
            call["grant"] = serde_json::Value::String(request_id);
            desk.client(&format!("app-{app}"), ACT_TIMEOUT).call("app.act", call).map_err(|e| {
                format!("The person allowed {app}.{action}, but it did not go through: {}", e.message)
            })
        }
        "denied" => Err(format!(
            "The person at the machine said no to {app}.{action}. Nothing was run. That is an \
             answer, not an error: do not ask again unless they bring it up."
        )),
        "consumed" => Err(format!(
            "The grant for {app}.{action} had already been spent, so nothing was run. Ask again if \
             it still needs doing."
        )),
        _ => Err(format!(
            "Nobody answered the card for {app}.{action} within {} s, so nothing was run. They were \
             probably away from the keyboard; say what you wanted to do and try again when they \
             are back.",
            desk.wait.as_secs()
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

        // Ask the app what this action costs before doing it. `app_action` itself is Standard —
        // enough to open a note — but an app may publish something that ends a process or deletes
        // a file, and that must meet the configured cap on its own terms rather than ride in on
        // the tool's. The app's dispatch decides again when the call arrives (see `beyond_cap`);
        // the app's *state* can change in between too, which is what `expect_revision` is for.
        let mut purpose = String::new();
        if let Ok(ref view) = describe(app) {
            if let Some(refusal) = beyond_cap(view, app, action, ctx.max_permission) {
                return refusal;
            }
            // The app's own sentence for the action, for the card if one has to go up.
            purpose = published(view, action)
                .and_then(|a| a.get("description").and_then(|d| d.as_str()))
                .unwrap_or_default()
                .to_string();
        }

        match act_asking(&Desk::session(), app, action, &action_args, expect, &purpose) {
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
fn describe_outcome(app: &str, action: &str, value: &serde_json::Value) -> String {
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


// ── Waiting for something to become true ──

/// How often an outstanding wait re-reads its target.
///
/// A `describe` costs about 0.4 ms, so twenty reads a second is under one percent of one core for
/// as long as somebody is actually waiting — and nothing at all when nobody is. That is the whole
/// argument for polling our own apps instead of instrumenting them: the cost is bounded, it is
/// paid only during a wait, and there is no setter anywhere that somebody can forget to annotate.
const WAIT_TICK: Duration = Duration::from_millis(50);

/// The longest any single wait may run.
///
/// Not a guess about how long work takes — a bound on how long one tool call may hold a worker.
/// Anything slower than this wants the job board, not a wait.
const MAX_WAIT: Duration = Duration::from_secs(120);

const DEFAULT_WAIT: Duration = Duration::from_secs(10);

/// One bounded question about an app's state.
///
/// Deliberately not a script. A predicate that could run arbitrary code would be a second, worse
/// action surface — and every predicate worth having is one of these five.
#[derive(Debug, Clone, PartialEq)]
enum Until {
    /// Anything at all differs from the baseline.
    Changed,
    /// A dotted path into `state` equals this value.
    Equals { path: String, value: serde_json::Value },
    /// A dotted path into `state` is anything other than this value.
    Differs { path: String, value: serde_json::Value },
    /// The summary, or the value at a path, contains this text. Case-insensitive.
    Contains { path: String, text: String },
    /// A dotted path into `state` exists at all — how a dialog that was not there is waited for.
    Exists { path: String },
}

impl Until {
    fn parse(spec: &serde_json::Value) -> Result<Until, String> {
        let op = spec.get("op").and_then(|v| v.as_str()).unwrap_or("changed");
        let path = spec.get("path").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let value = spec.get("value").cloned();

        let needs_path = |what: &str| -> Result<String, String> {
            if path.is_empty() {
                Err(format!("`{what}` needs a `path`, e.g. \"open_note\" or \"dialog.title\""))
            } else {
                Ok(path.clone())
            }
        };
        let needs_value = |what: &str| -> Result<serde_json::Value, String> {
            value.clone().ok_or_else(|| format!("`{what}` needs a `value` to compare against"))
        };

        match op {
            "changed" => Ok(Until::Changed),
            "eq" => Ok(Until::Equals { path: needs_path("eq")?, value: needs_value("eq")? }),
            "ne" => Ok(Until::Differs { path: needs_path("ne")?, value: needs_value("ne")? }),
            "exists" => Ok(Until::Exists { path: needs_path("exists")? }),
            "contains" => {
                let text = needs_value("contains")?;
                let text = text.as_str().map(str::to_string).unwrap_or_else(|| text.to_string());
                // An empty path means the summary, which is the common case: "wait until the
                // window says Saved".
                Ok(Until::Contains { path, text })
            }
            other => Err(format!(
                "unknown `op` {other}; use changed, eq, ne, contains or exists"
            )),
        }
    }

    /// Whether this holds of `view`, given what the same app looked like before.
    fn holds(&self, view: &serde_json::Value, baseline: &serde_json::Value) -> bool {
        match self {
            Until::Changed => {
                revision_of(view) != revision_of(baseline) || revision_of(view).is_empty()
            }
            Until::Equals { path, value } => at(view, path).as_ref() == Some(value),
            Until::Differs { path, value } => at(view, path).as_ref() != Some(value),
            Until::Exists { path } => at(view, path).is_some_and(|v| !v.is_null()),
            Until::Contains { path, text } => {
                let haystack = if path.is_empty() {
                    view.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string()
                } else {
                    match at(view, path) {
                        Some(serde_json::Value::String(s)) => s,
                        Some(other) => other.to_string(),
                        None => String::new(),
                    }
                };
                haystack.to_lowercase().contains(&text.to_lowercase())
            }
        }
    }

    fn describe(&self) -> String {
        match self {
            Until::Changed => "anything changes".to_string(),
            Until::Equals { path, value } => format!("{path} becomes {value}"),
            Until::Differs { path, value } => format!("{path} stops being {value}"),
            Until::Exists { path } => format!("{path} appears"),
            Until::Contains { path, text } => {
                if path.is_empty() {
                    format!("the summary mentions {text:?}")
                } else {
                    format!("{path} mentions {text:?}")
                }
            }
        }
    }
}

fn revision_of(view: &serde_json::Value) -> String {
    view.get("revision").and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// A dotted path into a view's `state`.
fn at(view: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    let mut here = view.get("state")?;
    for segment in path.split('.').filter(|s| !s.is_empty()) {
        here = here.get(segment)?;
    }
    Some(here.clone())
}

pub struct AwaitAppTool;

impl Tool for AwaitAppTool {
    fn name(&self) -> &'static str {
        "await_app"
    }
    /// Standard rather than Safe, because `then_act` really does act.
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
                "name": "await_app",
                "description": "Wait until a Yantrik app reaches some state — a dialog appears, a \
                                save finishes, a list stops being empty. Give `then_act` when you \
                                are waiting for the result of an action: the app is read *before* \
                                the action runs, so a result that arrives immediately is not \
                                missed. Use this instead of acting and then describing, which \
                                cannot tell 'already true' from 'just became true'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string", "description": "App id, e.g. notes" },
                        "until": {
                            "type": "object",
                            "description": "What to wait for. {op: changed} — anything differs. \
                                            {op: eq|ne, path, value} — a field in the app's state \
                                            reaches (or leaves) a value. {op: exists, path} — a \
                                            field appears. {op: contains, text} — the summary \
                                            mentions text, or a field does if you give a path. \
                                            Paths are dotted, into the `state` from describe_app.",
                            "properties": {
                                "op": { "type": "string" },
                                "path": { "type": "string" },
                                "value": {},
                                "text": { "type": "string" }
                            }
                        },
                        "then_act": {
                            "type": "object",
                            "description": "An action to run once the baseline read is taken: \
                                            {action, args}. Optional.",
                            "properties": {
                                "action": { "type": "string" },
                                "args": { "type": "object" }
                            }
                        },
                        "timeout_ms": {
                            "type": "integer",
                            "description": "How long to wait. Default 10000, maximum 120000."
                        }
                    },
                    "required": ["app", "until"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let app = args["app"].as_str().unwrap_or("").trim();
        if app.is_empty() {
            return "Error: `app` is required. Call list_apps to see which are open.".to_string();
        }
        let until = match Until::parse(args.get("until").unwrap_or(&serde_json::json!({}))) {
            Ok(u) => u,
            Err(e) => return format!("Error: {e}"),
        };
        let budget = args
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_WAIT)
            .min(MAX_WAIT);

        // 1. Baseline, before anything else happens.
        //
        // The ordering is the whole point. Acting first and reading afterwards cannot distinguish
        // "the dialog opened" from "the dialog was already open", and a wait installed after a
        // click misses the result that arrived during the round trip.
        let baseline = match describe(app) {
            Ok(v) => v,
            Err(e) => return format!("'{app}' did not answer: {}", e),
        };

        // 2. Act, if this is a wait for the consequences of an action. Guarded on the baseline we
        //    just took, so an app that moved between the read and the dispatch refuses rather
        //    than acting on a state nobody decided about.
        let mut dispatched = String::new();
        if let Some(act) = args.get("then_act").filter(|v| v.is_object()) {
            let action = act.get("action").and_then(|v| v.as_str()).unwrap_or("").trim();
            if action.is_empty() {
                return "Error: `then_act` needs an `action`.".to_string();
            }
            if let Some(refusal) = beyond_cap(&baseline, app, action, ctx.max_permission) {
                return refusal;
            }
            let call = serde_json::json!({
                "action": action,
                "args": act.get("args").cloned().unwrap_or(serde_json::json!({})),
                "expect_revision": revision_of(&baseline),
            });
            match client(app, ACT_TIMEOUT).call("app.act", call) {
                Ok(reply) => dispatched = describe_outcome(app, action, &reply),
                Err(e) => return format!("{app}.{action} failed: {}", e.message),
            }
        }

        // 3. Reconcile, then await. The first check happens immediately: for an action that
        //    settled on return, the condition is already true and waiting 50 ms to notice would
        //    be 50 ms of nothing.
        let started = std::time::Instant::now();
        let mut polls = 0u32;
        let mut latest = baseline.clone();
        loop {
            if let Ok(view) = describe(app) {
                latest = view;
                polls += 1;
                if until.holds(&latest, &baseline) {
                    return held(app, &until, &latest, &dispatched, started.elapsed(), polls);
                }
            }
            if started.elapsed() >= budget {
                return timed_out(app, &until, &latest, &dispatched, budget, polls);
            }
            std::thread::sleep(WAIT_TICK);
        }
    }
}

fn held(
    app: &str,
    until: &Until,
    view: &serde_json::Value,
    dispatched: &str,
    elapsed: Duration,
    polls: u32,
) -> String {
    let summary = view.get("summary").and_then(|v| v.as_str()).unwrap_or("");
    let mut out = String::new();
    if !dispatched.is_empty() {
        out.push_str(dispatched);
        out.push('\n');
    }
    out.push_str(&format!(
        "Waited for {app}: {} — held after {} ms ({polls} reads).\nNow: {summary}\nrevision: {}",
        until.describe(),
        elapsed.as_millis(),
        revision_of(view),
    ));
    out
}

fn timed_out(
    app: &str,
    until: &Until,
    view: &serde_json::Value,
    dispatched: &str,
    budget: Duration,
    polls: u32,
) -> String {
    let summary = view.get("summary").and_then(|v| v.as_str()).unwrap_or("");
    let mut out = String::new();
    if !dispatched.is_empty() {
        out.push_str(dispatched);
        out.push('\n');
    }
    out.push_str(&format!(
        "Waited for {app}: {} — not observed in {} ms ({polls} reads).\nNow: {summary}\n\
         This is a read every {} ms, so a state that appeared and vanished in between was never \
         visible to it. Not observed is not the same as did not happen.",
        until.describe(),
        budget.as_millis(),
        WAIT_TICK.as_millis(),
    ));
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

    // ── The wait ──

    fn view(summary: &str, state: serde_json::Value, revision: &str) -> serde_json::Value {
        serde_json::json!({ "app": "notes", "summary": summary, "state": state, "revision": revision })
    }

    #[test]
    fn a_predicate_is_one_of_five_things_and_never_a_script() {
        assert_eq!(Until::parse(&serde_json::json!({ "op": "changed" })).unwrap(), Until::Changed);
        assert!(Until::parse(&serde_json::json!({ "op": "eval", "code": "1" })).is_err());
        // A comparison with nothing to compare against is a mistake worth naming rather than
        // silently treating as "changed".
        let err = Until::parse(&serde_json::json!({ "op": "eq", "path": "x" })).unwrap_err();
        assert!(err.contains("value"), "{err}");
        let err = Until::parse(&serde_json::json!({ "op": "exists" })).unwrap_err();
        assert!(err.contains("path"), "{err}");
    }

    #[test]
    fn waiting_for_a_dialog_that_was_not_there() {
        // The canonical reflex. `exists` is what distinguishes "the dialog opened" from "some
        // field happens to be null".
        let before = view("Notes", serde_json::json!({ "open_note": "x" }), "aaaa");
        let after = view(
            "Notes — Save changes?",
            serde_json::json!({ "open_note": "x", "dialog": { "title": "Save changes?" } }),
            "bbbb",
        );
        let until = Until::parse(&serde_json::json!({ "op": "exists", "path": "dialog" })).unwrap();

        assert!(!until.holds(&before, &before));
        assert!(until.holds(&after, &before));
    }

    #[test]
    fn a_dotted_path_reaches_into_nested_state() {
        let now = view("Notes", serde_json::json!({ "dialog": { "title": "Save changes?" } }), "b");
        let until = Until::parse(
            &serde_json::json!({ "op": "eq", "path": "dialog.title", "value": "Save changes?" }),
        )
        .unwrap();
        assert!(until.holds(&now, &now));

        // And a path through something that is not an object is absent, not a panic.
        let flat = view("Notes", serde_json::json!({ "dialog": 3 }), "c");
        assert!(!until.holds(&flat, &flat));
    }

    #[test]
    fn changed_compares_against_the_baseline_not_against_nothing() {
        // Why the baseline is taken before the action rather than after: without it, "changed"
        // has nothing to be different from, and every wait would return immediately.
        let before = view("Notes — a", serde_json::json!({ "n": 1 }), "1111");
        let same = view("Notes — a", serde_json::json!({ "n": 1 }), "1111");
        let moved = view("Notes — b", serde_json::json!({ "n": 2 }), "2222");

        assert!(!Until::Changed.holds(&same, &before), "an unchanged app has not changed");
        assert!(Until::Changed.holds(&moved, &before));
    }

    #[test]
    fn contains_reads_the_summary_when_no_path_is_given() {
        let now = view("Notes — Saved", serde_json::json!({}), "z");
        let until = Until::parse(&serde_json::json!({ "op": "contains", "value": "saved" })).unwrap();
        assert!(until.holds(&now, &now), "matching should not depend on capitalisation");

        let until = Until::parse(&serde_json::json!({ "op": "contains", "value": "error" })).unwrap();
        assert!(!until.holds(&now, &now));
    }

    // ── Asking the person (#154, item 3) ──

    /// One fake socket: answers each JSON-RPC line with whatever `reply` makes of the request,
    /// and records every request. `reply` returns the whole response object minus `jsonrpc`/`id`.
    /// `pub(crate)` because `window.rs` tests the same asking through the same kind of fake.
    #[cfg(unix)]
    pub(crate) fn fake(
        path: std::path::PathBuf,
        reply: impl Fn(&serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>> {
        use std::io::{BufRead, BufReader, Write};
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = heard.clone();
        let reply = std::sync::Arc::new(reply);
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind a fake socket");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let (log, reply) = (log.clone(), reply.clone());
                std::thread::spawn(move || {
                    let mut writer = stream.try_clone().unwrap();
                    for line in BufReader::new(stream).lines() {
                        let Ok(line) = line else { return };
                        let asked: serde_json::Value = serde_json::from_str(&line).unwrap();
                        log.lock().unwrap().push(asked["params"].clone());
                        let mut answer = reply(&asked["params"]);
                        answer["jsonrpc"] = "2.0".into();
                        answer["id"] = asked["id"].clone();
                        if writer.write_all(format!("{answer}\n").as_bytes()).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        heard
    }

    const GRANT_REFUSAL: &str = "GRANT: notes.purge is graded `sensitive` and this machine is in \
        ask mode, which runs nothing above `standard` without asking — so it was not run.";

    /// A desk in a scratch directory: an app that refuses without a grant and runs with
    /// `appr-1`, and a shell whose card is answered `answer` on the second poll.
    #[cfg(unix)]
    fn desk_with(
        tag: &str,
        answer: &'static str,
    ) -> (Desk, std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>, std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>) {
        let dir = std::env::temp_dir().join(format!("yantrik-companion-ask-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let app = fake(dir.join("app-notes.sock"), |params| {
            if params["action"] != "purge" {
                return serde_json::json!({"error": {"code": -32602, "message": "`nope` needs argument `id`"}});
            }
            match params["grant"].as_str() {
                None => serde_json::json!({"error": {"code": -32602, "message": GRANT_REFUSAL}}),
                Some("appr-1") => serde_json::json!({"result": {
                    "app": "notes", "action_id": "app-notes#2", "accepted": true, "settled": true,
                    "result": {"purged": 3}, "revision": "0123456789abcdef", "summary": "Notes — empty", "state": {}
                }}),
                Some(other) => serde_json::json!({"error": {"code": -32602, "message": format!("GRANT: `{other}` does not authorise notes.purge")}}),
            }
        });
        let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let shell = fake(dir.join("app-shell.sock"), move |params| match params["action"].as_str() {
            Some("request_approval") => serde_json::json!({"result": {"result": {"request_id": "appr-1", "status": "pending"}}}),
            Some("approval_status") => {
                let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let status = if n == 0 { "pending" } else { answer };
                serde_json::json!({"result": {"result": {"request_id": "appr-1", "status": status}}})
            }
            _ => serde_json::json!({"error": {"code": -32602, "message": "unknown action"}}),
        });
        let desk = Desk { dir: Some(dir), wait: Duration::from_millis(600), poll: Duration::from_millis(20) };
        (desk, app, shell)
    }

    #[cfg(unix)]
    #[test]
    fn a_grant_refusal_asks_the_person_and_acts_again_with_their_allow() {
        let (desk, app, shell) = desk_with("granted", "granted");
        let args = serde_json::json!({"folder": "Old"});
        let reply = act_asking(&desk, "notes", "purge", &args, "", "Purge a folder").expect("allowed, then ran");
        assert_eq!(reply["result"]["purged"], 3);

        let asked = shell.lock().unwrap().clone();
        assert_eq!(asked[0]["action"], "request_approval");
        assert_eq!(
            asked[0]["args"],
            serde_json::json!({"app": "notes", "action": "purge", "grade": "sensitive",
                               "args_json": {"folder": "Old"}, "purpose": "Purge a folder",
                               "requester": "companion"}),
            "the card is bound to the exact arguments the act carries"
        );
        assert!(asked[1..].iter().all(|p| p["action"] == "approval_status"));

        let acts = app.lock().unwrap().clone();
        assert_eq!(acts.len(), 2, "once refused, once with the grant: {acts:?}");
        assert!(acts[0].get("grant").is_none());
        assert_eq!(acts[1]["grant"], "appr-1");
        assert_eq!(acts[0]["args"], acts[1]["args"], "the same arguments both times");
    }

    #[cfg(unix)]
    #[test]
    fn a_denied_or_unanswered_card_runs_nothing_and_says_so() {
        let (desk, app, _) = desk_with("denied", "denied");
        let err = act_asking(&desk, "notes", "purge", &serde_json::json!({}), "", "").unwrap_err();
        assert!(err.contains("said no") && err.contains("Nothing was run"), "{err}");
        assert_eq!(app.lock().unwrap().len(), 1, "only the refused call reached the app");

        let (desk, app, _) = desk_with("silent", "pending");
        let err = act_asking(&desk, "notes", "purge", &serde_json::json!({}), "", "").unwrap_err();
        assert!(err.contains("Nobody answered"), "{err}");
        assert_eq!(app.lock().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn any_other_refusal_passes_through_and_nobody_is_asked() {
        let (desk, _, shell) = desk_with("other", "granted");
        let err = act_asking(&desk, "notes", "nope", &serde_json::json!({}), "", "").unwrap_err();
        assert_eq!(err, "notes.nope failed: `nope` needs argument `id`");
        assert!(shell.lock().unwrap().is_empty(), "no card for a refusal a card cannot answer");
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

    #[test]
    fn a_wait_that_ran_out_says_what_it_could_not_have_seen() {
        // The polling contract, stated where somebody will read it. A model told only "not found"
        // will report that the dialog never appeared.
        let now = view("Notes", serde_json::json!({}), "q");
        let text = timed_out(
            "notes",
            &Until::Exists { path: "dialog".into() },
            &now,
            "",
            Duration::from_millis(4000),
            80,
        );
        assert!(text.contains("Not observed is not the same as did not happen"), "{text}");
        assert!(text.contains("4000 ms"));
    }
}
