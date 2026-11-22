//! #1357 round 2: a command is drawn whole and always in view, and only a published command
//! parameter of a published open-ended action gets the 4,000-character room.

use crate::approvals::{Card, Status, Verified};
use serde_json::json;
use slint::Model;

fn card(app: &str, action: &str, open_ended: bool, args: Vec<String>) -> Card {
    Card {
        id: "appr-1".into(),
        requester: "Test".into(),
        verified: Verified::default(),
        app: app.into(),
        action: action.into(),
        grade: "sensitive".into(),
        purpose: "Run it. What it runs can do anything you can.".into(),
        summary: "Run it.".into(),
        args,
        target: String::new(),
        explained: String::new(),
        warning: String::new(),
        said: "Run it. What it runs can do anything you can.".into(),
        open_ended,
        caller_says: String::new(),
        can_session: true,
        wording: Default::default(),
        status: Status::Pending,
        record: String::new(),
        decided_at: String::new(),
        session: false,
        age_secs: 1,
    }
}

/// A 3,000-character command: the row carries it whole in `command`, the pinned "Exactly" line
/// does not cut it (it is not there), and no argument row repeats it.
#[test]
fn a_three_thousand_character_command_reaches_the_card_whole() {
    let command = "echo x && ".repeat(300);
    let rows = crate::approvals::args_rows_with(&json!({ "command": command, "cwd": "/tmp" }), crate::approval_bounds::COMMAND_CHARS);
    let row = crate::control_approvals::row_for(card("terminal", "run", true, rows));
    assert!(row.command.as_str().ends_with(command.as_str()), "the whole command, to its last character");
    assert!(!row.exactly.as_str().contains("characters in full"), "no pinned line cuts it");
    assert_eq!(row.args.iter().map(|a| a.to_string()).collect::<Vec<_>>(), vec!["cwd: /tmp".to_string()]);
    assert!(row.open_ended);
}

/// A card for an action whose own sentence does not declare it open-ended gets no command box,
/// whatever the caller wrote beside it.
#[test]
fn a_caller_cannot_make_an_action_open_ended() {
    let mut c = card("calendar", "delete_event", false, vec![format!("command: {}", "y".repeat(2000))]);
    c.said = format!("{} What it runs can do anything you can.", c.said);
    let row = crate::control_approvals::row_for(c);
    assert!(!row.open_ended && row.command.is_empty());
}

/// The room is the app's declaration plus a parameter it names; a long purpose, or any other
/// argument, keeps the old bound. A long purpose on an unrecoverable action is refused.
#[test]
fn the_room_is_only_for_the_command_argument_of_a_published_open_ended_action() {
    use crate::approval_whole::refusal_for;
    let long = "p".repeat(1000);
    // An unrecoverable action (not open-ended by its own sentence) with a long argument: refused.
    assert!(refusal_for("calendar", "delete_event", &json!({ "purpose": long }), None, true, false).is_some());
    // The same against an open-ended action: a parameter that is not the command is still held to the old bound...
    assert!(refusal_for("terminal", "run", &json!({ "command": "ls", "purpose": long }), None, true, true).is_some());
    // ...and so is a long value under a name that is not a command parameter.
    assert!(refusal_for("terminal", "run", &json!({ "script": long }), None, true, true).is_some());
    // The command parameter itself has the room.
    assert!(refusal_for("terminal", "run", &json!({ "command": long }), None, true, true).is_none());
    // The shell's own bound reads the app's published sentence for the room, never the caller's
    // purpose: a 4,000-character `command` on an action that is not open-ended is refused even
    // when the caller's purpose says it runs whatever it is given (#1357, round 4).
    let big = json!({ "command": "z".repeat(3000) });
    let runs = "It runs whatever it is given. What it runs can do anything you can.";
    use crate::approval_whole::request_refusal;
    assert!(request_refusal("calendar", "delete_event", &big, None, "sensitive", "Delete an event. It is not recoverable.", runs).is_some(), "the caller's words give no room");
    assert!(request_refusal("calendar", "delete_event", &big, None, "sensitive", "Delete an event. It is not recoverable.", "").is_some());
    assert!(request_refusal("terminal", "run", &big, None, "sensitive", runs, "").is_none(), "the app's own sentence does");
    // The handler itself calls this and nothing else for the bound.
    let source = include_str!("control_approvals.rs");
    assert!(source.contains("approval_whole::request_refusal("), "the handler bounds through the one function");
    assert!(!source.contains("approvals::open_ended(&said)"), "never the caller's purpose");
}

