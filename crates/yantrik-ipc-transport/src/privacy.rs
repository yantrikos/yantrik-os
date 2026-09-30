//! Private mode: while the person has it on, no agent sees or does anything on this desktop.
//!
//! The shell publishes it in `privacy.json`, beside the settings file, and it stays on until the
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

use std::path::PathBuf;

use serde_json::json;

use crate::gate::settings_path;

/// The file the shell publishes Private mode in, beside the settings file.
pub const PRIVACY_FILE: &str = "privacy.json";

/// What an agent is told while the person is private. One sentence, the same everywhere, so an
/// agent can recognise it and stop rather than retry.
pub const REFUSAL: &str = "PRIVATE: the person has turned on Private mode. Nothing on this desktop is shown to \
                           agents or done for them until they turn it off. Nothing was run.";

/// Where the shell publishes Private mode, or `None` when this process cannot tell whose home it
/// is. `HOME` when it is an absolute path, else the account's home from the password database: a
/// process started with `HOME` unset or relative read `./.config/yantrik/privacy.json`, found
/// nothing there, and took the person for not private (security review, 29 Sep 2026).
pub fn privacy_path() -> Option<PathBuf> {
    let settings = settings_path();
    if settings.is_absolute() {
        return Some(settings.with_file_name(PRIVACY_FILE));
    }
    account_home().map(|home| home.join(".config/yantrik").join(PRIVACY_FILE))
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

/// Whether the person is in Private mode now. Read per call. A process that cannot find the file
/// takes the person to be private.
pub fn is_private() -> bool {
    let Some(path) = privacy_path() else { return true };
    match std::fs::read_to_string(path) {
        Ok(text) => private_in(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
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
        std::fs::create_dir_all(dir)?;
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
    fn only_a_clear_false_is_not_private() {
        assert!(!private_in(r#"{"private": false, "since": 1}"#));
        assert!(private_in(r#"{"private": true}"#));
        for unclear in ["", "{", "[]", "{}", r#"{"private": "no"}"#, r#"{"private": 0}"#] {
            assert!(private_in(unclear), "{unclear:?} must read as private");
        }
    }
}
