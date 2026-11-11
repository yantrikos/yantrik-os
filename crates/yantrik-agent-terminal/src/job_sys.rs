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


/// `signal` to a job: its whole process group, or, when its leader is a `bwrap` (`walled`), to
/// everything in its session except the `bwrap`s.
///
/// A `bwrap` has no handler for TERM, so a TERM to the group ends it, and with it the pid
/// namespace: the kernel kills every process inside at once, a command that traps TERM to clean up
/// included. That would make the grace period a fiction. So the TERM goes to the command and what
/// it started, which are in the leader's session however `bwrap` hid their pids, and the `bwrap`
/// ends when they do. The KILL after the grace goes to the group, `bwrap` too, and that is the one
/// that takes the namespace down. Whatever left the session (a `setsid` daemon) is not reached by
/// a TERM, and is by that KILL.
pub(crate) fn signal_job(leader: i32, walled: bool, signal: i32) {
    if !walled {
        unsafe {
            libc::kill(-leader, signal);
        }
        return;
    }
    for pid in crate::proc::session_members(leader) {
        unsafe {
            libc::kill(pid, signal);
        }
    }
}

/// How the command's shell ended, when it ran inside `bwrap`, which reports a child that died of
/// signal `n` as the exit code 128+n and so cannot say the two apart.
///
/// `reported` is whether the wrapper's `EXIT` trap wrote the directory: it runs for every way out
/// but SIGKILL, so no word from it with a code in the signal range is a shell that was killed.
/// `asked` is whether the job was told to stop: then 128+SIGTERM is the TERM that was sent (bash
/// runs the trap on it and still dies of it). A TERM from elsewhere, or an `exit 143`, stays an exit
/// code, as it is in any shell.
pub(crate) fn as_the_shell_ended(end: End, reported: bool, asked: bool) -> End {
    match end {
        End::Exited(code) if (129..=128 + 64).contains(&code) && (!reported || (asked && code == 128 + libc::SIGTERM)) => {
            End::Signalled(code - 128)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bwrap_exit_codes_are_read_as_signals_only_where_that_is_what_they_were() {
        // The wrapper never wrote its directory: killed, nothing could run.
        assert_eq!(as_the_shell_ended(End::Exited(137), false, false), End::Signalled(9));
        // It did, and nobody asked it to stop: the shell's own `exit 137`.
        assert_eq!(as_the_shell_ended(End::Exited(137), true, false), End::Exited(137));
        // Asked to stop, and bash ran its trap on the TERM and died of it.
        assert_eq!(as_the_shell_ended(End::Exited(143), true, true), End::Signalled(15));
        assert_eq!(as_the_shell_ended(End::Exited(143), true, false), End::Exited(143));
        // Ordinary codes, and what `bwrap` died of itself, are what they are.
        assert_eq!(as_the_shell_ended(End::Exited(3), false, true), End::Exited(3));
        assert_eq!(as_the_shell_ended(End::Exited(128), false, true), End::Exited(128));
        assert_eq!(as_the_shell_ended(End::Signalled(9), false, true), End::Signalled(9));
    }
}
