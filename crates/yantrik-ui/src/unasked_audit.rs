//! What an unasked action is written down with (#1353, rounds 2 and 4).
//!
//! The shell writes the line itself, at the moment it decides to run an action unasked
//! ([`at_decision`], for the shell's own road and for an app's gate that asks the shell to record
//! one, [`gate_decision`]). The line holds the whole arguments up to
//! [`crate::audit_store::INLINE_CHARS`] with their SHA-256, longer ones whole in a side file the
//! line names (`audit_store`). Nothing is cut, and an action whose arguments cannot be written down
//! whole is not run.
//!
//! A caller's report of how it went ([`report`]) is the caller's: it goes in its own `reported`
//! field, on the shell's line when the full-argument hash matches one, and as a note labelled as a
//! report otherwise. A report never makes a "ran unasked" line of its own, so no caller can write
//! the record's account of what ran, or complete a line it did not cause.

use crate::audit_store::sha256;

/// What an entry's outcome says: the shell's own words, until and unless the caller reports.
pub const PENDING: &str = "ran unasked; outcome not yet reported";

/// The hash `audit_store::full` writes for `args`, to match a caller's later report to its line.
pub fn sha_of(args: &serde_json::Value) -> String {
    sha256(&args.to_string())
}

/// The shell's own line for an action it just answered `not_needed` for, when the mode or a rule,
/// not a card, let it run. `mode` is what answered (`auto`, `bypass`, `bypass_all`, `rule`,
/// `standing_rule`). `Err` when it could not be written down whole: then it must not run.
#[allow(clippy::too_many_arguments)]
pub fn at_decision(mode: &str, requester: &str, verified: &crate::approvals::Verified, app: &str, action: &str, args: &serde_json::Value, grade: &str) -> Result<(), String> {
    crate::mind_mode::record_checked(mode, requester, verified, app, action, args, grade, PENDING).1
}

/// A caller's report of how an unasked action went. When the shell wrote the line for it (same
/// app, action and full-argument hash, no report yet) the report is added to that line's `reported`
/// field and a note naming the hash is appended; otherwise it is a note that says it matched
/// nothing. The line's `outcome` is the shell's and is never rewritten.
#[allow(clippy::too_many_arguments)]
pub fn report(requester: &str, app: &str, action: &str, args: &serde_json::Value, outcome: &str) -> String {
    let sha = sha_of(args);
    let reported = outcome.trim().to_string();
    let matched = crate::mind_mode::with_audit(|list| {
        let at = list.iter().rposition(|e| e.app == app && e.action == action && e.args_sha == sha && e.reported.is_empty())?;
        list[at].reported = reported.clone();
        Some(list[at].clone())
    });
    let note = serde_json::json!({
        "kind": "caller_report",
        "unix": crate::mind_mode::unix_now(),
        "reported_by": requester.trim(),
        "reported_for": { "app": app, "action": action },
        "args_sha256": sha,
        "reported": reported,
        "matched_a_line_the_shell_wrote": matched.is_some(),
    });
    crate::audit_store::append_note(std::path::Path::new(&crate::mind_mode::audit_path()), &note);
    match matched {
        Some(entry) => entry.line(),
        None => format!("{app}.{action}: report noted; the shell has no unasked line for it, so none was changed"),
    }
}

#[cfg(test)]
#[path = "unasked_audit_tests.rs"]
mod tests;
