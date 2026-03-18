//! Yantrik IPC Contracts — service interface types and traits.
//!
//! This crate defines the **data shapes** and **operation signatures** that flow
//! between Yantrik services and the shell UI. All types are serde-serializable
//! for JSON-RPC transport (Phase 1) and future Cap'n Proto migration (Phase 3).
//!
//! No UI code, no backend logic — pure contracts only.

pub mod email;
pub mod calendar;
pub mod weather;
pub mod notes;
pub mod music;
pub mod system_monitor;
pub mod network;
pub mod notifications;
