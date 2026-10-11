//! The mind door (#411): where minds, running as their own account, reach the desktop.
//!
//! Every mind used to run as the person, so it reached every socket in the person's runtime
//! directory and everything else the person owns: their files, their compositor, their shell's
//! memory. Minds now run as `yantrik-mind`. They cannot open the person's runtime directory
//! (0700), so each service the person runs also listens here, in a directory the person owns and
//! the minds' group may only enter (`/run/yantrik-minds`, 2750 person:yantrik-minds, sockets 0660).
//!
//! On this door the kernel says who is calling (`SO_PEERCRED`): a connection is served only when
//! its uid is the mind account's, and a caller with that uid is a mind wherever it is met. The
//! person's own sockets never admit that uid, so the uid alone answers "is this a mind", with no
//! walk of `/proc` and nothing the caller can say about itself.
//!
//! The directory is made at boot by the updater's tmpfiles entry. When it is missing, or not
//! exactly as described, there is no door: the person's sockets work as before and nothing is
//! served to anybody else.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The mind account and its group.
pub const MIND_USER: &str = "yantrik-mind";
pub const MIND_GROUP: &str = "yantrik-minds";

/// Where the door is, unless `YANTRIK_MIND_RUN` names it (the mind units set it; tests use it).
pub const DEFAULT_DIR: &str = "/run/yantrik-minds";

/// The door directory's path, as named. Not a promise that it exists.
pub fn dir() -> PathBuf {
    std::env::var_os("YANTRIK_MIND_RUN")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DIR))
}

/// The mind account's uid, if this machine has one.
#[cfg(unix)]
pub fn mind_uid() -> Option<u32> {
    static UID: OnceLock<Option<u32>> = OnceLock::new();
    *UID.get_or_init(|| lookup_user(MIND_USER))
}

/// The minds' group, if this machine has one.
#[cfg(unix)]
pub fn mind_gid() -> Option<u32> {
    static GID: OnceLock<Option<u32>> = OnceLock::new();
    *GID.get_or_init(|| lookup_group(MIND_GROUP))
}

/// Whether a caller with this uid is a mind. The one test every door and every "who is asking"
/// uses. Never root, never this process's own uid: an account table that named either as the mind
/// account would make the person (or root) a mind, or a mind the person.
#[cfg(unix)]
pub fn is_mind(uid: u32) -> bool {
    // SAFETY: getuid cannot fail.
    let me = unsafe { libc::getuid() };
    uid != 0 && uid != me && mind_uid() == Some(uid)
}

#[cfg(not(unix))]
pub fn is_mind(_uid: u32) -> bool {
    false
}

#[cfg(unix)]
fn lookup_user(name: &str) -> Option<u32> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: getpwnam_r with a NUL-terminated name, a zeroed passwd and a buffer we own.
    let rc = unsafe { libc::getpwnam_r(name.as_ptr(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    (rc == 0 && !result.is_null()).then_some(pwd.pw_uid)
}

#[cfg(unix)]
fn lookup_group(name: &str) -> Option<u32> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 8192];
    let mut result: *mut libc::group = std::ptr::null_mut();
    // SAFETY: getgrnam_r with a NUL-terminated name, a zeroed group and a buffer we own.
    let rc = unsafe { libc::getgrnam_r(name.as_ptr(), &mut grp, buf.as_mut_ptr(), buf.len(), &mut result) };
    (rc == 0 && !result.is_null()).then_some(grp.gr_gid)
}

/// What a door directory must be for this process to serve on it: owned by this process's uid,
/// group the minds' group, mode exactly 2750 (the person writes, the minds' group enters, new
/// sockets take the group, nobody else sees in). Anything else and there is no door.
pub fn acceptable(owner: u32, group: u32, mode: u32, me: u32, minds: u32) -> bool {
    owner == me && group == minds && mode & 0o7777 == 0o2750
}

/// A door socket's mode with the door open (the minds' group may connect), and closed (only the
/// person may). Connecting to a socket needs write permission on the socket itself, so a closed
/// socket refuses the mind account however it reaches it — by its path, or through a handle it
/// kept from before (`/proc/self/fd/N` skips the directory's search check, never the socket's).
pub const SOCKET_OPEN: u32 = 0o660;
pub const SOCKET_CLOSED: u32 = 0o600;

