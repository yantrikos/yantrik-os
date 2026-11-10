//! The one move of the old root into the new (#1124, round 6, finding 1).
//!
//! Workshops used to be `~/.local/share/yantrik/workshops/<mind>/<name>`. They are now
//! `/var/lib/yantrik/workshops/<uid>/<mind>/<name>` (`home_paths::workshops`). The old place is
//! one a Mind's job can write to, at any time, so nothing in it is believed:
//!
//! - the move runs once per account, ever: the shell writes `.moved` inside `<uid>/` (a place
//!   no Mind's view holds) and the old path is never read again after it;
//! - a workshop is **copied**, never renamed, so no inode that some outside process holds lands
//!   in the root; every entry is judged on the descriptor it was opened as: a folder or a regular
//!   file of the person's, with one name for it, and nothing else (no link, device, pipe, socket,
//!   hard link, or name with a control or format character). One bad entry refuses that workshop;
//!   it stays where it was, and is said so;
//! - the old record is never read: it was forgeable by every earlier build. A moved workshop
//!   comes with no grant, and the person grants it again through `workshop_create`'s card;
//! - the new root is never made: no root, or one that is not the person's, moves nothing.

use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use yantrik_ipc_contracts::home_paths::workshops;

use crate::workshop_export::{kind_at, mkdir_at, names_in, remove_at, OPEN_DIR};
use crate::workshop_out::own;

/// What the move did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Moved {
    /// `<mind>/<name>` of each workshop copied into the root.
    pub landed: Vec<String>,
    /// Entries left where they were, and why.
    pub left: Vec<String>,
}

const OLD: [&str; 4] = [".local", "share", "yantrik", "workshops"];
const MAX_DEPTH: usize = 64;
/// Written in `<uid>/` once the move has been considered. A Mind's views do not hold that folder.
const MARKER: &CStr = c".moved";

fn c(name: &str) -> CString {
    CString::new(name.as_bytes()).unwrap_or_default()
}

/// The folder `name` in `dir`, opened without following a link, or `None` when it is not one.
fn open_dir(dir: RawFd, name: &str) -> Option<OwnedFd> {
    own(unsafe { libc::openat(dir, c(name).as_ptr(), OPEN_DIR) }).ok()
}

/// The folders from the home down to the old root, opened without following a link; the last is
/// the old root. `None` when there is none (or a link stands in the way).
fn open_old(home: &Path) -> Option<Vec<OwnedFd>> {
    let home_c = CString::new(home.as_os_str().as_bytes()).ok()?;
    let mut chain = vec![own(unsafe { libc::open(home_c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) }).ok()?];
    for name in OLD {
        let next = open_dir(chain.last()?.as_raw_fd(), name)?;
        chain.push(next);
    }
    Some(chain)
}

/// Copy the old root's workshops into the root, if this account has not been through it before.
/// `Ok(None)` when it has (or there is nothing to do and no root yet); `Err` when there is an old
/// root and the new one is not there to take it (it stays, and the move is tried again later).
pub fn migrate(home: &Path) -> Result<Option<Moved>, String> {
    let old = open_old(home);
    let held = match workshops::open_root(home) {
        Ok(held) => held,
        Err(why) => return if old.is_some() { Err(why) } else { Ok(None) },
    };
    let new_fd = held.open_dir().map_err(|e| format!("the workshops folder cannot be opened ({e})"))?;
    if kind_at(new_fd.as_raw_fd(), MARKER).is_some() {
        return Ok(None);
    }
    let mut done = Moved::default();
    if let Some(chain) = &old {
        move_all(chain, new_fd.as_raw_fd(), &mut done);
    }
    // Written whatever came of the move: the old path is not looked at again.
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    own(unsafe { libc::openat(new_fd.as_raw_fd(), MARKER.as_ptr(), flags, 0o600 as libc::c_uint) })
        .map_err(|e| format!("the move could not be recorded ({e}), so it is not done"))?;
    Ok(Some(done))
}

fn move_all(chain: &[OwnedFd], new_fd: RawFd, done: &mut Moved) {
    let Some(old) = chain.last() else { return };
    for mind in names_in(old.as_raw_fd()) {
        let mind_name = mind.to_string_lossy().into_owned();
        let named = crate::workshop::plain_name("a Mind's folder", &mind_name).is_ok();
        let Some(from) = named.then(|| open_dir(old.as_raw_fd(), &mind_name)).flatten() else {
            done.left.push(mind_name);
            continue;
        };
        let Some(to) = open_dir(new_fd, &mind_name).or_else(|| mkdir_at(new_fd, &mind, 0o700).ok()) else {
            done.left.push(mind_name);
            continue;
        };
        for shop in names_in(from.as_raw_fd()) {
            let shop_name = shop.to_string_lossy().into_owned();
            let named = crate::workshop::plain_name("a workshop's name", &shop_name).is_ok();
            if !named || kind_at(from.as_raw_fd(), &shop) != Some(libc::S_IFDIR) || kind_at(to.as_raw_fd(), &shop).is_some() {
                done.left.push(format!("{mind_name}/{shop_name}"));
                continue;
            }
            match copy_tree(from.as_raw_fd(), &shop, to.as_raw_fd(), &shop, 0) {
                Ok(()) => {
                    remove_at(from.as_raw_fd(), &shop, 0);
                    done.landed.push(format!("{mind_name}/{shop_name}"));
                }
                Err(e) => {
                    remove_at(to.as_raw_fd(), &shop, 0);
                    done.left.push(format!("{mind_name}/{shop_name} ({e})"));
                }
            }
        }
        // Empty now: gone. Otherwise what is left stays for the person to see.
        unsafe { libc::unlinkat(old.as_raw_fd(), mind.as_ptr(), libc::AT_REMOVEDIR) };
    }
    // The old record is not believed, and nothing reads it again.
    unsafe { libc::unlinkat(old.as_raw_fd(), c(".grants.json").as_ptr(), 0) };
    // Deepest first; each only if it is empty (`yantrik/` usually is not, and stays).
    for (parent, name) in chain.iter().zip(OLD).rev() {
        if unsafe { libc::unlinkat(parent.as_raw_fd(), c(name).as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            break;
        }
    }
}

fn fstat(fd: RawFd) -> Result<libc::stat, String> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    match unsafe { libc::fstat(fd, &mut st) } {
        0 => Ok(st),
        _ => Err(std::io::Error::last_os_error().to_string()),
    }
}

