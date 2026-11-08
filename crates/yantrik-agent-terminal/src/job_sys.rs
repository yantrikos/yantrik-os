//! The system calls around one job: the pipes, the child's file descriptors, reaping and the
//! directory the wrapper reports. Split from `job.rs`, which holds the job itself.

use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;

use crate::job::End;

/// In the child, between fork and exec: put the directory pipe on fd 3, open across exec, then
/// (`seccomp`) the filter on fd 4, and mark every other descriptor the child inherited close-on-exec.
///
/// Why close the rest (#1124, M3): `--unshare-net` does nothing about a socket that is already
/// open. A descriptor in the shell without `O_CLOEXEC` (a Wayland or X connection, a D-Bus
/// socket, an inotify fd from a C library) would otherwise reach a sealed command at
/// `/proc/self/fd/N`, and with it the display, the bus or the network. Only async-signal-safe
/// calls (fcntl, dup2, close, syscall).
pub(crate) fn hand_over_fds(pwd: RawFd, seccomp: Option<RawFd>) -> std::io::Result<()> {
    unsafe {
        // A source that sits on a slot we are about to fill is moved out of the way first.
        let (mut pwd, mut seccomp) = (pwd, seccomp);
        if let Some(s) = seccomp {
            if s == 3 || s == 4 {
                let moved = libc::fcntl(s, libc::F_DUPFD, 5);
                if moved < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                seccomp = Some(moved);
            }
        }
        if pwd == 4 {
            let moved = libc::fcntl(pwd, libc::F_DUPFD, 5);
            if moved < 0 {
                return Err(std::io::Error::last_os_error());
            }
            pwd = moved;
        }
        if pwd == 3 {
            let flags = libc::fcntl(3, libc::F_GETFD);
            if flags < 0 || libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        } else if libc::dup2(pwd, 3) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if let Some(s) = seccomp {
            if libc::dup2(s, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        close_from(if seccomp.is_some() { 5 } else { 4 });
    }
    Ok(())
}

/// Mark every descriptor from `first` up close-on-exec, so the command's `exec` drops them.
/// Close-on-exec and not close, because the standard library's own pipe that reports a failed
/// `exec` to the parent is among them and must live until the `exec`. `close_range` with
/// `CLOSE_RANGE_CLOEXEC` (Linux 5.11); on an older kernel, a loop to the soft limit, capped.
unsafe fn close_from(first: libc::c_uint) {
    const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
    if libc::syscall(libc::SYS_close_range, first, libc::c_uint::MAX, CLOSE_RANGE_CLOEXEC) == 0 {
        return;
    }
    let mut limit: libc::rlimit = std::mem::zeroed();
    let top = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 { limit.rlim_cur.min(65_536) as libc::c_int } else { 4096 };
    for fd in first as libc::c_int..top {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }
}

pub(crate) fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as RawFd; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Whether the leader has ended, without reaping it.
///
/// `WNOWAIT` is the point: while the leader is an unreaped zombie its pid — which is the group's
/// id — cannot be handed to anyone else, so a `SIGKILL` sent to the group after the grace period
/// can only reach this job's processes.
pub(crate) fn peek_exit(pid: i32) -> Option<End> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let done = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if done != 0 {
        // ECHILD: something else reaped it. The command is over and how it ended is lost.
        return (std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD))
            .then_some(End::Exited(-1));
    }
    if unsafe { info.si_pid() } == 0 {
        return None;
    }
    let status = unsafe { info.si_status() };
    match info.si_code {
        libc::CLD_EXITED => Some(End::Exited(status)),
        libc::CLD_KILLED | libc::CLD_DUMPED => Some(End::Signalled(status)),
        _ => None,
    }
}

/// The last complete directory the wrapper wrote, if it is still a directory.
pub(crate) fn reported_directory(pipe: &OwnedFd) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let raw = pipe.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    let mut file = File::from(pipe.try_clone().ok()?);
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = file.read(&mut buf) {
        if n == 0 || bytes.len() > 64 * 1024 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    // Only NUL-terminated records count; take the last. Anything after the last NUL is a record
    // cut short and says nothing.
    let complete = &bytes[..bytes.iter().rposition(|b| *b == 0)?];
    let last = complete.rsplit(|b| *b == 0).next()?;
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(last));
    (path.is_absolute() && path.is_dir()).then_some(path)
}

