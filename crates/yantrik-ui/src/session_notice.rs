//! The one notice `yantrik-update` leaves when it moves the desktop's start to system files (#548).
//!
//! `reconcile_session_start` writes it, as root, to `/opt/yantrik/update-state/session-start-notice`:
//! which keys of the old environment file stopped working, and that a reboot finishes the switch.
//! The shell reads it once, after unlock (the lock screen is not the place for it), and posts it as
//! an ordinary notification. The folder is root's, so the person cannot rename the file; what was
//! shown is recorded as the notice's own hash in `~/.local/state/yantrik/session-notice-shown`, and
//! the same text is never posted twice. A new update that writes a different notice is posted again.

use std::path::{Path, PathBuf};
use yantrik_app_runtime::notify;
use yantrik_ipc_contracts::notifications::Urgency;

const NOTICE: &str = "/opt/yantrik/update-state/session-start-notice";
/// The text is root's, but cap it: it becomes a notification body.
const MAX_BYTES: u64 = 4096;

fn shown_marker() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".local/state/yantrik/session-notice-shown"))
}

/// The text of `notice` when it exists, is a plain file and is small; None otherwise.
fn read_notice(notice: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(notice).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(notice).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A stable tag for `text` (FNV-1a), enough to tell one notice from the next.
fn tag(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// The notice to post now: `Some(text)` unless `marker` already holds its tag.
fn pending(notice: &Path, marker: &Path) -> Option<String> {
    let text = read_notice(notice)?;
    let seen = std::fs::read_to_string(marker)
        .map(|s| s.trim() == tag(&text))
        .unwrap_or(false);
    (!seen).then_some(text)
}

/// Post the notice once, after the screen is unlocked.
pub fn post_once() {
    let Some(marker) = shown_marker() else { return };
    let Some(text) = pending(Path::new(NOTICE), &marker) else {
        return;
    };
    notify::send(
        notify::Notification::new(
            "Yantrik",
            "The desktop now starts from system files".to_string(),
        )
        .body(text.clone())
        .urgency(Urgency::Normal),
    );
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&marker, tag(&text));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_is_posted_once_and_a_new_one_again() {
        let dir = std::env::temp_dir().join(format!("yantrik-notice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (n, m) = (dir.join("notice"), dir.join("shown"));
        assert_eq!(pending(&n, &m), None, "no file, nothing to post");
        std::fs::write(&n, "keys: XKB_DEFAULT_LAYOUT\n").unwrap();
        assert_eq!(pending(&n, &m).as_deref(), Some("keys: XKB_DEFAULT_LAYOUT"));
        std::fs::write(&m, tag("keys: XKB_DEFAULT_LAYOUT")).unwrap();
        assert_eq!(pending(&n, &m), None, "already shown");
        std::fs::write(&n, "another notice\n").unwrap();
        assert_eq!(pending(&n, &m).as_deref(), Some("another notice"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
