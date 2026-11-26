//! Private mode: while the person has it on, no agent sees or does anything on this desktop.
//!
//! The shell publishes it in `private-mode/privacy.json` (see below), and it stays on until the
//! person turns it off: through restarts and reboots, because a privacy switch that quietly turns
//! itself off is a trap. Every place an agent comes in reads it on every call, as the ceiling and
//! the mode are read, so turning it on takes effect on connections already open:
//!
//! - the mind door (`server::serve_door`, and the Python SDK's door): every request from the mind
//!   account is answered with [`REFUSAL`], describe and memory checks included;
//! - each surface's dispatch (`yantrik_app_runtime::control`, the Python SDK's surface): a call
//!   that carries an agent token is refused the same way.
//!
//! The file fails closed: absent is off (a desktop nobody ever made private), but a file that is
//! there and cannot be read or understood is on. Saying the person was not private when they
//! were is the mistake that matters.
//!
//! Only the shell writes it, and only for the person: the file lives in the person's own
//! configuration directory, which the mind account cannot write.
//!
//! It lives in a directory of its own, `~/.config/yantrik/private-mode/` (#1415). The perception
//! service sandboxes itself with Landlock and must read Private mode on every call, but nothing
//! else in `~/.config/yantrik` (the vault is there) may be readable to it. A rule on the file
//! would bind to one inode, and the shell replaces the file by rename, so the rule would go stale
//! at the first flip; a rule on the directory binds to an inode that never changes, and covers a
//! file created after the service started. The old place, `privacy.json` beside the settings file,
//! is still read when the new one has no file, and the shell moves it across at start
//! ([`migrate_legacy`]).

use std::path::PathBuf;

use serde_json::json;

use crate::gate::settings_path;

/// The file the shell publishes Private mode in.
pub const PRIVACY_FILE: &str = "privacy.json";

/// The directory it lives in, under the configuration directory: the only part of it a sandboxed
/// reader is granted.
pub const PRIVACY_DIR: &str = "private-mode";

/// What an agent is told while the person is private. One sentence, the same everywhere, so an
/// agent can recognise it and stop rather than retry.
pub const REFUSAL: &str = "PRIVATE: the person has turned on Private mode. Nothing on this desktop is shown to \
                           agents or done for them until they turn it off. Nothing was run.";

/// The configuration directory (`~/.config/yantrik`), or `None` when this process cannot tell
/// whose home it is. `HOME` when it is an absolute path, else the account's home from the
/// password database: a process started with `HOME` unset or relative read
/// `./.config/yantrik/privacy.json`, found nothing there, and took the person for not private
/// (security review, 29 Sep 2026).
fn config_dir() -> Option<PathBuf> {
    let settings = settings_path();
    if settings.is_absolute() {
        return settings.parent().map(PathBuf::from);
    }
    account_home().map(|home| home.join(".config/yantrik"))
}

/// The directory Private mode is published in, for a sandbox to be granted read access to.
pub fn privacy_dir() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(PRIVACY_DIR))
}

/// Where the shell publishes Private mode, or `None` when this process cannot tell whose home it
/// is. The one place every reader and the writer find it.
pub fn privacy_path() -> Option<PathBuf> {
    privacy_dir().map(|dir| dir.join(PRIVACY_FILE))
}

/// Where Private mode was published before #1415, beside the settings file. Read as a fallback
/// only.
pub fn legacy_privacy_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(PRIVACY_FILE))
}

#[cfg(unix)]
fn account_home() -> Option<PathBuf> {
    // SAFETY: getpwuid_r with our own uid, a zeroed passwd and a buffer we own.
    let uid = unsafe { libc::getuid() };
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
        return None;
    }
    let dir = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }.to_string_lossy().to_string();
    let dir = PathBuf::from(dir);
    dir.is_absolute().then_some(dir)
}

#[cfg(not(unix))]
fn account_home() -> Option<PathBuf> {
    None
}

/// Private mode's file could not be read, and which path it was.
#[derive(Debug)]
pub struct Unreadable {
    pub path: Option<PathBuf>,
    pub error: std::io::Error,
}

impl std::fmt::Display for Unreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.path {
            Some(p) => write!(f, "{}: {}", p.display(), self.error),
            None => write!(f, "{}", self.error),
        }
    }
}

