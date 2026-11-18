//! Who is at the other end of `browser-pipe.sock`, checked before the shell sends it a browser's
//! DevTools stream (#1151).
//!
//! The socket sits in a directory the person's own account owns, so any process of that account
//! can unlink the service's socket and bind its own. Whoever answers is therefore asked who it is,
//! from the kernel (`SO_PEERCRED`, filled in when the listener was made), and must be the browser
//! service this shell started: the pid the shell's service manager started, a child of this shell,
//! running a root-owned interpreter on the installed `yantrik-browser` script (compared by device
//! and inode, not by name). A pid number is read through `/proc`, and a number outlives the
//! process it named: the listener's maker can exit, hand its socket on, and have its number land
//! on the real service later (review of aff60d15, N1). So the kernel is also asked for a pidfd of
//! the maker itself (`SO_PEERPIDFD`), and that process must still be alive once `/proc` has been
//! read: a live process keeps its number, so what was read was its own. A process of the same
//! account can still rewrite its own argv; what makes this a boundary is the Minds running as
//! another account (#1137 step 3).

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// What the shell expects of the process at the socket.
pub struct ServiceIdentity {
    pub shell_pid: u32,
    pub uid: u32,
    pub proc: PathBuf,
    /// The installed `yantrik-browser`, which the service's interpreter is running.
    pub script: PathBuf,
    /// The pid the shell's service manager started `yantrik-browser` as, when it has one running.
    pub service_pid: Option<u32>,
}

/// Why a peer is not the service. `Unreadable` may pass on a second look (a process exec'ing);
/// `Impostor` will not; `Unprovable` is a kernel that cannot say, and will not either.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Unreadable(String),
    Impostor(String),
    Unprovable(String),
}

impl ServiceIdentity {
    /// This shell, and the service script installed beside it.
    pub fn installed(service_pid: Option<u32>) -> io::Result<Self> {
        let exe = std::env::current_exe()?;
        let script = exe.parent().map(|d| d.join("yantrik-browser")).ok_or_else(|| io::Error::other("the shell has no directory"))?;
        // SAFETY: getuid cannot fail.
        Ok(Self { shell_pid: std::process::id(), uid: unsafe { libc::getuid() }, proc: "/proc".into(), script, service_pid })
    }

    /// Check the process that made the listener `stream` is connected to: alive before `/proc` is
    /// read and alive after, so the number read in between was its own and nobody else's.
    pub fn check(&self, stream: &UnixStream) -> Result<(), Verdict> {
        let peer = Peer::of(stream)?;
        peer.alive()?;
        self.check_peer(peer.pid, peer.uid)?;
        peer.alive()
    }

    pub fn check_peer(&self, pid: u32, uid: u32) -> Result<(), Verdict> {
        if uid != self.uid {
            return Err(Verdict::Impostor(format!("pid {pid} runs as uid {uid}, not as this account")));
        }
        if let Some(started) = self.service_pid {
            if pid != started {
                return Err(Verdict::Impostor(format!("pid {pid} is not the browser service this shell started (pid {started})")));
            }
        }
        let at = self.proc.join(pid.to_string());
        let unreadable = |what: &str, e: io::Error| Verdict::Unreadable(format!("{what} of pid {pid}: {e}"));
        let parent = parent_of(&at.join("stat")).map_err(|e| unreadable("the parent", e))?;
        if parent != self.shell_pid {
            return Err(Verdict::Impostor(format!("pid {pid} was not started by this shell (its parent is {parent})")));
        }
        let interpreter = fs::metadata(at.join("exe")).map_err(|e| unreadable("the program", e))?;
        if interpreter.uid() != 0 {
            return Err(Verdict::Impostor(format!("pid {pid} runs a program that root does not own")));
        }
        let cmdline = fs::read(at.join("cmdline")).map_err(|e| unreadable("the command line", e))?;
        let named = cmdline.split(|b| *b == 0).nth(1).map(|b| PathBuf::from(String::from_utf8_lossy(b).into_owned()));
        let ours = fs::metadata(&self.script).map_err(|e| Verdict::Unreadable(format!("{}: {e}", self.script.display())))?;
        match named.and_then(|s| fs::metadata(s).ok()) {
            Some(theirs) if (theirs.dev(), theirs.ino()) == (ours.dev(), ours.ino()) => Ok(()),
            _ => Err(Verdict::Impostor(format!("pid {pid} is not running {}", self.script.display()))),
        }
    }
}

