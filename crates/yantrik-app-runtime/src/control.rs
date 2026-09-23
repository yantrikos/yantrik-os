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
//! app.describe {}                                  → { app, summary, state, revision, actions }
//! app.act      { action, args, expect_revision? }  → { accepted, action_id, settled, result,
//!                                                      revision, summary, state }
//! ```
//!
//! `describe` is a few hundred bytes of exact truth, always current. `act` is the same surface
//! turned around: the actions an app already exposes to its own buttons, offered to the mind by
//! name — so driving our own software never needs a synthetic mouse click either.
//!
//! The rule this establishes: **semantic for ours, visual for theirs.**
//!
//! # Accepted is not done
//!
//! `act` never answers with a bare success, because there is no honest way to read one. Three
//! different things could be meant by "it worked":
//!
//! 1. **accepted** — the guard passed and the handler ran;
//! 2. **state changed** — the app now reports the intended result;
//! 3. **presented** — that result reached a frame someone could see.
//!
//! An action that opens a note settles all three before the handler returns. An action that
//! starts a build settles only the first: the compiler does not exist yet. A caller that cannot
//! tell those apart will report a build as finished the instant it was started — so the response
//! says `accepted`, carries `settled`, and an action that merely schedules work declares itself
//! with [`Action::defers`] rather than leaving the caller to guess.
//!
//! # Compare and act, in one turn of the event loop
//!
//! `expect_revision` is the other half. A caller that reads state, decides, and then acts has a
//! gap in between in which the person at the keyboard can type, close the document or switch
//! windows — and the action lands on a world that no longer matches the reason for it. So the
//! comparison happens *inside* the same closure as the dispatch, on the UI thread, which is the
//! app's own serialization domain: between the check and the handler, nothing else can run.
//!
//! A revision from an earlier `describe` is a hint about whether to bother. `expect_revision` is
//! the guard. Only the guard is atomic, and a caller that compares revisions itself and then
//! calls `act` has rebuilt exactly the race this removes.
//!
//! # The ceiling
//!
//! Every action carries a grade — `safe < standard < sensitive < dangerous` — and the machine has
//! a ceiling for programmatic callers, `tool_permission` in the shell's `settings.yaml`. The
//! comparison used to happen only in the MCP bridge, which meant the OS's one real boundary was
//! enforced by one of its callers: `os_act` refused a `dangerous` action while `yos act` — and
//! every mind's own shell tool, and anything else that could open the socket — ran the same
//! action untouched. The check lives here now, in the dispatch every `app.act` crosses regardless
//! of who sent it, and the bridge keeps its copy as defence in depth and for the better message.
//!
//! A person at the keyboard is deliberately not a "programmatic caller". The shell's own buttons
//! invoke the same callbacks the action handlers invoke, but they never pass through this module —
//! there is no socket, no `app.act`, no dispatch. The ceiling binds the door minds come in by,
//! not the window the person is sitting at, and no caller-identity scheme is needed to say so,
//! because the two paths do not meet. (Who exactly *is* on the socket is issue #43; until then
//! every socket caller gets the one machine-wide ceiling.)
//!
//! # The mode, and the grant
//!
//! Under the ceiling the person has a *mode* — plan, ask, auto or bypass — that says what may run
//! without asking them. For a while that lived only in the MCP bridge: it read the mode, raised
//! the approval card when the mode said to, and acted once the person pressed Allow. `yos act`
//! and a raw client on the socket ran the same `sensitive` action in `ask` mode with no card and
//! no record (issues #49 and #116). Now the dispatch reads the mode the way it reads the ceiling
//! — the shell publishes it beside `settings.yaml` — and a call above what the mode allows must
//! carry a **grant**: the `request_id` that `request_approval` minted and a person's Allow turned
//! into one, which the dispatch spends through the shell before the handler runs — once the
//! ceiling has passed, so an Allow is never used up on an act the ceiling then refuses (#154).
//! Every door meets the same question; `yos act` and the bridge ask for the card on the caller's
//! behalf. `describe` needs nothing, and the ceiling stays above every mode and every grant.
//!
//! The rule itself is `yantrik_ipc_transport::gate`, re-exported below. Three services answer
//! `app.act` without a window — System Monitor, whose `kill_process` is `dangerous`,
//! Notifications and Weather — and until #153 they met none of it. They dispatch through the same
//! `yantrik_surface::Registry` as this module now, so they refuse in the same words, in the same
//! order, with the same argument checks.
//!
//! # What is here, and what is not
//!
//! The dispatch — the registry, the argument checks, the revision guard, the gate, the caller and
//! the agent token, answers finished later — is `yantrik-surface`, which has no UI dependency and
//! is what a service or an outside author links. This module is that dispatch plus the one thing
//! only a window needs: the hop to the thread that owns it, and back.
//!
//! One line the dispatch does not draw: plan mode's refusal of `standard`. The desktop's own
//! processes cross this socket with `standard` calls — an app asking the shell to `start_service`
//! the service it needs, a second launch handing its file to the open window — and until callers
//! carry an identity (#43) the dispatch cannot tell those from a mind. So every mode runs
//! `standard` here ([`SOCKET_FLOOR`]); plan's refusal of it stays the bridge's, as it always was,
//! and plan still refuses everything above it on every door, because the shell raises no card in
//! plan and so no grant can exist.
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
//! When the caller is owed the *result* of that time — a command's exit code, not "started" — the
//! handler hands the rest of its answer to [`answer_later`]: the handler returns at once and the UI
//! thread moves on, the work runs on the RPC side, and the caller's reply is its result. The RPC
//! side is a multi-threaded runtime that steps the waiting call out of the way
//! (`block_in_place`), so one caller waiting two minutes does not hold up every other caller of the
//! same socket.
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
use yantrik_ipc_transport::server::{PeerCred, RpcServer, ServiceHandler};
use yantrik_surface::{
    finish_later, next_action_id, refusal, ActCall, AgentTokenScope, CallerScope, Later, LaterScope,
    LocalRegistry, NO_SUCH_METHOD, UNANSWERED,
};

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
pub use yantrik_surface::service_id_for;

// ── The names one app answers to ─────────────────────────────────────

/// Every app that publishes a control surface, and the other names it is known by.
///
/// An app has up to three names and they are not always the same word: the id it publishes here
/// (`containers`), the program in `/opt/yantrik/bin` (`yantrik-container-manager`), and the
/// launcher's word for its tile (`containers`, but `sysmonitor` for System Monitor). Which of
/// them a caller happens to be holding decided whether it could describe the app at all:
///
/// ```text
/// yos ls                          → app-containers
/// yos describe container-manager  → "no socket for 'container-manager'"
/// ```
///
/// The app is `container-manager` in `/opt/yantrik/bin`, in the launcher's route table and in
/// `open_app`; only its socket was `containers`. A mind that found the app by the name everything
/// else calls it, and then asked it to describe itself, was refused — and had no way to learn
/// better from the refusal.
///
/// So the other names were written down once, here, and [`App::serve`] links each of them at the
/// socket the app binds. The ids are the apps' own and are not changed by this: the id an app
/// publishes is still what `describe` reports and what `yos ls` lists. What changes is that the
/// other names reach it.
///
/// Hyphens, because that is how an app publishes its own id (`download-manager`). [`fold`] makes
/// `container_manager`, `Container Manager` and `container-manager` one question, so a caller's
/// punctuation is not part of the name.
///
/// Every app's row has moved into its own `.desktop` file (`X-Yantrik-Surface`,
/// `X-Yantrik-Aliases`), which is where an app somebody else wrote declares the same things; the
/// shell reads those files and links every alias at the app's socket itself, for any surface —
/// one on this runtime, a Python one, Blender's addon (design/surface-sdk-2026-09-23.md §4). What
/// is left is the one surface no `.desktop` file describes: the desktop's own.
const SURFACES: &[(&str, &[&str])] = &[
    // Nothing "opens" the desktop, so it is in no launcher table; it does publish a surface, and
    // its own notifications' buttons and approval cards have to reach it.
    ("shell", &[]),
];

/// One spelling of a name, so the separator a caller arrived with is not part of the question.
fn fold(name: &str) -> String {
    name.trim().to_lowercase().replace([' ', '_'], "-")
}

/// The id whose control surface answers to `name`, whichever of the app's names that is.
///
/// `surface_id("container-manager")`, `surface_id("Container Manager")` and
/// `surface_id("containers")` are all `containers` — the id the socket is bound under. `None`
/// means no app of this desktop answers to that name at all, which is a different thing from an
/// app that is closed.
pub fn surface_id(name: &str) -> Option<&'static str> {
    let key = fold(name);
    SURFACES.iter().find_map(|(id, others)| {
        (*id == key || others.contains(&key.as_str())).then_some(*id)
    })
}

/// The other names `app_id`'s surface answers to. Empty for an app with one name, and for a name
/// that is not this desktop's.
pub fn other_names(app_id: &str) -> &'static [&'static str] {
    let key = fold(app_id);
    SURFACES
        .iter()
        .find(|(id, _)| *id == key)
        .map(|(_, others)| *others)
        .unwrap_or(&[])
}

