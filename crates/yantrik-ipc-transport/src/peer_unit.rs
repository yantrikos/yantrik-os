//! Whether the peer of an accepted connection is the Mind (#1176), decided once, at accept.
//!
//! The mind account (`yantrik-mind`) has more than one tenant: the Mind, and another harness
//! (Hermes, #1076) in a unit of its own. Where something is handed out, the uid alone is not
//! enough, so the peer's cgroup must be exactly [`MIND_CGROUP`], the Mind's unit.
//!
//! That is the unit as the boundary: anything the Mind runs inside its own unit (a code sandbox,
//! an MCP server, a plugin) counts as the Mind. A tool sandbox that is not meant to count needs a
//! unit of its own.
//!
//! The answer is read here, at accept, and kept with the connection as `PeerCred::the_mind`. It is
//! never read again from `/proc`: a held connection outlives its connector, the connector's pid is
//! free for reuse, and a later read would judge whichever process has the number by then.
//!
//! A connection is the Mind for as long as it lives, whoever holds it later: the flag rides with
//! the connection, so a process of the Mind's unit that hands its connected socket on has handed
//! on the Mind's standing too.
//!
//! The pid is not trusted on its own even at accept. The kernel's `SO_PEERPIDFD` (Linux 6.5)
//! names the peer as a pidfd, which cannot be reused. The cgroup is read through the pid, then
//! `poll(pidfd, POLLIN, 0)` confirms the pidfd's process has not exited: if it has not, the pid
//! was not freed and reused while the cgroup was read. Poll, not `pidfd_send_signal(pidfd, 0)`:
//! a signal is permission-checked, and the desktop (the person) has no right to signal the Mind
//! (`yantrik-mind`), so it answers `EPERM` for the very process that is alive and the Mind would
//! never be recognised. Poll on a pidfd checks no permission. It reads a zombie as exited, which
//! fails closed.
//!
//! Fails closed. A kernel without `SO_PEERPIDFD` (before 6.5), a peer that has already exited, a
//! `/proc` that cannot be read, no pid, a cgroup line that is not the unified `0::` one: none of
//! them is the Mind. On an older kernel that means the Mind is never recognised and is refused
//! what only the Mind gets; nothing is granted to anyone by mistake. The image's kernel is 6.12+.

use crate::server::PeerCred;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;

/// The cgroup the Mind runs in, as `/proc/<pid>/cgroup` names it on the unified hierarchy.
/// Exactly, not a prefix: the unit delegates no sub-cgroup, so only this path is the unit, and
/// `hermes-gateway.service` or a `.scope` beside it is not.
pub const MIND_CGROUP: &str = "/system.slice/yantrik-mind.service";

/// `SO_PEERPIDFD` (Linux 6.5), which older `libc` headers may not name.
const SO_PEERPIDFD: libc::c_int = 77;

/// The peer of a connection just accepted, with whether it is the Mind decided now.
/// `is_mind` is [`crate::mind_door::is_mind`]; a test names its own uid.
pub(crate) fn accepted(stream: &tokio::net::UnixStream, is_mind: fn(u32) -> bool) -> Option<PeerCred> {
    let c = stream.peer_cred().ok()?;
    let peer = PeerCred { pid: c.pid().unwrap_or(0), uid: c.uid(), gid: c.gid(), the_mind: false };
    let the_mind = the_mind_in(Path::new("/proc"), is_mind, stream.as_raw_fd(), peer);
    Some(PeerCred { the_mind, ..peer })
}

fn the_mind_in(proc: &Path, is_mind: fn(u32) -> bool, fd: RawFd, peer: PeerCred) -> bool {
    if !is_mind(peer.uid) || peer.pid <= 0 {
        return false;
    }
    let Some(pidfd) = peer_pidfd(fd) else { return false };
    the_mind_with(proc, peer, &pidfd)
}

