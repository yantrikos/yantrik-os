//! Yantrik App Runtime — shared infrastructure for standalone app binaries.
//!
//! Provides:
//! - `AppBuilder` for setting up standalone Slint app windows
//! - IPC client for communicating with the shell and services
//! - Common re-exports (Slint, serde_json, tracing, IPC types)
//!
//! # Quick start
//!
//! ```rust,ignore
//! use yantrik_app_runtime::prelude::*;
//!
//! slint::include_modules!();
//!
//! fn main() {
//!     init_tracing("yantrik-notes");
//!     let app = NotesApp::new().unwrap();
//!     // wire callbacks...
//!     // Not `run().unwrap()`: a logout ends the loop with an error, and that is not a crash.
//!     run_until_closed(&app, "yantrik-notes");
//!     // shutdown: runs however the loop ended
//! }
//! ```

// ── Re-exports ──────────────────────────────────────────────────────
pub use slint;
pub use serde_json;
pub use tracing;
pub use yantrik_ipc_contracts;
pub use yantrik_ipc_transport;

pub use yantrik_ipc_transport::SyncRpcClient;

pub use event_loop::{run_until_closed, run_until_ended, LoopEnd};

pub mod companion;
pub mod control;
pub mod event_loop;
pub mod instance;
pub mod notify;
pub mod problems;
pub mod service;
pub mod theme;

/// Held by every test in this crate that sets `XDG_RUNTIME_DIR`, or binds a socket under it.
///
/// The environment is process-wide and the test harness runs tests on parallel threads. Two
/// `notify` tests point the variable at `/tmp` for their own reasons; a test that had started
/// a server under the runner's `/run/user/<uid>` then looked for its socket where the variable
/// pointed *now*. On GitHub's runners that lost the race on four pull requests in one night,
/// none of which touched this crate, and passed on every rerun. A poisoned lock is fine to
/// take: whatever the last holder panicked about was its own business.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Commonly-needed imports for app authors.
pub mod prelude {
    pub use crate::{
        companion, control, init_tracing, instance, notify, run_until_closed, service, theme, SyncRpcClient,
    };
    pub use serde_json;
    pub use slint;
    pub use tracing;
}

/// Initialize tracing-subscriber with an env filter for the app.
pub fn init_tracing(app_name: &str) {
    // `--version`, answered here because this is the one line every app binary runs first.
    //
    // A machine used to give three answers about what it was, one of them a hardcoded
    // `Yantrik Terminal v0.1.0` printed into the terminal's first pane. Sixteen app mains each
    // reporting their own `CARGO_PKG_VERSION` is the same defect with more places to forget, so
    // the flag is handled once, from the string yantrik-version resolves. Same reasoning as the
    // SLINT_FULLSCREEN line below: the shared line is the only place a rule holds everywhere.
    yantrik_version::handle_version_flag(app_name);

    // An app is not the OS.
    //
    // The session exports SLINT_FULLSCREEN=1 because the shell IS the desktop and must not be a
    // window on something else. Children inherit the environment, so any launch path that
    // forgets to strip it hands an app a fullscreen window with no titlebar, no taskbar and no
    // way to close it. That happened: the Apps grid spawned a bare Command, and Notes opened
    // over the whole screen with nothing to press.
    //
    // The launcher strips it, and now so does this. Belt and braces, because the cost of the
    // belt failing is a window the user cannot get out of, and the cost of this line is
    // nothing: no app in this OS ever wants to start fullscreen.
    std::env::remove_var("SLINT_FULLSCREEN");

    let crate_name = app_name.replace('-', "_");
    let directive = format!("{crate_name}=info");

    // A panic becomes a problem record before it becomes a stack trace on stderr. The record
    // is local and scrubbed; nothing sends it. See `problems`.
    problems::install_panic_hook(app_name);

    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(directive.parse().expect("valid tracing directive"))
        // The runtime's own lines (instance guard, theme) must be visible too, or an
        // app that exits at once because another instance holds the slot says nothing.
        .add_directive("yantrik_app_runtime=info".parse().expect("valid directive"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        // The same lines, kept in a small ring so a problem record can carry the last forty.
        .with(RecentLines)
        .init();
}

/// A tracing layer that remembers the most recent log lines of this process, for the
/// `log_tail` of a problem record. It formats nothing to a sink; `problems::note_log_line` holds
/// a bounded ring and that is all.
struct RecentLines;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for RecentLines {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        struct Line(String);
        impl tracing::field::Visit for Line {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write as _;
                if field.name() == "message" {
                    let _ = write!(self.0, "{value:?}");
                } else {
                    let _ = write!(self.0, " {}={:?}", field.name(), value);
                }
            }
        }
        let mut line = Line(String::new());
        event.record(&mut line);
        let meta = event.metadata();
        problems::note_log_line(&format!("{} {}: {}", meta.level(), meta.target(), line.0));
    }
}

// Note: build-time helpers (slint_config) live in each app's build.rs
// since slint_build is a build-dependency, not a runtime dependency.
