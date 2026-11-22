//! The read-only toolchain a command's view carries, and the account's home (#1265, round 6).
//!
//! Name the safe: the list below is what may be shown. A toolchain entry is bound only if it is
//! itself a file or folder (a link is skipped, not followed), and inside it a file or folder whose
//! name is a secret's (`secrets::names_a_secret`) or a protected place's is masked, so a token left
//! in `~/.local/bin` or a registry cache is not read by a command that asked for nothing. An entry
//! with more than [`WALK_CAP`] things in it is not bound at all rather than bound half checked.
//! `~/.gitconfig` and `~/.config/git/config` are never bound: see `view_git`.

use std::ffi::{CStr, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::{is_protected, secrets};

/// The dot-places a build needs, read-only. Git's own settings are not here: `view_git`.
pub const TOOLCHAIN: &[&str] = &[
    ".config/git/ignore",
    ".config/git/attributes",
    ".local/bin",
    ".cargo/bin",
    ".cargo/registry",
    ".cargo/git",
    ".rustup",
];

/// The most things looked at inside one toolchain entry.
const WALK_CAP: usize = 50_000;

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// What to mask inside `entry`, or `None` when it is too big to have been checked.
fn secrets_in(entry: &Path) -> Option<Vec<(PathBuf, bool)>> {
    let (mut found, mut stack, mut seen) = (Vec::new(), vec![entry.to_path_buf()], 0usize);
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for child in read.flatten() {
            seen += 1;
            if seen > WALK_CAP {
                return None;
            }
            let path = child.path();
            let Ok(meta) = path.symlink_metadata() else { continue };
            let is_dir = meta.is_dir();
            if secrets::names_a_secret(&path) || is_protected(&path) {
                found.push((path, is_dir));
            } else if is_dir {
                stack.push(path);
            }
        }
    }
    Some(found)
}

/// The words that put the toolchain in the view read-only, then mask what is secret inside it.
pub fn toolchain_words(home: &Path) -> Vec<String> {
    let (mut binds, mut masks) = (Vec::new(), Vec::new());
    for rel in TOOLCHAIN {
        let at = home.join(rel);
        let Ok(meta) = at.symlink_metadata() else { continue };
        if meta.file_type().is_symlink() || secrets::names_a_secret(&at) {
            continue;
        }
        let inside = match meta.is_dir() {
            true => match secrets_in(&at) {
                Some(found) => found,
                None => continue,
            },
            false => Vec::new(),
        };
        binds.extend(["--ro-bind-try".to_string(), text(&at), text(&at)]);
        for (path, is_dir) in inside {
            match is_dir {
                true => masks.extend(["--tmpfs".to_string(), text(&path)]),
                false => masks.extend(["--ro-bind-try".to_string(), "/dev/null".to_string(), text(&path)]),
            }
        }
    }
    binds.extend(masks);
    binds
}

/// The home of the account this process runs as, from the account entry for the effective uid, not
/// from `$HOME`. `Err` when there is no entry, when it is not a real folder path, or when `$HOME`
/// says something else: a view built over the wrong home protects nothing.
pub fn account_home() -> Result<PathBuf, String> {
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(libc::geteuid(), &mut entry, buf.as_mut_ptr(), buf.len(), &mut found) };
    if rc != 0 || found.is_null() || entry.pw_dir.is_null() {
        return Err("this account has no home in the account database, so there is no home to protect".to_string());
    }
    let dir = PathBuf::from(OsStr::from_bytes(unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes()));
    if !dir.is_absolute() || dir.parent().is_none() {
        return Err(format!("the account's home is {}, which is not a home folder", dir.display()));
    }
    check_home(dir, std::env::var_os("HOME").as_deref())
}

/// `dir` (the account's home) if `$HOME` is unset or is the same folder; `Err` if it says otherwise.
pub(super) fn check_home(dir: PathBuf, env: Option<&OsStr>) -> Result<PathBuf, String> {
    let Some(env) = env else { return Ok(dir) };
    let (said, same) = (Path::new(env), |a: &Path, b: &Path| a == b || a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(x, y)| x == y));
    match same(said, &dir) {
        true => Ok(dir),
        false => Err(format!(
            "$HOME is {:?} but this account's home is {}; a command is not started over a home that is not the account's",
            said,
            dir.display()
        )),
    }
}
