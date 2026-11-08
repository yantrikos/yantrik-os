//! The marker on what a Mind made in its workshop (#1120, "Planting"; #1124, H2).
//!
//! A sealed command can write a script, a `package.json` with a `preinstall` hook, a `.desktop`
//! file, a `.git/config` with a `core.fsmonitor`, an `.envrc`. Written there it is harmless. The
//! harm is a later, unsealed run of something the Mind wrote: a person's `npm install`, a prompt
//! that calls `git status`, a double-click in Files. So the shell marks what a workshop holds, and
//! the shell and Files refuse to run or open it as an app without a card (`workshop_guard` in the
//! shell).
//!
//! # Two marks, and which one decides
//!
//! 1. **Where the file is.** Anything under a workshop the record grants is a workshop file, with
//!    no mark needed. This cannot be stripped from inside the sandbox, and it is the rule that
//!    decides.
//! 2. **The xattr `user.yantrik.workshop`**, put by the shell on the workshop and everything in it
//!    when a sealed command ends. It travels with a file that is moved out of the
//!    folder by `mv` and `cp -a`, where the location no longer says. It does **not** travel with a
//!    plain `cp`, `std::fs::copy` (the Files copy) or a GIO copy without all metadata: those make a
//!    new file with no `user.*` attribute, so a copy out of a workshop is an unmarked file. No rule
//!    here relies on the mark alone: a workshop's files are refused where they are, and a copy is
//!    a file the person made (#1124, N8). A sealed command could remove its
//!    own marks, so the filter denies `removexattr` (`seccomp`), and a file replaced by a rename
//!    over it is a new inode that the next sweep marks again. A filesystem with no user xattrs
//!    simply carries no mark; the location rule still holds.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// The attribute name, and the value it carries.
pub const XATTR: &str = "user.yantrik.workshop";
const VALUE: &[u8] = b"1";

fn c_path(path: &Path) -> Option<CString> {
    CString::new(path.as_os_str().as_bytes()).ok()
}

/// Mark one file or folder. Never follows a link (a link takes no `user.*` attribute); `false`
/// when the filesystem would not take it.
pub fn mark(path: &Path) -> bool {
    let (Some(p), Ok(name)) = (c_path(path), CString::new(XATTR)) else { return false };
    unsafe { libc::lsetxattr(p.as_ptr(), name.as_ptr(), VALUE.as_ptr().cast(), VALUE.len(), 0) == 0 }
}

/// Does this file or folder itself carry the mark?
pub fn is_marked(path: &Path) -> bool {
    let (Some(p), Ok(name)) = (c_path(path), CString::new(XATTR)) else { return false };
    unsafe { libc::lgetxattr(p.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) >= 0 }
}

/// Mark `root` and everything under it, at most `limit` entries. How many were visited.
///
/// The walk is by descriptor (`fdwalk`): a directory a sealed job has swapped for a link is never
/// entered, and the mark goes on the opened descriptor (`fsetxattr`), so it can land on nothing
/// but what the walk opened inside the workshop's own filesystem. It refuses a `root` that is
/// `$HOME` or above it, whatever the caller meant (#1124, N2). A sweep cut short leaves the rest
/// to the location rule.
pub fn mark_tree(root: &Path, limit: usize) -> usize {
    if is_home_or_above(root) {
        return 0;
    }
    let Ok(name) = CString::new(XATTR) else { return 0 };
    let mut put = |fd: std::os::fd::RawFd, _: &libc::stat| {
        if fd >= 0 {
            unsafe { libc::fsetxattr(fd, name.as_ptr(), VALUE.as_ptr().cast(), VALUE.len(), 0) };
        }
        true
    };
    crate::fdwalk::walk(root, limit, &mut put)
}

/// Is `path` the person's home, or a folder above it? Nothing here is ever marked, and a mark on
/// one is not believed: it would put every file the person has under a workshop's rule.
fn is_home_or_above(path: &Path) -> bool {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else { return false };
    let home = home.canonicalize().unwrap_or(home);
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    home.starts_with(&real)
}

/// Is `path`, or the nearest folder above it that exists, marked? A file that does not exist yet is
/// judged by its folder. The walk up stops at `$HOME`: a mark on the home or above it is ignored.
pub fn marked_or_under_marked(path: &Path) -> bool {
    let mut at = Some(path);
    while let Some(p) = at {
        if is_home_or_above(p) {
            return false;
        }
        if std::fs::symlink_metadata(p).is_ok() && is_marked(p) {
            return true;
        }
        at = p.parent();
    }
    false
}

