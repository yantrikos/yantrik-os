//! The one move of the old root into the new (#1124, round 5, the root's new place).
//!
//! Workshops used to be `~/.local/share/yantrik/workshops/<mind>/<name>`. They are now
//! `/var/lib/yantrik/workshops/<uid>/<mind>/<name>` (`home_paths::workshops`). Whatever a Mind
//! left in the old place is moved once, and safely: the old tree is a place a Mind's job wrote,
//! so nothing in it is followed.
//!
//! - every step is `openat(O_NOFOLLOW)` on a descriptor, the home down to the old root included;
//!   a link where a folder should be is left where it is and said so;
//! - a workshop is moved with `renameat`, which is atomic and keeps its inode; across filesystems
//!   (`/home` and `/var` are often two) it is copied by descriptor (folders, regular files and
//!   links as links, nothing else) and only then removed from the old place;
//! - a workshop that already exists in the new root is not touched;
//! - the new root is never made: no root, or one that is not the person's, moves nothing.
//!
//! The old grant record moves with them: a grant whose folder moved is written again, to its new
//! path, device and inode. The old record is then deleted, and so are the old folders once empty.

use std::ffi::CString;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use yantrik_ipc_contracts::home_paths::workshops;

use crate::workshop_export::{kind_at, mkdir_at, names_in, remove_at, OPEN_DIR};
use crate::workshop_out::own;

/// What the move did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Moved {
    pub workshops: usize,
    pub grants: usize,
    /// Entries left where they were: a link, a file, a name already in the new root.
    pub left: Vec<String>,
}

const OLD: [&str; 4] = [".local", "share", "yantrik", "workshops"];
const MAX_DEPTH: usize = 64;

fn c(name: &str) -> CString {
    CString::new(name.as_bytes()).unwrap_or_default()
}

/// The folder `name` in `dir`, opened without following a link, or `None` when it is not one.
fn open_dir(dir: RawFd, name: &str) -> Option<OwnedFd> {
    own(unsafe { libc::openat(dir, c(name).as_ptr(), OPEN_DIR) }).ok()
}

