//! The person's Browser's DevTools pipe, made here and handed to the browser service (#1137).
//!
//! Chromium started with `--remote-debugging-pipe` reads CDP on fd 3 and writes it on fd 4 and
//! listens on no port. The shell starts the browser (window, session environment, registry: the
//! one launcher), so it makes the stream: a connected socket pair, one end becoming the browser's
//! fds 3 and 4, the other sent to `yantrik-browser` over its `browser-pipe.sock` with
//! SCM_RIGHTS and then closed here. A socket pair, not two pipes: a pipe can be opened again
//! through /proc/<pid>/fd by any process of the same account, a socket cannot.
//!
//! Only the person's Browser gets one. A Mind View browser is launched with its own port, and
//! `browser_launch::mind_view_args` drops the pipe flag from whatever a caller passed.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub use yantrik_browser_ports::PIPE_FLAG;

/// How long the shell keeps offering the stream to a service that is not answering yet.
const OFFER_FOR: Duration = Duration::from_secs(30);
/// A second copy of a browser already open hands over and exits within this; it is not offered.
const SETTLE: Duration = Duration::from_millis(1500);

/// The service's hand-over socket, in the session's socket directory.
pub fn socket_path() -> PathBuf {
    yantrik_ipc_transport::server::socket_dir().join("browser-pipe.sock")
}

/// The shell's end of a browser's stream, until the service has it.
pub struct Handoff {
    mine: OwnedFd,
    theirs: OwnedFd,
}

fn socketpair() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: `fds` is two ints, as socketpair(2) wants.
    let rc = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: both fds are new and ours.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Make `command`'s fds 3 and 4 the browser's end of a new stream; the other end is the result.
