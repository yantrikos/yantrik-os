//! Where a Mind's workshops live, the one check that keeps in-process tools out of them, and the
//! view a program a Mind drives gets of the machine (#1120, #1124).
//!
//! A workshop is `/var/lib/yantrik/workshops/<uid>/<mind>/<name>`, and nowhere else:
//!
//! - `/var/lib/yantrik/workshops` is `root:root 0755` (a tmpfiles.d file the ISO and the bundle
//!   ship), so the person cannot rename it or anything above it;
//! - `<uid>/` is `0700`, owned by the person. The updater's root postinst makes it for every login
//!   account, and so does the first-run wizard. **The shell never creates it** ([`open_root`]): a
//!   root that is missing or not the person's means no workshop is granted;
//! - the shell holds an `O_PATH` descriptor for `<uid>/` and refuses if its (device, inode) moves.
//!
//! The first design put the root under `~/.local/share/yantrik`, and an unsealed job renamed an
//! ancestor (the person's own folders) and took the mount mask with it (round 5, finding 1). With
//! the root out of the person's writable tree there is no ancestor to rename.
//!
//! What a Mind builds there can hold anything a program reads or runs (a git filter, a package
//! hook, a script), so every program a Mind drives is in one of two mount views:
//!
//! - **sealed** (`yantrik_agent_terminal::Sealed`): sees its workshop and nothing else;
//! - **outside** ([`outside_command`]): sees the host, with a tmpfs over the whole of
//!   `/var/lib/yantrik/workshops` (every account's, not just this one) and over `/run/user/<uid>`,
//!   nothing bound back. The second hides the session bus, `systemd/private`, the Yantrik control
//!   sockets and PipeWire, so `systemd-run --user` and a token-less `yos act` do not reach the
//!   person's session from a Mind's job. The price, and it is decided (Fable, 8 Nov): a Mind's
//!   unsealed job (`agent_run`, `terminal.run`, a Mind's Terminal tab, `claude`, the coder's
//!   interpreter) **cannot call `yos`**, which needs the shell's socket in `/run/user`. A token is
//!   not bound back in. A Mind reaches the shell through its own tool calls, not through a job.
//!
//! The companion's own tools run in the shell's process and cannot be given a view. They refuse
//! any path for which [`Root::holds`] is true, which is a prefix test on where the path really is
//! and an inode test on every folder above it: one check, with no walk below the root and no
//! pattern to match, so a link, a `..`, a second mount of the root or a name the check has never
//! seen all land on the same answer.

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::resolved;

/// The folder every account's root is in: root's, `0755`.
pub const BASE: &str = "/var/lib/yantrik/workshops";

/// Where the runtime folder of account `uid` is: hidden from the outside view.
pub fn run_user(uid: u32) -> PathBuf {
    PathBuf::from(format!("/run/user/{uid}"))
}

/// A test's roots live in the test's own home instead of `/var/lib`, which a test cannot make.
/// Called by tests only; nothing a Mind drives can call it.
static UNDER_HOME: AtomicBool = AtomicBool::new(false);

#[doc(hidden)]
pub fn roots_under_home_for_tests() {
    UNDER_HOME.store(true, Ordering::SeqCst);
}

/// The folder all roots are in, for a person whose home is `home`.
pub fn base(home: &Path) -> PathBuf {
    match UNDER_HOME.load(Ordering::SeqCst) {
        true => home.join(".test-workshops"),
        false => PathBuf::from(BASE),
    }
}

fn euid() -> u32 {
    unsafe { libc::geteuid() }
}

/// The root for the person whose home is `home`: `<base>/<uid>`. Where it should be, not proof
/// that it is there or theirs: that is [`open_root`].
pub fn root(home: &Path) -> PathBuf {
    base(home).join(euid().to_string())
}

/// The root as it is shown to a person.
pub fn root_shown(home: &Path) -> String {
    root(home).display().to_string()
}

/// The workshops root, as a place and as a folder (device and inode).
pub struct Root {
    path: PathBuf,
    id: Option<(u64, u64)>,
}

impl Root {
    /// The root at `path`, as it is now: where it really is, and which folder it is.
    pub fn at(path: &Path) -> Root {
        let real = resolved(path).unwrap_or_else(|| path.to_path_buf());
        let id = std::fs::metadata(&real).ok().map(|m| (m.dev(), m.ino()));
        Root { path: real, id }
    }

    /// Is `path` the root or anything under it? True when it resolves (through every link, the
    /// part not made yet included) to a place under the root, or when any folder on the way to it
    /// is the root itself, which is how a second mount of the root is caught.
    pub fn holds(&self, path: &Path) -> bool {
        self.holds_at(path, 0)
    }

