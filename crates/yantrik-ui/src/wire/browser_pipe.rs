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
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::browser_offer::RETRY;
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

/// How long an offer may go unanswered before the shell says so. The person's Browser keeps being
/// offered while it runs, because closing its end would close the Browser. Mind View's does not:
/// a browser the service has not held by then is stopped (`stop_unheld`).
const SAY_AFTER: Duration = Duration::from_secs(30);
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
    pub(super) mine: OwnedFd,
    pub(super) theirs: OwnedFd,
    /// The inode of the browser's end, to find which process holds it once the browser runs.
    pub(super) inode: u64,
}

#[cfg(test)]
pub(super) fn socketpair() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let (mine, theirs) = std::os::unix::net::UnixStream::pair()?;
    Ok((mine.into(), theirs.into()))
}

/// Make `command`'s fds 3 and 4 the browser's end of a new stream; the other end is the result.
/// The one path of every browser that runs on a pipe, the Arcade's too: `yantrik_browser_ports::
/// stream::attach`. Nothing else of the shell's descriptors reaches the child.
pub fn attach(command: &mut Command) -> std::io::Result<Handoff> {
    let (mine, theirs) = yantrik_browser_ports::stream::attach(command)?;
    // The inode of the browser's end: the wall's bwrap holds no stream, so the process that does is
    // found by it (`browser_holder`).
    let inode = std::fs::File::from(theirs.try_clone()?).metadata()?.ino();
    Ok(Handoff { mine: mine.into(), theirs, inode })
}

