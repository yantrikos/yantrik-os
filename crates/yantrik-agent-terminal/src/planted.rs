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
//!    when a sealed command ends. It travels with a file that is moved or copied out of the
//!    folder (`mv`, `cp -a`), where the location no longer says. A sealed command could remove its
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

/// Mark `root` and everything under it, not following links, at most `limit` entries. How many
/// were visited. A sweep that is cut short leaves the rest to the location rule.
pub fn mark_tree(root: &Path, limit: usize) -> usize {
    let mut seen = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        mark(&dir);
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen >= limit {
                return seen;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => {
                    mark(&path);
                }
                _ => {}
            }
        }
    }
    seen
}

/// Is `path`, or the nearest folder above it that exists, marked? A file that does not exist yet is
/// judged by its folder.
pub fn marked_or_under_marked(path: &Path) -> bool {
    let mut at = Some(path);
    while let Some(p) = at {
        if std::fs::symlink_metadata(p).is_ok() && is_marked(p) {
            return true;
        }
        at = p.parent();
    }
    false
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
    fn an_unmarked_path_is_not_marked() {
        let tmp = std::env::temp_dir().join(format!("unmarked-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(!marked_or_under_marked(&tmp.join("x")));
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
