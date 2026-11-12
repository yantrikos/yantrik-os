//! A browser's DevTools pipe, made here and handed to the browser service: the person's Browser
//! (#1137) and Mind View's (#1200), through this one path.
//!
//! Chromium started with `--remote-debugging-pipe` reads CDP on fd 3 and writes it on fd 4 and
//! listens on no port. The shell starts the browser (window, session environment, registry: the
//! one launcher), so it makes the stream: a connected socket pair, one end becoming the browser's
//! fds 3 and 4, the other sent to `yantrik-browser` over its `browser-pipe.sock` with
//! SCM_RIGHTS and then closed here. A socket pair, not two pipes: a pipe can be opened again
//! through /proc/<pid>/fd by any process of the same account, a socket cannot.
//!
//! Chromium may close itself when its stream ends, so the shell keeps its own end open for as long
//! as the browser runs, and the hand-over connection stays open as a lease: when the service goes
//! (an update, a crash) the lease ends and the shell offers the stream to the next one. The service
//! is checked before anything is sent (`browser_peer`): the socket's directory belongs to the
//! person's account, so whoever binds the path is asked who it is.
//!
//! Only two launches get one, each said by the arm that makes it and never read off the arguments
//! (review of aff60d15, N3): the person's Browser (the dock's `Launch::Browser` arm,
//! `dock::spawn_persons_browser`) and Mind View's Chromium (a launch in Mind View's seat of the
//! `browser` app, `browser_launch::pipe_role`). A catalogue entry or a `.desktop` file whose
//! arguments carry the pipe switch is started like any other app, with no pipe. Neither browser has
//! a DevTools port: a port is reachable by every process of the machine, and a job that speaks raw
//! DevTools to it goes round the fence the browser service puts on a browser's tabs (#1200). Which
//! of the two a stream is for goes to the service with it (`Role`), and the service checks the
//! profile the process owns against the role, so neither can be offered as the other.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::browser_peer::{ServiceIdentity, Verdict};

pub use yantrik_browser_ports::PIPE_FLAG;

/// The shell's service manager, once it is up: it knows the pid it started `yantrik-browser` as.
static SERVICES: std::sync::OnceLock<yantrik_shell_core::service_manager::ServiceManager> = std::sync::OnceLock::new();

/// Tell this module which service manager starts the browser service (`main`, once).
pub fn note_service_manager(manager: yantrik_shell_core::service_manager::ServiceManager) {
    let _ = SERVICES.set(manager);
}

/// The pid the service manager has `yantrik-browser` running as, if it started one.
fn service_pid() -> Option<u32> {
    SERVICES.get().and_then(|m| m.pid("browser"))
}

/// How long an offer may go unanswered before the shell says so; it keeps offering while the
/// browser runs, because closing its end would close the Browser.
const SAY_AFTER: Duration = Duration::from_secs(30);
const RETRY: Duration = Duration::from_millis(500);
/// How often a lease is checked for a browser that has gone.
const LEASE_LOOK: Duration = Duration::from_secs(2);
/// A second copy of a browser already open hands over and exits within this; it is not offered.
const SETTLE: Duration = Duration::from_millis(1500);

/// Which browser a stream is for. The launch says it; the service checks it against the profile
/// the process owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Person,
    MindView,
}

impl Role {
    /// The word the service reads in the hand-over's header.
    pub fn side(self) -> &'static str {
        match self {
            Role::Person => "person",
            Role::MindView => "mind",
        }
    }
}

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

/// The launcher's one call: if this launch is one of the two browsers that get a pipe (`role`,
/// said by the launch itself, never read off the arguments), its fds 3 and 4 are made here and the
/// shell's end is returned with the role, to be offered once the browser runs. A pipe that cannot
/// be made is logged and the browser starts without one (it cannot then be driven).
pub fn attach_for(command: &mut Command, args: &[&str], role: Option<Role>) -> Option<(Role, Handoff)> {
    let role = role.filter(|role| wants(args, *role))?;
    attach(command).map_err(|e| tracing::warn!(error = %e, ?role, "No DevTools pipe for the browser")).ok().map(|h| (role, h))
}