// ── The ceiling, the mode and the grant ─────────────────────────────
//
// Every action carries a grade; the machine has a ceiling (`tool_permission`), the person has a
// mode, and a call above what the mode runs unasked must carry a grant — a person's Allow, spent
// through the shell. That rule lives in `yantrik_ipc_transport::gate` since issue #153: three
// services answer `app.act` without a window, and a service must not link Slint to be told no,
// so the rule moved below this crate, beside the socket client it needs for spending a grant, and
// is re-exported here unchanged: `control::configured_ceiling`, `control::mode_from`,
// `control::spend_grants_with` and the rest are what they were.
//
// What stays here is the half only a window has. Its grades live on the UI thread and file and
// socket IO does not, so the RPC thread reads the files and spends any grant (see
// `ControlRpc::dispatch`), and the registry decides with `gate::decide` inside the same turn of
// the event loop as the handler.
pub use yantrik_ipc_transport::gate::{
    configured_ceiling, configured_mode, decide, grant_of, mode_from, mode_path, permit,
    spend_grants_with, unrecoverable, Authority, Mode, AGENT_TOKEN, DEFAULT_MODE, LADDER, MODES,
    MODE_FILE, SOCKET_FLOOR, UNRECOVERABLE_PHRASES,
};
#[cfg(test)]
use yantrik_ipc_transport::gate::{agent_token_of, ceiling_from, DEFAULT_CEILING};

// ── What an app reports ─────────────────────────────────────────────
//
// The vocabulary itself — `View`, `Param`, `Action`, the action JSON schema and the
// revision hash — is pure data with no tie to Slint, and a standalone service must be able
// to build the identical envelope without pulling this runtime in. So it lives in
// `yantrik-ipc-contracts::control_surface`, and this module re-exports it: every existing
// `control::View` / `control::Action` / `control::Param` caller is unchanged, and the shell
// window and a headless service now share one definition of what an app is.
pub use yantrik_ipc_contracts::control_surface::{
    act_json, describe_json, Action, Param, View, PROTOCOL,
};

// ── The registry, which lives on the UI thread ──────────────────────
//
// `yantrik_surface::Registry` with closures that may capture Slint handles, which is why it
// never leaves the thread that owns the window.

type Registry = LocalRegistry;

thread_local! {
    /// Installed by [`App::serve`] on the thread that owns the window.
    static REGISTRY: RefCell<Option<Registry>> = const { RefCell::new(None) };
}

// ── Who is calling, which agent it is for, and answers that take time ──
//
// All three are `yantrik_surface`'s, and are the same functions a service's handler calls. What
// this module adds is carrying them across the hop: the caller and the agent token travel WITH
// the closure posted to the UI thread and are installed there for exactly the duration of that
// one dispatch (see `on_ui_thread`), and the rest of an answer a handler left with
// `answer_later` travels back to the RPC thread with the reply, where it runs off the UI thread.
//
// The handler signature is untouched: fourteen apps build `|args| { ... }` closures and none of
// them has to change. A handler that cares reads `control::caller()`; every other one never
// learns this exists.
pub use yantrik_surface::{agent_token, answer_later, caller, Caller};

/// The grade THIS app publishes for one of its own actions.
///
/// Reads the registry installed by [`App::serve`], so it answers only on the thread that owns
/// the window — which is where handlers run, and is the only place it is wanted. Over the socket
/// the same fact arrives as `permission` in `app.describe`; this is the local shortcut, and the
/// shell needs it because asking *itself* over its own socket from its own UI thread is a call
/// that cannot be answered until the call returns.
///
/// `None` means "this app has no action by that name", which a caller must not read as "it is
/// harmless": an unknown action has no grade, and the honest answer to a question about one is
/// a refusal, not a default.
pub fn published_grade(action: &str) -> Option<&'static str> {
    REGISTRY.with(|cell| cell.borrow().as_ref().and_then(|reg| reg.published_grade(action)))
}

/// Re-declare the grade THIS app publishes for one of its own actions, while it is running.
///
/// See `yantrik_surface::Registry::regrade` for why an app needs this (Studio's `generate`, whose
/// backend can move from the person's LAN to a hosted service by an action on the same surface).
/// Returns the grade now published, so a handler can say what the next call will be asked for.
/// `Err` leaves the published grade untouched: a typo must not quietly un-grade an action.
///
/// Like [`published_grade`], this reads the registry installed by [`App::serve`], so it answers
/// only on the thread that owns the window — which is where handlers run, and the only place a
/// grade can be changed without racing the dispatch that reads it.
///
/// It takes only a SHARED borrow of the registry, whose overrides sit behind their own lock.
/// That is not tidiness: the dispatch runs a handler from inside `REGISTRY.borrow()`, so the
/// first version of this — which took `borrow_mut` — panicked with "RefCell already borrowed"
/// the first time an action called it, killing the app. Calling this from a handler is the only
/// way it is ever meant to be used, so that was every use of it.
pub fn regrade(action: &str, permission: &'static str) -> Result<&'static str, String> {
    yantrik_surface::check_grade(action, permission)?;
    REGISTRY.with(|cell| match cell.borrow().as_ref() {
        None => Err("this app published no control surface, so there is no grade to change".to_string()),
        Some(registry) => registry.regrade(action, permission),
    })
}

thread_local! {
    /// On the RPC thread: the rest of the answer the dispatch that just came back handed over.
    static LATER_HANDED: RefCell<Option<Later>> = const { RefCell::new(None) };
}

/// Hand one closure to the thread that owns the window.
///
/// Boxed rather than generic so that the test stand-in below can take it back unrun when no
/// stand-in is installed; the box costs one allocation per RPC call, which is nothing beside
/// the round trip it is part of.
fn post_to_ui(job: Box<dyn FnOnce() + Send>) -> Result<(), String> {
    // In tests there is no Slint event loop and no window. The stand-in is a plain worker
    // thread fed by a channel — the same shape as the real hop (the closure crosses a thread
    // boundary, and the caller has to cross with it), which is the property under test.
    #[cfg(any(test, feature = "test-standin"))]
    let job = match test_ui_thread::post(job) {
        Ok(()) => return Ok(()),
        Err(unrun) => unrun,
    };

    slint::invoke_from_event_loop(job).map_err(|e| format!("app is not accepting requests: {e}"))
}

/// A stand-in for the thread that owns the window, for the tests that need a real socket.
///
/// The property worth testing is that the caller crosses the thread hop with its own request,
/// and that cannot be tested through a handler called directly — `Registry::act` never sees a
/// socket. It also cannot be tested through the real hop, because `slint::invoke_from_event_loop`
/// needs a running event loop, which needs a window, which needs a display the test machine does
/// not have. So the hop is a channel to a worker thread: same shape, same thread boundary, same
/// thread-local, no compositor.
///
/// One stand-in per test binary, because [`REGISTRY`] is a thread-local and the stand-in is the
/// thread that holds it.
///
/// Also built for another crate's tests under the `test-standin` feature (a dev-dependency only —
/// no shipped binary has it), so the shell can put its own rule on this real dispatch and reach it
/// through a socket: see [`serve_on_a_standin`].
#[cfg(any(test, feature = "test-standin"))]
mod test_ui_thread {
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Mutex, OnceLock};

    type Job = Box<dyn FnOnce() + Send>;

    static STANDIN: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();

    /// Start the stand-in and build the registry ON it.
    ///
    /// `build` rather than a `Registry`: a registry holds the app's own closures, which are not
    /// `Send` (they capture Slint handles in a real app), so it has to be made on the thread
    /// that will keep it. Returns only once the registry is in place, so a request that arrives
    /// immediately cannot find an empty one.
    pub(super) fn start(build: Box<dyn FnOnce() -> super::Registry + Send>) {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready, is_ready) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("control-test-ui".into())
            .spawn(move || {
                super::REGISTRY.with(|cell| *cell.borrow_mut() = Some(build()));
                let _ = ready.send(());
                while let Ok(job) = rx.recv() {
                    job();
                }
            })
            .expect("stand-in UI thread");
        is_ready.recv().expect("the stand-in installed its registry");
        STANDIN
            .set(Mutex::new(tx))
            .map_err(|_| ())
            .expect("only one stand-in per test binary");
    }

    /// Post to the stand-in, or hand the job straight back when there is none — which is every
    /// test but the socket ones, so nothing else in this module changes behaviour under
    /// `cfg(test)`.
    pub(super) fn post(job: Job) -> Result<(), Job> {
        let Some(tx) = STANDIN.get() else { return Err(job) };
        let tx = tx.lock().unwrap_or_else(|e| e.into_inner());
        tx.send(job).map_err(|e| e.0)
    }
}

