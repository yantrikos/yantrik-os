//! App registry — re-exports from yantrik-shell-core.
//!
//! All app registry logic (DesktopEntry, builtin_apps, scan, search)
//! lives in yantrik-shell-core. This module re-exports for backward compatibility.

pub use yantrik_shell_core::apps::*;