/// Whether a launch of `role` asks for a pipe. The person's Browser's own launch carries the flag
/// in its arguments (a Firefox there asks for none). Mind View's Chromium has its arguments made by
/// `browser_launch::mind_view_args`, which always adds it, and a launch that is not a Chromium is
/// given no role at all (`browser_launch::pipe_role`).
pub fn wants(args: &[&str], role: Role) -> bool {
    match role {
        Role::Person => args.contains(&PIPE_FLAG),
        Role::MindView => true,
    }
}

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

impl Handoff {
    /// The browser is running as `pid`: close the browser's end here and offer ours to the
    /// service, on a thread that lives as long as the browser. A browser that is gone by then (a
    /// second copy that handed over to the first) is not offered.
    pub fn offer(self, pid: u32, role: Role) {
        let Handoff { mine, theirs } = self;
        drop(theirs);
        let spawned = std::thread::Builder::new().name("browser-pipe".into()).spawn(move || {
            std::thread::sleep(SETTLE);
            // Read afresh for every offer: a restarted service has a new pid, and the service
            // manager records the current one on every start it performs.
            keep_offered(&socket_path(), &mine, pid, role, || ServiceIdentity::installed(service_pid()));
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "Could not start the thread that hands the Browser's pipe over");
        }
    }
}

/// Offer `mine` to the service until the browser exits; each time the service lets go, again.
/// `identity` is asked for before each offer, so the pid it expects is the service's current one.
fn keep_offered(path: &Path, mine: &OwnedFd, pid: u32, role: Role, identity: impl Fn() -> std::io::Result<ServiceIdentity>) {
    let started = Instant::now();
    let mut said = false;
    while alive(pid) {
        let identity = match identity() {
            Ok(identity) => identity,
            Err(e) => return tracing::error!(pid, error = %e, "Cannot tell which process is the browser service; the Browser's pipe is not offered"),
        };
        match send(path, mine, pid, role, &identity) {
            Ok(lease) => {
                tracing::info!(pid, "The browser's DevTools pipe is with the browser service");
                said = false;
                wait_for_lease_end(&lease, pid);
                tracing::info!(pid, "The browser service let go of the Browser's pipe; offering it again");
            }
            Err(Refusal::Final(why)) => {
                return tracing::warn!(pid, %why, "The Browser's DevTools pipe was not handed over, and will not be offered again; the Browser cannot be driven until it is opened again")
            }
            Err(Refusal::Retry(why)) => {
                if !said && started.elapsed() > SAY_AFTER {
                    said = true;
                    tracing::warn!(pid, %why, "The browser service has not taken the Browser's DevTools pipe yet; still offering it");
                }
                tracing::debug!(%why, "The browser service did not take the pipe yet");
            }
        }
        std::thread::sleep(RETRY);
    }
}

/// Block until the service closes the hand-over connection, or the browser is gone.
fn wait_for_lease_end(lease: &UnixStream, pid: u32) {
    lease.set_read_timeout(Some(LEASE_LOOK)).ok();
    let mut reader = lease;
    let mut one = [0u8; 1];
    while alive(pid) {
        match reader.read(&mut one) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
    }
}

/// Why an offer did not succeed: worth another try, or not.
#[derive(Debug)]
pub enum Refusal {
    Retry(String),
    Final(String),
}

