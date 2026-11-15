//! Which program is the desktop's own, by where its file is and not by what it is called.
//!
//! One rule for "this is `yantrik-ui` (or another of the desktop's binaries) and not a copy", used
//! by notifications-service for who is the desktop, and by an app's dispatch for who may press an
//! action only the shell's own press runs (`yantrik_surface::shell_only`). Two copies of it would
//! drift: the first version of the second one went by the file name alone, and a
//! `cp /usr/bin/python3 ~/.local/bin/yantrik-ui` passed it.

use std::path::Path;

use super::{exe_of, is_desktop_binary_in, is_installed_desktop_binary, is_program, same_user, PeerCred};

/// Whether `exe` is one of [`DESKTOP_BINARIES`] where this desktop keeps its programs: in
/// [`super::INSTALL_DIR`], or beside the program that is asking (`own_dir`, already judged by
/// [`own_dir_counts`]). The one rule for "this is the desktop's own program and not a copy": a
/// binary is judged by where it is, never by what it is called.
pub fn desktop_exe(exe: &str, own_dir: Option<&Path>) -> bool {
    is_installed_desktop_binary(exe) || own_dir.is_some_and(|d| is_desktop_binary_in(exe, d))
}

/// The directory this process runs from, when it may vouch for a binary beside it (see
/// [`own_dir_counts`]).
pub fn trusted_own_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).filter(|d| own_dir_counts(d))
}

/// Whether the directory this process runs from may vouch for a binary beside it.
///
/// A developer's cargo target directory is writable by that developer, so a file dropped there
/// named `yantrik-ui` would otherwise become the desktop (security review of #614). A debug
/// build is a developer's machine by definition; a release build counts its own directory only
/// when root owns it and neither group nor others can write to it.
pub fn own_dir_counts(dir: &Path) -> bool {
    #[cfg(unix)]
    let (uid, mode) = {
        use std::os::unix::fs::MetadataExt;
        match std::fs::metadata(dir) {
            Ok(m) => (m.uid(), m.mode()),
            Err(_) => return cfg!(debug_assertions),
        }
    };
    #[cfg(not(unix))]
    let (uid, mode) = (u32::MAX, 0o777);
    dir_vouches(cfg!(debug_assertions), uid, mode)
}

/// [`own_dir_counts`]'s judgement, from the facts, so the release rule can be tested from a
/// debug build.
pub fn dir_vouches(debug: bool, uid: u32, mode: u32) -> bool {
    debug || (uid == 0 && mode & 0o022 == 0)
}

/// Whether `exe` is one of the desktop's own binaries as installed, or beside this program when
/// its directory can be trusted. What notifications-service uses to say who is the desktop.
pub fn is_installed_desktop_program(exe: &str) -> bool {
    desktop_exe(exe, trusted_own_dir().as_deref())
}

/// [`is_the_installed_program`]'s rule for one executable, with this program's own directory
/// passed in so it can be tested without being installed.
fn installed_program(exe: &str, name: &str, own_dir: Option<&Path>) -> bool {
    is_program(exe, name) && desktop_exe(exe, own_dir)
}