fn parent_of(stat: &Path) -> io::Result<u32> {
    let text = fs::read_to_string(stat)?;
    // The command name is in parentheses and may hold spaces; the fields start after the last ')'.
    text.rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().nth(1))
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| io::Error::other("unreadable stat"))
}

/// The kernel's record of the process that made the listener: its pid and uid at `listen()`, and
/// a pidfd of that very process, which stays its own however its number is reused.
pub(crate) struct Peer {
    pub pid: u32,
    pub uid: u32,
    pidfd: OwnedFd,
}

impl Peer {
    pub(crate) fn of(stream: &UnixStream) -> Result<Peer, Verdict> {
        let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred` and `len` are the buffer getsockopt(SO_PEERCRED) fills in.
        let rc = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) };
        if rc != 0 {
            return Err(Verdict::Unreadable(format!("the kernel would not say who answers: {}", io::Error::last_os_error())));
        }
        let mut fd: libc::c_int = -1;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: `fd` and `len` are the buffer getsockopt(SO_PEERPIDFD) fills in.
        let rc = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERPIDFD, (&mut fd as *mut libc::c_int).cast(), &mut len) };
        if rc != 0 || fd < 0 {
            let e = io::Error::last_os_error();
            return Err(match e.raw_os_error() {
                // Linux before 6.5 has no SO_PEERPIDFD: the maker cannot be told from a later holder of its number.
                Some(libc::ENOPROTOOPT) => Verdict::Unprovable(format!("this kernel cannot name the process behind the socket (SO_PEERPIDFD): {e}")),
                // A maker already reaped gets no pidfd before 6.15 (EINVAL from pidfd_prepare; ESRCH on some
                // later kernels): the number SO_PEERCRED gave is nobody's now, or another process's.
                Some(libc::EINVAL) | Some(libc::ESRCH) => {
                    Verdict::Impostor(format!("pid {} is no longer the process that made the listener (it has exited: {e})", cred.pid))
                }
                _ => Verdict::Unreadable(format!("the kernel would not name the process behind the socket: {e}")),
            });
        }
        // SAFETY: the kernel gave us this fd.
        Ok(Peer { pid: cred.pid as u32, uid: cred.uid, pidfd: unsafe { OwnedFd::from_raw_fd(fd) } })
    }

    /// `Impostor` when the process the pidfd names has exited: the number in `pid` is then
    /// another process's, or nobody's.
    pub(crate) fn alive(&self) -> Result<(), Verdict> {
        // SAFETY: pidfd_send_signal with signal 0 only asks whether the process still exists.
        let rc = unsafe { libc::syscall(libc::SYS_pidfd_send_signal, self.pidfd.as_raw_fd(), 0, std::ptr::null::<libc::siginfo_t>(), 0u32) };
        match rc {
            0 => Ok(()),
            _ => Err(Verdict::Impostor(format!("pid {} is no longer the process that made the listener ({})", self.pid, io::Error::last_os_error()))),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    const ME: u32 = 1000;
    pub(crate) const SHELL: u32 = 500;
    const SERVICE: u32 = 700;

    /// A fake /proc with one process whose parent, program and command line are as given.
    fn fake_proc(name: &str, parent: u32, exe: &Path, cmdline: &[&str]) -> PathBuf {
        fake_proc_for(SERVICE, name, parent, exe, cmdline)
    }

    pub(crate) fn fake_proc_for(pid: u32, name: &str, parent: u32, exe: &Path, cmdline: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("yb-peer-{name}-{}", std::process::id()));
        let at = root.join(pid.to_string());
        fs::create_dir_all(&at).unwrap();
        fs::write(at.join("stat"), format!("{SERVICE} (python3 x) S {parent} 1 1 0\n")).unwrap();
        fs::write(at.join("cmdline"), cmdline.iter().map(|a| format!("{a}\0")).collect::<String>()).unwrap();
        let _ = fs::remove_file(at.join("exe"));
        symlink(exe, at.join("exe")).unwrap();
        root
    }

    fn identity(proc: PathBuf, script: &Path) -> ServiceIdentity {
        ServiceIdentity { shell_pid: SHELL, uid: ME, proc, script: script.into(), service_pid: None }
    }

    fn script(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("yb-script-{name}-{}", std::process::id()));
        fs::write(&path, "#!/usr/bin/python3\n").unwrap();
        path
    }

    const ROOT_OWNED: &str = "/bin/sh";

    /// An identity that this test process itself passes: a listener bound in a test is "the service".
    pub(crate) fn this_process_is_the_service(name: &str) -> ServiceIdentity {
        let s = script(name);
        let pid = std::process::id();
        // SAFETY: getuid cannot fail.
        let uid = unsafe { libc::getuid() };
        let proc = fake_proc_for(pid, name, SHELL, Path::new(ROOT_OWNED), &["python3", s.to_str().unwrap()]);
        ServiceIdentity { shell_pid: SHELL, uid, proc, script: s, service_pid: Some(pid) }
    }

    /// An identity that this test process does not pass: another script is the installed one.
    pub(crate) fn this_process_is_not_the_service(name: &str) -> ServiceIdentity {
        ServiceIdentity { script: script(&format!("{name}-other")), ..this_process_is_the_service(name) }
    }

    #[test]
    fn the_service_the_shell_started_passes() {
        let s = script("ok");
        let proc = fake_proc("ok", SHELL, Path::new(ROOT_OWNED), &["/usr/bin/python3", s.to_str().unwrap()]);
        assert_eq!(identity(proc, &s).check_peer(SERVICE, ME), Ok(()));
    }

    #[test]
    fn another_account_is_not_the_service() {
        let s = script("uid");
        let proc = fake_proc("uid", SHELL, Path::new(ROOT_OWNED), &["python3", s.to_str().unwrap()]);
        assert!(matches!(identity(proc, &s).check_peer(SERVICE, 0), Err(Verdict::Impostor(_))));
    }

    /// A process of the same account that bound the path itself was not started by the shell.
    #[test]
    fn a_process_the_shell_did_not_start_is_not_the_service() {
        let s = script("parent");
        let proc = fake_proc("parent", 1, Path::new(ROOT_OWNED), &["python3", s.to_str().unwrap()]);
        assert!(matches!(identity(proc, &s).check_peer(SERVICE, ME), Err(Verdict::Impostor(_))));
    }

    /// N1: the service manager knows which pid it started; another pid, however it reads, is not it.
    #[test]
    fn a_pid_other_than_the_one_the_shell_started_is_not_the_service() {
        let s = script("pid");
        let proc = fake_proc("pid", SHELL, Path::new(ROOT_OWNED), &["python3", s.to_str().unwrap()]);
        let known = ServiceIdentity { service_pid: Some(SERVICE + 1), ..identity(proc.clone(), &s) };
        assert!(matches!(known.check_peer(SERVICE, ME), Err(Verdict::Impostor(why)) if why.contains("not the browser service this shell started")));
        let known = ServiceIdentity { service_pid: Some(SERVICE), ..identity(proc, &s) };
        assert_eq!(known.check_peer(SERVICE, ME), Ok(()));
    }

    /// A copied interpreter, in a place the account owns, is not a root-owned program.
    #[test]
    fn a_copied_interpreter_is_not_the_service() {
        // SAFETY: getuid cannot fail.
        if unsafe { libc::getuid() } == 0 {
            return; // a copy made by root is root's
        }
        let s = script("copy");
        let copy = std::env::temp_dir().join(format!("yb-python-copy-{}", std::process::id()));
        fs::copy(ROOT_OWNED, &copy).unwrap();
        let proc = fake_proc("copy", SHELL, &copy, &["python3", s.to_str().unwrap()]);
        assert!(matches!(identity(proc, &s).check_peer(SERVICE, ME), Err(Verdict::Impostor(_))));
    }

    /// A script that is another file of the same name is not the installed one; nor is no script.
    #[test]
    fn a_script_that_only_looks_like_the_installed_one_is_not_the_service() {
        let installed = script("real");
        let copy = script("copy-of-real");
        let proc = fake_proc("script", SHELL, Path::new(ROOT_OWNED), &["python3", copy.to_str().unwrap()]);
        assert!(matches!(identity(proc, &installed).check_peer(SERVICE, ME), Err(Verdict::Impostor(_))));
        let none = fake_proc("none", SHELL, Path::new(ROOT_OWNED), &["python3", "-c", "pass"]);
        assert!(matches!(identity(none, &installed).check_peer(SERVICE, ME), Err(Verdict::Impostor(_))));
    }

    #[test]
    fn a_process_that_cannot_be_read_may_be_looked_at_again() {
        let s = script("gone");
        let empty = std::env::temp_dir().join(format!("yb-peer-empty-{}", std::process::id()));
        fs::create_dir_all(&empty).unwrap();
        assert!(matches!(identity(empty, &s).check_peer(SERVICE, ME), Err(Verdict::Unreadable(_))));
    }

    fn temp_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yb-peer-sock-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("browser-pipe.sock");
        let _ = fs::remove_file(&path);
        path
    }

    /// The kernel names this process as the maker of a listener it made, and says it is alive.
    #[test]
    fn the_maker_of_a_live_listener_is_named_and_alive() {
        let path = temp_socket("live");
        let _listener = UnixListener::bind(&path).unwrap();
        let stream = UnixStream::connect(&path).unwrap();
        let peer = Peer::of(&stream).unwrap();
        assert_eq!(peer.pid, std::process::id());
        assert_eq!(peer.alive(), Ok(()));
    }

    /// N1: a listener whose maker exited after handing it on reads as the maker's number, which
    /// may since have become the real service's. The pidfd is the maker's own, and says it is gone.
    #[test]
    fn a_listener_whose_maker_has_exited_is_not_the_service() {
        let path = temp_socket("maker-gone");
        // The maker binds, hands the listener to a child that holds it a while, and exits.
        let script = "import os, socket, sys\n\
                      s = socket.socket(socket.AF_UNIX)\n\
                      s.bind(sys.argv[1]); s.listen(1); os.set_inheritable(s.fileno(), True)\n\
                      if os.fork() == 0: os.execv('/bin/sleep', ['sleep', '5'])\n\
                      os._exit(0)\n";
        let Ok(status) = std::process::Command::new("python3").args(["-c", script, path.to_str().unwrap()]).status() else {
            return; // no python3 here: the kernel's own behaviour is what this test exercises
        };
        assert!(status.success());
        let stream = UnixStream::connect(&path).unwrap();
        // Before Linux 6.15 the kernel hands out no pidfd for a reaped maker; from 6.15 it hands out
        // one that says the process is gone. Either way the verdict is the same.
        match Peer::of(&stream) {
            Ok(peer) => {
                assert_ne!(peer.pid, std::process::id());
                assert!(matches!(peer.alive(), Err(Verdict::Impostor(why)) if why.contains("no longer the process that made the listener")));
            }
            Err(Verdict::Impostor(why)) => assert!(why.contains("no longer the process that made the listener"), "{why}"),
            Err(other) => panic!("a reaped maker must be an impostor, not {other:?}"),
        }
        assert!(matches!(this_process_is_the_service("maker-gone").check(&stream), Err(Verdict::Impostor(_))));
    }
}
