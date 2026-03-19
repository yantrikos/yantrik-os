//! Chat provider implementations.
//!
//! Each provider implements the ChatProvider trait and runs in its own thread.
//! Providers are transport-only — no AI logic.

pub mod telegram;
pub mod discord;
pub mod matrix;
pub mod irc;
pub mod slack;
pub mod whatsapp;
pub mod signal;
