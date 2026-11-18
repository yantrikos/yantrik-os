//! Everything the app does on the disk. Each function here runs in a job, never on the window's
//! thread, and returns what it found as plain data for the command that carries it back.

pub mod drafts;
pub mod legacy;
pub mod locate;
pub mod open;
pub mod read;
pub mod rules;
pub mod save;

// Keeping another program's version is the framework's.
pub use yantrik_app::documents::files::kept;

use std::path::PathBuf;

/// `~/` at the start of a path, as the person and the Mind both write it.
pub fn expanded(value: &str) -> PathBuf {
    match value.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(value),
    }
}
