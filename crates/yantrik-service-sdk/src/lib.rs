//! Yantrik Service SDK — eliminates boilerplate for standalone services.
//!
//! # Quick start
//!
//! ```rust,ignore
//! use yantrik_service_sdk::prelude::*;
//!
//! struct MyHandler;
//!
//! impl ServiceHandler for MyHandler {
//!     fn service_id(&self) -> &str { "my-service" }
//!     fn handle(&self, method: &str, params: serde_json::Value)
//!         -> Result<serde_json::Value, ServiceError> {
//!         todo!()
//!     }
//! }
//!
//! fn main() {
//!     ServiceBuilder::new("my-service")
//!         .handler(MyHandler)
//!         .run();
//! }
//! ```

use std::sync::Arc;

// ── Re-exports ──────────────────────────────────────────────────────
pub use serde_json;
pub use yantrik_ipc_contracts::email::ServiceError;
pub use yantrik_ipc_transport::server::{RpcServer, ServiceHandler};
/// The ceiling, mode and grant rule every `app.act` meets (docs/surface-protocol.md, section 7).
/// A [`Surface`] calls it for you, with the grade and the description it publishes, in the same
/// order and the same words as an app window's dispatch; it is here for a service that needs to
/// read the same files or spend a grant the same way.
pub use yantrik_ipc_transport::gate;
/// An agent's reach: what a role from the agent catalog may touch. A [`Surface`] holds every
/// call that carries an agent token to it, before any grant is spent and before the handler, as
/// an app window's dispatch does; it is here for a service that needs to read it itself.
pub use yantrik_ipc_transport::reach;

/// A service's control surface: `app.describe` and `app.act`, dispatched exactly as an app
/// window's are — the gate, the argument checks (present, known, typed), the revision guard,
/// stable per-call `action_id`s — with no UI dependency. Build a [`Surface`] from the service's
/// actions and hand it the two methods from `handle_from` with [`Surface::answer`]; the rest of
/// the service's methods stay its own.
///
/// ```rust,ignore
/// use yantrik_service_sdk::prelude::*;
/// use yantrik_service_sdk::{PeerCred, Surface};
///
/// struct Weather { surface: Surface }
///
/// impl ServiceHandler for Weather {
///     fn service_id(&self) -> &str { "weather" }
///     fn handle(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, ServiceError> {
///         self.handle_from(method, params, None)
///     }
///     fn handle_from(&self, method: &str, params: serde_json::Value, peer: Option<PeerCred>)
///         -> Result<serde_json::Value, ServiceError> {
///         if let Some(answer) = self.surface.answer(method, &params, peer) {
///             return answer;
///         }
///         match method { /* the service's own methods */ _ => todo!() }
///     }
/// }
/// ```
pub use yantrik_surface as surface;
pub use yantrik_surface::{
    agent_token, answer_later, caller, Action, Caller, Param, Surface, View,
};
pub use yantrik_ipc_transport::server::PeerCred;

/// A service's raw method that changes something answers only the desktop's own programs (#161):
/// the check `yantrik_ipc_transport::owner::desktop_programs_only` makes, as the refusal a
/// service returns. `-32001`, the code #332 gave the same refusal on the calendar and network.
pub fn desktop_programs_only(peer: Option<PeerCred>, method: &str) -> Result<(), ServiceError> {
    yantrik_ipc_transport::owner::desktop_programs_only(peer, method)
        .map_err(|message| ServiceError { code: -32001, message })
}

/// Commonly-needed imports for service authors.
///
/// The surface types are deliberately not in it: a dozen services glob-import this beside their
/// own contracts, and a second `View` or `Action` arriving by glob is an ambiguity waiting for
/// the first service that names one. Import them by name: `use yantrik_service_sdk::{Surface, …}`.
pub mod prelude {
    pub use crate::{run_service, ServiceBuilder, ServiceError};
    pub use crate::{RpcServer, ServiceHandler};
    pub use serde_json;
    pub use tracing;
}

// ── ServiceBuilder ──────────────────────────────────────────────────

/// Builder for configuring and launching a Yantrik service.
pub struct ServiceBuilder<H = ()> {
    service_id: String,
    handler: H,
}

impl ServiceBuilder<()> {
    /// Create a new builder for a service with the given identifier.
    ///
    /// The `service_id` is used for the default socket/port address and
    /// for the tracing filter directive.
    pub fn new(service_id: &str) -> Self {
        Self {
            service_id: service_id.to_string(),
            handler: (),
        }
    }

    /// Set the handler that implements [`ServiceHandler`].
    pub fn handler<H: ServiceHandler>(self, handler: H) -> ServiceBuilder<H> {
        ServiceBuilder {
            service_id: self.service_id,
            handler,
        }
    }
}

impl<H: ServiceHandler> ServiceBuilder<H> {
    /// Initialize tracing, create a tokio runtime, and start the RPC server.
    ///
    /// This method blocks until the server shuts down.
    pub fn run(self) {
        run_service(&self.service_id, self.handler);
    }
}

// ── Convenience function ────────────────────────────────────────────

/// One-shot helper: sets up tracing + tokio + RPC server and blocks.
///
/// Equivalent to:
/// ```rust,ignore
/// ServiceBuilder::new(id).handler(handler).run();
/// ```
pub fn run_service(id: &str, handler: impl ServiceHandler) {
    init_tracing(id);

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    rt.block_on(async {
        let handler = Arc::new(handler);
        let addr = RpcServer::default_address(id);
        let server = RpcServer::new(&addr);
        tracing::info!(service = id, "Starting service");
        if let Err(e) = server.serve(handler).await {
            tracing::error!(service = id, error = %e, "Service failed");
        }
    });
}

/// Initialize tracing-subscriber with an env filter and a service-level directive.
///
/// Public, and idempotent, because a service that does real work before it starts serving needs
/// to be able to log it. Perception opens privileged descriptors, applies a Landlock ruleset and
/// drops its capabilities *before* `run_service` is ever called — with tracing set up only inside
/// `run_service`, every one of those lines went nowhere, which is how a silently-failed privilege
/// drop stayed invisible.
pub fn init_tracing(service_id: &str) {
    // Convert "my-service" to "my_service" for the Rust module tracing filter.
    let crate_name = service_id.replace('-', "_");

    // Two directives, because the service id and the crate name are usually *not* the same: the
    // `a11y` service lives in the `a11y_service` crate, `perception` in `perception_service`. With
    // only the first, a service's own logs were filtered out by its own logging setup, and the
    // startup of every one of them looked silent unless someone thought to set RUST_LOG.
    let mut filter = if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::EnvFilter::from_default_env()
    } else {
        // A service run by hand should say what it is doing. ERROR-only is the wrong default for
        // a background process someone is watching to find out whether it started.
        tracing_subscriber::EnvFilter::new("info")
    };
    for directive in [format!("{crate_name}=info"), format!("{crate_name}_service=info")] {
        filter = filter.add_directive(directive.parse().expect("valid tracing directive"));
    }

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        // `try_init`, not `init`: calling this twice is not a bug worth a panic. A service that
        // set up logging for its own startup will reach `run_service` with a subscriber already
        // installed, and the right answer there is to keep the one that is working.
        .try_init()
        .ok();
}
