//! Yantrik App Runtime — shared infrastructure for standalone app binaries.
//!
//! Being replaced by `yantrik-app` (design/app-framework-2026-10-21.md). Most of what is
//! here is a re-export of it; this crate takes no new features.
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

// Moved to frameworks/yantrik-app and frameworks/yantrik-foundation (design/app-framework-2026-10-21.md,
// step 1), re-exported here unchanged so no app changes until its own framework pass.
pub use yantrik_app::{chrome, companion, control, event_loop, instance, present, run_until_closed, theme, window_chrome};
pub use yantrik_foundation::{place, problems};

/// Notifications moved to their own crate (the app framework's step 5); the name stays.
pub use yantrik_notifications as notify;
pub mod service;

/// Commonly-needed imports for app authors.
pub mod prelude {
    pub use crate::{
        companion, control, init_tracing, instance, notify, run_until_closed, service, theme, SyncRpcClient,
    };
    pub use serde_json;
    pub use slint;
    pub use tracing;
}

pub use yantrik_app::boot::init_tracing;

// Note: build-time helpers (slint_config) live in each app's build.rs
// since slint_build is a build-dependency, not a runtime dependency.
