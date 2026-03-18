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
pub mod sync_client;

pub use protocol::{RpcRequest, RpcResponse, RpcError, RPC_PARSE_ERROR, RPC_METHOD_NOT_FOUND, RPC_INTERNAL_ERROR};
pub use server::{RpcServer, ServiceHandler};
pub use client::RpcClient;
pub use sync_client::SyncRpcClient;
