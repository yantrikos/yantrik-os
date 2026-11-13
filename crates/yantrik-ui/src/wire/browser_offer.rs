//! Keeping a browser's DevTools stream on offer to the browser service for as long as the browser
//! runs (#1137, #1200). `browser_pipe` makes the stream and one offer; this is the loop around it,
//! and what it does with a browser nobody takes.

use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use super::browser_peer::ServiceIdentity;
use super::browser_pipe::{send, Refusal, Role};

pub(super) const RETRY: Duration = Duration::from_millis(500);
/// How often a lease is checked for a browser that has gone.
const LEASE_LOOK: Duration = Duration::from_secs(2);

/// The browser this shell started, held by a pidfd opened right after the spawn. A bare pid number
/// is freed when the reaper (`dock`) collects the child, and may then name another process of the
/// person's; the pidfd names the one process for good. So "is it still running" and "stop it" are
/// both asked of the pidfd, and the browser's group is signalled only while the pidfd says its
/// leader has not exited: an unreaped leader pins its pid as the group's id.
pub(super) struct Leader {
    pid: u32,
    fd: Option<OwnedFd>,
}

impl Leader {
    /// Open the pidfd for `pid`, a child just spawned: before anything has reaped it. A kernel
    /// without `pidfd_open` (before 5.3) leaves the bare pid, as before.
    pub(super) fn of(pid: u32) -> Leader {
        // SAFETY: pidfd_open takes a pid and flags; the result is a new fd of ours or -1.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0 as libc::c_uint) };
        if fd < 0 {
            tracing::warn!(pid, error = %std::io::Error::last_os_error(), "No pidfd for the browser; it is watched by its pid");
            return Leader { pid, fd: None };
        }
        // SAFETY: `fd` is a new descriptor and ours.
        Leader { pid, fd: Some(unsafe { OwnedFd::from_raw_fd(fd as i32) }) }
    }

    /// Whether the browser's leader is still running (a pidfd turns readable when it exits).
    fn alive(&self) -> bool {
        let Some(fd) = &self.fd else {
            // SAFETY: signal 0 only asks whether the process exists.
            return unsafe { libc::kill(self.pid as i32, 0) == 0 };
        };
        let mut one = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd, no wait.
        unsafe { libc::poll(&mut one, 1, 0) <= 0 }
    }

    /// SIGKILL to the browser and its group, if the leader has not exited. The browser leads a
    /// group of its own (`attach_for`), so its helpers go with it. Says whether it signalled.
    fn kill(&self) -> bool {
        let Some(fd) = &self.fd else {
            // SAFETY: SIGKILL to the group of a child this process started, and to the child itself.
            unsafe {
                libc::kill(-(self.pid as i32), libc::SIGKILL);
                return libc::kill(self.pid as i32, libc::SIGKILL) == 0;
            }
        };
        if !self.alive() {
            return false;
        }
        // SAFETY: SIGKILL to the group the unreaped leader pins, then to the leader through its pidfd.
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
            libc::syscall(libc::SYS_pidfd_send_signal, fd.as_raw_fd(), libc::SIGKILL, std::ptr::null::<libc::siginfo_t>(), 0u32) == 0
        }
    }
}

