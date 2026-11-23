//! #1357 round 2: what an unasked action is written down with.

use super::*;
use serde_json::json;

fn lines() -> Vec<serde_json::Value> {
    std::fs::read_to_string(crate::mind_mode::audit_path()).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn mine(app: &str, command: &str) -> Vec<serde_json::Value> {
    lines().into_iter().filter(|v| v["app"] == app && v["args_full"]["text"].as_str().is_some_and(|t| t.contains(command)) || v["app"] == app && v["args_full"]["file"].is_string()).collect()
}

/// A 1,000-character command run unasked is on the audit line whole, with its hash.
#[test]
fn a_thousand_character_command_run_unasked_is_in_the_audit_whole() {
    let command = format!("echo {}", "ab".repeat(500));
    let args = json!({ "command": command });
    at_decision("bypass", "Test", &crate::approvals::Verified::default(), "audit-test-a", "run", &args, "sensitive").unwrap();
    let found = mine("audit-test-a", &command);
    let line = found.last().expect("the line is in the file");
    assert!(line["args_full"]["text"].as_str().unwrap().contains(&command), "whole, not 60 characters");
    assert_eq!(line["args_full"]["sha256"], sha_of(&args));
    assert_eq!(line["outcome"], PENDING);
}

/// Past the inline bound the whole text goes to a file named on the line.
#[test]
fn a_longer_one_is_stored_whole_in_a_side_file() {
    let command = "z".repeat(crate::audit_store::INLINE_CHARS + 500);
    let args = json!({ "command": command });
    let audit = crate::mind_mode::audit_path();
    let field = crate::audit_store::full(&args, &crate::audit_store::side_dir(std::path::Path::new(&audit))).unwrap();
    let name = field["file"].as_str().expect("a file is named");
    let kept = std::fs::read_to_string(crate::audit_store::side_dir(std::path::Path::new(&audit)).join(name)).unwrap();
    assert_eq!(kept, args.to_string(), "the file holds all of it");
    assert!(field["text"].is_null(), "and the line does not carry a cut copy");
    assert_eq!(field["sha256"], sha_of(&args));
}

/// A caller's report is the caller's: its own field, on the shell's line, and the shell's own
/// outcome is not rewritten. One entry, however it reports.
#[test]
fn a_callers_report_goes_in_its_own_field_on_the_shells_line() {
    let args = json!({ "command": "ls /tmp/audit-test-b" });
    let verified = crate::approvals::Verified::default();
    let before = crate::mind_mode::recent(1000).iter().filter(|e| e.app == "audit-test-b").count();
    at_decision("bypass_all", "Test", &verified, "audit-test-b", "run", &args, "sensitive").unwrap();
    let said = report("Test", "audit-test-b", "run", &args, "ok");
    assert!(said.contains("(as reported)"), "{said}");
    let entry = crate::mind_mode::recent(1000).into_iter().rev().find(|e| e.app == "audit-test-b").unwrap();
    assert_eq!(entry.reported, "ok");
    assert_eq!(entry.outcome, PENDING, "the shell's own words are not the caller's");
    assert_eq!(crate::mind_mode::recent(1000).iter().filter(|e| e.app == "audit-test-b").count(), before + 1);
    assert!(lines().iter().any(|v| v["kind"] == "caller_report" && v["args_sha256"] == sha_of(&args) && v["reported"] == "ok" && v["matched_a_line_the_shell_wrote"] == true));
}

/// A report for something the shell never wrote makes no "ran unasked" line: no entry, no app or
/// action field a reader would take for the shell's own, and it says it matched nothing.
#[test]
fn a_report_for_nothing_the_shell_wrote_is_a_labelled_note_only() {
    let said = report("Forger", "audit-test-c", "format_disk", &json!({ "dev": "/dev/x" }), "ok");
    assert!(said.contains("none was changed"), "{said}");
    // No entry for it: judged by its own app, since tests running beside this one add entries of
    // their own to the shared list (counting the list raced them).
    assert!(!crate::mind_mode::recent(1000).iter().any(|e| e.app == "audit-test-c"), "no entry was made for it");
    let note = lines().into_iter().rev().find(|v| v["reported_for"]["app"] == "audit-test-c").unwrap();
    assert_eq!(note["kind"], "caller_report");
    assert_eq!(note["matched_a_line_the_shell_wrote"], false);
    assert!(note["app"].is_null() && note["action"].is_null(), "not shaped like an entry");
}

/// Two commands that share their first 60 characters are two lines, and a report completes the one
/// whose full hash it carries, not the first that looks alike.
#[test]
fn a_report_is_matched_on_the_full_hash_not_on_cut_rows() {
    let head = "h".repeat(80);
    let (a, b) = (json!({ "command": format!("{head}-A") }), json!({ "command": format!("{head}-B") }));
    let verified = crate::approvals::Verified::default();
    at_decision("bypass", "T", &verified, "audit-test-d", "run", &a, "sensitive").unwrap();
    at_decision("bypass", "T", &verified, "audit-test-d", "run", &b, "sensitive").unwrap();
    report("T", "audit-test-d", "run", &a, "done-a");
    let mine: Vec<_> = crate::mind_mode::recent(1000).into_iter().filter(|e| e.app == "audit-test-d").collect();
    assert_eq!(mine[0].reported, "done-a");
    assert_eq!(mine[1].reported, "", "the other line is not touched");
}

/// The shell writes the line where it decides, before it answers, whoever reports afterwards.
#[test]
fn the_shell_writes_the_line_when_it_answers_not_needed() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/control_approvals.rs")).unwrap();
    let ran = source.find("Decision::Run { unasked } =>").expect("the run arm");
    let wrote = source[ran..].find("crate::unasked_audit::at_decision(").expect("it writes the line itself");
    let answered = source[ran..].find("return Ok(answer)").expect("and then answers");
    assert!(wrote < answered);
}