/// What `private-mode` is, looked at without following a link.
enum Place {
    Absent,
    Directory,
    /// A link, a file, anything that is not a plain directory.
    Other,
}

fn place(dir: &std::path::Path) -> std::io::Result<Place> {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_dir() => Ok(Place::Directory),
        Ok(_) => Ok(Place::Other),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Place::Absent),
        Err(e) => Err(e),
    }
}

/// Whether the person is in Private mode now, or why that cannot be told. The old file is read
/// only when `private-mode` is not there at all (`lstat` says `ENOENT`): a `private-mode` that is
/// a link or a file, or a directory without the file, is an error, which [`is_private`] takes as
/// private and a sandboxed service reports at start. Otherwise a dangling link at `private-mode`
/// would send every reader to an old file that says false.
pub fn read_state() -> Result<bool, Unreadable> {
    let no_home = || {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory");
        Unreadable { path: None, error }
    };
    let (Some(dir), Some(file), Some(old)) = (privacy_dir(), privacy_path(), legacy_privacy_path()) else {
        return Err(no_home());
    };
    let fail = |path: &std::path::Path, error: std::io::Error| Unreadable { path: Some(path.to_path_buf()), error };
    match place(&dir) {
        Err(e) => Err(fail(&dir, e)),
        Ok(Place::Other) => {
            Err(fail(&dir, std::io::Error::new(std::io::ErrorKind::InvalidData, "is not a plain directory")))
        }
        Ok(Place::Directory) => match std::fs::read_to_string(&file) {
            Ok(text) => Ok(private_in(&text)),
            Err(e) => Err(fail(&file, e)),
        },
        Ok(Place::Absent) => match std::fs::read_to_string(&old) {
            Ok(text) => Ok(private_in(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(fail(&old, e)),
        },
    }
}

/// Whether the person is in Private mode now. Read per call. A process that cannot find the file,
/// or cannot read it, takes the person to be private.
pub fn is_private() -> bool {
    read_state().unwrap_or(true)
}

/// What [`ensure_dir`] found.
#[derive(PartialEq)]
enum Ensured {
    /// Made now.
    Created,
    /// Already the person's plain directory.
    Existing,
    /// Something else was there (a link, a file, someone else's directory) and was moved aside.
    Replaced,
}

/// Make sure `private-mode` is a plain directory of the person's, mode 0700. Anything else there
/// is moved aside to `private-mode.bad-<time>` (never deleted: it is evidence), and the directory
/// made fresh. `rename` moves a link itself and `DirBuilder` never follows one.
#[cfg(unix)]
fn ensure_dir(dir: &std::path::Path) -> std::io::Result<Ensured> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // SAFETY: getuid takes nothing and cannot fail.
    let uid = unsafe { libc::getuid() };
    let mut moved = false;
    for _ in 0..3 {
        match std::fs::symlink_metadata(dir) {
            Ok(m) if m.file_type().is_dir() && m.uid() == uid => {
                if m.permissions().mode() & 0o077 != 0 {
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
                }
                return Ok(if moved { Ensured::Replaced } else { Ensured::Existing });
            }
            Ok(_) => {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                let aside = dir.with_file_name(format!("{PRIVACY_DIR}.bad-{nanos:x}"));
                std::fs::rename(dir, &aside)?;
                tracing::error!(path = %dir.display(), moved_to = %aside.display(), "private-mode was not the person's plain directory; moved aside");
                moved = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::DirBuilder::new().mode(0o700).create(dir) {
                    Ok(()) => return Ok(if moved { Ensured::Replaced } else { Ensured::Created }),
                    // Raced with another maker: look again.
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::Other, "private-mode could not be made a plain directory"))
}

#[cfg(not(unix))]
fn ensure_dir(dir: &std::path::Path) -> std::io::Result<Ensured> {
    std::fs::create_dir_all(dir).map(|()| Ensured::Existing)
}

/// Move Private mode from the old file to the new one (the shell, at start). Written to the new
/// place first, then the old file removed, so a crash between leaves both and the new one wins.
/// Also makes sure the directory exists, because a sandboxed reader is granted it at its own
/// start and can grant nothing that is not there. Returns whether a file was moved.
///
/// A directory with no file in it reads as private, so one is never left that way: a fresh
/// desktop (directory just made, nothing old to move) gets an explicit "off". But where something was moved aside, or a directory of the
/// person's is missing its file, what the switch said is lost, and it is published as on: the
/// person turns it off.
pub fn migrate_legacy(since_unix: u64) -> std::io::Result<bool> {
    let (Some(new), Some(old)) = (privacy_path(), legacy_privacy_path()) else { return Ok(false) };
    let Some(dir) = new.parent() else { return Ok(false) };
    let ensured = ensure_dir(dir)?;
    if ensured == Ensured::Replaced {
        // The old file is what whoever tampered left, so it does not speak for the person.
        publish(true, since_unix)?;
        return Ok(false);
    }
    if std::fs::symlink_metadata(&new).is_ok() {
        return Ok(false);
    }
    if std::fs::symlink_metadata(&old).is_err() {
        publish(ensured != Ensured::Created, since_unix)?;
        return Ok(false);
    }
    // Same meaning the readers give it: anything unclear moves across as private.
    let private = std::fs::read_to_string(&old).map(|t| private_in(&t)).unwrap_or(true);
    publish(private, since_unix)?;
    let _ = std::fs::remove_file(&old);
    Ok(true)
}

/// The file's meaning: `{"private": true|false, ...}`. Anything else reads as private.
pub fn private_in(text: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => v.get("private").and_then(serde_json::Value::as_bool).unwrap_or(true),
        Err(_) => true,
    }
}

/// Publish Private mode (the shell only). Written whole to a new file of a name nobody could have
/// prepared, created exclusively and never through a link, then renamed over the old one: a
/// fixed `privacy.json.tmp` could be made a directory, so Private never turned on, or a link, so
/// the write landed somewhere else (security review, 29 Sep 2026).
pub fn publish(private: bool, since_unix: u64) -> std::io::Result<()> {
    use std::io::Write;
    let path = privacy_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory to publish Private mode in"))?;
    if let Some(dir) = path.parent() {
        ensure_dir(dir)?;
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tmp = path.with_file_name(format!(".{PRIVACY_FILE}.{}.{nanos:x}", std::process::id()));
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let body = json!({ "private": private, "since": since_unix }).to_string();
    let written = open.open(&tmp).and_then(|mut f| f.write_all(body.as_bytes()).and_then(|()| f.sync_all()));
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, &path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_is_in_a_directory_of_its_own() {
        let path = privacy_path().expect("a home");
        assert_eq!(path.file_name().unwrap(), PRIVACY_FILE);
        assert_eq!(path.parent().unwrap(), privacy_dir().unwrap());
        assert_eq!(privacy_dir().unwrap().file_name().unwrap(), PRIVACY_DIR);
        assert_ne!(legacy_privacy_path().unwrap(), path);
    }

    #[test]
    fn only_a_clear_false_is_not_private() {
        assert!(!private_in(r#"{"private": false, "since": 1}"#));
        assert!(private_in(r#"{"private": true}"#));
        for unclear in ["", "{", "[]", "{}", r#"{"private": "no"}"#, r#"{"private": 0}"#] {
            assert!(private_in(unclear), "{unclear:?} must read as private");
        }
    }

    #[cfg(unix)]
    #[test]
    fn anything_but_the_persons_plain_directory_is_moved_aside_and_read_as_private() {
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("yantrik-privacy-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let dir = home.join(".config/yantrik/private-mode");
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        // A dangling link where the directory goes.
        std::os::unix::fs::symlink("/nonexistent-yantrik", &dir).unwrap();
        assert!(matches!(place(&dir), Ok(Place::Other)));
        assert!(matches!(ensure_dir(&dir), Ok(Ensured::Replaced)));
        assert!(std::fs::symlink_metadata(&dir).unwrap().file_type().is_dir());
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let aside = std::fs::read_dir(dir.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with("private-mode.bad-"));
        assert!(aside, "moved aside, not deleted");
        // A loose mode is tightened.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(ensure_dir(&dir), Ok(Ensured::Existing)));
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(&home);
    }
}