type Live = std::sync::Mutex<Option<std::collections::HashMap<String, usize>>>;
/// Sealed jobs running now, per agent.
static LIVE: Live = std::sync::Mutex::new(None);

/// A sealed job of `agent` has started.
pub(crate) fn job_begins(agent: &crate::AgentId) {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    *live.get_or_insert_default().entry(agent.to_string()).or_default() += 1;
}

/// A sealed job of `agent` has ended. When it was the agent's last one, sweep `workshop` (if
/// given): while another job still runs it may be swapping directories under the sweep (#1124,
/// N2), and the one that ends last does the sweep for all.
pub(crate) fn job_ends(agent: &crate::AgentId, workshop: Option<&Path>) {
    let last = {
        let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        let map = live.get_or_insert_default();
        let n = map.entry(agent.to_string()).or_default();
        *n = n.saturating_sub(1);
        let last = *n == 0;
        if last {
            map.remove(&agent.to_string());
        }
        last
    };
    if let (true, Some(w)) = (last, workshop) {
        mark_tree(w, 300_000);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// Whether this temp dir's filesystem takes user xattrs at all (tmpfs before 6.6 does not).
    fn supported(dir: &Path) -> bool {
        let probe = dir.join("probe");
        std::fs::write(&probe, "x").unwrap();
        mark(&probe)
    }

    #[test]
    fn a_swept_workshop_is_marked_all_the_way_down_and_links_are_left_alone() {
        let tmp = std::env::current_dir().unwrap().join(format!("target-planted-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("a/b")).unwrap();
        if !supported(&tmp) {
            eprintln!("skipped: this filesystem takes no user xattrs");
            std::fs::remove_dir_all(&tmp).unwrap();
            return;
        }
        std::fs::write(tmp.join("a/b/run.sh"), "x").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", tmp.join("a/link")).unwrap();
        let seen = mark_tree(&tmp, 1000);
        assert!(seen >= 3);
        assert!(is_marked(&tmp) && is_marked(&tmp.join("a/b")) && is_marked(&tmp.join("a/b/run.sh")));
        assert!(!is_marked(&tmp.join("a/link")), "a link is never marked, and never followed");
        assert!(marked_or_under_marked(&tmp.join("a/b/not-yet-written.desktop")));
        // It travels with a copy that keeps attributes, and a rename keeps the inode's mark.
        std::fs::rename(tmp.join("a/b/run.sh"), tmp.join("moved.sh")).unwrap();
        assert!(is_marked(&tmp.join("moved.sh")));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_sweep_never_follows_a_swapped_link_and_never_marks_the_home_or_above() {
        let tmp = std::env::current_dir().unwrap().join(format!("target-swap-{}", std::process::id()));
        let (shop, outside) = (tmp.join("shop"), tmp.join("outside"));
        std::fs::create_dir_all(shop.join("real")).unwrap();
        std::fs::create_dir_all(outside.join("inner")).unwrap();
        if !supported(&tmp) {
            std::fs::remove_dir_all(&tmp).unwrap();
            return;
        }
        std::fs::write(outside.join("inner/f"), "x").unwrap();
        // What a concurrent job leaves for the sweep to find: a link where a folder was.
        std::os::unix::fs::symlink(&outside, shop.join("d")).unwrap();
        mark_tree(&shop, 1000);
        assert!(is_marked(&shop.join("real")));
        assert!(!is_marked(&outside) && !is_marked(&outside.join("inner")) && !is_marked(&outside.join("inner/f")));
        // The home, and what is above it, are not swept even when asked.
        if let Some(home) = std::env::var_os("HOME") {
            let home = std::path::PathBuf::from(home);
            assert_eq!(mark_tree(&home, 10), 0);
            assert_eq!(mark_tree(home.parent().unwrap_or(&home), 10), 0);
            assert!(!marked_or_under_marked(&home));
        }
        // And a sweep waits for the Mind's last sealed job.
        let agent = crate::AgentId("sweep-test".into());
        std::fs::write(shop.join("late"), "x").unwrap();
        job_begins(&agent);
        job_begins(&agent);
        job_ends(&agent, Some(&shop));
        assert!(!is_marked(&shop.join("late")), "a job is still running: no sweep yet");
        job_ends(&agent, Some(&shop));
        assert!(is_marked(&shop.join("late")));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn an_unmarked_path_is_not_marked() {
        let tmp = std::env::temp_dir().join(format!("unmarked-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(!marked_or_under_marked(&tmp.join("x")));
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