/// Move the old root's content into the root, if there is any. `Ok(None)` when there is no old
/// root; `Err` when there is one and the new root is not there to take it (it stays).
pub fn migrate(home: &Path) -> Result<Option<Moved>, String> {
    let home_c = CString::new(home.as_os_str().as_bytes()).map_err(|_| "a home with a NUL in it".to_string())?;
    let home_fd = own(unsafe { libc::open(home_c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) })?;
    // The old root, a name at a time: a link anywhere on the way means there is nothing to move.
    let mut parent = home_fd;
    let mut chain = Vec::new();
    for name in OLD {
        let Some(next) = open_dir(parent.as_raw_fd(), name) else { return Ok(None) };
        chain.push(parent);
        parent = next;
    }
    let old = parent;
    let held = workshops::open_root(home)?;
    let new_fd = held.open_dir().map_err(|e| format!("the workshops folder cannot be opened ({e})"))?;
    let mut done = Moved::default();
    let record = crate::workshop::record_in(home);
    let old_record = read_old_record(old.as_raw_fd());
    for mind in names_in(old.as_raw_fd()) {
        let mind_name = mind.to_string_lossy().into_owned();
        if mind_name.starts_with('.') {
            continue;
        }
        let Some(from) = open_dir(old.as_raw_fd(), &mind_name) else {
            done.left.push(mind_name);
            continue;
        };
        let to = match open_dir(new_fd.as_raw_fd(), &mind_name) {
            Some(to) => to,
            None => match mkdir_at(new_fd.as_raw_fd(), &mind, 0o700) {
                Ok(to) => to,
                Err(e) => {
                    done.left.push(format!("{mind_name} ({e})"));
                    continue;
                }
            },
        };
        for shop in names_in(from.as_raw_fd()) {
            let shop_name = shop.to_string_lossy().into_owned();
            if shop_name.starts_with('.') || kind_at(from.as_raw_fd(), &shop) != Some(libc::S_IFDIR) || kind_at(to.as_raw_fd(), &shop).is_some() {
                done.left.push(format!("{mind_name}/{shop_name}"));
                continue;
            }
            match move_one(from.as_raw_fd(), to.as_raw_fd(), &shop_name) {
                Ok(()) => {
                    done.workshops += 1;
                    done.grants += regrant(&record, home, old_record.as_ref(), &mind_name, &shop_name);
                }
                Err(e) => done.left.push(format!("{mind_name}/{shop_name} ({e})")),
            }
        }
        // Empty now: gone. Otherwise what is left stays for the person to see.
        unsafe { libc::unlinkat(old.as_raw_fd(), mind.as_ptr(), libc::AT_REMOVEDIR) };
    }
    unsafe { libc::unlinkat(old.as_raw_fd(), c(".grants.json").as_ptr(), 0) };
    // Deepest first; each only if it is empty (`yantrik/` usually is not, and stays).
    for (parent, name) in chain.iter().zip(OLD).rev() {
        if unsafe { libc::unlinkat(parent.as_raw_fd(), c(name).as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            break;
        }
    }
    Ok(Some(done))
}

/// The old grant record, read through the old root's descriptor without following a link.
fn read_old_record(old: RawFd) -> Option<serde_json::Value> {
    use std::io::Read;
    let fd = own(unsafe { libc::openat(old, c(".grants.json").as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) }).ok()?;
    let mut text = String::new();
    std::fs::File::from(fd).take(1 << 20).read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write the grant again at the new folder, if the old record had one for this Mind and the
/// folder stands. Counted.
fn regrant(record: &Path, home: &Path, old: Option<&serde_json::Value>, mind: &str, shop: &str) -> usize {
    let Ok(new) = crate::workshop::folder_for(home, mind, shop, false) else { return 0 };
    let granted = old.and_then(|v| v["grants"].as_array()).into_iter().flatten().find(|g| {
        g["agent"].as_str() == Some(mind) && g["folder"].as_str().is_some_and(|f| f.ends_with(&format!("/{mind}/{shop}")))
    });
    let Some(at) = granted.and_then(|g| g["granted_unix"].as_u64()) else { return 0 };
    usize::from(crate::workshop::grant(record, mind, &new, at).is_ok())
}

/// Move `name` from `from` to `to`: renamed where both are one filesystem, else copied and removed.
fn move_one(from: RawFd, to: RawFd, name: &str) -> Result<(), String> {
    let n = c(name);
    if unsafe { libc::renameat(from, n.as_ptr(), to, n.as_ptr()) } == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() != Some(libc::EXDEV) {
        return Err(err.to_string());
    }
    if let Err(e) = copy_tree(from, &n, to, &n, 0) {
        remove_at(to, &n, 0);
        return Err(e);
    }
    remove_at(from, &n, 0);
    Ok(())
}

/// A copy of `src_name` (in `src`) as `dst_name` (in `dst`), without following a link: folders
/// with their modes, regular files by content with their modes (no setuid), links as links.
fn copy_tree(src: RawFd, src_name: &CString, dst: RawFd, dst_name: &CString, depth: usize) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err("folders nested too deep to move".into());
    }
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatat(src, src_name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mode = (st.st_mode & 0o777) as libc::mode_t;
    match st.st_mode & libc::S_IFMT {
        libc::S_IFDIR => {
            let from = own(unsafe { libc::openat(src, src_name.as_ptr(), OPEN_DIR) })?;
            // Writable while it is filled; the mode it had is put back after.
            let into = mkdir_at(dst, dst_name, 0o700)?;
            for child in names_in(from.as_raw_fd()) {
                copy_tree(from.as_raw_fd(), &child, into.as_raw_fd(), &child, depth + 1)?;
            }
            unsafe { libc::fchmod(into.as_raw_fd(), mode) };
            Ok(())
        }
        libc::S_IFREG => {
            let fd = own(unsafe { libc::openat(src, src_name.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) })?;
            let out = unsafe {
                libc::openat(dst, dst_name.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600 as libc::c_uint)
            };
            let mut out = own(out).map(std::fs::File::from)?;
            std::io::copy(&mut std::fs::File::from(fd), &mut out).map_err(|e| e.to_string())?;
            unsafe { libc::fchmod(out.as_raw_fd(), mode) };
            Ok(())
        }
        libc::S_IFLNK => {
            let mut buf = vec![0u8; 4096];
            let n = unsafe { libc::readlinkat(src, src_name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
            if n < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            buf.truncate(n as usize);
            let target = CString::new(buf).map_err(|_| "a link with a NUL".to_string())?;
            match unsafe { libc::symlinkat(target.as_ptr(), dst, dst_name.as_ptr()) } {
                0 => Ok(()),
                _ => Err(std::io::Error::last_os_error().to_string()),
            }
        }
        // A pipe, a socket, a device: not a thing to carry.
        _ => Ok(()),
    }
}

/// Done once per run: the old root is looked for until it has been dealt with.
pub fn once(home: &Path) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.load(Ordering::SeqCst) {
        return;
    }
    match migrate(home) {
        Ok(None) => DONE.store(true, Ordering::SeqCst),
        Ok(Some(m)) => {
            tracing::info!("workshops: moved {} workshop(s) and {} grant(s) into {}; left in place: {:?}", m.workshops, m.grants, workshops::root(home).display(), m.left);
            DONE.store(m.left.is_empty(), Ordering::SeqCst);
        }
        // No usable root yet: the old content stays where it is, and the grants refuse meanwhile.
        Err(why) => tracing::warn!("workshops: the old folder was not moved: {why}"),
    }
}

#[cfg(all(test, unix))]
#[path = "workshop_move_tests.rs"]
mod tests;