/// The launcher's one call: if this launch is one of the two browsers that get a pipe (`role`,
/// said by the launch itself, never read off the arguments), its fds 3 and 4 are made here and the
/// shell's end is returned with the role, to be offered once the browser runs. A pipe that cannot
/// be made is logged and the browser starts without one (it cannot then be driven).
pub fn attach_for(command: &mut Command, args: &[&str], role: Option<Role>) -> Option<(Role, Handoff)> {
    let role = role.filter(|role| wants(args, *role))?;
    // Debian's chromium wrapper reads these and passes them to the browser: a port named there
    // would be a listener this launch never asked for.
    command.env_remove("CHROMIUM_FLAGS").env_remove("CHROMIUM_USER_FLAGS");
    if role == Role::MindView {
        // A group of its own, so a Mind View browser nobody holds can be stopped with its helpers.
        command.process_group(0);
    }
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

/// The person's Browser the shell itself started with a pipe, while it runs (0: none). Mind View's
/// browser is not recorded here: the notice below is about the person's Browser.
static OWN_BROWSER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn own_browser() -> Option<u32> {
    match OWN_BROWSER.load(std::sync::atomic::Ordering::SeqCst) {
        0 => None,
        pid => alive(pid).then_some(pid),
    }
}

/// Forget `holder` as the shell's own Browser, but only once it has exited.
fn clear_once_gone(holder: u32, every: Duration) {
    while alive(holder) {
        std::thread::sleep(every);
    }
    let _ = OWN_BROWSER.compare_exchange(holder, 0, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst);
}

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Raise the "not drivable" notice for `app_id`, but only for a clean hand-over exit: the launch
/// has exited (`pid` is gone), nothing has recorded why (a `Failed` record is the real reason and
/// is never overwritten), the shell has no Browser of its own, and the person's account runs
/// another one that is theirs (the Minds' browsers are not).
fn note_not_drivable(app_id: &str, proc: &Path, pid: u32, uid: u32, profiles: &yantrik_browser_ports::Profiles) {
    if !alive(pid)
        && crate::running::last_launch_failure(app_id).is_none()
        && own_browser().is_none()
        && super::browser_holder::another_browser_is_open(proc, None, uid, profiles)
    {
        tracing::warn!(pid, "The Browser joined one Yantrik did not start; it cannot be driven");
        crate::running::mark_launch_failed(app_id, app_id, super::browser_holder::NOT_DRIVABLE, 0);
    } else {
        tracing::warn!(pid, "No process under the Browser's launch holds its DevTools pipe");
    }
}

impl Handoff {
    /// The browser is running as `pid`: close the browser's end here and offer ours to the
    /// service, on a thread that lives as long as the browser. A browser that is gone by then (a
    /// second copy that handed over to the first) is not offered.
    ///
    /// `pid` is the process the shell spawned. In the wall that is the outer bwrap, which holds no
    /// stream: the service is told the process that does (`browser_holder`), and the shell keeps
    /// following `pid` (the `leader`) for the app's life.
    pub fn offer(self, pid: u32, role: Role) {
        let Handoff { mine, theirs, inode } = self;
        drop(theirs);
        // Opened here, right after the spawn and before the reaper thread exists, while the pid
        // still names this child and nobody else.
        let leader = super::browser_offer::Leader::of(pid);
        let spawned = std::thread::Builder::new().name("browser-pipe".into()).spawn(move || {
            std::thread::sleep(SETTLE);
            let proc = Path::new("/proc");
            let Some(holder) = super::browser_offer::find_holder(&leader, role, RETRY, SAY_AFTER, || super::browser_holder::holder_of(proc, pid, inode)) else {
                // (Mind View's browser was looked for until `SAY_AFTER`, and stopped if alive.)
                // This launch did not become a Browser with the pipe: it handed over to one that
                // was already open (or died, which the reaper reports). If the shell has none of
                // its own and the person's account runs another, the dock joined an instance
                // nothing can drive (one `xdg-open` started, outside the wall): say so.
                // SAFETY: getuid has no failure mode.
                let uid = unsafe { libc::getuid() };
                if role == Role::Person {
                    note_not_drivable("browser", proc, pid, uid, &yantrik_browser_ports::Profiles::current());
                }
                return;
            };
            if role == Role::Person {
                OWN_BROWSER.store(holder, std::sync::atomic::Ordering::SeqCst);
            }
            // Read afresh for every offer: a restarted service has a new pid, and the service
            // manager records the current one on every start it performs.
            super::browser_offer::keep_offered(&socket_path(), &mine, &leader, holder, role, SAY_AFTER, || ServiceIdentity::installed(service_pid()));
            if role == Role::Person {
                // `keep_offered` can return with the Browser still running (a final refusal): the
                // slot is cleared only once the holder is gone, and a later process that reuses its
                // pid must not pass for it.
                clear_once_gone(holder, RETRY);
            }
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "Could not start the thread that hands the Browser's pipe over");
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
        // A service that went away mid-answer has not said no.
        "" => Err(retry("the browser service closed the connection without answering".into())),
        // "retry: ..." is the service saying it could not tell yet (the browser has not opened its
        // profile, /proc was unreadable once); anything else it refuses is final.
        other if other.starts_with("retry:") => Err(retry(other.into())),
        other => Err(Refusal::Final(format!("the browser service answered: {other}"))),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use super::super::browser_peer::tests::{this_process_is_not_the_service, this_process_is_the_service};
    use std::os::unix::net::UnixListener;

    /// `OWN_BROWSER` is one slot for the whole process: the tests that set or read it take turns.
    static OWN_SLOT: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn fake_proc(name: &str, cmdline: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yb-notice-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("900").join("fd")).unwrap();
        std::fs::write(dir.join("900").join("cmdline"), cmdline.replace(' ', "\0")).unwrap();
        dir
    }

    /// The Browser's slot is cleared when the holder is gone, and not before: a `keep_offered`
    /// that gave up (a final refusal) leaves the Browser running, and it is still the shell's own.
    #[test]
    fn the_own_browser_slot_is_cleared_only_once_the_holder_has_exited() {
        let _slot = OWN_SLOT.lock().unwrap_or_else(|e| e.into_inner());
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        OWN_BROWSER.store(pid, std::sync::atomic::Ordering::SeqCst);
        let waiter = std::thread::spawn(move || clear_once_gone(pid, Duration::from_millis(20)));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(own_browser(), Some(pid), "the holder is running: it is still the shell's own");
        child.kill().unwrap();
        child.wait().unwrap();
        waiter.join().unwrap();
        assert_eq!(own_browser(), None);
        assert_eq!(OWN_BROWSER.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    fn a_gone_pid() -> u32 {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    fn profiles() -> yantrik_browser_ports::Profiles {
        yantrik_browser_ports::Profiles { mind_view: "/p/mind-view".into(), identities: "/p/identities".into() }
    }

    /// The notice is for a clean hand-over exit only: a `Failed` record is the real reason, and
    /// stays; the person's own other browser is what raises it.
    #[test]
    fn a_failed_launch_record_is_not_overwritten_by_the_notice() {
        let _slot = OWN_SLOT.lock().unwrap_or_else(|e| e.into_inner());
        let me = unsafe { libc::getuid() };
        let proc = fake_proc("failed", "/usr/bin/chromium --no-first-run");
        let pid = a_gone_pid();
        crate::running::mark_launch_failed("notice-test-failed", "chromium", "exit status: 1", 300);
        note_not_drivable("notice-test-failed", &proc, pid, me, &profiles());
        let kept = crate::running::last_launch_failure("notice-test-failed").unwrap();
        assert_eq!((kept.status.as_str(), kept.lived_ms), ("exit status: 1", 300));
        // With nothing recorded, the same situation raises it.
        note_not_drivable("notice-test-clean", &proc, pid, me, &profiles());
        let said = crate::running::last_launch_failure("notice-test-clean").unwrap();
        assert_eq!(said.status, super::super::browser_holder::NOT_DRIVABLE);
    }

    #[test]
    fn the_notice_is_not_raised_for_a_minds_browser_or_a_launch_still_running() {
        let _slot = OWN_SLOT.lock().unwrap_or_else(|e| e.into_inner());
        let me = unsafe { libc::getuid() };
        let minds = fake_proc("minds", "/usr/bin/chromium --headless=new --remote-debugging-port=9300");
        note_not_drivable("notice-test-minds", &minds, a_gone_pid(), me, &profiles());
        assert!(crate::running::last_launch_failure("notice-test-minds").is_none());
        let other = fake_proc("running", "/usr/bin/chromium --no-first-run");
        note_not_drivable("notice-test-running", &other, std::process::id(), me, &profiles());
        assert!(crate::running::last_launch_failure("notice-test-running").is_none(), "the launch has not exited");
    }

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

    /// Review LOW-5: the wrapper's flag variables never reach either browser.
    #[test]
    fn the_wrappers_flag_variables_are_cleared_from_both_browsers() {
        for role in [Role::Person, Role::MindView] {
            let mut command = Command::new("sh");
            command.args(["-c", "printf '%s|%s' \"${CHROMIUM_FLAGS-unset}\" \"${CHROMIUM_USER_FLAGS-unset}\""]);
            command.env("CHROMIUM_FLAGS", "--remote-debugging-port=9333").env("CHROMIUM_USER_FLAGS", "--remote-debugging-port=9334");
            let (_, _handoff) = attach_for(&mut command, &[PIPE_FLAG], Some(role)).unwrap();
            let out = command.output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), "unset|unset", "{role:?}");
        }
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
        let Handoff { mine, theirs, .. } = handoff;
        drop(theirs);
        let mut stream = UnixStream::from(mine);
        stream.write_all(b"ping\n").unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        child.wait().unwrap();
        assert_eq!(out, b"{\"got\":\"ping\"}\0");
    }

    /// What a service does with one connection: the header line and the fd, from one recvmsg.
    pub(in super::super) fn receive(conn: &UnixStream) -> (String, OwnedFd) {
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

    pub(in super::super) fn temp_socket(name: &str) -> (PathBuf, UnixListener) {
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
