//! `app.describe` / `app.act` — how an app tells the mind what it holds, and takes instruction.
//!
//! # Why this exists
//!
//! The companion could already see the desktop, in the only way it had: `grim` takes a
//! screenshot, the PNG is base64'd and posted to a vision model, and the model reports what the
//! pixels look like. That is the right answer for a foreign application — we did not write
//! Firefox and it owes us no account of itself. It is the wrong answer for our own software.
//! Every one of the sixteen apps under `apps/` already knows exactly which note is open, which
//! track is playing, which message is selected; asking a vision model to *infer* that from a
//! photograph of our own window is expensive, slow, lossy, and needs a GPU we do not always have.
//!
//! So an app publishes its state instead. Two methods on the socket bus every app already
//! speaks:
//!
//! ```text
//! app.describe {}                  → { app, summary, state, actions: [...] }
//! app.act      { action, args }    → { result }
//! ```
//!
//! `describe` is a few hundred bytes of exact truth, always current. `act` is the same surface
//! turned around: the actions an app already exposes to its own buttons, offered to the mind by
//! name — so driving our own software never needs a synthetic mouse click either.
//!
//! The rule this establishes: **semantic for ours, visual for theirs.**
//!
//! # Threading
//!
//! Both closures run on the UI thread, because that is the only thread allowed to touch a Slint
//! window. The RPC server runs on its own thread and hands work across with
//! [`slint::invoke_from_event_loop`], then blocks on a channel for the answer. That is a
//! deliberate trade: a `describe` that reads a handful of properties costs microseconds, and
//! taking the answer from the live UI is the whole point — a cached copy would be exactly the
//! stale second-hand account this module exists to replace.
//!
//! An `act` that would take real time must still not run inline; do what the app's own button
//! does and hand off to a worker.
//!
//! # Usage
//!
//! ```rust,ignore
//! use yantrik_app_runtime::control::{self, Action, Param, View};
//!
//! control::App::new("notes")
//!     .describe({
//!         let ui = app.as_weak();
//!         move || {
//!             let ui = ui.unwrap();
//!             View::new(format!("Notes — {}", ui.get_current_title()))
//!                 .with("open_note", ui.get_current_title().to_string())
//!                 .with("unsaved", ui.get_is_modified())
//!         }
//!     })
//!     .action(
//!         Action::new("open_note", "Open a note by title").arg(Param::text("title")),
//!         move |args| { /* … */ Ok(serde_json::json!({ "opened": true })) },
//!     )
//!     .serve();
//! ```

use std::cell::RefCell;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use yantrik_ipc_contracts::email::ServiceError;
use yantrik_ipc_transport::server::{RpcServer, ServiceHandler};

/// How long the RPC thread waits for the UI thread to answer.
///
/// A `describe` reading Slint properties is effectively instant. This budget exists for the case
/// where the UI thread is genuinely stuck — a modal, a long paint, a blocking call someone should
/// not have made — and the caller deserves a timeout it can report rather than a hang.
const UI_ROUNDTRIP: Duration = Duration::from_secs(3);

/// Service ids are prefixed so an app cannot collide with the service of the same name.
///
/// `notes` is already taken by notes-service, which stores notes; `app-notes` is the window a
/// person is looking at. They are different things and must not share a socket.
pub fn service_id_for(app_id: &str) -> String {
    format!("app-{app_id}")
}

// ── What an app reports ─────────────────────────────────────────────

/// One app's account of itself.
pub struct View {
    /// One line a person could read: `Notes — editing "Kernel asks", 412 words, unsaved`.
    ///
    /// Present so a caller surveying every open window pays one line per app instead of parsing
    /// sixteen state objects.
    pub summary: String,
    /// The structured view-model. An object; keys are the app's own vocabulary.
    pub state: serde_json::Value,
}

impl View {
    pub fn new(summary: impl Into<String>) -> Self {
        Self { summary: summary.into(), state: serde_json::json!({}) }
    }

