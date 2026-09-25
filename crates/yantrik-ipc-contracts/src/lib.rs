//! Yantrik IPC Contracts — service interface types and traits.
//!
//! This crate defines the **data shapes** and **operation signatures** that flow
//! between Yantrik services and the shell UI. All types are serde-serializable
//! for JSON-RPC transport (Phase 1) and future Cap'n Proto migration (Phase 3).
//!
//! No UI code, no backend logic — pure contracts only.

/// `companion.ask` answers with this code when the shell replied but no model did.
///
/// The distinction earns its own code because the fallback is not an answer: with no model
/// behind it the companion produces plausible-looking canned text, and an app that cannot tell
/// the two apart shows that text as the model's words — and offers to write it into documents.
/// An app that sees this code says no model answered — none set up, or the one set up did not
/// answer — and changes nothing. (A server-defined code in JSON-RPC's reserved range; -32001 is
/// already the mail service's "no account".)
pub const ERR_NO_MODEL: i32 = -32002;

pub mod email;
pub mod calendar;
pub mod weather;
pub mod notes;
pub mod music;
pub mod system_monitor;
pub mod network;
pub mod notifications;
pub mod control_surface;