/// For another crate's tests: serve the surface `build` makes on a real socket, answered through
/// this module's own dispatch (`ControlRpc`: the call read, the grant, the hop, `Registry::act`) on
/// a stand-in for the UI thread instead of a window. Returns the socket's address once something
/// can connect to it.
///
/// `build` runs on the stand-in, because an app's closures are not `Send`. The socket is bound in
/// `runtime_dir`, which the environment points at only until the bind: the address is a path after
/// that. One per test binary, like the stand-in itself.
#[cfg(all(unix, feature = "test-standin"))]
pub fn serve_on_a_standin(
    build: impl FnOnce() -> App + Send + 'static,
    runtime_dir: &std::path::Path,
) -> String {
    use std::os::unix::net::UnixStream;

    let (tx, rx) = mpsc::channel::<(String, usize)>();
    test_ui_thread::start(Box::new(move || {
        let app = build();
        let _ = tx.send((app.registry.app_id().to_string(), app.registry.action_count()));
        app.registry
    }));
    let (app_id, actions) = rx.recv().expect("the stand-in built the surface");

    let _env = crate::env_lock();
    let before = std::env::var_os("XDG_RUNTIME_DIR");
    std::fs::create_dir_all(runtime_dir).expect("a runtime dir for the test socket");
    std::env::set_var("XDG_RUNTIME_DIR", runtime_dir);
    let address = RpcServer::default_address(&service_id_for(&app_id));
    serve_rpc(&app_id, actions);
    let mut bound = false;
    for _ in 0..1000 {
        if UnixStream::connect(&address).is_ok() {
            bound = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    match before {
        Some(dir) => std::env::set_var("XDG_RUNTIME_DIR", dir),
        None => std::env::remove_var("XDG_RUNTIME_DIR"),
    }
    assert!(bound, "nothing ever bound {address}");
    address
}

/// Ask the UI thread to run `job` and wait for its answer.
///
/// Returns `Err` when the event loop is not running or is too busy to answer inside
/// [`UI_ROUNDTRIP`] — both of which the caller should see as an error rather than a hang.
///
/// `who` rides along to the far side. It is installed there, not here: the handler runs on the
/// UI thread, so the UI thread is the only place a thread-local can be read by it. The rest of an
/// answer a handler left with [`answer_later`] rides back, for `ControlRpc::handle_from` to run.
fn on_ui_thread<T, F>(who: Option<Caller>, job: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Registry) -> T + Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel::<(Result<T, String>, Option<Later>)>(1);
    post_to_ui(Box::new(move || {
        let _scope = CallerScope::enter(who);
        let later = LaterScope::enter();
        let answer = REGISTRY.with(|cell| match cell.borrow().as_ref() {
            Some(reg) => Ok(job(reg)),
            None => Err("this app published no control surface".to_string()),
        });
        // The receiver is gone only if we already timed out; dropping the answer is correct.
        let _ = tx.send((answer, later.take()));
    }))?;

    let (answer, later) = rx
        .recv_timeout(UI_ROUNDTRIP)
        .map_err(|_| format!("app did not answer within {}s", UI_ROUNDTRIP.as_secs()))?;
    // For `ControlRpc::handle_from`, on this same thread, which finishes it. See `answer_later`.
    LATER_HANDED.with(|cell| *cell.borrow_mut() = later);
    answer
}

/// The UI thread did not answer in time, or there is no window to answer.
fn unanswered(message: String) -> ServiceError {
    ServiceError { code: UNANSWERED, message }
}

// ── The RPC surface ─────────────────────────────────────────────────

struct ControlRpc {
    service_id: String,
    /// The id the registry publishes under — what a grant is bound to, which is not the
    /// socket's `app-` name.
    app_id: String,
}

impl ServiceHandler for ControlRpc {
    fn service_id(&self) -> &str {
        &self.service_id
    }

    /// The transport's older entry point. Kept so the trait is satisfied for any caller that
    /// still uses it; it means "nobody said who was calling", which is exactly what `None` is.
    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        self.handle_from(method, params, None)
    }

    fn handle_from(
        &self,
        method: &str,
        params: serde_json::Value,
        peer: Option<PeerCred>,
    ) -> Result<serde_json::Value, ServiceError> {
        let who = peer.map(Caller::from);
        // Nothing left over from an earlier call on this thread can be taken for this one's.
        LATER_HANDED.with(|cell| cell.borrow_mut().take());
        let answer = self.dispatch(method, params, who);
        let later = LATER_HANDED.with(|cell| cell.borrow_mut().take());
        match (answer, later) {
            // The work runs here, off the UI thread, and the view beside its result is read
            // again on the UI thread once it has.
            (Ok(envelope), Some(later)) if method == "app.act" => finish_later(envelope, later, || {
                on_ui_thread(who, |reg| reg.snapshot()).ok()
            }),
            (answer, _) => answer,
        }
    }
}

impl ControlRpc {
    fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
        who: Option<Caller>,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "app.describe" => on_ui_thread(who, |reg| reg.describe()).map_err(unanswered),

            "app.act" => {
                let call = ActCall::parse(&params)?;
                // Agents catalog: the calling agent's reach (`yantrik_ipc_transport::reach`) —
                // read here, where IO belongs, and held to below before any grant is spent and
                // before the handler runs. No token, or a token with no reach, is not held.
                let reach = call.reach()?;
                let action_id = next_action_id(&self.service_id);

                // Read on this thread, enforced on the UI one: the settings and mode files are
                // IO, spending a grant is a round trip, and the dispatch closure is a turn of
                // the event loop.
                let mut authority = Authority::now();
                // A grant is spent only once everything that could still refuse the call without
                // asking anybody has passed: the action exists, the agent's reach covers it, its
                // arguments are right, and the ceiling allows its grade (#154) — or a person's
                // Allow is used up on an act that never runs. The declarations and the grades live
                // on the UI thread, so ask it first — one extra hop, only for a call that carries a
                // grant, which is one a person has just answered a card for. Should the app
                // regrade the action between this read and the dispatch, the dispatch still
                // decides on the grade it publishes then; the most that race can cost is the grant.
                call.spend_grant(&mut authority, &self.app_id, || {
                    let (name, args, reach) = (call.action.clone(), call.args.clone(), reach.clone());
                    on_ui_thread(who, move |reg| {
                        reg.within_reach(reach.as_ref(), &name).and_then(|()| reg.check_call(&name, &args))
                    })
                    .map_err(unanswered)?
                    .map_err(refusal)
                })?;
                call.log(&action_id, &authority, who);

                let ActCall { action, args, agent_token, expect_revision, .. } = call;
                on_ui_thread(who, move |reg| {
                    let _agent = AgentTokenScope::enter(agent_token);
                    // The reach, on the grade this surface publishes now — the one `act` decides on.
                    reg.within_reach(reach.as_ref(), &action).and_then(|()| {
                        reg.act(&action, &args, expect_revision.as_deref(), &action_id, &authority)
                    })
                })
                .map_err(unanswered)?
                // An action that legitimately refuses is an application error, not a transport
                // failure: -32602 keeps it out of the client's circuit breaker.
                .map_err(refusal)
            }

            other => Err(ServiceError {
                code: NO_SUCH_METHOD,
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
        Self { registry: Registry::new(app_id) }
    }

    /// What this app reports when asked. Runs on the UI thread; keep it cheap.
    pub fn describe(mut self, f: impl Fn() -> View + 'static) -> Self {
        self.registry.set_describe(Box::new(f));
        self
    }

    /// One thing this app can be asked to do. Runs on the UI thread.
    pub fn action(
        mut self,
        spec: Action,
        f: impl Fn(&serde_json::Value) -> Result<serde_json::Value, String> + 'static,
    ) -> Self {
        self.registry.add(spec, Box::new(f));
        self
    }

    /// Hold every act on this surface to a rule of the app's own, asked in the dispatch before
    /// anything else about the call — its arguments, the ceiling, the mode, any grant, the handler.
    /// `Err` is the caller's refusal, word for word. The shell holds its surface while the desktop
    /// is locked (`crate::lock` in yantrik-ui): one rule, in the one function every `app.act`
    /// crosses, rather than a check in each action that the next action would forget.
    pub fn hold(mut self, rule: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static) -> Self {
        self.registry.hold_with(Box::new(rule));
        self
    }

    /// Publish this surface on the socket bus.
    ///
    /// Must be called from the thread that owns the window, before `run()`. Failing to serve is
    /// not fatal: an app whose socket cannot be bound is still a working app, it is only invisible
    /// to the mind, and taking the window down over that would be the worse outcome.
    pub fn serve(self) {
        let app_id = self.registry.app_id().to_string();
        let action_count = self.registry.action_count();
        REGISTRY.with(|cell| *cell.borrow_mut() = Some(self.registry));
        serve_rpc(&app_id, action_count);
    }
}

/// Bind the socket and answer on it, on a thread of its own.
///
/// Split out of [`App::serve`] so the test below can put the registry on a stand-in UI thread
/// and still bind exactly the same server. `serve` itself is byte-for-byte what it always did.
fn serve_rpc(app_id: &str, action_count: usize) {
    {
        let service_id = service_id_for(app_id);
        let app_id = app_id.to_string();
        link_other_names(&app_id);
        std::thread::Builder::new()
            .name(format!("{service_id}-rpc"))
            .spawn(move || {
                // Multi-threaded, and small: a caller waiting on `answer_later` steps its worker
                // out of the way and the other keeps serving everyone else.
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
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
                        .serve(Arc::new(ControlRpc { service_id: service_id.clone(), app_id }))
                        .await
                    {
                        tracing::warn!(error = %e, "Control surface stopped");
                    }
                });
            })
            .ok();
    }
}

/// Put every other name this app answers to beside the socket it is about to bind.
///
/// A symlink, not a second listener: it is one surface, so a caller that follows
/// `app-container-manager.sock` has to land in the same process and read the same revision. Two
/// listeners would be two answers to the same question, and a `describe`/`act` pair split across
/// them is the race `expect_revision` exists to close.
///
/// Made before the bind rather than after it, because nothing here can be told when the bind
/// happened and polling for the file would be a second way to be wrong. A symlink to a socket
/// that does not exist yet is invisible to a caller — `os.path.exists` is false and `connect`
/// gets ENOENT — and starts working the moment the socket lands, which is the same fall-through
/// every caller already has for the stale socket of a closed window.
///
/// Nothing here is fatal. An app whose other names cannot be linked is still a working app,
/// reachable by the id it publishes, which is what it was before.
#[cfg(unix)]
fn link_other_names(app_id: &str) {
    let others = other_names(app_id);
    if others.is_empty() {
        return;
    }
    let dir = yantrik_ipc_transport::server::socket_dir();
    for name in link_names(&dir, app_id, others) {
        tracing::info!(name = %name, app = app_id, "Control surface also answers to this name");
    }
}

#[cfg(not(unix))]
fn link_other_names(_app_id: &str) {}

