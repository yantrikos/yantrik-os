//! What was written, and by whom.
//!
//! `crates/yantrik-os/src/files.rs` already watches directories with inotify, and inotify has one
//! shape of blindness that cannot be worked around: it reports that a file changed and never who
//! changed it. "Something wrote `report.odt`" is a fact with nowhere to go. "LibreOffice saved
//! `report.odt`" is a thing a person would say.
//!
//! fanotify hands over the acting pid with every event, which is the whole reason to prefer it and
//! most of the reason this service is privileged.
//!
//! Two choices worth defending:
//!
//! **`FAN_CLOSE_WRITE`, not "modified".** A save, not a keystroke. An editor with autosave will
//! emit a handful of these an hour; watching writes would emit thousands and drown everything
//! else in the ring.
//!
//! **Directory marks, not a filesystem mark.** `FAN_MARK_FILESYSTEM` would catch everything
//! including subdirectories created later, at the cost of being told about every write on the
//! disk and filtering afterwards — the daemon would *receive* events about `~/.ssh` and choose
//! not to report them. Marking only the directories in scope means those events are never
//! delivered at all. The scope becomes structural rather than a promise, which is the same reason
//! Landlock is here.
//!
//! The cost is real and stated plainly: a directory created after start is not watched until the
//! service restarts.

use std::collections::HashMap;
use std::ffi::CString;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::bus::Bus;
use crate::observation::{Actor, Kind};
use crate::scope::Resolved;

const FAN_CLOEXEC: libc::c_uint = 0x0000_0001;
const FAN_CLASS_NOTIF: libc::c_uint = 0x0000_0000;
const FAN_MARK_ADD: libc::c_uint = 0x0000_0001;
const FAN_MARK_ONLYDIR: libc::c_uint = 0x0000_0008;
const FAN_EVENT_ON_CHILD: u64 = 0x0800_0000;
const FAN_CLOSE_WRITE: u64 = 0x0000_0008;
const FAN_OPEN_EXEC: u64 = 0x0000_1000;

/// `struct fanotify_event_metadata` is 24 bytes: event_len, vers, reserved, metadata_len, mask,
/// fd, pid.
const METADATA_LEN: usize = 24;

/// How deep to walk a watched directory when placing marks.
///
/// Deep enough for a project tree, shallow enough that pointing the scope at something enormous
/// cannot take minutes at startup.
const MAX_DEPTH: usize = 8;

/// Ceiling on marks. Each is a kernel object; an unbounded walk of a large tree would be a way to
/// exhaust kernel memory by editing a config file.
const MAX_MARKS: usize = 4096;

/// Open the fanotify descriptor and place every mark, while still privileged.
///
/// Both halves need `CAP_SYS_ADMIN`, so both happen once on the main thread before the capability
/// is dropped. What remains afterwards is a descriptor the kernel writes events to — which is why
/// this service can give its privileges back and keep watching.
pub fn init_and_mark(scope: &Resolved) -> Result<(libc::c_int, usize), String> {
    if scope.watch.is_empty() {
        return Err("nothing is in scope".into());
    }
    let fd = init()?;

    let mut marked = 0usize;
    for root in &scope.watch {
        marked += mark_tree(fd, root, scope, &mut 0);
    }
    if marked == 0 {
        // Every directory refused. That is a failure, not a quiet success with nothing to watch.
        // SAFETY: closing our own fd on the failure path.
        unsafe { libc::close(fd) };
        return Err("no directory in scope could be marked".into());
    }
    Ok((fd, marked))
}

pub fn run(bus: Bus, scope: &Resolved, fd: libc::c_int) {
    // SAFETY: `fd` came from `init_and_mark` and ownership moves here.
    let mut file = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    let mut buf = [0u8; 8192];
    // Names are read from /proc, which costs a syscall; the same handful of processes cause
    // almost every event, so remembering them is worth one small map.
    let mut names: HashMap<i32, String> = HashMap::new();

    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                bus.push(
                    Kind::SourceFailed { source: "files".into(), reason: e.to_string() },
                    None,
                );
                tracing::warn!(error = %e, "fanotify stopped");
                return;
            }
        };

        let mut offset = 0usize;
        while offset + METADATA_LEN <= n {
            let event_len =
                u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
            if event_len < METADATA_LEN || offset + event_len > n {
                break;
            }
            let mask = u64::from_ne_bytes(buf[offset + 8..offset + 16].try_into().unwrap());
            let event_fd = i32::from_ne_bytes(buf[offset + 16..offset + 20].try_into().unwrap());
            let pid = i32::from_ne_bytes(buf[offset + 20..offset + 24].try_into().unwrap());
            offset += event_len;

            if event_fd < 0 {
                continue;
            }
            let path = resolve(event_fd);
            // The descriptor the kernel opened for us, closed as soon as the path is read. We
            // never read through it: seeing that a file was saved is the capability; reading what
            // was in it is the one Landlock takes away.
            // SAFETY: `event_fd` came from the kernel in this event and is not used again.
            unsafe { libc::close(event_fd) };

            let Some(path) = path else { continue };
            if !scope.allows(&path) {
                // Should be rare — the marks are already scoped — but a `never` path nested
                // inside a watched one lands here. Counted, never described.
                bus.note_out_of_scope();
                continue;
            }

            let name = names
                .entry(pid)
                .or_insert_with(|| {
                    std::fs::read_to_string(format!("/proc/{pid}/comm"))
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default()
                })
                .clone();
            if names.len() > 512 {
                names.clear();
            }

            let actor = Some(Actor { pid, name, parent: None });
            let path = path.to_string_lossy().to_string();
            if mask & FAN_OPEN_EXEC != 0 {
                bus.push(Kind::Executed { path }, actor);
            } else if mask & FAN_CLOSE_WRITE != 0 {
                bus.push(Kind::Saved { path }, actor);
            }
        }
    }
}