/// The cgroup read through the pid, then the pidfd asked whether its process is still alive.
fn the_mind_with(proc: &Path, peer: PeerCred, pidfd: &OwnedFd) -> bool {
    let in_the_unit = std::fs::read_to_string(proc.join(peer.pid.to_string()).join("cgroup"))
        .is_ok_and(|text| text.lines().any(|line| line.strip_prefix("0::") == Some(MIND_CGROUP)));
    in_the_unit && is_alive(pidfd)
}

/// The pidfd of the process on the other end of `fd`, as it was when the connection was made.
fn peer_pidfd(fd: RawFd) -> Option<OwnedFd> {
    let mut pidfd: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: getsockopt writes one int into a variable of ours, of the length we say.
    let rc = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, SO_PEERPIDFD, (&mut pidfd as *mut libc::c_int).cast(), &mut len) };
    // SAFETY: on success the kernel gave us a new descriptor that nothing else owns.
    (rc == 0 && pidfd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(pidfd) })
}

/// A pidfd becomes readable when its process exits (a zombie counts), whoever polls it and
/// whatever their uid. Nothing readable: the process has not exited.
fn is_alive(pidfd: &OwnedFd) -> bool {
    let mut poll = libc::pollfd { fd: pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: poll on one pollfd of ours, with no wait.
    unsafe { libc::poll(&mut poll, 1, 0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    const ACCOUNT: u32 = 990;
    fn the_account(uid: u32) -> bool {
        uid == ACCOUNT
    }

    /// A fake `/proc` of its own, with `put` writing a pid's cgroup file.
    struct FakeProc(std::path::PathBuf);

    impl FakeProc {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("yantrik-1176-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            FakeProc(dir)
        }
        fn put(&self, pid: i32, text: &str) {
            std::fs::create_dir_all(self.0.join(pid.to_string())).unwrap();
            std::fs::write(self.0.join(pid.to_string()).join("cgroup"), text).unwrap();
        }
    }

    impl Drop for FakeProc {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn me() -> PeerCred {
        PeerCred { pid: std::process::id() as i32, uid: ACCOUNT, gid: ACCOUNT, the_mind: false }
    }

    /// A pidfd of this very process: alive for as long as the test runs.
    fn own_pidfd() -> OwnedFd {
        // SAFETY: pidfd_open of our own pid; the result is a descriptor we own.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, std::process::id() as libc::c_int, 0) };
        assert!(fd >= 0, "pidfd_open needs Linux 5.3");
        // SAFETY: a new descriptor nothing else owns.
        unsafe { OwnedFd::from_raw_fd(fd as RawFd) }
    }

    /// A pidfd of a child that has exited and been reaped: the pid is free, the pidfd is not.
    fn dead_pidfd() -> OwnedFd {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        // SAFETY: pidfd_open of a child we hold, before it is reaped.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id() as libc::c_int, 0) };
        assert!(fd >= 0);
        child.wait().unwrap();
        // SAFETY: a new descriptor nothing else owns.
        unsafe { OwnedFd::from_raw_fd(fd as RawFd) }
    }

    #[test]
    fn the_mind_is_its_unit_exactly_and_a_peer_that_has_gone_is_nobody() {
        let proc = FakeProc::new("unit");
        let alive = own_pidfd();
        let cgroup = |line: &str| {
            proc.put(me().pid, line);
            the_mind_with(&proc.0, me(), &alive)
        };

        assert!(cgroup("0::/system.slice/yantrik-mind.service\n"), "the Mind's unit");
        assert!(!cgroup("0::/system.slice/hermes-gateway.service\n"), "a Hermes in its own unit is not the Mind");
        assert!(!cgroup("0::/system.slice/yantrik-mind.service/child\n"), "a child cgroup is not the unit");
        assert!(!cgroup("0::/system.slice/yantrik-mind.service.scope\n"), "a neighbour is not the unit");
        assert!(!cgroup("0::/user.slice/user-1000.slice/session-3.scope\n"), "a session");
        assert!(!cgroup("1:name=systemd:/system.slice/yantrik-mind.service\n"), "not the unified line");
        assert!(!cgroup("0::/system.slice/yantrik-mind.service (deleted)\n"), "a deleted cgroup");
        assert!(!cgroup(""), "an empty file");

        proc.put(me().pid, "0::/system.slice/yantrik-mind.service\n");
        assert!(!the_mind_with(&proc.0, me(), &dead_pidfd()), "in the unit as read, but its process has gone: the pid may be reused");
        assert!(!the_mind_with(&proc.0, PeerCred { pid: 99_999_999, ..me() }, &alive), "no /proc entry for the pid");
    }

    /// Over a real socket, with the kernel's own `SO_PEERPIDFD`. A kernel before 6.5 has none and
    /// answers "not the Mind", which is the fail-closed behaviour and is asserted instead.
    #[test]
    fn a_real_socket_peer_is_the_mind_only_in_the_unit_and_only_for_the_mind_account() {
        let proc = FakeProc::new("socket");
        let (ours, _theirs) = UnixStream::pair().unwrap();
        let supported = peer_pidfd(ours.as_raw_fd()).is_some();
        let expect = |yes: bool| yes && supported;

        proc.put(me().pid, "0::/system.slice/yantrik-mind.service\n");
        assert_eq!(the_mind_in(&proc.0, the_account, ours.as_raw_fd(), me()), expect(true));
        assert!(!the_mind_in(&proc.0, the_account, ours.as_raw_fd(), PeerCred { uid: 1000, ..me() }), "the unit with another uid");
        assert!(!the_mind_in(&proc.0, the_account, ours.as_raw_fd(), PeerCred { pid: 0, ..me() }), "no pid named");
        assert!(!the_mind_in(&proc.0, |_| false, ours.as_raw_fd(), me()), "no mind account on the machine");

        proc.put(me().pid, "0::/system.slice/hermes-gateway.service\n");
        assert!(!the_mind_in(&proc.0, the_account, ours.as_raw_fd(), me()), "a Hermes in the account");
        if !supported {
            eprintln!("SO_PEERPIDFD is not supported here: asserted fail-closed only");
        }
    }

    /// The decision is made at accept and kept: the connection carries `the_mind`, and nothing
    /// reads `/proc` for it afterwards. What the pid's cgroup says later changes nothing.
    #[test]
    fn what_was_decided_at_accept_is_kept_when_proc_changes_afterwards() {
        let proc = FakeProc::new("kept");
        let (ours, _theirs) = UnixStream::pair().unwrap();
        if peer_pidfd(ours.as_raw_fd()).is_none() {
            eprintln!("SO_PEERPIDFD is not supported here: nothing to keep");
            return;
        }

        // A Hermes at accept stays not the Mind, though its pid later shows the Mind's unit (the
        // number reused by a process of the Mind's).
        proc.put(me().pid, "0::/system.slice/hermes-gateway.service\n");
        let at_accept = PeerCred { the_mind: the_mind_in(&proc.0, the_account, ours.as_raw_fd(), me()), ..me() };
        proc.put(me().pid, "0::/system.slice/yantrik-mind.service\n");
        assert!(!at_accept.the_mind, "decided at accept: not the Mind, and nothing re-reads /proc for it");

        // The Mind at accept stays the Mind, though its pid later shows another unit.
        proc.put(me().pid, "0::/system.slice/yantrik-mind.service\n");
        let at_accept = PeerCred { the_mind: the_mind_in(&proc.0, the_account, ours.as_raw_fd(), me()), ..me() };
        proc.put(me().pid, "0::/system.slice/hermes-gateway.service\n");
        assert!(at_accept.the_mind, "decided at accept: the Mind");
    }

    /// Opens a pidfd of `pid`.
    fn pidfd_of(pid: libc::pid_t) -> OwnedFd {
        // SAFETY: pidfd_open of a pid the caller names; the result is a descriptor we own.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        assert!(fd >= 0, "pidfd_open({pid}) failed");
        // SAFETY: a new descriptor nothing else owns.
        unsafe { OwnedFd::from_raw_fd(fd as RawFd) }
    }

    /// Runs `child` in a forked process, which may use only raw system calls, and answers its pid.
    fn fork_child(child: impl FnOnce() -> libc::c_int) -> libc::pid_t {
        // SAFETY: the child runs one closure of raw system calls and `_exit`s; it returns nowhere.
        unsafe {
            match libc::fork() {
                -1 => panic!("fork failed"),
                0 => libc::_exit(child()),
                pid => pid,
            }
        }
    }

    fn wait_for(pid: libc::pid_t) -> libc::c_int {
        let mut status = 0;
        // SAFETY: waitpid on our own child.
        unsafe { libc::waitpid(pid, &mut status, 0) };
        libc::WEXITSTATUS(status)
    }

    /// The desktop is the person and the Mind is `yantrik-mind`: a live process of another uid
    /// must read as alive. `pidfd_send_signal` answered EPERM here (the round-2 head), so the real
    /// Mind was never recognised. Not root: pid 1 is root's. Root: a child drops to another uid
    /// and asks about a process of a third.
    #[test]
    fn a_live_process_of_another_uid_reads_as_alive() {
        // SAFETY: geteuid cannot fail.
        if unsafe { libc::geteuid() } != 0 {
            assert!(is_alive(&pidfd_of(1)), "pid 1 is alive, and root's");
            return;
        }
        use std::os::unix::process::CommandExt;
        let mut target = std::process::Command::new("sleep").arg("30").uid(65534).spawn().unwrap();
        let pidfd = pidfd_of(target.id() as libc::pid_t);
        let asker = fork_child(|| {
            // SAFETY: setuid to a uid that owns nothing here; checked before it is relied on.
            if unsafe { libc::setuid(65533) } != 0 {
                return 2;
            }
            if is_alive(&pidfd) { 0 } else { 1 }
        });
        let verdict = wait_for(asker);
        target.kill().unwrap();
        target.wait().unwrap();
        assert_eq!(verdict, 0, "an asker of another uid, no CAP_KILL, still sees it alive (2 = could not drop uid)");
    }

    /// A child process connects to a listener at `path` and answers its pid; with `stay` it waits
    /// to be killed, else it exits at once. The listener's backlog holds the connection.
    fn connect_from_child(path: &std::path::Path, stay: bool) -> libc::pid_t {
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (slot, byte) in addr.sun_path.iter_mut().zip(path.as_os_str().as_encoded_bytes()) {
            *slot = *byte as libc::c_char;
        }
        fork_child(|| {
            // SAFETY: socket and connect with an address built before the fork.
            unsafe {
                let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
                let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
                if libc::connect(fd, (&addr as *const libc::sockaddr_un).cast(), len) != 0 {
                    return 3;
                }
                if stay {
                    libc::pause();
                }
            }
            0
        })
    }

    /// A peer that connects and exits before `accepted` runs is not the Mind, even where `/proc`
    /// still shows the Mind's unit for its pid (a stand-in for the number reused by the Mind).
    /// Over a real socket from another process: the live peer is the Mind (the control), the
    /// exited one is not.
    #[test]
    fn a_peer_that_exited_before_accept_is_not_the_mind() {
        let proc = FakeProc::new("exited");
        let dir = std::env::temp_dir().join(format!("yantrik-1176-sock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("door.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let named = |pid| PeerCred { pid, uid: ACCOUNT, gid: ACCOUNT, the_mind: false };

        let gone = connect_from_child(&path, false);
        assert_eq!(wait_for(gone), 0, "the child connected");
        let (after_exit, _) = listener.accept().unwrap();
        proc.put(gone, "0::/system.slice/yantrik-mind.service\n");
        assert!(!the_mind_in(&proc.0, the_account, after_exit.as_raw_fd(), named(gone)), "gone before accept");

        let live = connect_from_child(&path, true);
        let (held, _) = listener.accept().unwrap();
        proc.put(live, "0::/system.slice/yantrik-mind.service\n");
        let supported = peer_pidfd(held.as_raw_fd()).is_some();
        assert_eq!(the_mind_in(&proc.0, the_account, held.as_raw_fd(), named(live)), supported, "a live peer in the unit");
        // SAFETY: kill and reap our own child.
        unsafe { libc::kill(live, libc::SIGKILL) };
        wait_for(live);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