/// Offer `mine` to the service until the browser exits; each time the service lets go, again.
/// `identity` is asked for before each offer, so the pid it expects is the service's current one.
/// `leader` is the process the shell spawned, watched and stopped; `holder` is the process that
/// holds the stream (the same one unless the browser runs in the wall).
///
/// Mind View's browser is stopped when the service has not taken it within `bound` of the browser
/// starting or of the service letting go, or at a final refusal: a Mind View browser nobody holds
/// is unfenced, and every later launch would hand over to it and exit, so the service would never
/// hold Mind View again (#1200). The person's Browser is never stopped: it is theirs.
pub(super) fn keep_offered(path: &Path, mine: &OwnedFd, leader: &Leader, holder: u32, role: Role, bound: Duration, identity: impl Fn() -> std::io::Result<ServiceIdentity>) {
    // The pid the service is told is the process that holds the stream (`holder`): the browser
    // itself, which in the wall is below the pid the shell spawned (`leader`).
    let pid = holder;
    let mut unheld_since = Instant::now();
    let mut said = false;
    while leader.alive() {
        let identity = match identity() {
            Ok(identity) => identity,
            Err(e) => {
                tracing::error!(pid, error = %e, "Cannot tell which process is the browser service; the Browser's pipe is not offered");
                if role == Role::MindView {
                    stop_unheld(leader, "nothing can say which process is the browser service");
                }
                return;
            }
        };
        match send(path, mine, pid, role, &identity) {
            Ok(lease) => {
                tracing::info!(pid, "The browser's DevTools pipe is with the browser service");
                said = false;
                wait_for_lease_end(&lease, leader);
                tracing::info!(pid, "The browser service let go of the Browser's pipe; offering it again");
                unheld_since = Instant::now();
            }
            Err(Refusal::Final(why)) => {
                if role == Role::MindView {
                    return stop_unheld(leader, &why);
                }
                return tracing::warn!(pid, %why, "The Browser's DevTools pipe was not handed over, and will not be offered again; the Browser cannot be driven until it is opened again");
            }
            Err(Refusal::Retry(why)) => {
                if unheld_since.elapsed() > bound {
                    if role == Role::MindView {
                        return stop_unheld(leader, &why);
                    }
                    if !said {
                        said = true;
                        tracing::warn!(pid, %why, "The browser service has not taken the Browser's DevTools pipe yet; still offering it");
                    }
                }
                tracing::debug!(%why, "The browser service did not take the pipe yet");
            }
        }
        std::thread::sleep(RETRY);
    }
}

/// Stops a Mind View browser the service does not hold: it is this shell's own child (held by its
/// pidfd, so never another process that reused the number) and leads a group of its own
/// (`attach_for`), so its helpers go with it. The next `browser` call opens one
/// the service takes.
fn stop_unheld(leader: &Leader, why: &str) {
    tracing::warn!(pid = leader.pid, %why, "Mind View's browser was not taken by the browser service; stopping it, since nothing holds or fences it");
    leader.kill();
}

