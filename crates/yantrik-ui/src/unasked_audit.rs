//! What an unasked action is written down with (#1353, round 2).
//!
//! The audit line used to carry the arguments as the card shows them, each value cut at 60
//! characters, so a command that plain Bypass now runs without a card was kept as its first
//! sentence. The person reads the record afterwards to learn what was done, and a record that
//! drops the end of a command is the hidden-tail problem again. Here the line holds the whole
//! arguments up to [`INLINE_CHARS`], with their SHA-256; anything longer is stored whole in a
//! file under the audit directory, and the line gives the file's name. Nothing is cut.
//!
//! The shell writes the line itself, at the moment it answers `not_needed` ([`at_decision`]): a
//! caller's report of the outcome (`record_unasked_action`) only completes that line
//! ([`complete`]), so an unasked action is on record whether or not its caller reports.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The most characters of arguments kept inline on an audit line.
pub const INLINE_CHARS: usize = 4000;
/// Side files kept; the oldest go first.
const SIDE_FILES_KEPT: usize = 200;
/// What an entry's outcome says until the caller reports one.
pub const PENDING: &str = "ran unasked; outcome not yet reported";

/// The `args_full` field of an audit line for `args`: the exact text, its hash, and where the
/// text is kept when it is too long to sit on the line.
pub fn full(args: &serde_json::Value, dir: &Path) -> serde_json::Value {
    let text = args.to_string();
    let sha = hex(&Sha256::digest(text.as_bytes()));
    if text.chars().count() <= INLINE_CHARS {
        return serde_json::json!({ "sha256": sha, "text": text });
    }
    let name = format!("{sha}.json");
    let kept = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(dir.join(&name), &text));
    if let Err(e) = kept {
        tracing::warn!(error = %e, "could not store the whole arguments of an unasked action");
        // Never less than the whole: with no file to point at, the line carries it all.
        return serde_json::json!({ "sha256": sha, "text": text, "file_failed": true });
    }
    prune(dir);
    serde_json::json!({ "sha256": sha, "file": name, "chars": text.chars().count() })
}

/// The directory beside the audit file that holds the long ones.
pub fn side_dir(audit_path: &str) -> PathBuf {
    Path::new(audit_path).with_file_name("mind-audit-args")
}

/// The hash `full` writes for `args`, to match a caller's later report to the line the shell wrote.
pub fn sha_of(args: &serde_json::Value) -> String {
    hex(&Sha256::digest(args.to_string().as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn prune(dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<_> = read.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= SIDE_FILES_KEPT {
        return;
    }
    files.sort();
    for (_, path) in &files[..files.len() - SIDE_FILES_KEPT] {
        let _ = std::fs::remove_file(path);
    }
}

/// The shell's own line for an action it just answered `not_needed` for, when the mode or a rule,
/// not a card, let it run. `mode` is what answered (`auto`, `bypass`, `bypass_all`, `rule`,
/// `standing_rule`). Completed later by [`report`].
#[allow(clippy::too_many_arguments)]
pub fn at_decision(mode: &str, requester: &str, verified: &crate::approvals::Verified, app: &str, action: &str, args: &serde_json::Value, grade: &str) {
    crate::mind_mode::record(mode, requester, verified, app, action, args, grade, PENDING);
}

/// A caller's report of how an unasked action went. When the shell already wrote the line for it
/// ([`at_decision`], same app, action and arguments, outcome still pending) the outcome is added
/// to that line and to a line of its own in the file; otherwise it is a line of its own, as before.
#[allow(clippy::too_many_arguments)]
pub fn report(mode: &str, requester: &str, verified: &crate::approvals::Verified, app: &str, action: &str, args: &serde_json::Value, grade: &str, outcome: &str) -> crate::mind_mode::AuditEntry {
    let rows = crate::approvals::args_rows(args);
    let done = crate::mind_mode::with_audit(|list| {
        let at = list.iter().rposition(|e| e.app == app && e.action == action && e.outcome == PENDING && e.args == rows)?;
        list[at].outcome = outcome.trim().to_string();
        Some(list[at].clone())
    });
    let Some(entry) = done else {
        return crate::mind_mode::record(mode, requester, verified, app, action, args, grade, outcome);
    };
    let line = serde_json::json!({ "unix": entry.unix, "outcome_of": sha_of(args), "outcome": entry.outcome });
    append_line(&crate::mind_mode::audit_path(), &line.to_string());
    entry
}

fn append_line(path: &str, line: &str) {
    use std::io::Write;
    let wrote = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = wrote {
        tracing::warn!(error = %e, "could not write the outcome of an unasked action");
    }
}

#[cfg(test)]
#[path = "unasked_audit_tests.rs"]
mod tests;
