//! Everything the editor does on the disk. Each function here runs in a job, never on the
//! window's thread, and returns what it found as plain data for the command that carries it back.

pub mod answer;
pub mod drafts;
pub mod kept;
pub mod legacy;
pub mod locate;
pub mod open;
pub mod read;
pub mod save;

use std::path::PathBuf;

/// `~/` at the start of a path, as the person and the Mind both write it.
pub fn expanded(value: &str) -> PathBuf {
    match value.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(value),
    }
}
