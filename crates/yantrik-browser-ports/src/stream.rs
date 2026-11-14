//! A browser's DevTools stream, made for a command about to be spawned: the one hand-over path of
//! every browser that runs on a pipe (#1200). The shell's two browsers (the person's and Mind
//! View's) and the Arcade's headless one all start through `attach`; nothing copies it.
//!
//! Chromium started with `--remote-debugging-pipe` reads CDP on its fd 3 and writes it on its fd 4,
//! and listens on no port. The stream is a connected socket pair, not two pipes: a pipe can be
//! opened again through `/proc/<pid>/fd` by any process of the same account, a socket cannot.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::Command;

/// `close_range`'s flag that marks the range close-on-exec instead of closing it.
const CLOSE_RANGE_CLOEXEC: u32 = 4;

/// Makes `command`'s fds 3 and 4 the browser's end of a new stream. Returns this end, and the
/// browser's own, which must stay open until the command has been spawned and then be dropped, so
/// the stream ends when the browser does. Nothing else of this process's descriptors reaches the
/// child: both ends are close-on-exec, and an fd this process inherited without that flag (a
/// parent's jobserver pipe, a session's redirections) is made close-on-exec in the child.
pub fn attach(command: &mut Command) -> std::io::Result<(UnixStream, OwnedFd)> {
    use std::os::unix::process::CommandExt as _;
    let (mine, theirs) = UnixStream::pair()?; // std's pair is close-on-exec on both ends
    // Above 4, so dup2 onto 3 and 4 cannot land on itself; the copy is close-on-exec too.
    // SAFETY: plain fcntl on an fd we own.
    let high = unsafe { libc::fcntl(theirs.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
    if high < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `high` is a new fd and ours.
    let theirs = unsafe { OwnedFd::from_raw_fd(high) };
    let raw = theirs.as_raw_fd();
    // SAFETY: only async-signal-safe calls (dup2, close_range, fcntl) between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(raw, 3) < 0 || libc::dup2(raw, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            cloexec_from_5();
            Ok(())
        });
    }
    Ok((mine, theirs))
}

/// Marks every fd from 5 up close-on-exec, so the browser holds fds 0 to 4 and nothing else. They
/// are marked, not closed: std's own close-on-exec pipe that reports a failed `exec` stays open
/// until the exec, so a failed one is still an error from `spawn`. Runs between fork and exec, so
/// it makes raw syscalls only: no allocation, no directory listing.
fn cloexec_from_5() {
    // SAFETY: close_range and fcntl on fds the child owns; both are async-signal-safe.
    unsafe {
        if libc::syscall(libc::SYS_close_range, 5u32, u32::MAX, CLOSE_RANGE_CLOEXEC) == 0 {
            return;
        }
        // An older kernel: mark one by one up to the soft limit, bounded in case it is huge.
        let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        let top = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 { limit.rlim_cur.min(65536) as i32 } else { 1024 };
        for fd in 5..top {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::IntoRawFd;

    /// A child reads fd 3 and writes fd 4 on the one stream, and sees no other descriptor of ours
    /// even though one was inherited without close-on-exec.
    #[test]
    fn a_child_reads_fd_3_and_writes_fd_4_and_holds_nothing_else() {
        // An fd of ours with no close-on-exec, as a session's redirection would be.
        // Moved above 10 whatever number the runner's own descriptors leave free (a low one would
        // be 3, 4 or 5, which `attach` makes or keeps), and inheritable.
        let opened = std::fs::File::open("/dev/null").unwrap().into_raw_fd();
        let leaked = unsafe { libc::fcntl(opened, libc::F_DUPFD, 10) };
        unsafe { libc::close(opened) };
        assert!(leaked > 5, "the fd under test must be one the child could keep");
        let mut command = Command::new("sh");
        command.arg("-c").arg("read -r line <&3; echo \"got:$line\" >&4; ls /proc/self/fd | tr '\\n' ' ' >&4");
        let (mut mine, theirs) = attach(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        drop(theirs);
        mine.write_all(b"hello\n").unwrap();
        let mut out = String::new();
        mine.read_to_string(&mut out).unwrap();
        child.wait().unwrap();
        unsafe { libc::close(leaked) };
        assert!(out.starts_with("got:hello\n"), "fd 3 to fd 4: {out:?}");
        let fds = out.trim_start_matches("got:hello\n");
        for fd in fds.split_whitespace().map(|f| f.parse::<u32>().unwrap()) {
            assert!(fd <= 5, "an fd of ours leaked to the browser: {out:?}");
        }
    }

    /// std's own exec-error pipe survives the pass, so a command that cannot be executed is an
    /// error from `spawn` and not a child that exits at once.
    #[test]
    fn a_command_that_cannot_be_executed_is_an_error_from_spawn() {
        let mut command = Command::new("/nonexistent/browser");
        let (_mine, _theirs) = attach(&mut command).unwrap();
        let err = command.spawn().expect_err("the exec error was lost");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
