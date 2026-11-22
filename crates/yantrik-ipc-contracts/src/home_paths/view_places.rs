//! The read-only toolchain a command's view carries, and the account's home (#1265, rounds 6, 7).
//!
//! Name the safe: the list below is what may be shown. A toolchain entry is bound only if it is
//! itself a file or folder, with no link at it or on the way to it (resolved and compared, as
//! `reach_place` does), and inside it a file or folder whose
//! name is a secret's (`secrets::names_a_secret`) or a protected place's is masked, so a token left
//! in `~/.local/bin` or a registry cache is not read by a command that asked for nothing. An entry
//! with more than [`WALK_CAP`] things in it, or that takes more than [`WALK_TIME`] to look through,
//! is not bound at all rather than bound half checked. A file with a plain name is not caught
//! whatever is in it: this is a deny list over contents.
//! `~/.gitconfig` and `~/.config/git/config` are never bound: see `view_git`.

use std::ffi::{CStr, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{is_protected_in, secrets};

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

/// The most things looked at inside one toolchain entry, and the most time spent on one.
const WALK_CAP: usize = 50_000;
const WALK_TIME: Duration = Duration::from_millis(200);

/// Folders that are looked at only at their top: their children's names are checked, and nothing is
/// walked below them (a registry's unpacked sources, a git checkout, the docs a toolchain ships).
/// That is what keeps a used registry inside the cap; a secret in a plain-named file deeper is not
/// caught, which SECURITY.md says.
const TOP_ONLY: &[(&str, &str)] = &[("registry", "src"), ("git", "checkouts"), ("share", "doc"), ("share", "man"), ("share", "info")];

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A folder or file as it was when the view was planned: its device and inode, never a link.
/// Looked at again just before the command is built ([`Seen::unchanged`]).
#[derive(Debug, Clone)]
pub struct Seen {
    path: PathBuf,
    id: (u64, u64),
}

impl Seen {
    /// `None` for a link or for something that is not there.
    pub fn of(path: &Path) -> Option<Seen> {
        let meta = path.symlink_metadata().ok().filter(|m| !m.file_type().is_symlink())?;
        Some(Seen { path: path.to_path_buf(), id: (meta.dev(), meta.ino()) })
    }

    /// Whether it is still the same, and still not a link.
    pub fn unchanged(&self) -> bool {
        Seen::of(&self.path).is_some_and(|now| now.id == self.id)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// What to mask inside `entry` (judged against `home`), or `None` when it is too big, or took too
/// long, to have been checked.
pub(super) fn secrets_in(home: &Path, entry: &Path, cap: usize, budget: Duration) -> Option<Vec<(PathBuf, bool)>> {
    let (started, mut found, mut seen) = (Instant::now(), Vec::new(), 0usize);
    // (folder, whether only its children's names are looked at)
    let mut stack = vec![(entry.to_path_buf(), false)];
    while let Some((dir, top_only)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for child in read.flatten() {
            seen += 1;
            if seen > cap || started.elapsed() > budget {
                return None;
            }
            let path = child.path();
            let Ok(meta) = path.symlink_metadata() else { continue };
            let is_dir = meta.is_dir();
            if secrets::names_a_secret(&path) || is_protected_in(&path, Some(home)) {
                found.push((path, is_dir));
            } else if is_dir && !top_only {
                let parts: Vec<_> = path.strip_prefix(entry.parent().unwrap_or(entry)).iter().flat_map(|r| r.components()).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                let top = parts.len() >= 2 && TOP_ONLY.iter().any(|(a, b)| parts[parts.len() - 2] == *a && parts[parts.len() - 1] == *b);
                stack.push((path, top));
            }
        }
    }
    Some(found)
}

/// Whether `entry` is the place it is written as under `home`: no link on the way (`.cargo` or
/// `.config/git` is often one under a dotfile manager) and none at the end. Resolved and compared,
/// the same test `reach_place` makes.
fn is_where_written(home: &Path, rel: &str) -> bool {
    let at = home.join(rel);
    let real_home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    at.canonicalize().is_ok_and(|real| real == real_home.join(rel))
}

/// The words that put the toolchain in the view read-only, then mask what is secret inside it, and
/// what was bound, to be looked at again last. An entry is skipped, never bound half checked, when a
/// link is on its way, when it is named like a secret, or when it is too big or too slow to walk.
pub fn toolchain_words(home: &Path) -> (Vec<String>, Vec<Seen>) {
    toolchain_words_with(home, WALK_CAP, WALK_TIME)
}

pub(super) fn toolchain_words_with(home: &Path, cap: usize, budget: Duration) -> (Vec<String>, Vec<Seen>) {
    let (mut binds, mut masks, mut bound) = (Vec::new(), Vec::new(), Vec::new());
    for rel in TOOLCHAIN {
        let at = home.join(rel);
        let Some(seen) = Seen::of(&at) else { continue };
        if !is_where_written(home, rel) || secrets::names_a_secret(&at) {
            continue;
        }
        let inside = match at.is_dir() {
            true => match secrets_in(home, &at, cap, budget) {
                Some(found) => found,
                None => continue,
            },
            false => Vec::new(),
        };
        binds.extend(["--ro-bind-try".to_string(), text(&at), text(&at)]);
        bound.push(seen);
        for (path, is_dir) in inside {
            match is_dir {
                true => masks.extend(["--tmpfs".to_string(), text(&path)]),
                false => masks.extend(["--ro-bind-try".to_string(), "/dev/null".to_string(), text(&path)]),
            }
        }
    }
    binds.extend(masks);
    (binds, bound)
}

thread_local! {
    /// A test's own account home. The doors ask the account database, and a test has a temp folder.
    static HOME_FOR_TESTS: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Called by tests only: this thread's account has this home. `None` ends it.
#[doc(hidden)]
pub fn account_home_for_tests(home: Option<PathBuf>) {
    HOME_FOR_TESTS.with(|h| *h.borrow_mut() = home);
}

/// Whether two paths are the same folder, as written or as resolved.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    a == b || a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(x, y)| x == y)
}

/// The home of the account this process runs as, from the account entry for the effective uid, not
/// from `$HOME`. `Err` when there is no entry, when it is not a real folder path, or when `$HOME`
/// says something else: a view built over the wrong home protects nothing.
pub fn account_home() -> Result<PathBuf, String> {
    if let Some(home) = HOME_FOR_TESTS.with(|h| h.borrow().clone()) {
        return Ok(home);
    }
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
    let said = Path::new(env);
    match same_folder(said, &dir) {
        true => Ok(dir),
        false => Err(format!(
            "$HOME is {:?} but this account's home is {}; a command is not started over a home that is not the account's",
            said,
            dir.display()
        )),
    }
}
