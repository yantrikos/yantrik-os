//! What the Mind may search for in its own words: the person's grants, as root wrote them.
//!
//! The Mind's egress planner lets a web search carry only the person's own words unless the
//! person granted more. A grant is one capability ([`CAPABILITY`]) for one agent, for one run,
//! one session of its harness, or always (design/mind-egress-2026-09-29.md, section 6). Root
//! writes them (`yantrik-update mind-grant`), whole, to [`GRANTS_PATH`]; nothing here writes.
//! This is the reader, with the trust checks every reader makes, the Mind's included:
//!
//! - the directory is not a link, is owned by root (uid and gid 0) and is not group- or
//!   world-writable (`lstat`);
//! - the file is opened `O_NOFOLLOW`, and `fstat` on that descriptor (not a second `stat` of the
//!   path) shows a regular file owned by root with no group or other write bit;
//! - it parses, `version` is 1, and each grant checks; one that does not, or has expired, is
//!   dropped. A capability this build does not know is dropped, never widened into one it does.
//!   So is one granted more than [`SKEW_SECS`] in the future: written under a clock that was
//!   ahead, it would otherwise hold until that future date.
//!
//! A file that fails any of these is read as **no grants**: the Mind asks.

use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Every grant in force, written by root.
pub const GRANTS_PATH: &str = "/run/yantrik-mind-egress/grants.json";
/// The one capability there is.
pub const CAPABILITY: &str = "web_search_own_words";
/// The one agent that holds it: the first-party Yantrik Mind's harness id.
pub const AGENT: &str = "mind";
/// The layout this build reads.
pub const VERSION: u64 = 1;
/// The longest a run or session grant lasts.
pub const MOST_SECS: u64 = 24 * 60 * 60;
/// How far in the future a grant's `granted_at` may be and still be read: clock skew between
/// the writer and a reader, no more.
pub const SKEW_SECS: u64 = 300;
/// The largest file read.
const BIGGEST: u64 = 64 * 1024;

/// One grant, as the file has it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// `g-` and 12 hex digits: what `yantrik-update mind-grant revoke` takes.
    pub id: String,
    pub agent: String,
    pub capability: String,
    /// `run`, `session` or `always`.
    pub scope: String,
    /// The run id or the harness session's id ([`session_scope_id`]); `null` for `always`.
    pub scope_id: Option<String>,
    pub granted_at: u64,
    /// At most 24 h after `granted_at`; `null` for `always`.
    pub expires_at: Option<u64>,
    /// `person` (a tap on the card, or Settings) or `run-starter` (`--scope run` at the CLI).
    pub granted_by: String,
}

#[derive(Deserialize)]
struct File {
    version: u64,
    grants: Vec<serde_json::Value>,
}

impl Grant {
    /// Whether this is a grant this build reads, in force at `now`.
    pub fn valid_at(&self, now: u64) -> bool {
        let id_ok = self.id.len() == 14
            && self.id.starts_with("g-")
            && self.id[2..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !id_ok || self.agent != AGENT || self.capability != CAPABILITY {
            return false;
        }
        // Not granted in the future, beyond the skew a reader allows.
        if now.checked_add(SKEW_SECS).is_none_or(|latest| self.granted_at > latest) {
            return false;
        }
        match (self.scope.as_str(), self.scope_id.as_deref(), self.expires_at) {
            ("always", None, None) => self.granted_by == "person",
            (scope @ ("run" | "session"), Some(id), Some(expires)) => {
                scope_id_ok(id)
                    && self.granted_at < expires
                    && self.granted_at.checked_add(MOST_SECS).is_some_and(|most| expires <= most)
                    && now < expires
                    && self.granted_by == if scope == "run" { "run-starter" } else { "person" }
            }
            _ => false,
        }
    }

    /// Whether it lets `agent` search in its own words in harness session `session` (its
    /// [`session_scope_id`]) or run `run`, at `now`.
    pub fn covers(&self, agent: &str, session: Option<&str>, run: Option<&str>, now: u64) -> bool {
        self.valid_at(now)
            && self.agent == agent
            && match self.scope.as_str() {
                "always" => true,
                "session" => session.is_some() && self.scope_id.as_deref() == session,
                "run" => run.is_some() && self.scope_id.as_deref() == run,
                _ => false,
            }
    }
}

/// Whether `s` is a run or session id a grant may name: 1 to 64 of `A-Z a-z 0-9 . _ : -`.
pub fn scope_id_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_:".contains(&b))
}

