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
//!
//! A run grant also has a one-time run secret (#667 review, round 2, M2-r2). `mind-grant add
//! --scope run` prints 128 random bits to whoever ran it, once, and root keeps only their SHA-256
//! in [`RUN_SECRETS_PATH`], under the same trust checks. The shell stamps a run on a turn only for
//! a `send_message run=ID run_secret=…` whose secret hashes to the one kept for that run's grant
//! in force ([`run_secret_ok`]), so the run id, readable by every account in [`GRANTS_PATH`], is
//! not enough to spend the grant.

use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Every grant in force, written by root.
pub const GRANTS_PATH: &str = "/run/yantrik-mind-egress/grants.json";
/// The SHA-256 of each run grant's one-time secret, written by root beside [`GRANTS_PATH`].
pub const RUN_SECRETS_PATH: &str = "/run/yantrik-mind-egress/run-secrets.json";
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
    parse(&read_trusted(path, owner)?, now)
}

/// The bytes of `path`, if it and its directory pass the trust checks with `owner` as the only
/// writer.
fn read_trusted(path: &Path, owner: (u32, u32)) -> Result<Vec<u8>, String> {
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
        return Err(format!("{} is too large", path.display()));
    }
    Ok(raw)
}

/// One run's secret, as root keeps it: never the secret, only its SHA-256.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSecret {
    /// The run grant it belongs to.
    grant: String,
    run: String,
    /// Lowercase hex SHA-256 of the secret as printed (32 lowercase hex digits).
    sha256: String,
    expires_at: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunSecrets {
    version: u64,
    #[allow(dead_code)]
    written_at: u64,
    runs: Vec<serde_json::Value>,
}

/// Whether `secret` is the one-time secret of a run grant for `run` in force at `now`: it hashes
/// to the SHA-256 root kept in `secrets_path` for that run, and the grant that entry names is in
/// `grants_path`, in force, scoped to that run. Both files pass the trust checks with `owner` as
/// their only writer. `Err` says why not, never what the secret or its hash was.
pub fn run_secret_ok(secrets_path: &Path, grants_path: &Path, owner: (u32, u32), run: &str, secret: &str, now: u64) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    if secret.len() != 32 || !secret.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err("a run secret is the 32 lowercase hex digits `mind-grant add --scope run` printed".into());
    }
    let file: RunSecrets = serde_json::from_slice(&read_trusted(secrets_path, owner)?)
        .map_err(|e| format!("{} is not run secrets: {e}", secrets_path.display()))?;
    if file.version != VERSION {
        return Err(format!("run secrets version {} is not one this build reads", file.version));
    }
    let digest: String = Sha256::digest(secret.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    // Every entry is compared, all the way through, so the time taken says nothing about which.
    let same = |a: &str, b: &str| a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0;
    let mut matched: Option<String> = None;
    for entry in file.runs.into_iter().filter_map(|e| serde_json::from_value::<RunSecret>(e).ok()) {
        if same(&entry.sha256, &digest) && entry.run == run && now < entry.expires_at {
            matched = Some(entry.grant);
        }
    }
    let grant = matched.ok_or_else(|| format!("that is not the run secret of a run grant in force for {run:?}"))?;
    let grants = read(grants_path, owner, now)?;
    if grants.iter().any(|g| g.id == grant && g.scope == "run" && g.scope_id.as_deref() == Some(run)) {
        Ok(())
    } else {
        Err(format!("the run grant for {run:?} that secret belongs to is no longer in force"))
    }
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

    // ── M2-r2: a run is spent only with its one-time secret ──

    /// `d` holding a run grant for `run` and the SHA-256 of `secret` for it, as root writes them.
    fn run_files(d: &std::path::Path, run: &str, secret: &str, expires: u64) {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;
        let mut g = grant("run", Some(run), Some(expires));
        g["granted_at"] = json!(expires - 3600);
        std::fs::write(d.join("grants.json"), serde_json::to_vec(&json!({"version": 1, "written_at": 1, "grants": [g]})).unwrap()).unwrap();
        let digest: String = Sha256::digest(secret.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let runs = json!({"version": 1, "written_at": 1, "runs": [{"grant": "g-0123456789ab", "run": run, "sha256": digest, "expires_at": expires}]});
        std::fs::write(d.join("run-secrets.json"), serde_json::to_vec(&runs).unwrap()).unwrap();
        for f in ["grants.json", "run-secrets.json"] {
            std::fs::set_permissions(d.join(f), std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn check(d: &std::path::Path, run: &str, secret: &str, now: u64) -> Result<(), String> {
        run_secret_ok(&d.join("run-secrets.json"), &d.join("grants.json"), me(), run, secret, now)
    }

    #[test]
    fn the_run_secret_spends_its_own_run_and_nothing_else() {
        let d = scratch("secret");
        run_files(&d, "research-42", SECRET, 10_000);
        assert_eq!(check(&d, "research-42", SECRET, 9_000), Ok(()));
        // The run id alone, or with a wrong secret, is nothing.
        assert!(check(&d, "research-42", "ffffffffffffffffffffffffffffffff", 9_000).is_err());
        assert!(check(&d, "research-42", "", 9_000).is_err());
        assert!(check(&d, "research-42", &SECRET.to_uppercase(), 9_000).is_err(), "only the form printed");
        // The hash, read from the file, is not the secret.
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(SECRET.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        assert!(check(&d, "research-42", &hash, 9_000).is_err());
        // Another run, or the same secret after the grant expired.
        assert!(check(&d, "research-43", SECRET, 9_000).is_err());
        assert!(check(&d, "research-42", SECRET, 10_000).is_err());
    }

    #[test]
    fn a_run_secret_without_its_grant_in_force_or_in_an_untrusted_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("secret-revoked");
        run_files(&d, "research-42", SECRET, 10_000);
        // Revoked: the grant is gone from the grants file, though its secret line lingers.
        std::fs::write(d.join("grants.json"), br#"{"version": 1, "written_at": 1, "grants": []}"#).unwrap();
        assert!(check(&d, "research-42", SECRET, 9_000).unwrap_err().contains("no longer in force"));
        // A secrets file anyone may write is not believed.
        run_files(&d, "research-42", SECRET, 10_000);
        std::fs::set_permissions(d.join("run-secrets.json"), std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(check(&d, "research-42", SECRET, 9_000).is_err());
        // Nor is a missing one.
        std::fs::remove_file(d.join("run-secrets.json")).unwrap();
        assert!(check(&d, "research-42", SECRET, 9_000).is_err());
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