    /// Add one field to the state object.
    pub fn with(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        if let Some(map) = self.state.as_object_mut() {
            map.insert(key.to_string(), value.into());
        }
        self
    }

    /// Replace the whole state object at once, for an app that builds it elsewhere.
    pub fn state(mut self, state: serde_json::Value) -> Self {
        self.state = state;
        self
    }
}

// ── What an app accepts ─────────────────────────────────────────────

/// One argument of an action.
#[derive(Clone)]
pub struct Param {
    pub name: String,
    /// JSON Schema primitive: `string`, `number`, `integer`, `boolean`.
    pub kind: &'static str,
    pub required: bool,
    pub description: String,
}

impl Param {
    pub fn text(name: &str) -> Self {
        Self { name: name.into(), kind: "string", required: true, description: String::new() }
    }
    pub fn number(name: &str) -> Self {
        Self { name: name.into(), kind: "number", required: true, description: String::new() }
    }
    pub fn flag(name: &str) -> Self {
        Self { name: name.into(), kind: "boolean", required: true, description: String::new() }
    }
    /// Mark this argument optional. The handler must cope with it being absent.
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }
    pub fn describe(mut self, description: &str) -> Self {
        self.description = description.into();
        self
    }
}

/// One thing an app can be asked to do.
#[derive(Clone)]
pub struct Action {
    pub name: String,
    pub description: String,
    pub params: Vec<Param>,
    /// How much damage this can do, in the companion's vocabulary:
    /// `safe`, `standard`, `sensitive`, `dangerous`.
    ///
    /// Declared per action rather than per surface because apps do not have one risk level:
    /// reading which note is open and killing a process arrive through the same door. The
    /// caller compares this against its own ceiling; the app states the fact.
    pub permission: &'static str,
}

impl Action {
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            params: Vec::new(),
            // Steering someone's window is not free, so the floor is `standard`, not `safe`.
            permission: "standard",
        }
    }

    pub fn arg(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    /// Declare this action riskier (or safer) than the default `standard`.
    ///
    /// Use `dangerous` for anything that destroys work or state a person cannot get back:
    /// killing a process, deleting a file, sending mail.
    pub fn risk(mut self, permission: &'static str) -> Self {
        self.permission = permission;
        self
    }

    /// The action as JSON Schema, so a caller can hand it to a model unmodified.
    fn schema(&self) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for p in &self.params {
            properties.insert(
                p.name.clone(),
                serde_json::json!({ "type": p.kind, "description": p.description }),
            );
            if p.required {
                required.push(serde_json::Value::String(p.name.clone()));
            }
        }
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "permission": self.permission,
            "parameters": {
                "type": "object",
                "properties": serde_json::Value::Object(properties),
                "required": required,
            }
        })
    }
}

// ── The registry, which lives on the UI thread ──────────────────────

type DescribeFn = Box<dyn Fn() -> View>;
type ActFn = Box<dyn Fn(&serde_json::Value) -> Result<serde_json::Value, String>>;

struct Registry {
    app_id: String,
    describe: Option<DescribeFn>,
    actions: Vec<(Action, ActFn)>,
}

impl Registry {
    fn describe(&self) -> serde_json::Value {
        let view = match &self.describe {
            Some(f) => f(),
            None => View::new(format!("{} (no description published)", self.app_id)),
        };
        serde_json::json!({
            "app": self.app_id,
            "summary": view.summary,
            "state": view.state,
            "actions": self.actions.iter().map(|(a, _)| a.schema()).collect::<Vec<_>>(),
        })
    }

    fn act(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        let Some((spec, run)) = self.actions.iter().find(|(a, _)| a.name == name) else {
            let known: Vec<&str> = self.actions.iter().map(|(a, _)| a.name.as_str()).collect();
            return Err(format!("unknown action `{name}`; this app offers: {}", known.join(", ")));
        };
        // Checked here rather than in every handler: a missing argument is the most common way a
        // model gets a call wrong, and the error should name the argument, not panic in the app.
        for p in spec.params.iter().filter(|p| p.required) {
            if args.get(&p.name).is_none() {
                return Err(format!("`{name}` needs argument `{}`", p.name));
            }
        }
        run(args)
    }
}

