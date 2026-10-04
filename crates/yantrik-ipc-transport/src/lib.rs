//! Yantrik IPC Transport — JSON-RPC 2.0 over Unix domain sockets.
//!
//! Provides async server and client for service communication.
//! Services implement [`ServiceHandler`] to dispatch incoming RPC calls.
//! The shell uses [`RpcClient`] to call service methods.
//!
//! Wire format: newline-delimited JSON (one JSON-RPC message per line).

pub mod protocol;
pub mod server;
pub mod client;
pub mod gate;
pub mod mind_door;
pub mod owner;
pub mod peer_identity;
/// A caller's words as one plain line: no control or bidi characters (#614).
pub mod plain_text;
/// Private mode: while it is on, no agent sees or does anything on this desktop.
pub mod privacy;
// An agent's reach: what a role from the agent catalog may touch (design/desk-and-mind, section 5).
pub mod reach;
pub mod service;
pub mod sync_client;

pub use protocol::{RpcRequest, RpcResponse, RpcError, RPC_PARSE_ERROR, RPC_METHOD_NOT_FOUND, RPC_INTERNAL_ERROR};
pub use server::{PeerCred, RpcServer, ServiceHandler};
pub use client::RpcClient;
pub use sync_client::SyncRpcClient;
