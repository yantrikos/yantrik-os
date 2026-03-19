//! Yantrik Chat — Universal chat transport layer.
//!
//! Provides a unified `ChatProvider` trait, canonical event model, background
//! router, per-conversation policy engine, and SQLite-backed conversation store.
//! Each provider runs in its own thread using blocking I/O — no async runtime.

pub mod model;
pub mod provider;
pub mod store;
pub mod router;
pub mod policy;
pub mod manager;
pub mod providers;

pub use model::*;
pub use provider::*;
pub use policy::{ConversationPolicy, ReplyMode};