/// Nothing else of the shell's descriptors reaches the child: every one here is close-on-exec.
pub fn attach(command: &mut Command) -> std::io::Result<Handoff> {
    let (mine, theirs) = socketpair()?;
    // Above 4, so dup2 onto 3 and 4 cannot land on itself; close-on-exec, so it is gone after exec.
    // SAFETY: plain fcntl on an fd we own.
    let high = unsafe { libc::fcntl(theirs.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
    if high < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `high` is a new fd and ours.
    let theirs = unsafe { OwnedFd::from_raw_fd(high) };
    let raw = theirs.as_raw_fd();
    // SAFETY: only async-signal-safe calls (dup2) between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(raw, 3) < 0 || libc::dup2(raw, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(Handoff { mine, theirs })
}

/// The launcher's one call: if this launch is the person's Browser asking for a pipe, its fds 3 and
/// 4 are made here and the shell's end is returned, to be offered once the browser runs. A pipe
/// that cannot be made is logged and the browser starts without one (it cannot then be driven).
pub fn attach_for(command: &mut Command, args: &[&str], mind_view: bool) -> Option<Handoff> {
    if !wants(args, mind_view) {
        return None;
    }
    attach(command).map_err(|e| tracing::warn!(error = %e, "No DevTools pipe for the Browser")).ok()
}

/// Whether a launch with these arguments is the person's Browser asking for a pipe.
pub fn wants(args: &[&str], mind_view: bool) -> bool {
    !mind_view && args.contains(&PIPE_FLAG)
}

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

impl Handoff {
    /// The browser is running as `pid`: close the browser's end here and offer ours to the
    /// service, on a thread, retrying while it is not up. A browser that is gone by then (a
    /// second copy that handed over to the first) is not offered.
    pub fn offer(self, pid: u32) {
        let Handoff { mine, theirs } = self;
        drop(theirs);
        let spawned = std::thread::Builder::new().name("browser-pipe".into()).spawn(move || {
            std::thread::sleep(SETTLE);
            let deadline = Instant::now() + OFFER_FOR;
            while alive(pid) {
                match send(&socket_path(), &mine, pid) {
                    Ok(()) => return tracing::info!(pid, "The Browser's DevTools pipe is with the browser service"),
                    Err(e) if Instant::now() < deadline => {
                        tracing::debug!(error = %e, "The browser service did not take the pipe yet");
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    Err(e) => {
                        return tracing::warn!(pid, error = %e, "The browser service never took the Browser's DevTools pipe; the Browser cannot be driven until it is opened again")
                    }
                }
            }
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "Could not start the thread that hands the Browser's pipe over");
        }
    }
}

/// One hand-over: the JSON line `{"pid":N}` with `fd` in SCM_RIGHTS, then the service's reply line.
pub fn send(path: &Path, fd: &OwnedFd, pid: u32) -> Result<(), String> {
    let stream = UnixStream::connect(path).map_err(|e| format!("{}: {e}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let line = format!("{{\"pid\":{pid}}}\n");
    let mut iov = libc::iovec { iov_base: line.as_ptr() as *mut _, iov_len: line.len() };
    // SAFETY: CMSG_SPACE is arithmetic.
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    let mut control = vec![0u8; space];
    // SAFETY: a zeroed msghdr pointed at buffers that outlive the call; one SCM_RIGHTS message
    // carrying one fd, laid out with the CMSG macros.
    let sent = unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd.as_raw_fd());
        libc::sendmsg(stream.as_raw_fd(), &msg, 0)
    };
    if sent < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).map_err(|e| e.to_string())?;
    match reply.trim() {
        "ok" => Ok(()),
        other => Err(format!("the browser service answered: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    #[test]
    fn only_the_persons_browser_asks_for_a_pipe() {
        assert!(wants(&["--no-first-run", PIPE_FLAG], false));
        assert!(!wants(&[PIPE_FLAG], true), "Mind View's browser is never given the person's pipe");
        assert!(!wants(&["--no-first-run"], false));
    }

    /// The child's fd 4 is the stream's other end, and what it writes there arrives NUL-ended.
    #[test]
    fn the_child_speaks_on_fd_4_and_hears_on_fd_3() {
        let mut command = Command::new("sh");
        command.args(["-c", "read line <&3; printf '{\"got\":\"%s\"}\\0' \"$line\" >&4"]);
        let handoff = attach(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let Handoff { mine, theirs } = handoff;
        drop(theirs);
        let mut stream = UnixStream::from(mine);
        stream.write_all(b"ping\n").unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        child.wait().unwrap();
        assert_eq!(out, b"{\"got\":\"ping\"}\0");
    }

    /// The service receives the pid and a working fd, and the shell reads its "ok".
    #[test]
    fn the_stream_reaches_the_service_over_its_socket() {
        let dir = std::env::temp_dir().join(format!("yb-pipe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("browser-pipe.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (mine, theirs) = socketpair().unwrap();
        let server = std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            // SAFETY: CMSG_SPACE is arithmetic.
            let mut control = vec![0u8; unsafe { libc::CMSG_SPACE(4) } as usize];
            let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
            // SAFETY: buffers outlive the call; the message is read with the CMSG macros.
            let (n, fd) = unsafe {
                let mut msg: libc::msghdr = std::mem::zeroed();
                msg.msg_iov = &mut iov;
                msg.msg_iovlen = 1;
                msg.msg_control = control.as_mut_ptr().cast();
                msg.msg_controllen = control.len() as _;
                let n = libc::recvmsg(conn.as_raw_fd(), &mut msg, 0);
                let cmsg = libc::CMSG_FIRSTHDR(&msg);
                assert_eq!((*cmsg).cmsg_type, libc::SCM_RIGHTS);
                (n, std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>()))
            };
            let mut conn = conn;
            conn.write_all(b"ok\n").unwrap();
            // SAFETY: the kernel gave us this fd.
            (String::from_utf8_lossy(&buf[..n as usize]).to_string(), unsafe { OwnedFd::from_raw_fd(fd) })
        });
        send(&path, &mine, 4242).unwrap();
        let (line, received) = server.join().unwrap();
        assert_eq!(line.trim(), "{\"pid\":4242}");
        // The received fd is the same stream: a write on it arrives at `theirs`.
        UnixStream::from(received).write_all(b"x").unwrap();
        let mut one = [0u8; 1];
        UnixStream::from(theirs).read_exact(&mut one).unwrap();
        assert_eq!(&one, b"x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refusal_is_an_error_the_shell_logs() {
        let dir = std::env::temp_dir().join(format!("yb-pipe-no-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (mine, _theirs) = socketpair().unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            let _ = conn.read(&mut buf);
            conn.write_all(b"refused: only the desktop's shell hands over a browser\n").unwrap();
        });
        let err = send(&path, &mine, 1).unwrap_err();
        server.join().unwrap();
        assert!(err.contains("refused"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