/// Link `others` at `app_id`'s socket inside `dir`, and report the names that now reach it.
///
/// Separated from the directory lookup so it can be tested in a directory of its own. Relative
/// link targets on purpose: the socket directory is moved by nothing, and a relative target
/// survives being read from a different mount view of the same runtime dir.
#[cfg(unix)]
fn link_names(dir: &std::path::Path, app_id: &str, others: &[&str]) -> Vec<String> {
    let target = format!("{}.sock", service_id_for(app_id));
    let mut linked = Vec::new();
    for name in others {
        let link = dir.join(format!("{}.sock", service_id_for(name)));
        // What is there already is from an earlier run of this app, or from an earlier release
        // that bound this name for real. Either way it is not something to connect to now, and
        // leaving it would leave the other name pointing at nothing.
        match std::fs::read_link(&link) {
            Ok(existing) if existing == std::path::Path::new(&target) => {
                linked.push((*name).to_string());
                continue;
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&link);
            }
            Err(_) if link.symlink_metadata().is_ok() => {
                let _ = std::fs::remove_file(&link);
            }
            Err(_) => {}
        }
        match std::os::unix::fs::symlink(&target, &link) {
            Ok(()) => linked.push((*name).to_string()),
            Err(e) => tracing::warn!(
                name = *name,
                app = app_id,
                error = %e,
                "Could not link one of this app's other names; callers holding it cannot reach it"
            ),
        }
    }
    linked
}

// ── Finding the others ──────────────────────────────────────────────

/// The app ids that currently have a control socket in this session.
///
/// A socket file outlives a crashed process, so this is a list of candidates, not of live apps —
/// callers should treat a failed `app.describe` as "gone" rather than as an error worth reporting.
///
/// One app, one entry: the other names an app answers to are symlinks to its socket (see
/// `link_names`), and listing those would report one open window twice under two names.
#[cfg(unix)]
pub fn running_apps() -> Vec<String> {
    let dir = yantrik_ipc_transport::server::socket_dir();
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_type().is_ok_and(|kind| kind.is_symlink()))
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

