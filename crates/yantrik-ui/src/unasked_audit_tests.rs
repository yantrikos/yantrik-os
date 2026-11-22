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
    at_decision("bypass", "Test", &crate::approvals::Verified::default(), "audit-test-a", "run", &args, "sensitive");
    let found = mine("audit-test-a", &command);
    let line = found.last().expect("the line is in the file");
    assert!(line["args_full"]["text"].as_str().unwrap().contains(&command), "whole, not 60 characters");
    assert_eq!(line["args_full"]["sha256"], sha_of(&args));
    assert_eq!(line["outcome"], PENDING);
}

/// Past the inline bound the whole text goes to a file named on the line.
#[test]
fn a_longer_one_is_stored_whole_in_a_side_file() {
    let command = "z".repeat(INLINE_CHARS + 500);
    let args = json!({ "command": command });
    let field = full(&args, &side_dir(&crate::mind_mode::audit_path()));
    let name = field["file"].as_str().expect("a file is named");
    let kept = std::fs::read_to_string(side_dir(&crate::mind_mode::audit_path()).join(name)).unwrap();
    assert_eq!(kept, args.to_string(), "the file holds all of it");
    assert!(field["text"].is_null(), "and the line does not carry a cut copy");
    assert_eq!(field["sha256"], sha_of(&args));
}

/// A caller's later report completes the shell's line and does not write a second one.
#[test]
fn a_callers_report_completes_the_shells_line() {
    let args = json!({ "command": "ls /tmp/audit-test-b" });
    let verified = crate::approvals::Verified::default();
    let before = crate::mind_mode::recent(1000).iter().filter(|e| e.app == "audit-test-b").count();
    at_decision("bypass_all", "Test", &verified, "audit-test-b", "run", &args, "sensitive");
    let done = report("bypass_all", "Test", &verified, "audit-test-b", "run", &args, "sensitive", "ok");
    assert_eq!(done.outcome, "ok");
    let after = crate::mind_mode::recent(1000).iter().filter(|e| e.app == "audit-test-b").count();
    assert_eq!(after, before + 1, "one entry, completed");
    assert!(lines().iter().any(|v| v["outcome_of"] == sha_of(&args) && v["outcome"] == "ok"), "the file says how it went");
    // A report for something the shell never saw is a line of its own, as before.
    report("bypass", "Test", &verified, "audit-test-b", "other", &json!({}), "sensitive", "ok");
    assert_eq!(crate::mind_mode::recent(1000).iter().filter(|e| e.app == "audit-test-b").count(), before + 2);
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
