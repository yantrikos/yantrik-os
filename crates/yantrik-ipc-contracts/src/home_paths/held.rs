//! Places a command's view is bound from, held open (#1265, round 7).
//!
//! `bwrap` resolves a bind's source by its path when it starts, which is later than any check the
//! caller made, so a place swapped for a link in that moment is bound from where the link leads. A
//! place bound from an **open descriptor** (`--ro-bind-fd`) is bound as the place that was opened:
//! there is no second lookup to swap. [`Held`] is those descriptors, and the one way every door
//! that starts a view keeps them across the `exec` and closes all the others ([`Held::seal`]).
//!
//! Each place is opened one part at a time from the home, every part `O_PATH | O_NOFOLLOW`, so a
//! link anywhere on the way is refused, never followed (the equivalent of `openat2` with
//! `RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH`, which an older kernel does not have).

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

/// A held descriptor is moved to a number at or above this, so it is never one a door puts a pipe on
/// (`3`, `4`) or one a std `Command` fills for the child's standard streams.
const NUMBERED_FROM: RawFd = 64;

/// The open descriptors of the places a view is bound from, in the order its `--ro-bind-fd` words
/// name them. Closed when this is dropped, so the owner keeps it until the child is started.
#[derive(Debug, Default)]
pub struct Held {
    fds: Vec<OwnedFd>,
}

fn last_error(what: &str, at: &Path) -> String {
    format!("{what} {}: {}", at.display(), std::io::Error::last_os_error())
}

fn open_at(dir: RawFd, name: &std::ffi::OsStr) -> Result<OwnedFd, std::io::Error> {
    let c = CString::new(name.as_bytes()).map_err(|_| std::io::Error::other("a NUL in a name"))?;
    match unsafe { libc::openat(dir, c.as_ptr(), libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC) } {
        -1 => Err(std::io::Error::last_os_error()),
        fd => Ok(unsafe { OwnedFd::from_raw_fd(fd) }),
    }
}

fn kind(fd: RawFd) -> Option<libc::mode_t> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    (unsafe { libc::fstat(fd, &mut st) } == 0).then_some(st.st_mode & libc::S_IFMT)
}

/// Where an open descriptor is, as the kernel says: the path to compare with the one that was asked
/// for. `None` for a descriptor of a deleted place.
pub fn where_is(fd: RawFd) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{fd}")).ok().filter(|p| p.is_absolute())
}

/// Open `place` (a path inside `home`) without following a link on the way or at the end. `Err` is
/// a sentence for the person.
pub fn open_beneath(home: &Path, place: &Path) -> Result<OwnedFd, String> {
    let Ok(rest) = place.strip_prefix(home) else {
        return Err(format!("{} is not inside the home", place.display()));
    };
    let home_c = CString::new(home.as_os_str().as_bytes()).map_err(|_| "a NUL in the home".to_string())?;
    let top = unsafe { libc::open(home_c.as_ptr(), libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if top < 0 {
        return Err(last_error("cannot open the home", home));
    }
    let mut at = unsafe { OwnedFd::from_raw_fd(top) };
    for part in rest.components() {
        let Component::Normal(name) = part else {
            return Err(format!("{} has a part that is not a plain name", place.display()));
        };
        at = open_at(at.as_raw_fd(), name).map_err(|e| format!("cannot open {} without following a link ({e})", place.display()))?;
        // `O_PATH | O_NOFOLLOW` opens a link itself, not what it leads to: a link is a refusal.
        if kind(at.as_raw_fd()) == Some(libc::S_IFLNK) {
            return Err(format!("{} is, or is below, a link", place.display()));
        }
    }
    Ok(at)
}

impl Held {
    /// What is held, for the words and for a test.
    pub fn len(&self) -> usize {
        self.fds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fds.is_empty()
    }

    /// Hold `fd`, moved above the numbers a door uses, and return the number it has now: the one
    /// the `--ro-bind-fd` word names (the child has the same table).
    pub fn keep(&mut self, fd: OwnedFd) -> Result<RawFd, String> {
        let moved = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, NUMBERED_FROM) };
        if moved < 0 {
            return Err(last_error("cannot hold a place open", Path::new("")));
        }
        self.fds.push(unsafe { OwnedFd::from_raw_fd(moved) });
        Ok(moved)
    }

    /// In the child, between the fork and the exec: every descriptor from `keep_below` up is made
    /// close-on-exec, then the held ones are not. `keep_below` is the first number the door does not
    /// use (a door that put a pipe on `3` passes `4`). Only async-signal-safe calls; nothing is
    /// allocated.
    ///
    /// # Safety
    /// Only to be called from `pre_exec`, once, after the door's own descriptors are in place.
    pub unsafe fn seal(&self, keep_below: libc::c_uint) -> std::io::Result<()> {
        const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
        if libc::syscall(libc::SYS_close_range, keep_below, libc::c_uint::MAX, CLOSE_RANGE_CLOEXEC) != 0 {
            let mut limit: libc::rlimit = std::mem::zeroed();
            let top = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 { limit.rlim_cur.min(65_536) as libc::c_int } else { 4096 };
            for fd in keep_below as libc::c_int..top {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                }
            }
        }
        for fd in &self.fds {
            let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// The numbers held, for a test.
    pub fn numbers(&self) -> Vec<RawFd> {
        self.fds.iter().map(|f| f.as_raw_fd()).collect()
    }
}
