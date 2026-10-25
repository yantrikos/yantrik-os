//! YantrikApp: what every app's process is made of (design/app-framework-2026-10-21.md).
//!
//! Step 1 of that design moved `yantrik-app-runtime`'s modules here unchanged; step 2 adds the
//! `App` trait, the runner, the bridge, effects, jobs and timers beside them. Apps still reach
//! the old modules through `yantrik_app_runtime`, which re-exports every one, until each moves
//! in its own framework pass.

pub use slint;

pub mod app;
pub mod boot;
pub mod chrome;
pub mod companion;
pub mod control;
pub mod cx;
pub mod desktop;
pub mod documents;
pub mod drafts;
pub mod effects;
pub mod event_loop;
pub mod instance;
pub mod intents;
pub mod jobs;
pub mod launch;
pub mod lock;
pub mod paths;
pub mod persist;
pub mod present;
pub mod queue;
pub mod quit;
pub mod rail;
pub mod refused;
pub mod rows;
pub mod runner;
pub mod services;
pub mod settings;
pub mod shown;
pub mod theme;
pub mod timers;
pub mod watch;
pub mod window;

pub use app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
pub use boot::init_tracing;
pub use cx::Cx;
pub use desktop::{DesktopOutcome, DesktopRequest};
pub use drafts::Drafts;
// The Mind and notifications, typed, as their own crates (step 5); named here so an app has
// one dependency.
pub use yantrik_mind as mind;
pub use yantrik_notifications as notifications;
pub use yantrik_notifications::Notification;
pub use effects::Effects;
pub use event_loop::run_until_closed;
pub use intents::{Agents, Args, Intent, Intents, InvocationId, Origin, Settle};
pub use jobs::{Cancel, Concurrency, JobId, JobKey, Scope};
pub use launch::Launch;
pub use paths::Paths;
pub use rail::{Rail, RailCommand};
pub use refused::{Explain, Refused};
pub use runner::run;
pub use services::Services;
pub use settings::{Health, Settings, SettingsError};
pub use timers::TimerId;
pub use window::Window;
// Moved to Foundation (L0): the one rule for an address, and the problem records every process
// writes. Named here so the modules above keep their `crate::problems` paths.
pub use yantrik_foundation::{place, problems};
// The machine's ceiling and mode, as the rail and a test of it name them.
pub use yantrik_ipc_transport::gate;

/// Held by every test in this crate that sets `XDG_RUNTIME_DIR`, or binds a socket under it.
///
/// The environment is process-wide and the test harness runs tests on parallel threads. A test
/// that had started a server under the runner's `/run/user/<uid>` then looked for its socket
/// where the variable pointed *now*, and lost that race on four pull requests in one night. A
/// poisoned lock is fine to take: whatever the last holder panicked about was its own business.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