/// A harness session's id as a grant names it: the first 16 hex digits of the SHA-256 of the
/// session the harness was given at attach. Never the session itself, which answers for the
/// harness: the grants file is readable by every account.
pub fn session_scope_id(session: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(session.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// The grants in force in `path`, if it passes the trust checks with `owner` (uid, gid) as the
/// only writer — `(0, 0)` on a machine, the test's own account in a test. `Err` says why the
/// file is not believed; a reader takes that as no grants.
pub fn read(path: &Path, owner: (u32, u32), now: u64) -> Result<Vec<Grant>, String> {
    let dir = path.parent().ok_or("no directory")?;
    let d = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !d.is_dir() || (d.uid(), d.gid()) != owner || d.mode() & 0o022 != 0 {
        return Err(format!("{} is not a directory only its writer may change", dir.display()));
    }
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let m = f.metadata().map_err(|e| e.to_string())?;
    if !m.file_type().is_file() || m.uid() != owner.0 || m.mode() & 0o022 != 0 {
        return Err(format!("{} is not a file only its writer may change", path.display()));
    }
    let mut raw = Vec::new();
    (&mut f).take(BIGGEST + 1).read_to_end(&mut raw).map_err(|e| e.to_string())?;
    if raw.len() as u64 > BIGGEST {
        return Err("the grants file is too large".into());
    }
    parse(&raw, now)
}

/// The grants in force in a file's bytes: version 1, each grant checked, expired ones dropped.
pub fn parse(raw: &[u8], now: u64) -> Result<Vec<Grant>, String> {
    let file: File = serde_json::from_slice(raw).map_err(|e| format!("not grants: {e}"))?;
    if file.version != VERSION {
        return Err(format!("grants version {} is not one this build reads", file.version));
    }
    Ok(file
        .grants
        .into_iter()
        .filter_map(|g| serde_json::from_value::<Grant>(g).ok())
        .filter(|g| g.valid_at(now))
        .collect())
}

/// The grants in force on this machine now; none when the file is missing or not believed.
pub fn in_force() -> Vec<Grant> {
    read(Path::new(GRANTS_PATH), (0, 0), now()).unwrap_or_default()
}

/// Unix seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn me() -> (u32, u32) {
        // SAFETY: neither can fail.
        unsafe { (libc::geteuid(), libc::getegid()) }
    }

    fn grant(scope: &str, scope_id: Option<&str>, expires: Option<u64>) -> serde_json::Value {
        json!({"id": "g-0123456789ab", "agent": "mind", "capability": CAPABILITY, "scope": scope,
               "scope_id": scope_id, "granted_at": 1000, "expires_at": expires,
               "granted_by": if scope == "run" { "run-starter" } else { "person" }})
    }

    fn file(grants: Vec<serde_json::Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({"version": 1, "written_at": 1000, "grants": grants})).unwrap()
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("yantrik-grants-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        d
    }

    #[test]
    fn the_format_reads_as_the_writer_writes_it() {
        let raw = file(vec![grant("session", Some("5f1c2a9e0b7d4c3e"), Some(5000)), grant("always", None, None)]);
        let grants = parse(&raw, 2000).unwrap();
        assert_eq!(grants.len(), 2);
        assert!(grants[0].covers("mind", Some("5f1c2a9e0b7d4c3e"), None, 2000));
        assert!(!grants[0].covers("mind", Some("another"), None, 2000), "a session grant is that session's");
        assert!(grants[1].covers("mind", None, None, 2000), "always is always");
        assert!(!grants[1].covers("pi", None, None, 2000), "and only the Mind's");
    }

    #[test]
    fn an_unknown_capability_a_bad_shape_or_an_expired_grant_is_dropped() {
        let mut other = grant("always", None, None);
        other["capability"] = json!("shell");
        let mut extra = grant("always", None, None);
        extra["also"] = json!(true);
        let long = grant("session", Some("s"), Some(1000 + MOST_SECS + 1));
        let expired = grant("session", Some("s"), Some(1500));
        let mut self_made = grant("session", Some("s"), Some(5000));
        self_made["granted_by"] = json!("mind");
        let raw = file(vec![other, extra, long, expired, self_made]);
        assert_eq!(parse(&raw, 2000).unwrap(), vec![]);
        assert!(parse(br#"{"version": 2, "grants": []}"#, 0).is_err(), "a version it does not know");
        assert!(parse(b"not json", 0).is_err());
    }

    #[test]
    fn a_trusted_file_is_read_and_a_link_or_a_writable_directory_is_not() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("trust");
        let path = d.join("grants.json");
        std::fs::write(&path, file(vec![grant("always", None, None)])).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(read(&path, me(), 2000).unwrap().len(), 1);
        assert!(read(&path, (me().0 + 1, me().1), 2000).is_err(), "owned by someone else");

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(read(&path, me(), 2000).is_err(), "a file others may write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o775)).unwrap();
        assert!(read(&path, me(), 2000).is_err(), "a directory others may write");
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();

        let link = d.join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read(&link, me(), 2000).is_err(), "a link at the path is not followed");

        let linked_dir = scratch("trust-link").join("dir");
        std::os::unix::fs::symlink(&d, &linked_dir).unwrap();
        assert!(read(&linked_dir.join("grants.json"), me(), 2000).is_err(), "nor a directory that is a link");
        assert!(read(&d.join("missing.json"), me(), 2000).is_err(), "a missing file: no grants");
    }

    #[test]
    fn a_grant_dated_in_the_future_is_dropped_beyond_the_skew() {
        let at = |granted_at: u64, scope: &str| {
            let mut g = if scope == "always" { grant("always", None, None) } else { grant("session", Some("s"), Some(granted_at + 3600)) };
            g["granted_at"] = json!(granted_at);
            g
        };
        let now = 1_759_650_000;
        // The review's probe: granted_at 4000000000 was kept at now=1759650000.
        assert_eq!(parse(&file(vec![at(4_000_000_000, "always")]), now).unwrap(), vec![]);
        assert_eq!(parse(&file(vec![at(4_000_000_000, "session")]), now).unwrap(), vec![]);
        assert_eq!(parse(&file(vec![at(now + SKEW_SECS + 1, "always")]), now).unwrap(), vec![], "just past the skew");
        assert_eq!(parse(&file(vec![at(now + SKEW_SECS, "always")]), now).unwrap().len(), 1, "within the skew");
        assert_eq!(parse(&file(vec![at(now + SKEW_SECS, "session")]), now).unwrap().len(), 1);
        assert_eq!(parse(&file(vec![at(now - 10, "session")]), now).unwrap().len(), 1, "an ordinary grant");
        // No overflow, in a debug build or any: a granted_at or a now at the top of the range.
        let mut top = grant("session", Some("s"), Some(u64::MAX));
        top["granted_at"] = json!(u64::MAX - 1);
        assert_eq!(parse(&file(vec![top.clone()]), now).unwrap(), vec![]);
        assert_eq!(parse(&file(vec![top]), u64::MAX).unwrap(), vec![]);
        assert_eq!(parse(&file(vec![grant("always", None, None)]), u64::MAX).unwrap(), vec![]);
    }

    #[test]
    fn a_run_grant_names_the_run_and_nothing_else() {
        let raw = file(vec![grant("run", Some("research-42"), Some(5000))]);
        let g = &parse(&raw, 2000).unwrap()[0];
        assert!(g.covers("mind", Some("5f1c2a9e0b7d4c3e"), Some("research-42"), 2000));
        assert!(!g.covers("mind", Some("5f1c2a9e0b7d4c3e"), None, 2000), "a turn in no run");
        assert!(!g.covers("mind", Some("5f1c2a9e0b7d4c3e"), Some("research-43"), 2000), "another run");
        assert!(scope_id_ok("research-42") && !scope_id_ok("") && !scope_id_ok("a b") && !scope_id_ok(&"x".repeat(65)));
    }

    #[test]
    fn a_session_is_named_by_a_digest_never_by_itself() {
        let id = session_scope_id("s3-0123456789abcdef");
        assert_eq!(id.len(), 16);
        assert!(!id.contains("s3-"));
        assert_eq!(id, session_scope_id("s3-0123456789abcdef"), "the same session, the same id");
    }
}