/// One hand-over: connect, check who answers, then the JSON line `{"pid":N,"side":"person"|"mind"}` with `fd` in
/// SCM_RIGHTS, then the service's reply line. The connection is returned: it is the lease, and
/// stays open while the service holds the stream.
pub fn send(path: &Path, fd: &OwnedFd, pid: u32, role: Role, identity: &ServiceIdentity) -> Result<UnixStream, Refusal> {
    let retry = |e: String| Refusal::Retry(e);
    let stream = UnixStream::connect(path).map_err(|e| retry(format!("{}: {e}", path.display())))?;
    match identity.check(&stream) {
        Ok(()) => {}
        Err(Verdict::Unreadable(why)) => return Err(retry(why)),
        Err(Verdict::Impostor(why)) | Err(Verdict::Unprovable(why)) => {
            return Err(Refusal::Final(format!("what answers on {} is not the browser service ({why}); nothing was sent", path.display())))
        }
    }
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let line = format!("{{\"pid\":{pid},\"side\":\"{}\"}}\n", role.side());
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
        return Err(retry(std::io::Error::last_os_error().to_string()));
    }
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).map_err(|e| retry(e.to_string()))?;
    match reply.trim() {
        "ok" => Ok(stream),
        other => Err(Refusal::Final(format!("the browser service answered: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::browser_peer::tests::{this_process_is_not_the_service, this_process_is_the_service};
    use std::os::unix::net::UnixListener;

    /// N3: the pipe goes with a launch that was given a role by the arm that makes it; a catalogue
    /// entry or a `.desktop` file whose arguments carry the flag has no role, so no pipe.
    #[test]
    fn only_a_launch_with_a_role_asks_for_a_pipe() {
        assert!(wants(&["--no-first-run", PIPE_FLAG], Role::Person));
        assert!(!wants(&["--no-first-run"], Role::Person), "a Firefox there asks for none");
        assert!(wants(&[], Role::MindView), "Mind View's arguments always carry it");
        let mut command = Command::new("true");
        assert!(attach_for(&mut command, &[PIPE_FLAG], None).is_none(), "the flag in a launch's arguments is not what makes it a browser");
        assert!(attach_for(&mut command, &[PIPE_FLAG], Some(Role::Person)).is_some_and(|(role, _)| role == Role::Person));
        assert!(attach_for(&mut command, &[], Some(Role::MindView)).is_some_and(|(role, _)| role == Role::MindView));
    }

    /// #1200: the header names the side, in the words the service reads.
    #[test]
    fn the_header_says_which_browser_the_stream_is_for() {
        assert_eq!((Role::Person.side(), Role::MindView.side()), ("person", "mind"));
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

    /// What a service does with one connection: the header line and the fd, from one recvmsg.
    fn receive(conn: &UnixStream) -> (String, OwnedFd) {
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
        // SAFETY: the kernel gave us this fd.
        (String::from_utf8_lossy(&buf[..n as usize]).to_string(), unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn temp_socket(name: &str) -> (PathBuf, UnixListener) {
        let dir = std::env::temp_dir().join(format!("yb-pipe-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("browser-pipe.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        (path, listener)
    }

    /// The service receives the pid and a working fd, and the shell reads its "ok".
    #[test]
    fn the_stream_reaches_the_service_over_its_socket() {
        let (path, listener) = temp_socket("reach");
        let (mine, theirs) = socketpair().unwrap();
        let server = std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let got = receive(&conn);
            (&conn).write_all(b"ok\n").unwrap();
            (got, conn)
        });
        let lease = send(&path, &mine, 4242, Role::MindView, &this_process_is_the_service("handover")).unwrap();
        let ((line, received), _conn) = server.join().unwrap();
        assert_eq!(line.trim(), "{\"pid\":4242,\"side\":\"mind\"}");
        drop(lease);
        // The received fd is the same stream: a write on it arrives at `theirs`.
        UnixStream::from(received).write_all(b"x").unwrap();
        let mut one = [0u8; 1];
        UnixStream::from(theirs).read_exact(&mut one).unwrap();
        assert_eq!(&one, b"x");
    }

    /// H1: a process that bound the path is not given the stream unless it is the service.
    #[test]
    fn a_listener_that_is_not_the_service_is_sent_nothing() {
        let (path, listener) = temp_socket("impostor");
        let (mine, _theirs) = socketpair().unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            conn.read_to_end(&mut seen).unwrap();
            seen
        });
        let err = send(&path, &mine, 4242, Role::Person, &this_process_is_not_the_service("impostor")).unwrap_err();
        assert!(matches!(&err, Refusal::Final(why) if why.contains("not the browser service")), "{err:?}");
        assert!(server.join().unwrap().is_empty(), "not a byte, and no fd, reached the impostor");
    }

    /// M3: when the service lets go (it restarted), the shell offers the same stream again, so the
    /// Browser is never left on a pipe nobody holds; it stops when the browser exits.
    #[test]
    fn the_stream_is_offered_again_when_the_service_lets_go() {
        let (path, listener) = temp_socket("lease");
        let (mine, theirs) = socketpair().unwrap();
        let mut browser = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = browser.id();
        let shell = std::thread::spawn(move || keep_offered(&path, &mine, pid, Role::MindView, || Ok(this_process_is_the_service("lease"))));
        // The first service takes the stream and goes away; the second takes it again and stays.
        let (first, _) = listener.accept().unwrap();
        let (_, one) = receive(&first);
        (&first).write_all(b"ok\n").unwrap();
        drop((first, one));
        let (second, _) = listener.accept().unwrap();
        let (_, two) = receive(&second);
        (&second).write_all(b"ok\n").unwrap();
        UnixStream::from(two).write_all(b"y").unwrap();
        let mut got = [0u8; 1];
        UnixStream::from(theirs).read_exact(&mut got).unwrap();
        assert_eq!(&got, b"y", "the second service holds the browser's own stream");
        browser.kill().unwrap();
        browser.wait().unwrap();
        shell.join().unwrap();
    }

    /// M3 with a real restart: the second service is another process, with another pid, which
    /// the service manager has recorded by the time the shell offers again. The check reads the
    /// current pid, so the restarted service is accepted (the first one was the test process).
    #[test]
    fn a_restarted_service_with_a_new_pid_is_offered_the_stream() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let (path, listener) = temp_socket("restart");
        let (mine, _theirs) = socketpair().unwrap();
        let mut browser = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = browser.id();
        let recorded = Arc::new(AtomicU32::new(std::process::id()));
        let identity = {
            let recorded = recorded.clone();
            move || {
                let mut now = this_process_is_the_service("restart");
                now.service_pid = Some(recorded.load(Ordering::SeqCst));
                Ok(now)
            }
        };
        let offered = path.clone();
        let shell = std::thread::spawn(move || keep_offered(&offered, &mine, pid, Role::Person, identity));
        let (first, _) = listener.accept().unwrap();
        let _ = receive(&first);
        (&first).write_all(b"ok
").unwrap();
        // The service goes away; a new process takes its place, and the manager records its pid.
        drop((first, listener));
        let _ = std::fs::remove_file(&path);
        let script = r#"
import socket, sys
sys.stdin.readline()
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.listen(1)
c, _ = s.accept()
c.recvmsg(64, 64)
c.sendall(b"ok" + bytes([10]))
print("held", flush=True)
c.recv(1)
"#;
        let mut service = Command::new("python3")
            .args(["-c", script, path.to_str().unwrap()])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let new_pid = service.id();
        super::super::browser_peer::tests::fake_proc_for(new_pid, "restart", super::super::browser_peer::tests::SHELL, Path::new("/bin/sh"), &["python3", this_process_is_the_service("restart").script.to_str().unwrap()]);
        recorded.store(new_pid, Ordering::SeqCst);
        service.stdin.as_mut().unwrap().write_all(b"go
").unwrap();
        let mut held = String::new();
        BufReader::new(service.stdout.take().unwrap()).read_line(&mut held).unwrap();
        assert_eq!(held.trim(), "held", "the restarted service (pid {new_pid}) was handed the stream");
        browser.kill().unwrap();
        browser.wait().unwrap();
        service.kill().unwrap();
        service.wait().unwrap();
        shell.join().unwrap();
    }

    #[test]
    fn a_refusal_is_an_error_the_shell_logs() {
        let (path, listener) = temp_socket("refusal");
        let (mine, _theirs) = socketpair().unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 64];
            let _ = conn.read(&mut buf);
            conn.write_all(b"refused: only the desktop's shell hands over a browser\n").unwrap();
        });
        let err = send(&path, &mine, 1, Role::Person, &this_process_is_the_service("refusal")).unwrap_err();
        server.join().unwrap();
        assert!(matches!(&err, Refusal::Final(why) if why.contains("refused")), "{err:?}");
    }
}
