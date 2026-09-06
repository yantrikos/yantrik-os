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
//!     init_tracing("notes");
//!     let app = NotesApp::new().unwrap();
//!     // wire callbacks...
//!     app.run().unwrap();
//! }
//! ```

// ── Re-exports ──────────────────────────────────────────────────────
pub use slint;
pub use serde_json;
pub use tracing;
pub use yantrik_ipc_contracts;
pub use yantrik_ipc_transport;

pub use yantrik_ipc_transport::SyncRpcClient;

pub mod companion;
pub mod control;
pub mod instance;
pub mod theme;

/// Commonly-needed imports for app authors.
pub mod prelude {
    pub use crate::{companion, control, init_tracing, instance, theme, SyncRpcClient};
    pub use serde_json;
    pub use slint;
    pub use tracing;
}

/// Initialize tracing-subscriber with an env filter for the app.
pub fn init_tracing(app_name: &str) {
    let crate_name = app_name.replace('-', "_");
    let directive = format!("{crate_name}=info");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(directive.parse().expect("valid tracing directive"))
                // The runtime's own lines (instance guard, theme) must be visible too, or an
                // app that exits at once because another instance holds the slot says nothing.
                .add_directive("yantrik_app_runtime=info".parse().expect("valid directive")),
        )
        .init();
}

// Note: build-time helpers (slint_config) live in each app's build.rs
// since slint_build is a build-dependency, not a runtime dependency.
