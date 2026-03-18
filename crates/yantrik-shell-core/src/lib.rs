//! Yantrik Shell Core — app registry, screen routing, .desktop file scanning.
//!
//! Pure Rust, no Slint dependency. Provides the shell's understanding of
//! installed applications, built-in apps, and screen ID routing.

pub mod apps;
pub mod screens;
pub mod service_manager;