/// A copy of `src_name` (in `src`) as `dst_name` (in `dst`): folders (private) and regular files
/// (by content, owner's bits only) of the person's, nothing else. Each is judged on the
/// descriptor it was opened as, so a swap after the listing changes nothing.
fn copy_tree(src: RawFd, src_name: &CString, dst: RawFd, dst_name: &CString, depth: usize) -> Result<(), String> {
    let shown = src_name.to_string_lossy();
    if depth >= MAX_DEPTH {
        return Err("folders nested too deep to move".into());
    }
    let text = src_name.to_str().unwrap_or("");
    if text.is_empty() || text.chars().any(|c| c.is_control() || crate::approvals::is_format_char(c)) {
        return Err(format!("{shown:?} has a name that is not plain"));
    }
    let fd = own(unsafe { libc::openat(src, src_name.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) })
        .map_err(|_| format!("{shown} is a link or cannot be read"))?;
    let st = fstat(fd.as_raw_fd())?;
    if st.st_uid != unsafe { libc::geteuid() } {
        return Err(format!("{shown} is not the person's"));
    }
    match st.st_mode & libc::S_IFMT {
        libc::S_IFDIR => {
            let into = mkdir_at(dst, dst_name, 0o700)?;
            for child in names_in(fd.as_raw_fd()) {
                copy_tree(fd.as_raw_fd(), &child, into.as_raw_fd(), &child, depth + 1)?;
            }
            Ok(())
        }
        libc::S_IFREG if st.st_nlink == 1 => {
            let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            let out = own(unsafe { libc::openat(dst, dst_name.as_ptr(), flags, 0o600 as libc::c_uint) })?;
            let mut out = std::fs::File::from(out);
            std::io::copy(&mut std::fs::File::from(fd), &mut out).map_err(|e| e.to_string())?;
            unsafe { libc::fchmod(out.as_raw_fd(), (st.st_mode & 0o700) as libc::mode_t) };
            Ok(())
        }
        _ => Err(format!("{shown} is a link, a device, a pipe, a socket or a file with a second name")),
    }
}

/// How many landed workshops the notice names before it says "and N more".
const NAMED: usize = 5;

impl Moved {
    /// What the person is told, once: where the folders landed, that they carry no grant, and how
    /// to get a workshop a Mind can use. `None` when the move found nothing.
    pub fn notice(&self, home: &Path) -> Option<(String, String)> {
        if self.landed.is_empty() && self.left.is_empty() {
            return None;
        }
        let mut body = String::new();
        if !self.landed.is_empty() {
            let shown: Vec<&str> = self.landed.iter().take(NAMED).map(String::as_str).collect();
            let more = self.landed.len().saturating_sub(NAMED);
            body = format!(
                "{} old workshop folder(s) moved to {}: {}{}. They carry no grant, so no Mind can use them. \
                 `workshop_create` makes a new workshop (you approve it on a card); copy what you want into it by hand.",
                self.landed.len(),
                workshops::root_shown(home),
                shown.join(", "),
                if more > 0 { format!(" and {more} more") } else { String::new() },
            );
        }
        if !self.left.is_empty() {
            let left: Vec<&str> = self.left.iter().take(NAMED).map(String::as_str).collect();
            body.push_str(&format!(" Left where they were, because something in them was not a plain file or folder of yours: {}.", left.join(", ")));
        }
        Some(("Your old workshops were moved".to_string(), body.trim().to_string()))
    }
}

/// Run the move at the shell's start, off the caller's thread, so the notice does not wait for a
/// workshop tool to be called.
pub fn at_start() {
    let home = crate::workshop::home();
    let _ = std::thread::Builder::new().name("yos-workshop-move".into()).spawn(move || once(&home));
}

/// Run the move until this account has been through it.
pub fn once(home: &Path) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    static DONE: AtomicBool = AtomicBool::new(false);
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    if DONE.load(Ordering::SeqCst) {
        return;
    }
    match migrate(home) {
        Ok(None) => DONE.store(true, Ordering::SeqCst),
        Ok(Some(m)) => {
            tracing::info!(
                "workshops: copied {} workshop(s) into {}, with no grant (grant each again with workshop_create); left in place: {:?}",
                m.landed.len(),
                workshops::root(home).display(),
                m.left
            );
            // The marker is already written, so this goes out once, here, and never again.
            if let Some((title, body)) = m.notice(home) {
                yantrik_app_runtime::notify::send(yantrik_app_runtime::notify::Notification::new("Yantrik", title).body(body));
            }
            DONE.store(true, Ordering::SeqCst);
        }
        // No usable root yet: the old content stays where it is, and the grants refuse meanwhile.
        Err(why) => tracing::warn!("workshops: the old folder was not moved: {why}"),
    }
}

#[cfg(all(test, unix))]
#[path = "workshop_move_tests.rs"]
mod tests;