thread_local! {
    /// Installed by [`App::serve`] on the thread that owns the window.
    static REGISTRY: RefCell<Option<Registry>> = const { RefCell::new(None) };
}

/// Ask the UI thread to run `job` and wait for its answer.
///
/// Returns `Err` when the event loop is not running or is too busy to answer inside
/// [`UI_ROUNDTRIP`] — both of which the caller should see as an error rather than a hang.
fn on_ui_thread<T, F>(job: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Registry) -> T + Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel::<Result<T, String>>(1);
    slint::invoke_from_event_loop(move || {
        let answer = REGISTRY.with(|cell| match cell.borrow().as_ref() {
            Some(reg) => Ok(job(reg)),
            None => Err("this app published no control surface".to_string()),
        });
        // The receiver is gone only if we already timed out; dropping the answer is correct.
        let _ = tx.send(answer);
    })
    .map_err(|e| format!("app is not accepting requests: {e}"))?;

    rx.recv_timeout(UI_ROUNDTRIP)
        .map_err(|_| format!("app did not answer within {}s", UI_ROUNDTRIP.as_secs()))?
}

// ── The RPC surface ─────────────────────────────────────────────────

struct ControlRpc {
    service_id: String,
}

impl ServiceHandler for ControlRpc {
    fn service_id(&self) -> &str {
        &self.service_id
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "app.describe" => on_ui_thread(|reg| reg.describe())
                .map_err(|m| ServiceError { code: -32000, message: m }),

            "app.act" => {
                let action = params
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if action.is_empty() {
                    return Err(ServiceError {
                        code: -32602,
                        message: "act needs a non-empty `action`".into(),
                    });
                }
                let args = params.get("args").cloned().unwrap_or(serde_json::json!({}));

                tracing::info!(action = %action, "app.act");
                let outcome = on_ui_thread(move |reg| reg.act(&action, &args))
                    .map_err(|m| ServiceError { code: -32000, message: m })?;

                match outcome {
                    Ok(result) => Ok(serde_json::json!({ "result": result })),
                    // An action that legitimately refuses is an application error, not a
                    // transport failure: -32602 keeps it out of the client's circuit breaker.
                    Err(message) => Err(ServiceError { code: -32602, message }),
                }
            }

            other => Err(ServiceError {
                code: -32601,
                message: format!("unknown method `{other}`; this app serves app.describe, app.act"),
            }),
        }
    }
}

// ── Building one ────────────────────────────────────────────────────

/// An app's control surface, under construction.
///
/// Build it on the UI thread and finish with [`App::serve`].
pub struct App {
    registry: Registry,
}

impl App {
    pub fn new(app_id: &str) -> Self {
        Self { registry: Registry { app_id: app_id.into(), describe: None, actions: Vec::new() } }
    }

    /// What this app reports when asked. Runs on the UI thread; keep it cheap.
    pub fn describe(mut self, f: impl Fn() -> View + 'static) -> Self {
        self.registry.describe = Some(Box::new(f));
        self
    }

    /// One thing this app can be asked to do. Runs on the UI thread.
    pub fn action(
        mut self,
        spec: Action,
        f: impl Fn(&serde_json::Value) -> Result<serde_json::Value, String> + 'static,
    ) -> Self {
        self.registry.actions.push((spec, Box::new(f)));
        self
    }

    /// Publish this surface on the socket bus.
    ///
    /// Must be called from the thread that owns the window, before `run()`. Failing to serve is
    /// not fatal: an app whose socket cannot be bound is still a working app, it is only invisible
    /// to the mind, and taking the window down over that would be the worse outcome.
    pub fn serve(self) {
        let app_id = self.registry.app_id.clone();
        let action_count = self.registry.actions.len();
        REGISTRY.with(|cell| *cell.borrow_mut() = Some(self.registry));

        let service_id = service_id_for(&app_id);
        std::thread::Builder::new()
            .name(format!("{service_id}-rpc"))
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::warn!(error = %e, "No runtime; app is not describable");
                        return;
                    }
                };
                runtime.block_on(async {
                    let address = RpcServer::default_address(&service_id);
                    tracing::info!(
                        address = %address,
                        actions = action_count,
                        "Control surface listening (app.describe / app.act)"
                    );
                    if let Err(e) = RpcServer::new(&address)
                        .serve(Arc::new(ControlRpc { service_id: service_id.clone() }))
                        .await
                    {
                        tracing::warn!(error = %e, "Control surface stopped");
                    }
                });
            })
            .ok();
    }
}