/// Close the door (`true`) or open it (`false`): every door socket in `dir` that this account owns
/// goes to [`SOCKET_CLOSED`], or back from it to [`SOCKET_OPEN`]. What the person's Private mode
/// does, beside every door's own refusal: an app opened before an update runs the old code, which
/// knows nothing of Private mode (found on VM 520, 29 Sep 2026). Answers with how many sockets it
/// changed.
///
/// The directory itself is never changed. The person is not in the minds' group, and Linux drops
/// the setgid bit when anyone outside a directory's group changes its mode — so a door closed by
/// `chmod` on the directory came back 0750, which no app serves on (security review of #498).
///
/// Only a directory that is exactly the door — this account's own, the minds' group, 2750,
/// opened without following a link — is touched; anything else is left as it is, and said.
/// Opening only moves a socket from exactly [`SOCKET_CLOSED`], the mode closing gave it.
///
/// A connection already open when the door closes is not cut: that is each door's own refusal
/// (`server::PrivateDoor`), which an app built before Private mode does not have.
#[cfg(unix)]
pub fn close_door(dir: &Path, closed: bool) -> std::io::Result<usize> {
    let minds = mind_gid()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "this machine has no minds' group"))?;
    close_door_in(dir, closed, minds)
}

#[cfg(unix)]
fn close_door_in(dir: &Path, closed: bool, minds: u32) -> std::io::Result<usize> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    struct Fd(libc::c_int);
    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: an fd this function opened, closed once.
            unsafe { libc::close(self.0) };
        }
    }
    let c = CString::new(dir.as_os_str().as_bytes())?;
    // SAFETY: a NUL-terminated path; the fd is owned by `Fd` from here on.
    let raw = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd = Fd(raw);
    // SAFETY: fstat on an open fd into a zeroed stat.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd.0, &mut st) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: getuid cannot fail.
    let me = unsafe { libc::getuid() };
    if !acceptable(st.st_uid, st.st_gid, st.st_mode as u32, me, minds) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is not the door (owner, group or mode {:o}); left as it is", dir.display(), st.st_mode & 0o7777),
        ));
    }
    let mut changed = 0;
    let mut first_error = None;
    // The entries of the directory just checked, through its fd rather than its path again.
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", fd.0))? {
        let Ok(entry) = entry else { continue };
        let Ok(name) = CString::new(entry.file_name().as_bytes()) else { continue };
        // SAFETY: fstatat relative to the directory we opened, never following a link.
        let mut s: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(fd.0, name.as_ptr(), &mut s, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            continue;
        }
        if s.st_mode & libc::S_IFMT != libc::S_IFSOCK || s.st_uid != me {
            continue;
        }
        let mode = s.st_mode as u32 & 0o777;
        // Closing takes only what the group was given (its write, which connect needs), so a
        // socket that never let the group in is not changed, and so not widened on opening.
        let want = if closed && mode & 0o020 != 0 {
            SOCKET_CLOSED
        } else if !closed && mode == SOCKET_CLOSED {
            SOCKET_OPEN
        } else {
            continue;
        };
        // SAFETY: fchmodat relative to the directory we opened. The entry was just seen to be a
        // socket, not a link, in a directory nobody but this account and root may write.
        if unsafe { libc::fchmodat(fd.0, name.as_ptr(), want as libc::mode_t, 0) } == 0 {
            changed += 1;
        } else if first_error.is_none() {
            first_error = Some(std::io::Error::last_os_error());
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(changed),
    }
}

#[cfg(not(unix))]
pub fn close_door(_dir: &Path, _closed: bool) -> std::io::Result<usize> {
    Ok(0)
}

/// The mode a door socket is bound with: closed while the person's Private mode is on, so an app
/// started during it opens no way in. The shell opens it when Private mode ends.
pub fn socket_mode() -> u32 {
    if crate::privacy::is_private() {
        SOCKET_CLOSED
    } else {
        SOCKET_OPEN
    }
}

/// The door directory, when this process should serve on it.
#[cfg(unix)]
pub fn serving_dir() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    mind_uid()?;
    let minds = mind_gid()?;
    let dir = dir();
    let meta = std::fs::metadata(&dir).ok()?;
    // SAFETY: getuid cannot fail.
    let me = unsafe { libc::getuid() };
    if !meta.is_dir() || !acceptable(meta.uid(), meta.gid(), meta.mode(), me, minds) {
        tracing::debug!(dir = %dir.display(), "the mind door is not set up as expected; not serving on it");
        return None;
    }
    Some(dir)
}

#[cfg(not(unix))]
pub fn serving_dir() -> Option<PathBuf> {
    None
}

/// Which services open a door: the harness (where a mind attaches) and the app surfaces, whose
/// every change goes through the shell's graded `app.act`. Nothing else, by name, until it is
/// judged safe for a mind to call. The companion runs tools as the person, a11y drives any window
/// ungraded, perception streams the person's file activity: on a door each would hand a mind back
/// the authority its own account exists to take away.
pub fn opens_a_door(socket_name: &str) -> bool {
    let Some(id) = socket_name.strip_suffix(".sock") else { return false };
    id == "harness" || id.starts_with("app-")
}

