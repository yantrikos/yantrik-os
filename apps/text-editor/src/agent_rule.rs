//! Which files an agent may have the editor read and write (#443).
//!
//! The editor runs as the person, and `describe editor` hands back the first 4000 characters of
//! the tab in front. So `open` for an agent was reading any file the person can read
//! (`~/.ssh/id_ed25519`, then `describe`), and `save_as` was writing any file the person can
//! write: `~/.bashrc` with `overwrite=true` is code run as the person at their next login. An
//! agent names only files in the person's home outside its protected places, by the rule every
//! side shares (`yantrik_ipc_contracts::home_paths`). The person, choosing a file in the window
//! or with no token on the call, is not asked.
//!
//! `save_as` stays graded `standard`: a grade is fixed per action, and raising it would put the
//! ordinary new-tab-then-`save_as` a mind uses to write its work behind an approval. What made
//! overwriting dangerous was where, not whether: the files that run as the person are on the
//! protected list, so an agent may replace a file only where it may write one at all.
//!
//! Each rule looks at the disk (whether the path is there, a file, a link), so each runs in the
//! job that opens or saves, never on the window's thread.

use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::home_paths;

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Refuse an agent's `open` of anything but a file in the home, outside its protected places.
pub fn may_read(agent: bool, path: &Path) -> Result<(), String> {
    if !agent {
        return Ok(());
    }
    home_paths::may_read_file(&path.to_string_lossy(), &home())
}

/// Refuse an agent's `save_as` or `save` to anywhere but a file in the home, outside its
/// protected places, in a folder already there, and not through a link.
pub fn may_write(agent: bool, path: &Path) -> Result<(), String> {
    if !agent {
        return Ok(());
    }
    home_paths::may_write_file(&path.to_string_lossy(), &home())
}

/// Whether a tab's file is one no agent is shown: a place an agent may not read. Decided when the
/// tab takes the file (opened, saved as, followed after a move) and kept with the tab. A tab with
/// no file holds only what was typed or given to it, and is shown.
pub fn hidden_from_agents(path: &Path) -> bool {
    home_paths::may_read_file(&path.to_string_lossy(), &home()).is_err()
}
