//! Who is at the other end of `browser-pipe.sock`, checked before the shell sends it a browser's
//! DevTools stream (#1151).
//!
//! The socket sits in a directory the person's own account owns, so any process of that account
//! can unlink the service's socket and bind its own. Whoever answers is therefore asked who it is,
//! from the kernel (`SO_PEERCRED`, filled in when the listener was made), and must be the browser
//! service this shell started: a child of this shell, running a root-owned interpreter on the
//! installed `yantrik-browser` script (compared by device and inode, not by name). A process of the
//! same account can rewrite its own argv, so this stops the cheap replacement, not a determined
//! one; what makes that a boundary is the Minds running as another account (#1137 step 3).

use std::fs;
use std::io;
use std::os::fd::AsRawFd;
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
}

/// Why a peer is not the service. `Unreadable` may pass on a second look (a process exec'ing);
/// `Impostor` will not.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Unreadable(String),
    Impostor(String),
}

impl ServiceIdentity {
    /// This shell, and the service script installed beside it.
    pub fn installed() -> io::Result<Self> {
        let exe = std::env::current_exe()?;
        let script = exe.parent().map(|d| d.join("yantrik-browser")).ok_or_else(|| io::Error::other("the shell has no directory"))?;
        // SAFETY: getuid cannot fail.
        Ok(Self { shell_pid: std::process::id(), uid: unsafe { libc::getuid() }, proc: "/proc".into(), script })
    }

    /// Check the process that answers on `stream`.
    pub fn check(&self, stream: &UnixStream) -> Result<(), Verdict> {
        let (pid, uid) = peer_of(stream).map_err(|e| Verdict::Unreadable(format!("the kernel would not say who answers: {e}")))?;
        self.check_peer(pid, uid)
    }

    pub fn check_peer(&self, pid: u32, uid: u32) -> Result<(), Verdict> {
        if uid != self.uid {
            return Err(Verdict::Impostor(format!("pid {pid} runs as uid {uid}, not as this account")));
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

/// (pid, uid) of the process that made the listener `stream` is connected to.
fn peer_of(stream: &UnixStream) -> io::Result<(u32, u32)> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are the buffer getsockopt(SO_PEERCRED) fills in.
    let rc = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((cred.pid as u32, cred.uid))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    const ME: u32 = 1000;
    const SHELL: u32 = 500;
    const SERVICE: u32 = 700;

    /// A fake /proc with one process whose parent, program and command line are as given.
    fn fake_proc(name: &str, parent: u32, exe: &Path, cmdline: &[&str]) -> PathBuf {
        fake_proc_for(SERVICE, name, parent, exe, cmdline)
    }

    fn fake_proc_for(pid: u32, name: &str, parent: u32, exe: &Path, cmdline: &[&str]) -> PathBuf {
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
        ServiceIdentity { shell_pid: SHELL, uid: ME, proc, script: script.into() }
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
        ServiceIdentity { shell_pid: SHELL, uid, proc, script: s }
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
}