/// The dispatch's own tests — the order of the checks, every sentence, the ceiling, the mode, the
/// guard, the argument types — live with the dispatch, in `yantrik-surface`. These are the ones
/// only a window has: the registry on the UI thread, the free `published_grade` / `regrade` a
/// handler calls there, the hop across and back over a real socket, and the names.
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A ceiling that binds nothing, for the tests that are about everything *except* the
    /// boundary.
    const OPEN: &str = "dangerous";

    /// Authority that binds nothing: the ceiling and the mode both at the top of the ladder.
    fn open() -> Authority {
        Authority { ceiling: OPEN.into(), mode: Mode::named("bypass"), granted: false }
    }

    /// A machine at `ceiling`, in a mode that asks about nothing under it: the ceiling tests.
    fn under(ceiling: &str) -> Authority {
        Authority { ceiling: ceiling.into(), mode: Mode::named("bypass"), granted: false }
    }

    /// An open ceiling and the mode under test, with or without a grant spent for the call.
    fn in_mode(mode: &str, granted: bool) -> Authority {
        Authority { ceiling: OPEN.into(), mode: Mode::named(mode), granted }
    }

    type Act = Box<dyn Fn(&serde_json::Value) -> Result<serde_json::Value, String>>;

    /// A registry built the way [`App`] builds one.
    fn registry(
        app_id: &str,
        describe: Option<Box<dyn Fn() -> View>>,
        actions: Vec<(Action, Act)>,
    ) -> Registry {
        let mut app = App::new(app_id);
        if let Some(f) = describe {
            app = app.describe(f);
        }
        for (spec, f) in actions {
            app = app.action(spec, f);
        }
        app.registry
    }

    #[test]
    fn service_ids_do_not_collide_with_services() {
        // notes-service owns `notes`; the Notes window must not bind the same socket.
        assert_eq!(service_id_for("notes"), "app-notes");
        assert_ne!(service_id_for("notes"), "notes");
    }

    // The guide's table of surfaces is generated from the apps' `.desktop` files now, by
    // `surfaces::tests::the_guide_lists_every_surface_this_desktop_has` in yantrik-ui.

    /// The desktop's own surface answers to its name however it is spelled, and an app's names are
    /// not this table's any more: they are its `.desktop` file's, read by the shell.
    #[test]
    fn every_name_an_app_is_known_by_reaches_the_id_it_publishes() {
        for spelling in ["shell", "  Shell  ", "SHELL"] {
            assert_eq!(surface_id(spelling), Some("shell"), "{spelling}");
        }
        assert!(other_names("shell").is_empty());
        // "No such name here" is not "that app is closed": the catalogue answers for apps.
        assert_eq!(surface_id("containers"), None);
        assert_eq!(surface_id("no-such-app"), None);
        assert_eq!(surface_id(""), None);
    }

    /// No name is claimed twice, and every name is written the way the folding leaves it.
    ///
    /// A second claim on one name would be resolved by whichever row came first, silently, and a
    /// name stored with an underscore could never match anything, because every lookup folds.
    #[test]
    fn no_two_apps_answer_to_the_same_name() {
        let mut seen = std::collections::HashSet::new();
        for (id, others) in SURFACES {
            for name in std::iter::once(id).chain(others.iter()) {
                assert!(seen.insert(*name), "`{name}` is claimed by two apps");
                assert_eq!(fold(name), *name, "`{name}` is not folded, so nothing can match it");
            }
        }
    }

    /// The other names of a running app are its socket under another name, not another socket.
    #[cfg(unix)]
    #[test]
    fn another_name_for_an_app_points_at_the_socket_it_bound() {
        let dir = std::env::temp_dir().join(format!("yantrik-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("app-containers.sock");
        std::fs::write(&socket, b"stands in for the bound socket").unwrap();

        let others: &[&str] = &["container-manager"];
        let linked = link_names(&dir, "containers", others);
        assert_eq!(linked, vec!["container-manager".to_string()]);
        let link = dir.join("app-container-manager.sock");
        assert_eq!(std::fs::read_link(&link).unwrap().to_str(), Some("app-containers.sock"));
        assert_eq!(std::fs::read(&link).unwrap(), std::fs::read(&socket).unwrap());

        // Run twice, as a reopened app does: the second link is the same link, not an error.
        assert_eq!(link_names(&dir, "containers", others).len(), 1);

        // A real file under that name, left by a release that bound it for real, is replaced —
        // otherwise the other name would go on answering out of a socket nothing is behind.
        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, b"an older release bound this name").unwrap();
        assert_eq!(link_names(&dir, "containers", others).len(), 1);
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_view_builds_an_object() {
        let v = View::new("Notes — 3 open").with("count", 3).with("unsaved", true);
        assert_eq!(v.summary, "Notes — 3 open");
        assert_eq!(v.state["count"], 3);
        assert_eq!(v.state["unsaved"], true);
    }

    /// An `App` is the dispatch: what it builds refuses and answers as `yantrik-surface` does,
    /// with the argument types checked.
    #[test]
    fn an_app_dispatches_through_the_surface_crate() {
        let reg = registry(
            "notes",
            Some(Box::new(|| View::new("Notes \u{2014} Kernel asks"))),
            vec![(
                Action::new("rename", "Rename the open note").arg(Param::text("to")),
                Box::new(|args| Ok(serde_json::json!({ "renamed_to": args["to"].clone() }))),
            )],
        );
        let answer = reg.act("rename", &serde_json::json!({"to": "x"}), None, "app-notes#1", &open()).unwrap();
        assert_eq!(answer["accepted"], true);
        assert_eq!(answer["result"]["renamed_to"], "x");
        assert_eq!(
            reg.act("rename", &serde_json::json!({}), None, "app-notes#2", &open()).unwrap_err(),
            "`rename` needs argument `to`"
        );
        assert_eq!(
            reg.act("rename", &serde_json::json!({"to": true}), None, "app-notes#3", &open()).unwrap_err(),
            "`rename` argument `to` must be a string, and a boolean arrived"
        );
        // An integer for text is its digits: the handler reads the type it declared.
        let answer = reg.act("rename", &serde_json::json!({"to": 7}), None, "app-notes#5", &open()).unwrap();
        assert_eq!(answer["result"]["renamed_to"], "7");
        assert_eq!(
            reg.act("nope", &serde_json::json!({}), None, "app-notes#4", &open()).unwrap_err(),
            "unknown action `nope`; this app offers: rename"
        );
    }

    /// A rule an app declares with [`App::hold`] is asked by the dispatch itself, before the
    /// arguments, the ceiling and the handler — the shell's `LOCKED:` is one of these — and what
    /// it lets through runs as before.
    #[test]
    fn a_hold_the_app_declares_is_asked_by_the_dispatch() {
        let ran = Rc::new(Cell::new(false));
        let app = App::new("shell")
            .action(Action::new("show_screen", "Switch screens").arg(Param::text("screen")), {
                let ran = ran.clone();
                move |_| {
                    ran.set(true);
                    Ok(serde_json::json!({ "showing": "desktop" }))
                }
            })
            .action(Action::new("lock", "Lock the session").risk("safe"), |_| {
                Ok(serde_json::json!({ "locked": true }))
            })
            .hold(|name| match name {
                "lock" => Ok(()),
                _ => Err("LOCKED: held".to_string()),
            });
        let reg = app.registry;
        let screen = serde_json::json!({ "screen": "desktop" });
        assert_eq!(reg.act("show_screen", &screen, None, "app-shell#1", &open()).unwrap_err(), "LOCKED: held");
        assert_eq!(reg.act("show_screen", &serde_json::json!({}), None, "app-shell#2", &under("safe")).unwrap_err(), "LOCKED: held");
        assert_eq!(reg.check_call("show_screen", &screen).unwrap_err(), "LOCKED: held", "before a grant is spent");
        assert!(!ran.get(), "held, and the handler ran anyway");
        assert_eq!(reg.act("lock", &serde_json::json!({}), None, "app-shell#3", &open()).unwrap()["result"]["locked"], true);
    }

    #[test]
    fn the_ceiling_comes_from_the_settings_file() {
        // Same file, same key, same default as the shell's Settings screen — a boundary that
        // reads a different source than the one a person can see is a boundary nobody can
        // predict. Missing or unrecognised falls back to the shipped default, never open.
        assert_eq!(ceiling_from("dark_mode: true\ntool_permission: standard\n"), "standard");
        assert_eq!(ceiling_from("tool_permission: \"safe\"\n"), "safe");
        assert_eq!(ceiling_from("dark_mode: true\n"), DEFAULT_CEILING, "absent key");
        assert_eq!(ceiling_from(""), DEFAULT_CEILING, "empty file");
        assert_eq!(ceiling_from("tool_permission: whenever-i-feel_like_it\n"), DEFAULT_CEILING);
    }

    #[test]
    fn an_apps_own_grade_can_be_read_without_a_round_trip() {
        // The shell needs this to check a caller's *claimed* grade against the app's real one,
        // and for its own actions it cannot ask over the socket: the answer would have to come
        // from the UI thread that is making the call. `None` for an action that does not exist,
        // because an unknown action has no grade and defaulting one would invent a permission.
        REGISTRY.with(|cell| {
            *cell.borrow_mut() = Some(registry(
                "shell",
                None,
                vec![
                    (Action::new("files_delete", "Delete a file").risk("dangerous"), Box::new(|_| Ok(serde_json::Value::Null))),
                    (Action::new("open_app", "Open an app"), Box::new(|_| Ok(serde_json::Value::Null))),
                ],
            ))
        });

        assert_eq!(published_grade("files_delete"), Some("dangerous"));
        assert_eq!(published_grade("open_app"), Some("standard"), "unstated risk is standard");
        assert_eq!(published_grade("no_such_action"), None);

        REGISTRY.with(|cell| *cell.borrow_mut() = None);
        assert_eq!(published_grade("files_delete"), None, "and nothing is served here now");
    }

    #[test]
    fn an_action_can_be_regraded_while_the_app_runs_and_the_ceiling_follows() {
        // Studio's shape: `generate` is graded when the surface is published, and the backend it
        // sends prompts to can be changed afterwards by another action on the same surface. The
        // grade has to move with it, or a caller under a `standard` ceiling can be talked into
        // sending a prompt off the machine by an action that never asked for anything.
        REGISTRY.with(|cell| {
            *cell.borrow_mut() = Some(registry(
                "studio",
                None,
                vec![(
                    Action::new("generate", "Make a picture from a sentence"),
                    Box::new(|_| Ok(serde_json::json!({"queued": 1}))),
                )],
            ))
        });
        let act_under = |ceiling: &str, id: &str| {
            REGISTRY.with(|cell| {
                cell.borrow().as_ref().unwrap().act("generate", &serde_json::json!({}), None, id, &under(ceiling))
            })
        };

        assert!(act_under("standard", "studio#1").is_ok());
        assert_eq!(regrade("generate", "sensitive").unwrap(), "sensitive");
        assert_eq!(published_grade("generate"), Some("sensitive"), "the two readers disagree");
        let err = act_under("standard", "studio#2").unwrap_err();
        assert!(err.starts_with("CEILING:") && err.contains("graded `sensitive`"), "{err}");
        assert!(act_under("sensitive", "studio#3").is_ok());
        // And `describe` — what a caller reads before deciding — reports the new grade, so the
        // card a person is shown is the card the dispatch will enforce.
        REGISTRY.with(|cell| {
            let described = cell.borrow().as_ref().unwrap().describe();
            assert_eq!(described["actions"][0]["permission"], serde_json::json!("sensitive"));
        });

        // Down again, because the backend can be pointed back at a machine the person owns.
        assert_eq!(regrade("generate", "standard").unwrap(), "standard");
        assert!(act_under("standard", "studio#4").is_ok());
        REGISTRY.with(|cell| *cell.borrow_mut() = None);
    }

    #[test]
    fn a_grade_this_os_does_not_define_leaves_the_action_at_the_one_it_had() {
        REGISTRY.with(|cell| {
            *cell.borrow_mut() = Some(registry(
                "studio",
                None,
                vec![
                    (Action::new("generate", "Make a picture").risk("standard"), Box::new(|_| Ok(serde_json::Value::Null))),
                    (Action::new("refresh", "Read the gallery again"), Box::new(|_| Ok(serde_json::Value::Null))),
                ],
            ))
        });

        // A typo must not quietly un-grade an action, which is what writing the string through
        // without checking it would do.
        let err = regrade("generate", "catastrophic").unwrap_err();
        assert!(err.contains("not a level this OS defines"), "{err}");
        assert!(err.contains(LADDER[0]) && err.contains(LADDER[3]), "{err} does not name the ladder");
        assert_eq!(published_grade("generate"), Some("standard"), "the grade moved anyway");

        // Nor may one action's regrade touch another, or an action that does not exist.
        let err = regrade("no_such_action", "sensitive").unwrap_err();
        assert!(err.contains("no action `no_such_action`"), "{err}");
        assert!(err.contains("generate") && err.contains("refresh"), "{err} does not name what is there");
        assert_eq!(published_grade("refresh"), Some("standard"), "an unknown action regraded a known one");

        REGISTRY.with(|cell| *cell.borrow_mut() = None);
        let err = regrade("generate", "sensitive").unwrap_err();
        assert!(err.contains("published no control surface"), "{err}");
        // And an undefined grade is refused as that whether or not anything is published.
        let err = regrade("generate", "catastrophic").unwrap_err();
        assert!(err.contains("not a level this OS defines"), "{err}");
    }

    #[test]
    fn a_handler_can_regrade_from_inside_its_own_dispatch() {
        // The test the first two were missing, and the only way `regrade` is ever actually used.
        //
        // Real callers run every handler from INSIDE `REGISTRY.borrow()` (see `on_ui_thread`), so
        // the first shipped version — which took `borrow_mut` — panicked with "RefCell already
        // borrowed" the moment Studio's `set_backend` ran, and took the whole app down with it.
        // The config had already been written by then, so the app came back pointed at a hosted
        // service with the grade never raised: precisely the state `regrade` exists to prevent.
        //
        // This mirrors the dispatch: the borrow is held across `act`, exactly as it is in
        // `on_ui_thread`.
        REGISTRY.with(|cell| {
            *cell.borrow_mut() = Some(registry(
                "studio",
                None,
                vec![
                    (
                        Action::new("generate", "Make a picture from a sentence"),
                        Box::new(|_| Ok(serde_json::json!({"queued": 1}))),
                    ),
                    (
                        Action::new("set_backend", "Choose where pictures are made").risk("sensitive"),
                        Box::new(|_| {
                            // A handler, doing the one thing this function is for.
                            let now = regrade("generate", "sensitive")?;
                            Ok(serde_json::json!({ "generate_is_graded": now }))
                        }),
                    ),
                ],
            ))
        });

        let answered = REGISTRY.with(|cell| {
            let installed = cell.borrow();
            let registry = installed.as_ref().unwrap();
            registry
                .act("set_backend", &serde_json::json!({}), None, "studio#1", &under("sensitive"))
                .expect("set_backend must not take the app down")
        });
        assert_eq!(answered["accepted"], serde_json::json!(true));
        assert_eq!(answered["result"]["generate_is_graded"], serde_json::json!("sensitive"));

        // And the move took effect for every reader, still from inside the same kind of borrow.
        REGISTRY.with(|cell| {
            let installed = cell.borrow();
            let registry = installed.as_ref().unwrap();
            let err = registry
                .act("generate", &serde_json::json!({}), None, "studio#2", &under("standard"))
                .unwrap_err();
            assert!(err.starts_with("CEILING:") && err.contains("graded `sensitive`"), "{err}");
            let described = registry.describe();
            assert_eq!(described["actions"][0]["permission"], serde_json::json!("sensitive"));
        });
        assert_eq!(published_grade("generate"), Some("sensitive"));

        REGISTRY.with(|cell| *cell.borrow_mut() = None);
    }

    /// The dispatch reads the action's own description, not only its grade: Calendar's
    /// `delete_event` is `sensitive` and says "It is not recoverable", and in auto the shell and
    /// the bridge asked about it while `yos act` ran it (map gap 4 of the surface SDK). Asked
    /// about on this door too now; bypass still asks nobody.
    #[test]
    fn what_the_app_says_cannot_be_undone_is_asked_about_in_auto() {
        let delete = |ran: Rc<Cell<bool>>| {
            registry(
                "calendar",
                Some(Box::new(|| View::new("Calendar"))),
                vec![(
                    Action::new("delete_event", "Take an event off the calendar. It is not recoverable")
                        .risk("sensitive")
                        .arg(Param::text("id")),
                    Box::new(move |_: &serde_json::Value| {
                        ran.set(true);
                        Ok(serde_json::json!({ "deleted": true }))
                    }),
                )],
            )
        };
        let ran = Rc::new(Cell::new(false));
        let err = delete(ran.clone())
            .act("delete_event", &serde_json::json!({"id": "e1"}), None, "calendar#1", &in_mode("auto", false))
            .unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("cannot be undone"), "{err}");
        assert!(!ran.get(), "the handler must not have run");

        let ran = Rc::new(Cell::new(false));
        delete(ran.clone())
            .act("delete_event", &serde_json::json!({"id": "e1"}), None, "calendar#2", &in_mode("bypass", false))
            .expect("bypass asks nobody");
        assert!(ran.get());
    }

    // ── The grant ──

    /// Blender's `render`, graded `sensitive`, over a flag that says whether it ran.
    fn render_surface(ran: Rc<Cell<bool>>) -> Registry {
        registry(
            "blender",
            Some(Box::new(|| View::new("Blender \u{2014} cube.blend"))),
            vec![(
                Action::new("render", "Render the scene").risk("sensitive").arg(Param::text("out")),
                Box::new(move |args| {
                    ran.set(true);
                    Ok(serde_json::json!({ "rendered_to": args["out"].clone() }))
                }),
            )],
        )
    }

    /// A stand-in for the shell's store: `fresh-*` ids are grants that hold once, for exactly
    /// `blender.render {"out": "x.png"}`; anything else is refused in the shell's own words.
    /// Installed once per test binary, because the spender is process-wide as the shell's is,
    /// and only by the tests that spend — every other test never attaches a grant.
    fn spend_through_a_stand_in_shell() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let spent = std::sync::Mutex::new(std::collections::HashSet::<String>::new());
            spend_grants_with(move |id, app, action, args| {
                if let Some((a, x, bound)) = allowed(id) {
                    if (a.as_str(), x.as_str(), &bound) != (app, action, args) {
                        return Err(format!("`{id}` was approved for {a}.{x} with {bound}, and this call carries {args}."));
                    }
                    let mut spent = spent.lock().unwrap_or_else(|e| e.into_inner());
                    if !spent.insert(id.to_string()) {
                        return Err(format!("`{id}` was already used."));
                    }
                    SPENT_GRANTS.lock().unwrap_or_else(|e| e.into_inner()).push(id.to_string());
                    return Ok(());
                }
                if !id.starts_with("fresh-") {
                    return Err(format!("no approval request `{id}` — it may have been dropped when the shell restarted. Ask again."));
                }
                if app != "blender" || action != "render" || *args != serde_json::json!({"out": "x.png"}) {
                    return Err(format!(
                        "`{id}` was approved for `blender.render` with arguments {{\"out\":\"x.png\"}}, and this call carries {args}. Nothing was authorised."
                    ));
                }
                let mut spent = spent.lock().unwrap_or_else(|e| e.into_inner());
                if !spent.insert(id.to_string()) {
                    return Err(format!("`{id}` was already used. A grant authorises one action once; this second use authorises nothing."));
                }
                Ok(())
            });
        });
    }

    /// Grants a person has allowed on the stand-in shell, beyond its `fresh-*` ones: exactly
    /// `app.action(args)`, once.
    static ALLOWED: std::sync::Mutex<Vec<(String, String, String, serde_json::Value)>> =
        std::sync::Mutex::new(Vec::new());
    /// The allowed grants the stand-in has spent.
    static SPENT_GRANTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    fn allow(id: &str, app: &str, action: &str, args: serde_json::Value) {
        ALLOWED.lock().unwrap_or_else(|e| e.into_inner()).push((id.into(), app.into(), action.into(), args));
    }

    fn allowed(id: &str) -> Option<(String, String, serde_json::Value)> {
        ALLOWED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(g, ..)| g == id)
            .map(|(_, a, x, args)| (a.clone(), x.clone(), args.clone()))
    }

    fn spent(id: &str) -> bool {
        SPENT_GRANTS.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|g| g == id)
    }

    /// Spend `id` for `blender.render`, graded `sensitive`, under `authority`, the way the RPC
    /// thread does before anything reaches the UI thread.
    fn spend_for_render(mut authority: Authority, id: &str, args: &serde_json::Value) -> Result<Authority, String> {
        authority.spend(id, "blender", "render", "sensitive", args).map(|()| authority)
    }

    /// A grant is spent on the RPC thread, before anything reaches the UI thread: a spent one,
    /// one bound to other arguments, and one that never existed each end the call there, with
    /// the shell's reason in the refusal. Without this, "with a grant it runs" would be "with
    /// any string called grant it runs".
    #[test]
    fn a_spent_or_wrong_grant_is_refused_before_anything_is_dispatched() {
        spend_through_a_stand_in_shell();
        let args = serde_json::json!({"out": "x.png"});

        let first = spend_for_render(open(), "fresh-1", &args).expect("a fresh grant holds");
        assert!(first.granted);

        let err = spend_for_render(open(), "fresh-1", &args).unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("already used"), "replayed: {err}");

        let err = spend_for_render(open(), "fresh-2", &serde_json::json!({"out": "y.png"})).unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("this call carries"), "swapped: {err}");

        let err = spend_for_render(open(), "made-up", &args).unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("no approval request"), "invented: {err}");
    }

    /// #154, item 2: a grant spent before the grade was looked at, then refused by the ceiling,
    /// was a person's Allow used up on an act that never ran. The ceiling comes first now.
    #[test]
    fn a_grant_is_not_spent_on_an_act_the_ceiling_refuses() {
        spend_through_a_stand_in_shell();
        let args = serde_json::json!({"out": "x.png"});

        let err = spend_for_render(under("standard"), "fresh-154", &args).unwrap_err();
        assert!(err.starts_with("CEILING:"), "the ceiling's refusal, not the grant's: {err}");
        assert!(err.contains("graded `sensitive`") && err.contains("`standard` ceiling"), "{err}");

        let raised = spend_for_render(under("sensitive"), "fresh-154", &args)
            .expect("the refusal above the ceiling left the grant unspent");
        assert!(raised.granted);
        let err = spend_for_render(open(), "fresh-154", &args).unwrap_err();
        assert!(err.contains("already used"), "and it still holds only once: {err}");

        // And the dispatch reaches the same answer on the UI thread, whatever was spent: the
        // ceiling is decided again there, on the grade `describe` is showing at that moment.
        let mut granted = under("standard");
        granted.granted = true;
        let ran = Rc::new(Cell::new(false));
        let err = render_surface(ran.clone()).act("render", &args, None, "blender#1", &granted).unwrap_err();
        assert!(err.starts_with("CEILING:"), "{err}");
        assert!(!ran.get());
    }

    /// Same file, same shape as the shell writes (see `mind_mode::policy_json`), and the same
    /// fallback as the bridge: anything unreadable is `ask`, never something looser. The
    /// shell's own test drives its writer through this reader.
    #[test]
    fn the_mode_comes_from_the_file_the_shell_writes() {
        let now = 1_800_000_000;
        assert_eq!(mode_from(r#"{"mode":"auto","session_rules":[]}"#, now).name, "auto");
        let with_rule = mode_from(
            r#"{"mode":"ask","session_rules":[{"app":"calendar","action":"move_event"}]}"#,
            now,
        );
        assert_eq!(with_rule.name, "ask");
        assert_eq!(with_rule.session_rules, vec![("calendar".to_string(), "move_event".to_string())]);

        // Missing, empty, not JSON, or a mode this OS does not define: `ask`.
        for text in ["", "{}", "mode: auto", r#"{"mode":"yolo"}"#, r#"{"mode":"BYPASS"}"#] {
            assert_eq!(mode_from(text, now).name, DEFAULT_MODE, "{text:?}");
        }

        // A bypass with an end honours it: live before, back to what it was after — and a
        // "previous" of bypass, which the shell never writes, comes back as `ask`.
        let bypass = format!(r#"{{"mode":"bypass","previous":"auto","bypass_expires_unix":{}}}"#, now + 60);
        assert_eq!(mode_from(&bypass, now).name, "bypass");
        assert_eq!(mode_from(&bypass, now + 60).name, "auto");
        let odd = format!(r#"{{"mode":"bypass","previous":"bypass","bypass_expires_unix":{}}}"#, now);
        assert_eq!(mode_from(&odd, now).name, DEFAULT_MODE);
        assert_eq!(mode_from(r#"{"mode":"bypass","previous":"ask","bypass_expires_unix":null}"#, now).name, "bypass");

        // And each mode's column of the table: what it runs unasked.
        for (mode, top) in MODES {
            assert_eq!(LADDER[Mode::named(mode).allows()], top, "{mode}");
        }
        assert_eq!(LADDER[Mode::named("yolo").allows()], "standard", "an unknown mode reads as ask");
    }

    #[test]
    fn the_agent_token_is_lifted_off_the_arguments() {
        let mut args = serde_json::json!({"command": "ls", "agent_token": "x"});
        let params = serde_json::json!({"agent_token": "  tok-b  "});
        assert_eq!(agent_token_of(&params, &mut args).as_deref(), Some("tok-b"));
        assert_eq!(args, serde_json::json!({"command": "ls"}));
        let mut args = serde_json::json!({});
        assert_eq!(agent_token_of(&serde_json::json!({"agent_token": " "}), &mut args), None, "blank is none");
    }

    // ── Across the hop, over a real socket ──

    /// The socket the tests below talk to: one served surface per test binary, because the UI
    /// stand-in is one per binary (see `test_ui_thread`). `who` reports the caller as the handler
    /// sees it; `slow` finishes its answer off the UI thread with [`answer_later`]; `echo` hands back
    /// its arguments and agent token; `nuke` is graded off the ladder, for the ceiling.
    #[cfg(unix)]
    fn served_test_surface() -> &'static str {
        use std::os::unix::net::UnixStream;
        use std::sync::OnceLock;

        static ADDRESS: OnceLock<String> = OnceLock::new();
        ADDRESS.get_or_init(|| {
            const APP: &str = "caller-test";

            // The server binds wherever `XDG_RUNTIME_DIR` points when its thread gets there, and
            // the connect below looks wherever it points then. Nothing else may move it in between
            // — see `env_lock` — and it points at a directory this test owns, not at the runner's
            // `/run/user/<uid>`, which need not exist on a machine with no login session. Once
            // something has connected, the address is a path and the variable no longer matters.
            let _env = crate::env_lock();
            let runtime =
                std::env::temp_dir().join(format!("yantrik-ui-hop-test-{}", std::process::id()));
            std::fs::create_dir_all(&runtime).expect("a runtime dir of our own");
            std::env::set_var("XDG_RUNTIME_DIR", &runtime);

            test_ui_thread::start(Box::new(|| {
                registry(
                    APP,
                    Some(Box::new(|| View::new("caller-test"))),
                    vec![
                        (
                            // `safe` so the machine ceiling cannot refuse this on a developer's box
                            // that has tightened `tool_permission`; the ceiling has its own tests.
                            Action::new("who", "Report who is calling").risk("safe"),
                            Box::new(|_| {
                                // The handler's own view, on the thread the handler actually runs
                                // on. If the caller had been left on the socket thread this would
                                // be null.
                                Ok(match caller() {
                                    Some(c) => serde_json::json!({ "pid": c.pid, "uid": c.uid }),
                                    None => serde_json::Value::Null,
                                })
                            }),
                        ),
                        (
                            Action::new("slow", "Take `ms` milliseconds to answer, off the UI thread")
                                .risk("safe")
                                .arg(Param::integer("ms"))
                                .arg(Param::flag("refuse").optional()),
                            Box::new(|args| {
                                let ms = args["ms"].as_u64().unwrap_or(0);
                                let refuse = args["refuse"].as_bool().unwrap_or(false);
                                // Read here, where it is set, and carried into the work.
                                let pid = caller().map(|c| c.pid);
                                let work = move || {
                                    std::thread::sleep(Duration::from_millis(ms));
                                    if refuse {
                                        return Err(format!("refused after {ms} ms"));
                                    }
                                    Ok(serde_json::json!({ "slept_ms": ms, "pid": pid }))
                                };
                                answer_later(work)
                                    .map(|()| serde_json::json!("replaced by the work's own answer"))
                                    .or_else(|work| work())
                            }),
                        ),
                        (
                            // What a handler that records or shows its arguments would record or
                            // show — an approval card, an audit line — and the token beside them.
                            Action::new("echo", "Answer with the arguments and the agent token as the handler got them")
                                .risk("safe")
                                .arg(Param::text("command").optional()),
                            Box::new(|args| {
                                Ok(serde_json::json!({ "args": args, "agent_token": agent_token() }))
                            }),
                        ),
                        (
                            // Graded off the ladder, so the ceiling refuses it whatever the machine
                            // running the tests has in its settings file.
                            Action::new("nuke", "Refused by the ceiling on every machine").risk("catastrophic"),
                            Box::new(|_| Ok(serde_json::json!("never reached"))),
                        ),
                    ],
                )
            }));
            serve_rpc(APP, 4);

            // Thirty seconds is a bound on a hung server, not a budget for a slow one: the server
            // binds on its own thread after building a tokio runtime, and the failure this loop
            // used to report was never slowness but the environment race described at `env_lock`.
            let address = RpcServer::default_address(&service_id_for(APP));
            for _ in 0..1000 {
                if UnixStream::connect(&address).is_ok() {
                    return address;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            panic!("nothing ever bound {address}");
        })
    }

    /// One JSON-RPC line out, one back, on a connection of its own.
    #[cfg(unix)]
    fn call(request: &str) -> serde_json::Value {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;

        let mut socket = UnixStream::connect(served_test_surface()).expect("connect");
        socket.write_all(format!("{request}\n").as_bytes()).expect("write the request");
        let mut line = String::new();
        BufReader::new(socket).read_line(&mut line).expect("read the reply");
        serde_json::from_str(&line).expect(&line)
    }

    /// The checker an author runs (`yos check`, docs/surface-protocol.md) against this dispatch,
    /// over a real socket: the protocol written down, read by a program in another language, and
    /// this code agreeing with it refusal for refusal. The served surface grades `nuke` off the
    /// ladder on purpose, so the checker has to fail exactly the two checks that say so and pass
    /// every other — including the missing, undeclared, mistyped and stale probes, which only a
    /// surface that answered like the protocol's dispatch is sent.
    #[cfg(unix)]
    #[test]
    fn yos_check_reads_this_dispatch_as_the_protocol() {
        let address = served_test_surface();
        let yos = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os/yos");
        let run = std::process::Command::new("python3").arg(&yos).args(["check", address, "--json"]).output();
        let Ok(run) = run else {
            eprintln!("skipped: no python3 to run yos check with");
            return;
        };
        let text = String::from_utf8_lossy(&run.stdout);
        let report: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("yos check --json said {text} ({e}); stderr: {}", String::from_utf8_lossy(&run.stderr)));
        let rows = report["surfaces"][0]["checks"].as_array().cloned().unwrap_or_default();
        let status = |name: &str| {
            rows.iter().find(|r| r["check"] == name).map(|r| r["status"].as_str().unwrap_or("").to_string())
        };
        let failed: Vec<String> = rows
            .iter()
            .filter(|r| r["status"] == "fail")
            .map(|r| format!("{}: {}", r["check"], r["saw"]))
            .collect();
        assert_eq!(
            rows.iter().filter(|r| r["status"] == "fail").map(|r| r["check"].as_str().unwrap_or("")).collect::<Vec<_>>(),
            vec!["schema", "grades"],
            "only the deliberately off-ladder `nuke` fails: {failed:#?}"
        );
        for check in [
            "ping", "describe", "protocol", "params", "secrets", "revision", "method", "empty", "unknown",
            "missing", "undeclared", "types", "stale",
        ] {
            assert_eq!(status(check).as_deref(), Some("pass"), "{check}: {text}");
        }
        assert_eq!(run.status.code(), Some(1), "a failed check is a non-zero exit");
    }

    /// A person's Allow is not used up on a call its own arguments refuse. Over the real socket, on
    /// the window's door: a grant for `slow {"ms": 10}` carried by a call whose `ms` is not an
    /// integer is refused for the argument — before the ceiling, before the spend — and is still
    /// whole afterwards: the same grant with the arguments right runs, once. A grant bound to the
    /// arguments as sent holds for a call that sends them so, and the handler reads them converted.
    #[cfg(unix)]
    #[test]
    fn a_malformed_call_is_refused_before_its_grant_is_spent_on_the_window_door() {
        spend_through_a_stand_in_shell();
        allow("window-slow", "caller-test", "slow", serde_json::json!({"ms": 10}));
        let act = |args: serde_json::Value, grant: &str| {
            call(&serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "app.act",
                "params": { "action": "slow", "args": args, "grant": grant },
            })
            .to_string())
        };

        let reply = act(serde_json::json!({"ms": "ten"}), "window-slow");
        assert_eq!(
            reply["error"]["message"], "`slow` argument `ms` must be an integer, and a string arrived",
            "{reply}"
        );
        let reply = act(serde_json::json!({"ms": 10, "loud": true}), "window-slow");
        assert_eq!(reply["error"]["message"], "`slow` has no argument `loud`; it takes: ms, refuse", "{reply}");
        assert!(!spent("window-slow"), "refused for its arguments, and the Allow was used up anyway");

        let reply = act(serde_json::json!({"ms": 10}), "window-slow");
        assert_eq!(reply["result"]["result"]["slept_ms"], 10, "the same grant, the arguments right: {reply}");
        assert!(spent("window-slow"));

        // Bound to the arguments as sent: `"10"` on the card is `"10"` on the call, and the
        // handler reads 10.
        allow("window-slow-text", "caller-test", "slow", serde_json::json!({"ms": "10"}));
        let reply = act(serde_json::json!({"ms": "10"}), "window-slow-text");
        assert_eq!(reply["result"]["result"]["slept_ms"], 10, "{reply}");
        assert!(spent("window-slow-text"));
    }

    /// Agents catalog: an agent started from a role is held to the role's reach on the real
    /// dispatch — its token names a reach (here through an installed reader, as the shell installs
    /// its own registry), an act on its surfaces runs, one off them is refused in the reach's words
    /// before the handler and before any grant is spent, and a token with no reach is not held.
    #[cfg(unix)]
    #[test]
    fn an_agent_is_held_to_its_reach_on_the_socket_before_any_grant_is_spent() {
        use yantrik_ipc_transport::reach;

        spend_through_a_stand_in_shell();
        reach::read_reach_with(|token| {
            (token == "tok-reach-reviewer").then(|| reach::Reach {
                agent: "deepseek:c-reach1".into(),
                role: "reviewer".into(),
                name: "Reviewer".into(),
                surfaces: vec!["caller-test.echo".into(), "caller-test.nuke".into()],
                ceiling: "safe".into(),
            })
        });
        let act = |action: &str, token: &str, grant: Option<&str>| {
            let mut params = serde_json::json!({ "action": action, "args": {}, "agent_token": token });
            if let Some(grant) = grant {
                params["grant"] = grant.into();
            }
            call(&serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "app.act", "params": params }).to_string())
        };

        let reply = act("echo", "tok-reach-reviewer", None);
        assert_eq!(reply["result"]["result"]["agent_token"], "tok-reach-reviewer", "on its surfaces it runs: {reply}");

        let reply = act("who", "tok-reach-reviewer", None);
        let err = reply["error"]["message"].as_str().unwrap_or_default();
        assert!(err.starts_with("REACH: caller-test.who is outside the Reviewer's reach"), "{reply}");
        assert!(err.contains("`deepseek:c-reach1` is the Reviewer"), "{err}");
        assert_eq!(reply["error"]["code"], -32602, "a policy answer, not a transport fault");

        // A grade off the ladder is refused by the reach before the machine's ceiling is asked.
        let reply = act("nuke", "tok-reach-reviewer", None);
        assert!(reply["error"]["message"].as_str().unwrap_or_default().starts_with("REACH: caller-test.nuke is graded"), "{reply}");

        // Refused by the reach, a person's Allow is not used up on it.
        let reply = act("who", "tok-reach-reviewer", Some("fresh-reach"));
        assert!(reply["error"]["message"].as_str().unwrap_or_default().starts_with("REACH:"), "{reply}");
        spend_for_render(open(), "fresh-reach", &serde_json::json!({"out": "x.png"}))
            .expect("the reach's refusal spent nothing");

        // Another agent's token, with no reach, is not held; and the person's call has none.
        let reply = act("who", "tok-no-reach", None);
        assert!(reply["error"].is_null(), "{reply}");
        let reply = call(r#"{"jsonrpc":"2.0","id":9,"method":"app.act","params":{"action":"who","args":{}}}"#);
        assert!(reply["error"].is_null(), "{reply}");
    }

    /// See `test_ui_thread` for why the hop is a channel.
    #[cfg(unix)]
    #[test]
    fn the_caller_reaches_the_handler_across_the_ui_hop() {
        use std::os::unix::fs::MetadataExt;

        let reply = call(r#"{"jsonrpc":"2.0","id":1,"method":"app.act","params":{"action":"who","args":{}}}"#);
        let seen = &reply["result"]["result"];
        assert!(
            !seen.is_null(),
            "the handler saw no caller at all — the credentials did not cross the hop: {reply}"
        );
        assert_eq!(
            seen["pid"].as_u64(),
            Some(u64::from(std::process::id())),
            "the kernel's pid for this connection is this test process: {reply}"
        );
        // The uid the kernel reported has to be the uid that owns the socket — this test is both
        // ends of the connection, so anything else means the field is not the peer's.
        let owner = std::fs::metadata(served_test_surface()).expect("the socket exists").uid();
        assert_eq!(seen["uid"].as_u64(), Some(u64::from(owner)), "{reply}");
        assert!(reply["result"]["action_id"].as_str().unwrap().starts_with("app-caller-test#"), "{reply}");
    }

    /// The shell's `agent_run` owes its caller an exit code that may be minutes away. The handler
    /// hands the wait to `answer_later`; the caller gets the work's own result, and in the
    /// meantime the socket and the UI thread both go on answering everybody else.
    #[cfg(unix)]
    #[test]
    fn an_answer_that_takes_time_is_finished_off_the_ui_thread_and_holds_up_nobody() {
        use std::time::Instant;

        served_test_surface();
        let asked = Instant::now();
        let slow = std::thread::spawn(|| {
            call(r#"{"jsonrpc":"2.0","id":1,"method":"app.act","params":{"action":"slow","args":{"ms":1500}}}"#)
        });

        // While that one waits: another caller, another connection, served at once. `describe`
        // runs on the UI stand-in, so this also shows the UI thread is not the one waiting.
        std::thread::sleep(Duration::from_millis(200));
        let glance = Instant::now();
        let described = call(r#"{"jsonrpc":"2.0","id":2,"method":"app.describe","params":{}}"#);
        assert_eq!(described["result"]["app"], "caller-test", "{described}");
        assert!(
            glance.elapsed() < Duration::from_millis(700),
            "a describe waited {:?} behind a slow act",
            glance.elapsed()
        );

        let reply = slow.join().expect("the slow call");
        assert!(asked.elapsed() >= Duration::from_millis(1500), "the reply is the finished work");
        assert_eq!(reply["result"]["accepted"], true, "{reply}");
        assert_eq!(reply["result"]["result"]["slept_ms"], 1500, "the work's result, not the handler's: {reply}");
        assert_eq!(
            reply["result"]["result"]["pid"].as_u64(),
            Some(u64::from(std::process::id())),
            "the caller read in the handler reached the work: {reply}"
        );
        assert!(reply["result"]["revision"].as_str().is_some(), "the envelope keeps its view: {reply}");

        // Work that refuses is refused to the caller, as a handler's refusal would be.
        let refused = call(
            r#"{"jsonrpc":"2.0","id":3,"method":"app.act","params":{"action":"slow","args":{"ms":10,"refuse":true}}}"#,
        );
        assert_eq!(refused["error"]["message"], "refused after 10 ms", "{refused}");
        assert_eq!(refused["error"]["code"], -32602, "an application refusal, not a transport fault");

        // And a wrong type is refused on the socket as it is in the dispatch, before the handler.
        let mistyped = call(
            r#"{"jsonrpc":"2.0","id":4,"method":"app.act","params":{"action":"slow","args":{"ms":"soon"}}}"#,
        );
        assert_eq!(
            mistyped["error"]["message"],
            "`slow` argument `ms` must be an integer, and a string arrived",
            "{mistyped}"
        );
        assert_eq!(mistyped["error"]["code"], -32602);
    }

    /// The token rides beside `args` and reaches the handler through `agent_token()`; `args` —
    /// what an approval card shows and an audit line keeps — never holds it, even when a caller
    /// puts it there.
    #[cfg(unix)]
    #[test]
    fn an_agent_token_reaches_the_handler_beside_the_arguments_and_never_among_them() {
        let reply = call(
            r#"{"jsonrpc":"2.0","id":1,"method":"app.act","params":{"action":"echo","args":{"command":"ls"},"agent_token":"tok-7f3a"}}"#,
        );
        let seen = &reply["result"]["result"];
        assert_eq!(seen["agent_token"], "tok-7f3a", "the handler reads the token: {reply}");
        assert_eq!(seen["args"], serde_json::json!({"command": "ls"}), "and its args are only args: {reply}");

        // Smuggled inside `args` as well: taken out, not used, and not refused as an undeclared
        // argument either — the call goes on as if it had never been there.
        let reply = call(
            r#"{"jsonrpc":"2.0","id":2,"method":"app.act","params":{"action":"echo","args":{"command":"ls","agent_token":"smuggled"},"agent_token":"tok-7f3a"}}"#,
        );
        assert_eq!(reply["result"]["result"]["args"], serde_json::json!({"command": "ls"}), "{reply}");
        assert_eq!(reply["result"]["result"]["agent_token"], "tok-7f3a", "the one beside args wins: {reply}");
        assert!(!reply.to_string().contains("smuggled"), "nothing in the reply carries it: {reply}");

        // Only inside `args`: stripped, and the handler sees no token at all.
        let reply = call(
            r#"{"jsonrpc":"2.0","id":3,"method":"app.act","params":{"action":"echo","args":{"agent_token":"smuggled"}}}"#,
        );
        assert_eq!(reply["result"]["result"]["args"], serde_json::json!({}), "{reply}");
        assert!(reply["result"]["result"]["agent_token"].is_null(), "{reply}");

        // No token, no token.
        let reply = call(r#"{"jsonrpc":"2.0","id":4,"method":"app.act","params":{"action":"echo","args":{}}}"#);
        assert!(reply["result"]["result"]["agent_token"].is_null(), "{reply}");
    }

    /// #154 through the real dispatch: the RPC thread asks the UI thread for the grade before it
    /// offers a grant to the shell, so an act the ceiling refuses, and an action the app does not
    /// have, spend nothing. Before, both came back `GRANT: … does not authorise …` — the grant had
    /// already gone to the shell by the time anything looked at the action.
    #[cfg(unix)]
    #[test]
    fn a_grant_on_the_socket_is_offered_to_the_shell_only_past_the_ceiling() {
        spend_through_a_stand_in_shell();
        let act = |action: &str, args: serde_json::Value, grant: &str| {
            let request = serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "app.act",
                "params": { "action": action, "args": args, "grant": grant },
            });
            let reply = call(&request.to_string());
            reply["error"]["message"].as_str().unwrap_or_default().to_string()
        };

        let err = act("nuke", serde_json::json!({}), "fresh-socket");
        assert!(err.starts_with("CEILING:"), "the ceiling's answer, not the shell's: {err}");

        let err = act("nope", serde_json::json!({}), "fresh-socket");
        assert!(err.starts_with("unknown action `nope`"), "the app's answer, not the shell's: {err}");

        // Past the ceiling the grant does go to the shell, and one that does not hold ends the
        // call in the shell's words.
        let err = act("who", serde_json::json!({}), "made-up");
        assert!(err.starts_with("GRANT:") && err.contains("no approval request"), "{err}");

        // What a grant is spent against is the arguments with any agent token a caller put among
        // them already lifted off (see `agent_token`): the shell is shown `{"command":"ls"}` and
        // the grant is bound to that, never to a token. The stand-in shell names what it was
        // handed, which is how this can be seen from here.
        let err = act("echo", serde_json::json!({"command": "ls", "agent_token": "smuggled"}), "fresh-echo");
        assert!(err.starts_with("GRANT:") && err.contains(r#"this call carries {"command":"ls"}"#), "{err}");
        assert!(!err.contains("smuggled"), "the token reached the shell as an argument: {err}");

        // And the grant the two refusals carried was never spent.
        spend_for_render(open(), "fresh-socket", &serde_json::json!({"out": "x.png"}))
            .expect("nothing spent `fresh-socket` on the way to either refusal");
    }
}