/// The check on a call only the desktop's own `name` may make (`name` is one of
/// [`DESKTOP_BINARIES`]): the caller runs as this user, and the kernel says its program is that
/// binary *where the desktop keeps it* ([`super::INSTALL_DIR`], or beside this program in a directory it
/// may trust). Stronger than [`must_be_program`], which goes by the file name alone: one
/// `cp /usr/bin/python3 ~/.local/bin/yantrik-ui` passes that, and does not pass this. `Err` is a
/// sentence for the refusal, without anything the caller sent.
///
/// It still judges a process by `/proc` at the moment of the call, so code already running as the
/// person can exec the real binary after it connects, or preload its own code into it: the
/// same-user limit #154 records for the whole control surface. What it ends is a copy, a script
/// and a renamed interpreter.
pub fn is_the_installed_program(peer: Option<PeerCred>, name: &str) -> Result<(), String> {
    let Some(peer) = peer else {
        return Err(format!("the kernel would not say which process is calling, and this answers {name} only"));
    };
    if !same_user(Some(peer)) {
        return Err(format!("this answers {name} only, and the caller does not run as this user"));
    }
    match exe_of(peer.pid) {
        Some(exe) if installed_program(&exe, name, trusted_own_dir().as_deref()) => Ok(()),
        Some(_) => Err(format!("this answers {name} only, and the caller (pid {}) is another program", peer.pid)),
        None => Err(format!("this answers {name} only, and the caller (pid {}) could not be identified", peer.pid)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner::{must_be_program, own_euid, SHELL_BINARY};

    #[test]
    fn a_release_build_trusts_its_own_directory_only_when_root_holds_it() {
        assert!(dir_vouches(true, 1000, 0o755), "a debug build is a developer's machine");
        assert!(dir_vouches(false, 0, 0o755), "root-owned, written by nobody else");
        assert!(!dir_vouches(false, 1000, 0o755), "the developer's own target directory");
        assert!(!dir_vouches(false, 0, 0o775), "group-writable");
        assert!(!dir_vouches(false, 0, 0o757), "world-writable");
    }

    #[test]
    fn a_desktop_binary_is_judged_by_where_it_is_not_what_it_is_called() {
        // A developer's build counts beside this program's own build, and nowhere else.
        let build = Path::new("/home/yantrik/targets/dev/release");
        assert!(desktop_exe("/home/yantrik/targets/dev/release/yantrik-ui", Some(build)));
        assert!(!desktop_exe("/tmp/yantrik-ui", Some(build)));
        assert!(!desktop_exe("/home/yantrik/targets/dev/release/yantrik-ui", None));
        assert!(desktop_exe("/opt/yantrik/bin/yantrik-ui (deleted)", None), "replaced by an update");
        assert!(installed_program("/opt/yantrik/bin/yantrik-ui", SHELL_BINARY, None));
        assert!(!installed_program("/opt/yantrik/bin/notifications-service", SHELL_BINARY, None), "another desktop binary");
        assert!(!installed_program("/home/me/.local/bin/yantrik-ui", SHELL_BINARY, None), "the right name in the wrong place");
    }

    /// Start `program` with `arg`, trying again while the system says it is busy: the copy was
    /// just written, and a descriptor that is still open for writing in a process another test
    /// thread forked makes the exec fail with ETXTBSY until that process execs (rust-lang/rust#114554).
    fn spawn_when_not_busy(program: &Path, arg: &str) -> std::process::Child {
        let mut tries = 0;
        loop {
            match std::process::Command::new(program).arg(arg).spawn() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 10 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                other => return other.expect("the copy runs"),
            }
        }
    }

    /// A program still open for writing is busy to exec, and starts once it is closed.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_program_still_open_for_writing_is_started_once_it_is_closed() {
        let dir = std::env::temp_dir().join(format!("yantrik-owner-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sleep = ["/usr/bin/sleep", "/bin/sleep"].into_iter().find(|p| Path::new(p).exists()).expect("a sleep to copy");
        let copy = dir.join("busy-sleep");
        std::fs::copy(sleep, &copy).unwrap();
        let open = std::fs::OpenOptions::new().append(true).open(&copy).unwrap();
        let busy = std::process::Command::new(&copy).arg("30").spawn().unwrap_err();
        assert_eq!(busy.kind(), std::io::ErrorKind::ExecutableFileBusy, "the exec itself is refused while it is open");
        let closer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(60));
            drop(open);
        });
        let mut child = spawn_when_not_busy(&copy, "30");
        let _ = child.kill();
        let _ = child.wait();
        closer.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A live process whose program is a copy of `sleep` named `yantrik-ui` in a directory of its
    /// own: the right name, not where the desktop keeps it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_renamed_copy_of_another_program_is_not_the_installed_one() {
        let dir = std::env::temp_dir().join(format!("yantrik-owner-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sleep = ["/usr/bin/sleep", "/bin/sleep"].into_iter().find(|p| Path::new(p).exists()).expect("a sleep to copy");
        let copy = dir.join(SHELL_BINARY);
        std::fs::copy(sleep, &copy).unwrap();
        let mut child = spawn_when_not_busy(&copy, "30");
        let me = PeerCred { pid: child.id() as i32, uid: own_euid(), gid: 0, the_mind: false };

        // The old, name-only rule takes it for the shell; this one does not.
        assert!(must_be_program(Some(me), SHELL_BINARY).is_ok(), "the name alone passes the weaker rule");
        let why = is_the_installed_program(Some(me), SHELL_BINARY).unwrap_err();
        assert!(why.contains("another program"), "{why}");
        assert!(is_the_installed_program(None, SHELL_BINARY).is_err());
        let other_user = PeerCred { uid: own_euid().wrapping_add(1), ..me };
        assert!(is_the_installed_program(Some(other_user), SHELL_BINARY).is_err(), "another account");

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

