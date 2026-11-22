//! #1353: a command that must be asked is shown whole, up to 4,000 characters.

use super::*;
use serde_json::json;

fn run(command: &str) -> serde_json::Value {
    json!({ "command": command })
}

#[test]
fn a_thousand_character_command_is_asked_and_drawn_whole() {
    let command = "echo hello && ".repeat(72);
    assert!(command.chars().count() >= 1000);
    assert_eq!(refusal_for(&run(&command), None, true, true), None);
    let rows = approvals::args_rows_with(&run(&command), COMMAND_CHARS);
    let drawn = clip_row_for(true, &rows[0]);
    assert!(drawn.ends_with(command.as_str()), "the whole command is drawn, uncut");
    assert!(!drawn.contains("characters in full"));
    // Not a command: the old bound still refuses it.
    assert!(refusal_for(&run(&command), None, true, false).is_some());
}

#[test]
fn a_five_thousand_character_command_is_refused_with_a_way_forward() {
    let said = refusal_for(&run(&"x".repeat(5000)), None, true, true).expect("refused");
    assert!(said.contains("split it into shorter commands, or write it to a file in a visible folder and run that file"), "{said}");
    assert!(said.contains("4000"), "{said}");
    assert_eq!(refusal_for(&run(&"x".repeat(4000)), None, true, true), None, "4,000 is the last that fits");
}

#[test]
fn every_other_argument_keeps_the_six_hundred_and_forty_character_total() {
    let big = json!({ "command": "y".repeat(3000), "cwd": "a".repeat(200), "note": "b".repeat(200), "extra": "c".repeat(200), "more": "d".repeat(200) });
    let said = refusal_for(&big, None, true, true).expect("the others come to more than the total");
    assert!(said.contains("640"), "{said}");
    let ok = json!({ "command": "y".repeat(3000), "cwd": "/tmp" });
    assert_eq!(refusal_for(&ok, None, true, true), None);
}

/// A command nobody is asked about is never measured against the card: the refusal sits after
/// the mode's decision, which returns first when the mode runs it.
#[test]
fn a_command_that_is_not_asked_is_never_length_checked() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/control_approvals.rs")).unwrap();
    let ran = source.find("Decision::Run { unasked } =>").expect("the run arm");
    let checked = source.find("approval_whole::refusal_for(").expect("the bound");
    assert!(ran < checked, "the decision comes before the card's bound");
}
