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
use crate::app_reach::{self, published};

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
#[derive(Clone)]
pub(crate) struct Desk {
    pub(crate) dir: Option<std::path::PathBuf>,
    pub(crate) wait: Duration,
    pub(crate) poll: Duration,
}

impl Desk {
    pub(crate) fn session() -> Desk {
        Desk { dir: None, wait: APPROVAL_WAIT, poll: APPROVAL_POLL }
    }

    /// One call to the shell, which the approvals module lives in.
    pub(crate) fn shell_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, yantrik_ipc_transport::protocol::RpcError> {
        self.client("app-shell", SHELL_TIMEOUT).call(method, params)
    }

    fn client(&self, service: &str, timeout: Duration) -> SyncRpcClient {
        let address = match &self.dir {
            Some(dir) => dir.join(format!("{service}.sock")).to_string_lossy().to_string(),
            None => yantrik_ipc_transport::server::RpcServer::default_address(service),
        };
        SyncRpcClient::new(&address).with_timeout(timeout)
    }
}

/// Put the card for `app.action` in front of the person: the shell's `request_approval`. The
/// answer is the shell's own (`request_id` and `pending`, or `not_needed` where the mode runs the
/// action unasked); an `Err` is a sentence for the model, and nothing was raised.
pub(crate) fn raise_card(
    desk: &Desk,
    app: &str,
    action: &str,
    grade: &str,
    args: &serde_json::Value,
    purpose: &str,
) -> Result<serde_json::Value, String> {
    desk.client("app-shell", SHELL_TIMEOUT)
        .call(
            "app.act",
            serde_json::json!({ "action": "request_approval", "args": {
                "app": app, "action": action, "grade": grade, "args_json": args,
                "purpose": purpose, "requester": REQUESTER,
            }}),
        )
        .map_err(|e| {
            format!("{app}.{action} needs the person's Allow and they could not be asked: {}", e.message)
        })
}

/// Wait up to the desk's patience for the person's answer to card `request_id`: `granted`,
/// `denied`, `consumed`, or `pending` when nobody answered in time. A poll that fails is not an
/// answer; the card is still up, so it asks again.
pub(crate) fn await_answer(desk: &Desk, request_id: &str) -> String {
    let deadline = Instant::now() + desk.wait;
    let mut status = "pending".to_string();
    while Instant::now() < deadline {
        std::thread::sleep(desk.poll.min(deadline.saturating_duration_since(Instant::now())));
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
    status
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
    let asked = raise_card(desk, app, action, grade, args, purpose)?;
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

    let status = await_answer(desk, &request_id);

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

        // Ask the app what this action costs before doing it. `app_action` itself is Standard —
        // enough to open a note — but an app may publish something that ends a process or deletes
        // a file, and that must meet the configured cap on its own terms rather than ride in on
        // the tool's. The app's dispatch decides again when the call arrives (see `beyond_cap`);
        // the app's *state* can change in between too, which is what `expect_revision` is for.
        // A Yantrik app that is not running is started with no window first, and the act waits for
        // it; one that cannot run in time is dropped, never queued (#978 step 1).
        // What the call names says whether it reaches out (a browser app, a person-only action), so
        // that is settled before anything is started for it.
        if let Some(refusal) = app_reach::refused_before_start(args) {
            return refusal;
        }
        if let Err(said) = crate::app_start::ensure(app, true) {
            return said;
        }
        let described = describe(app);
        if let Some(refusal) = app_reach::refused_unattended(app, action, described.as_ref().ok()) {
            return refusal;
        }
        let mut purpose = String::new();
        if let Ok(ref view) = described {
            if let Some(refusal) = beyond_cap(view, app, action, ctx.max_permission) {
                return refusal;
            }
            // The app's own sentence for the action, for the card if one has to go up.
            purpose = app_reach::purpose_of(view, action);
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
}