// ── Finding the others ──────────────────────────────────────────────

/// The app ids that currently have a control socket in this session.
///
/// A socket file outlives a crashed process, so this is a list of candidates, not of live apps —
/// callers should treat a failed `app.describe` as "gone" rather than as an error worth reporting.
#[cfg(unix)]
pub fn running_apps() -> Vec<String> {
    let dir = yantrik_ipc_transport::server::socket_dir();
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| {
            name.strip_suffix(".sock")
                .and_then(|stem| stem.strip_prefix("app-"))
                .map(|id| id.to_string())
        })
        .collect();
    ids.sort();
    ids
}

#[cfg(not(unix))]
pub fn running_apps() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_ids_do_not_collide_with_services() {
        // notes-service owns `notes`; the Notes window must not bind the same socket.
        assert_eq!(service_id_for("notes"), "app-notes");
        assert_ne!(service_id_for("notes"), "notes");
    }

    #[test]
    fn a_view_builds_an_object() {
        let v = View::new("Notes — 3 open").with("count", 3).with("unsaved", true);
        assert_eq!(v.summary, "Notes — 3 open");
        assert_eq!(v.state["count"], 3);
        assert_eq!(v.state["unsaved"], true);
    }

    #[test]
    fn an_action_becomes_json_schema() {
        let schema = Action::new("open_note", "Open a note by title")
            .arg(Param::text("title").describe("The note's title"))
            .arg(Param::flag("focus").optional())
            .schema();

        assert_eq!(schema["name"], "open_note");
        // Unstated risk is `standard`: steering someone's window is never free.
        assert_eq!(schema["permission"], "standard");
        assert_eq!(schema["parameters"]["properties"]["title"]["type"], "string");
        assert_eq!(schema["parameters"]["properties"]["focus"]["type"], "boolean");
        // Only the required argument is listed as required.
        assert_eq!(schema["parameters"]["required"], serde_json::json!(["title"]));
    }

    #[test]
    fn an_action_can_declare_itself_dangerous() {
        let schema = Action::new("kill_process", "End a process").risk("dangerous").schema();
        assert_eq!(schema["permission"], "dangerous");
    }

    #[test]
    fn a_missing_argument_is_named_not_guessed() {
        let reg = Registry {
            app_id: "notes".into(),
            describe: None,
            actions: vec![(
                Action::new("open_note", "Open a note").arg(Param::text("title")),
                Box::new(|_| Ok(serde_json::json!("never reached"))),
            )],
        };

        let err = reg.act("open_note", &serde_json::json!({})).unwrap_err();
        assert!(err.contains("title"), "the error must name the missing argument: {err}");
    }

    #[test]
    fn an_unknown_action_lists_the_real_ones() {
        let reg = Registry {
            app_id: "notes".into(),
            describe: None,
            actions: vec![(
                Action::new("open_note", "Open a note"),
                Box::new(|_| Ok(serde_json::Value::Null)),
            )],
        };

        let err = reg.act("nope", &serde_json::json!({})).unwrap_err();
        assert!(err.contains("open_note"), "a wrong guess should be correctable: {err}");
    }

    #[test]
    fn describe_falls_back_when_the_app_published_nothing() {
        let reg = Registry { app_id: "notes".into(), describe: None, actions: Vec::new() };
        let out = reg.describe();
        assert_eq!(out["app"], "notes");
        assert_eq!(out["actions"], serde_json::json!([]));
    }
}