    /// The folders that are the root, an account's folder, a Mind's folder or a workshop, by
    /// device and inode. A second view of any of them (a bind or FUSE mount) is as much the root
    /// as a second view of the root is (#1124, L2); this reads three levels and follows no link.
    fn folder_ids(&self) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = self.id.into_iter().collect();
        let mut level = vec![self.path.clone()];
        for (depth, cap) in [(0, 64usize), (1, 256), (2, 64)] {
            let mut next = Vec::new();
            for dir in &level {
                let Ok(entries) = std::fs::read_dir(dir) else { continue };
                for e in entries.flatten().take(cap) {
                    let Some(m) = e.metadata().ok().filter(|m| m.is_dir()) else { continue };
                    out.push((m.dev(), m.ino()));
                    if depth < 2 {
                        next.push(e.path());
                    }
                }
            }
            level = next;
        }
        out
    }

    fn holds_at(&self, path: &Path, depth: u8) -> bool {
        let Some(real) = resolved(path) else { return false };
        if real.starts_with(&self.path) {
            return true;
        }
        let ids = self.folder_ids();
        if !ids.is_empty() && real.ancestors().any(|a| std::fs::metadata(a).is_ok_and(|m| ids.contains(&(m.dev(), m.ino())))) {
            return true;
        }
        // A link on the way that leads nowhere yet (a write through it would make the file there)
        // is judged by where it points.
        let dangling = path.ancestors().find(|a| a.symlink_metadata().is_ok_and(|m| m.is_symlink()) && a.canonicalize().is_err());
        let Some(link) = dangling.filter(|_| depth < 8) else { return false };
        let (Ok(target), Ok(rest)) = (std::fs::read_link(link), path.strip_prefix(link)) else { return false };
        let target = link.parent().map_or(target.clone(), |p| p.join(&target));
        self.holds_at(&target.join(rest), depth + 1)
    }
}

/// Is `path` in a workshop of anyone on this machine? The whole base is held, so another
/// account's workshops are as out of reach as the person's own.
pub fn holds(path: &Path, home: &Path) -> bool {
    Root::at(&base(home)).holds(path)
}

/// `Err(why)` for a tool to return as is when `dir` is in a workshop of this session's home (`$HOME`):
/// a program is not started with its working directory there, because a program keeps a
/// directory it starts in whatever is mounted over its path.
pub fn refuse(dir: &str) -> Result<(), String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    match holds(Path::new(dir), &home) {
        true => Err("Error: that is in a Mind's workshop; a program is not started there. Nothing was run.".to_string()),
        false => Ok(()),
    }
}

/// The `bwrap` words of the outside view: the host as it is (devices included), a tmpfs over each
/// place in `hidden` with nothing bound back, a fresh `/proc` of a pid namespace so no host
/// process's `/proc/<pid>/root` shows them, and no capability left to take a tmpfs off. Up to and
/// including the `--`.
pub fn outside_args(hidden: &[PathBuf]) -> Vec<String> {
    let mut a: Vec<String> = ["--dev-bind", "/", "/"].map(String::from).to_vec();
    for h in hidden {
        a.extend(["--tmpfs".to_string(), h.to_string_lossy().into_owned()]);
    }
    a.extend(["--proc", "/proc", "--unshare-pid", "--die-with-parent", "--cap-drop", "ALL", "--"].map(String::from));
    a
}

/// Where `bwrap` is on this machine, if it is.
pub fn find_bwrap() -> Option<&'static str> {
    ["/usr/bin/bwrap", "/bin/bwrap", "/usr/local/bin/bwrap"].into_iter().find(|p| Path::new(p).exists())
}

fn unready(path: &Path, why: &str) -> String {
    format!(
        "the workshops folder {} {why}. The installer and the updater make it (the shell never does), so no workshop is granted: ask the person to run `sudo yantrik-update`.",
        path.display()
    )
}

/// One pinned root: the folder's (device, inode) and the `O_PATH` descriptor that holds it.
type Pin = (PathBuf, (u64, u64), Arc<OwnedFd>);
static PINS: Mutex<Vec<Pin>> = Mutex::new(Vec::new());

fn stat_fd(fd: i32) -> Result<libc::stat, String> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    match unsafe { libc::fstat(fd, &mut st) } {
        0 => Ok(st),
        _ => Err(std::io::Error::last_os_error().to_string()),
    }
}

fn open_path(path: &Path) -> Result<OwnedFd, std::io::Error> {
    let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| std::io::Error::other("a NUL in the path"))?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    match fd {
        -1 => Err(std::io::Error::last_os_error()),
        fd => Ok(unsafe { OwnedFd::from_raw_fd(fd) }),
    }
}

