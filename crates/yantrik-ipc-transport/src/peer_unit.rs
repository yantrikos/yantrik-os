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
//! The pid is not trusted on its own even at accept. The kernel's `SO_PEERPIDFD` (Linux 6.5)
//! names the peer as a pidfd, which cannot be reused. The cgroup is read through the pid, then
//! `pidfd_send_signal(pidfd, 0)` confirms the pidfd's process is still the one alive: if it is,
//! the pid was not freed and reused while the cgroup was read.
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

/// Signal 0 checks, and sends nothing: it succeeds only for a process that has not exited.
fn is_alive(pidfd: &OwnedFd) -> bool {
    // SAFETY: pidfd_send_signal with a descriptor we own, signal 0, and no siginfo.
    unsafe { libc::syscall(libc::SYS_pidfd_send_signal, pidfd.as_raw_fd(), 0, std::ptr::null::<libc::c_void>(), 0) == 0 }
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
}