/// Where the door socket for the service listening at `address` goes: the same file name in the
/// door directory, for a door service's socket in this session's own socket directory. Anything
/// else (another service, a test's temp path, an explicit address) gets no door.
pub fn door_for(address: &Path, socket_dir: &Path, door: &Path) -> Option<PathBuf> {
    let name = address.file_name()?;
    (address.parent()? == socket_dir && opens_a_door(name.to_str()?)).then(|| door.join(name))
}

/// The address a client in a mind's process dials for a service: the door, when this process
/// was told where it is (`YANTRIK_MIND_RUN`); `None` for everyone else, who use their own
/// runtime directory.
pub fn client_address(service_id: &str) -> Option<String> {
    std::env::var_os("YANTRIK_MIND_RUN")
        .filter(|v| !v.is_empty())
        .map(|d| format!("{}/{service_id}.sock", PathBuf::from(d).display()))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn closing_the_door_closes_every_socket_and_opening_gives_back_only_what_it_closed() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("yantrik-door-close-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o2750)).unwrap();
        let gid = std::fs::metadata(&dir).unwrap().gid();
        let mode = |p: &Path| std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o7777;
        let a = dir.join("a.sock");
        let b = dir.join("b.sock");
        let _la = std::os::unix::net::UnixListener::bind(&a).unwrap();
        let _lb = std::os::unix::net::UnixListener::bind(&b).unwrap();
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(SOCKET_OPEN)).unwrap();
        std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o640)).unwrap();
        let file = dir.join("note");
        std::fs::write(&file, "x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();

        assert_eq!(close_door_in(&dir, true, gid).unwrap(), 1);
        assert_eq!(mode(&a), SOCKET_CLOSED);
        assert_eq!(mode(&b), 0o640, "a socket that never let the group connect is left as it is");
        assert_eq!(mode(&file), 0o640, "only sockets");
        assert_eq!(mode(&dir), 0o2750, "the directory is never changed: its setgid must survive");
        assert_eq!(close_door_in(&dir, true, gid).unwrap(), 0, "closing twice changes nothing");

        assert_eq!(close_door_in(&dir, false, gid).unwrap(), 1);
        assert_eq!(mode(&a), SOCKET_OPEN);
        assert_eq!(mode(&b), 0o640, "a Private-mode cycle widens nothing");

        // Not the door (wrong group, wrong mode, a link to it) and nothing is touched.
        assert!(close_door_in(&dir, true, gid.wrapping_add(1)).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert!(close_door_in(&dir, true, gid).is_err());
        assert_eq!(mode(&a), SOCKET_OPEN);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o2750)).unwrap();
        let link = dir.with_extension("link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(close_door_in(&link, true, gid).is_err(), "never through a link");
        assert_eq!(mode(&a), SOCKET_OPEN);
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    #[test]
    fn a_door_is_served_only_on_a_directory_exactly_as_the_tmpfiles_entry_makes_it() {
        let (me, minds) = (1000, 990);
        assert!(acceptable(me, minds, 0o42750, me, minds), "person:yantrik-minds 2750");
        assert!(!acceptable(0, minds, 0o42750, me, minds), "someone else's directory");
        assert!(!acceptable(me, 100, 0o42750, me, minds), "another group could enter");
        assert!(!acceptable(me, minds, 0o42770, me, minds), "the minds could plant a socket");
        assert!(!acceptable(me, minds, 0o40750, me, minds), "without setgid the sockets are not the group's");
        assert!(!acceptable(me, minds, 0o42755, me, minds), "anyone could look in");
    }

    #[test]
    fn only_a_service_socket_of_this_session_gets_a_door() {
        let run = Path::new("/run/user/1000/yantrik");
        let door = Path::new("/run/yantrik-minds");
        assert_eq!(
            door_for(Path::new("/run/user/1000/yantrik/app-shell.sock"), run, door),
            Some(PathBuf::from("/run/yantrik-minds/app-shell.sock"))
        );
        assert_eq!(door_for(Path::new("/tmp/test-x/app.sock"), run, door), None);
    }

    #[test]
    fn only_the_harness_and_the_app_surfaces_open_a_door() {
        for yes in ["harness.sock", "app-shell.sock", "app-notes.sock", "app-editor.sock"] {
            assert!(opens_a_door(yes), "{yes}");
        }
        for no in ["companion.sock", "a11y.sock", "perception.sock", "notifications.sock", "email.sock", "harness", "vault.sock"] {
            assert!(!opens_a_door(no), "{no} must not be on the mind door");
        }
    }
}