/// Block until the service closes the hand-over connection, or the browser is gone.
fn wait_for_lease_end(lease: &UnixStream, leader: &Leader) {
    lease.set_read_timeout(Some(LEASE_LOOK)).ok();
    let mut reader = lease;
    let mut one = [0u8; 1];
    while leader.alive() {
        match reader.read(&mut one) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::browser_peer::tests::this_process_is_the_service;
    use super::super::browser_pipe::tests::{receive, temp_socket};
    use super::super::browser_pipe::{attach_for, socketpair, Handoff, PIPE_FLAG};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::process::Command;

    /// M3: when the service lets go (it restarted), the shell offers the same stream again, so the
    /// Browser is never left on a pipe nobody holds; it stops when the browser exits.
    #[test]
    fn the_stream_is_offered_again_when_the_service_lets_go() {
        let (path, listener) = temp_socket("lease");
        let (mine, theirs) = socketpair().unwrap();
        let mut browser = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = browser.id();
        let shell = std::thread::spawn(move || keep_offered(&path, &mine, &Leader::of(pid), pid, Role::MindView, Duration::from_secs(30), || Ok(this_process_is_the_service("lease"))));
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
        let shell = std::thread::spawn(move || keep_offered(&offered, &mine, &Leader::of(pid), pid, Role::Person, Duration::from_secs(30), identity));
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

    /// A service that answers every offer with `reply` until `stop` is set; counts the offers.
    fn answering(listener: UnixListener, reply: &'static str, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
        use std::sync::atomic::Ordering;
        let offers = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = offers.clone();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((conn, _)) => {
                        conn.set_nonblocking(false).unwrap();
                        let _ = receive(&conn);
                        counted.fetch_add(1, Ordering::SeqCst);
                        (&conn).write_all(reply.as_bytes()).unwrap();
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        offers
    }

    /// A real child started the way the launcher starts a browser of `role`, and the shell's end of
    /// its stream.
    fn a_browser(role: Role) -> (std::process::Child, Handoff) {
        let mut command = Command::new("sleep");
        command.arg("60");
        let (_, handoff) = attach_for(&mut command, &[PIPE_FLAG], Some(role)).unwrap();
        (command.spawn().unwrap(), handoff)
    }

    /// Waits for `child` to end, and says whether it was killed.
    fn was_killed(child: &mut std::process::Child) -> bool {
        use std::os::unix::process::ExitStatusExt;
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if let Some(status) = child.try_wait().unwrap() {
                return status.signal() == Some(libc::SIGKILL);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// #1200 review, MEDIUM-2: a Mind View browser the service refuses for good is stopped, not
    /// left up in Mind View with no fence on its tabs.
    #[test]
    fn a_final_refusal_stops_mind_views_browser_and_its_group() {
        let (path, listener) = temp_socket("final-mind");
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let offers = answering(listener, "refused: that is not Mind View's browser\n", stop.clone());
        let (mut child, Handoff { mine, theirs, .. }) = a_browser(Role::MindView);
        drop(theirs);
        assert_eq!(unsafe { libc::getpgid(child.id() as i32) }, child.id() as i32, "it leads a group of its own");
        keep_offered(&path, &mine, &Leader::of(child.id()), child.id(), Role::MindView, Duration::from_secs(30), || Ok(this_process_is_the_service("final-mind")));
        assert!(was_killed(&mut child), "the browser nobody holds is stopped");
        assert_eq!(offers.load(std::sync::atomic::Ordering::SeqCst), 1);
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// A browser that does not yet own its profile is offered again, and stopped only when the
    /// bound runs out; the service never answered anything but "retry".
    #[test]
    fn a_browser_not_yet_on_its_profile_is_offered_again_and_then_stopped() {
        let (path, listener) = temp_socket("retry-mind");
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let offers = answering(listener, "retry: that browser does not run on Mind View's profile (yet)\n", stop.clone());
        let (mut child, Handoff { mine, theirs, .. }) = a_browser(Role::MindView);
        drop(theirs);
        keep_offered(&path, &mine, &Leader::of(child.id()), child.id(), Role::MindView, Duration::from_millis(1200), || Ok(this_process_is_the_service("retry-mind")));
        assert!(was_killed(&mut child), "stopped once the bound ran out");
        assert!(offers.load(std::sync::atomic::Ordering::SeqCst) >= 2, "offered again before that");
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The person's Browser is theirs: a refusal is logged, and it is never stopped.
    #[test]
    fn the_persons_browser_is_never_stopped_for_a_refusal() {
        let (path, listener) = temp_socket("final-person");
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _offers = answering(listener, "refused: nope\n", stop.clone());
        let (mut child, Handoff { mine, theirs, .. }) = a_browser(Role::Person);
        drop(theirs);
        keep_offered(&path, &mine, &Leader::of(child.id()), child.id(), Role::Person, Duration::from_millis(50), || Ok(this_process_is_the_service("final-person")));
        assert!(child.try_wait().unwrap().is_none(), "still running");
        child.kill().unwrap();
        child.wait().unwrap();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The service's own words decide: "retry:" and an empty answer are worth another try, any
    /// other refusal is final.
    #[test]
    fn a_retry_answer_and_a_silent_service_are_not_final() {
        for (reply, retry) in [("retry: not yet\n", true), ("", true), ("refused: no\n", false)] {
            let (path, listener) = temp_socket("kinds");
            let (mine, _theirs) = socketpair().unwrap();
            let server = std::thread::spawn(move || {
                let (conn, _) = listener.accept().unwrap();
                let _ = receive(&conn);
                (&conn).write_all(reply.as_bytes()).unwrap();
            });
            let err = send(&path, &mine, 1, Role::MindView, &this_process_is_the_service("kinds")).unwrap_err();
            server.join().unwrap();
            assert_eq!(matches!(err, Refusal::Retry(_)), retry, "{reply:?}: {err:?}");
        }
    }

    /// #1200 review, LOW-1: the pidfd names the one process. A leader that has exited and been
    /// reaped (so its number is free to be reused) is not alive, and nothing is signalled for it;
    /// one still running is stopped.
    #[test]
    fn a_leader_that_was_reaped_is_not_signalled_and_a_live_one_is() {
        let mut gone = Command::new("sleep").arg("60").spawn().unwrap();
        let leader = Leader::of(gone.id());
        assert!(leader.fd.is_some(), "this kernel has pidfd_open");
        assert!(leader.alive());
        gone.kill().unwrap();
        gone.wait().unwrap();
        assert!(!leader.alive(), "reaped");
        assert!(!leader.kill(), "nothing is signalled for a process that has gone");

        let mut running = Command::new("sleep").arg("60").spawn().unwrap();
        let leader = Leader::of(running.id());
        assert!(leader.kill());
        assert!(was_killed(&mut running));
    }
}