/// The card's Allow and "for this session" are held by the command box's own measure, and the box
/// is drawn outside Details.
#[test]
fn the_command_box_is_pinned_and_holds_allow_by_its_own_measure() {
    let lens = include_str!("../../yantrik-ui-slint/ui/components/intent_lens.slint");
    let details = lens.find("if root.details-open : VerticalLayout {").expect("details");
    let boxed = lens.find("if root.data.open-ended : CommandBox {").expect("the command box");
    assert!(boxed < details, "the box is not inside Details");
    assert!(lens.contains("root.cmd-ready-id == root.data.id"), "allow-ready waits for the box");
    assert!(!lens.contains("args-h"), "no stale height decides it");
    let component = include_str!("../../yantrik-ui-slint/ui/components/command_box.slint");
    assert!(component.contains("flick.viewport-height <= root.height") && component.contains("root.reached = root.id"));
    // Session button too: both sit behind `card.allow-ready`.
    assert!(lens.matches("ready: card.allow-ready").count() >= 2);
}

/// The bound and the card choose the same argument as the command, through one function, even when
/// an open-ended action takes both `command` and `text` (#1357, round 4).
#[test]
fn the_bound_and_the_card_choose_the_same_command() {
    use crate::approval_bounds::{command_key, command_row};
    for (c, t) in [("ls".to_string(), "x".repeat(900)), ("y".repeat(900), "ls".to_string()), ("same".to_string(), "same".to_string())] {
        let args = json!({ "command": c, "text": t, "cwd": "/tmp" });
        let rows = crate::approvals::args_rows_with(&args, crate::approval_bounds::COMMAND_CHARS);
        let key = command_key(&args).unwrap();
        let at = command_row(&rows).unwrap();
        assert!(rows[at].starts_with(&format!("{key}: ")), "{key} vs {}", rows[at]);
    }
}

/// Every new request starts the box at the top, and Allow is held until the end has been in view
/// for THIS request, after its top (#1357, round 4). Source check: Slint is not run in the tests.
#[test]
fn the_command_box_starts_at_the_top_for_every_request() {
    let component = include_str!("../../yantrik-ui-slint/ui/components/command_box.slint");
    assert!(component.contains("changed id =>") && component.contains("flick.viewport-y = 0px;"), "the scroll resets for a new request");
    assert!(component.contains("root.reached = \"\";"), "and what was read for the last one is forgotten");
    assert!(component.contains("root.top-seen == root.id && (flick.viewport-height"), "ready needs the top of THIS request");
    assert!(component.contains("if (root.top-seen == root.id && self.viewport-y <="), "the end counts only after the top");
}

/// A command with one very long unbroken token (a path, a base64 string, a URL) is wrapped inside the
/// box, never cut at its right edge: the box clips and has no horizontal scroll, so a word that was
/// not broken would hide the end of its line while the height said Allow could be live (#1357,
/// round 7). `word-wrap` is the one that breaks a word too long for the line "at any location" in
/// every renderer; `char-wrap` is Qt and Software only (Slint 1.17's TextWrap), and `no-wrap` would
/// overflow. Source check, as the card's other layout tests: Slint is not run in the tests.
#[test]
fn the_command_box_breaks_a_very_long_unbroken_token_instead_of_cutting_it() {
    let component = include_str!("../../yantrik-ui-slint/ui/components/command_box.slint");
    let text = component.find("text: root.command;").expect("the command text");
    let drawn = &component[text..component[text..].find('}').map(|n| text + n).unwrap()];
    assert!(drawn.contains("wrap: word-wrap;"), "a token wider than the box is broken at any place: {drawn}");
    assert!(!drawn.contains("no-wrap") && !drawn.contains("char-wrap"), "{drawn}");
    assert!(!component.contains("horizontal-stretch") || !component.contains("min-width: root.command"), "the text is never given a width of its own");
    // The one thing that could still hide an end is a measure that ignores the lines drawn: Allow is
    // held until the viewport's end has been in view, and that end is the wrapped text's.
    assert!(component.contains("flick.viewport-height <= root.height") && component.contains("root.reached = root.id"));
}
