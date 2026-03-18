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

/// Commonly-needed imports for service authors.
pub mod prelude {
    pub use crate::{run_service, ServiceBuilder, ServiceError};
    pub use crate::{RpcServer, ServiceHandler};
    pub use serde_json;
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
fn init_tracing(service_id: &str) {
    // Convert "my-service" to "my_service" for the Rust module tracing filter.
    let crate_name = service_id.replace('-', "_");
    let directive = format!("{crate_name}=info");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(directive.parse().expect("valid tracing directive")),
        )
        .init();
}
