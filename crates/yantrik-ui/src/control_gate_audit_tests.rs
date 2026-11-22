//! #1357 round 4: an app's gate has the shell write the line.

use super::*;
use serde_json::json;

fn lines() -> Vec<serde_json::Value> {
    std::fs::read_to_string(crate::mind_mode::audit_path()).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[test]
fn the_shell_writes_the_line_with_its_own_mode_and_the_whole_arguments() {
    let command = format!("echo {}", "q".repeat(1500));
    write("an app's gate (test)", "gate-test-a", "run", "sensitive", "bypass_all", &json!({ "command": command }), false).unwrap();
    let line = lines().into_iter().rev().find(|v| v["app"] == "gate-test-a").expect("the line is in the file");
    assert!(line["args_full"]["text"].as_str().unwrap().contains(&command), "whole");
    assert!(line["mode"].as_str().unwrap().starts_with(crate::mind_mode::current().as_str()), "the shell's own mode: {}", line["mode"]);
    assert!(line["mode"].as_str().unwrap().contains("the gate read bypass_all") || crate::mind_mode::current().as_str() == "bypass_all");
    assert!(line["requester"].as_str().unwrap().starts_with("an app's gate"));
}

#[test]
fn arguments_past_a_megabyte_are_not_written_and_so_not_run() {
    let huge = json!({ "command": "x".repeat(crate::audit_store::SIDE_FILE_MAX + 10) });
    let err = write("an app's gate (test)", "gate-test-b", "run", "sensitive", "bypass", &huge, false).unwrap_err();
    assert!(err.contains("written down in full"), "{err}");
    assert!(!lines().iter().any(|v| v["app"] == "gate-test-b"), "no line says it ran");
}

/// The Mind cannot ask the shell to write this line, and the shell's own registry writes in-process.
#[test]
fn the_mind_cannot_send_it() {
    let guard = include_str!("../../yantrik-companion-tools/src/app_guard.rs");
    assert!(guard.contains("(\"shell\", \"record_gate_decision\")"));
    assert_eq!(yantrik_ipc_transport::unasked::ACTION, "record_gate_decision");
}
