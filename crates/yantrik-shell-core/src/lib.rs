//! Yantrik Shell Core — app registry, screen routing, .desktop file scanning.
//!
//! Pure Rust, no Slint dependency. Provides the shell's understanding of
//! installed applications, built-in apps, and screen ID routing.

pub mod agent_open;
pub mod apps;
pub mod mind_windows;
pub mod office;
pub mod screens;
pub mod service_manager;
