//! What `/proc` says about a shell. Every function here reads files, so none runs on the UI
//! thread: a probe is a job, and the answer to a close or to an action is read in one.

use std::path::PathBuf;

use crate::model::Facts;
use crate::session::Session;

/// The name of the shell's foreground job, if it has one.
///
/// `has_children` already reads `/proc/<pid>/task/<pid>/children` to answer whether something is
/// running; the same file names it, and "running: sleep" is the difference between a summary a
/// mind can act on and one that only says a directory.
pub fn running(pid: u32) -> Option<String> {
    let children = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
    let child = children.split_whitespace().next()?;
    let name = std::fs::read_to_string(format!("/proc/{child}/comm")).ok()?;
    let name = name.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Where the shell is and what is running in it, read now.
pub fn read(session: &Session) -> Facts {
    Facts { cwd: session.cwd(), running: running(session.pid()), busy: session.has_children() }
}

/// The folder a new tab opens in when none was asked for: the one the active shell is sitting
/// in, else the process's own, else home, else the root.
pub fn start_dir(active: Option<&Session>) -> PathBuf {
    active
        .map(Session::cwd)
        .filter(|p| p.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pid_with_no_children_runs_nothing() {
        assert_eq!(running(u32::MAX), None);
    }

    #[test]
    fn the_start_directory_is_a_folder() {
        assert!(start_dir(None).is_dir());
    }
}
