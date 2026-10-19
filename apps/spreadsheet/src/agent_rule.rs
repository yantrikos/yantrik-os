//! Which files an agent may have ySheets read and write (#443), by the rule the Editor and
//! Images already follow (`yantrik_ipc_contracts::home_paths`).
//!
//! The sheet runs as the person, and `get_range` hands back what a file holds. So `open` for an
//! agent is limited to files in the person's home outside its protected places, and `save` and
//! `save_over` to the same: `~/.bashrc` is code run as the person at their next login, and it is
//! on the protected list. The person, choosing a file in the window or calling with no token,
//! is not asked.

use std::path::{Path, PathBuf};

use yantrik_app_runtime::control::agent_is_calling;
use yantrik_ipc_contracts::home_paths;

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

pub fn may_read(path: &Path) -> Result<(), String> {
    if !agent_is_calling() {
        return Ok(());
    }
    home_paths::may_read_file(&path.to_string_lossy(), &home())
}

pub fn may_write(path: &Path) -> Result<(), String> {
    if !agent_is_calling() {
        return Ok(());
    }
    home_paths::may_write_file(&path.to_string_lossy(), &home())
}

/// Why the open file's cells are kept from this caller, if they are: a file the person opened
/// from a place no agent may read is not read back or saved for one (`surface::run`), and
/// `describe` leaves its cells, status line and notice out.
pub fn hidden_from_caller(path: Option<&Path>) -> Option<String> {
    if !agent_is_calling() {
        return None;
    }
    home_paths::may_read_file(&path?.to_string_lossy(), &home()).err()
}