/// The first folder above `path` that is not root's, or that group or others can write to: someone
/// who can write there can rename what is below it.
fn writable_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors().skip(1).filter(|a| !a.as_os_str().is_empty()).find_map(|a| {
        let m = std::fs::metadata(a).ok();
        m.is_none_or(|m| m.uid() != 0 || m.mode() & 0o022 != 0).then(|| a.to_path_buf())
    })
}

/// The person's root, held. Names are resolved relative to the descriptor ([`OpenRoot::via`]).
pub struct OpenRoot {
    path: PathBuf,
    fd: Arc<OwnedFd>,
}

impl OpenRoot {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The root opened for reading, from the pinned descriptor.
    pub fn open_dir(&self) -> Result<OwnedFd, std::io::Error> {
        let fd = unsafe { libc::openat(self.fd.as_raw_fd(), c".".as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        match fd {
            -1 => Err(std::io::Error::last_os_error()),
            fd => Ok(unsafe { OwnedFd::from_raw_fd(fd) }),
        }
    }

    /// `path` (the root or something under it, as a path) reached through the descriptor, so a
    /// lookup follows the folder that was pinned and not whatever the name leads to now.
    pub fn via(&self, path: &Path) -> PathBuf {
        let rest = path.strip_prefix(&self.path).unwrap_or(Path::new(""));
        PathBuf::from(format!("/proc/self/fd/{}", self.fd.as_raw_fd())).join(rest)
    }
}

/// The root for `home`, if it is there and is the person's. Never made here. `Err(why)` (to be
/// returned as a refusal and raised as a notice) when:
///
/// - `BASE` is not root's, or is writable by anyone else;
/// - the root is missing, is a link, is not the person's, or has any group or other permission;
/// - the root is not the folder it was the first time (device and inode), or is not where its
///   descriptor says it is: it was moved, or replaced.
pub fn open_root(home: &Path) -> Result<OpenRoot, String> {
    let path = root(home);
    if !UNDER_HOME.load(Ordering::SeqCst) {
        let b = open_path(Path::new(BASE)).map_err(|e| unready(Path::new(BASE), &format!("cannot be opened ({e})")))?;
        let st = stat_fd(b.as_raw_fd())?;
        if st.st_uid != 0 || st.st_mode & 0o022 != 0 {
            return Err(unready(Path::new(BASE), "is not root's, or others can write to it"));
        }
        if let Some(above) = writable_ancestor(Path::new(BASE)) {
            return Err(unready(&above, "is above the workshops and is not root's, or others can write to it"));
        }
    }
    let fd = open_path(&path).map_err(|e| unready(&path, &format!("is missing or is a link ({e})")))?;
    let st = stat_fd(fd.as_raw_fd())?;
    if st.st_uid != euid() || st.st_mode & 0o077 != 0 {
        return Err(unready(&path, "is not the person's alone (it must be owned by them, mode 0700)"));
    }
    let here = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap_or_default();
    if here != path {
        return Err(unready(&path, &format!("is not where it was (it is at {})", here.display())));
    }
    let id = (st.st_dev as u64, st.st_ino as u64);
    let mut pins = PINS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, pinned, held)) = pins.iter().find(|(p, _, _)| *p == path) {
        if *pinned != id {
            return Err(unready(&path, "is not the folder it was (it was moved or replaced since the shell first saw it); restart the shell once it is put right"));
        }
        return Ok(OpenRoot { path, fd: held.clone() });
    }
    let held = Arc::new(fd);
    pins.push((path.clone(), id, held.clone()));
    Ok(OpenRoot { path, fd: held })
}

/// `program` and `args` as a program a Mind drives should be started: in the outside view. The
/// view hides the whole of [`BASE`] and the person's `/run/user/<uid>`, whichever is there. With
/// no `bwrap`, a machine that holds workshops (anything in the root) starts nothing; one that
/// holds none (`workshop_create` refuses without `bwrap`) starts the command as it was.
pub fn outside_command(home: &Path, program: &str, args: &[String]) -> Result<(String, Vec<String>), String> {
    let is_dir = |p: &Path| p.symlink_metadata().is_ok_and(|m| m.is_dir());
    let hidden: Vec<PathBuf> = [base(home), run_user(euid())].into_iter().filter(|p| is_dir(p)).collect();
    let holds_any = std::fs::read_dir(root(home)).is_ok_and(|mut d| d.next().is_some());
    let Some(bwrap) = find_bwrap() else {
        return match holds_any {
            true => Err("this machine holds Minds' workshops and has no bubblewrap (bwrap) to hide them from a program; nothing was run.".to_string()),
            false => Ok((program.to_string(), args.to_vec())),
        };
    };
    let mut argv = outside_args(&hidden);
    argv.push(program.to_string());
    argv.extend(args.iter().cloned());
    Ok((bwrap.to_string(), argv))
}

#[cfg(all(test, unix))]
#[path = "workshops_tests.rs"]
mod tests;