fn init() -> Result<libc::c_int, String> {
    // SAFETY: constant arguments; returns a descriptor or -1.
    let fd = unsafe {
        libc::fanotify_init(FAN_CLOEXEC | FAN_CLASS_NOTIF, (libc::O_RDONLY | libc::O_CLOEXEC) as u32)
    };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        return Err(match err.raw_os_error() {
            // Worth naming exactly: this is the capability that makes the service privileged, and
            // "operation not permitted" on its own sends people looking at file modes.
            Some(libc::EPERM) => "fanotify_init: needs CAP_SYS_ADMIN".to_string(),
            _ => format!("fanotify_init: {err}"),
        });
    }
    Ok(fd)
}

/// Mark `dir` and its subdirectories, returning how many marks were placed.
fn mark_tree(fd: libc::c_int, dir: &Path, scope: &Resolved, depth: &mut usize) -> usize {
    if *depth > MAX_DEPTH {
        return 0;
    }
    if !scope.allows(dir) && !scope.watch.iter().any(|w| w == dir) {
        return 0;
    }

    let mut placed = if mark_one(fd, dir).is_ok() { 1 } else { 0 };

    let Ok(entries) = std::fs::read_dir(dir) else { return placed };
    for entry in entries.flatten() {
        if placed >= MAX_MARKS {
            tracing::warn!(limit = MAX_MARKS, "fanotify mark limit reached; deeper paths unwatched");
            break;
        }
        let path = entry.path();
        // `is_dir` follows symlinks; a link out of the scope would silently widen it, and a link
        // back into it would mark the same tree twice.
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        *depth += 1;
        placed += mark_tree(fd, &path, scope, depth);
        *depth -= 1;
    }
    placed
}

fn mark_one(fd: libc::c_int, dir: &Path) -> Result<(), ()> {
    let Ok(c_dir) = CString::new(dir.as_os_str().as_bytes()) else { return Err(()) };
    // SAFETY: `c_dir` is NUL-terminated and outlives the call; AT_FDCWD with an absolute path.
    let rc = unsafe {
        libc::fanotify_mark(
            fd,
            FAN_MARK_ADD | FAN_MARK_ONLYDIR,
            FAN_CLOSE_WRITE | FAN_OPEN_EXEC | FAN_EVENT_ON_CHILD,
            libc::AT_FDCWD,
            c_dir.as_ptr(),
        )
    };
    if rc < 0 {
        Err(())
    } else {
        Ok(())
    }
}

/// The path behind a descriptor the kernel handed us.
fn resolve(fd: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{fd}")).ok().filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_metadata_layout_matches_the_kernels() {
        // event_len(4) + vers(1) + reserved(1) + metadata_len(2) + mask(8) + fd(4) + pid(4).
        assert_eq!(METADATA_LEN, 24);
    }

    #[test]
    fn we_ask_for_saves_and_execs_and_nothing_else() {
        let mask = FAN_CLOSE_WRITE | FAN_OPEN_EXEC | FAN_EVENT_ON_CHILD;
        // FAN_MODIFY would fire on every write and bury everything else in the ring.
        const FAN_MODIFY: u64 = 0x0000_0002;
        const FAN_OPEN: u64 = 0x0000_0020;
        assert_eq!(mask & FAN_MODIFY, 0, "watching writes would be a flood, not a signal");
        assert_eq!(mask & FAN_OPEN, 0, "every open of every file is not perception");
    }

    #[test]
    fn a_descriptor_that_resolves_to_nothing_is_dropped() {
        // Descriptor 9999 is not open in the test process.
        assert!(resolve(9999).is_none());
    }
}
