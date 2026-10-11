//! Tests for approval_bounds.rs: each rule a request is refused by before a card is raised, and
//! the cut after escaping that keeps the card a fixed size (third security review of #639).
use super::*;
use serde_json::json;

const PADDED_RM: &str = "echo tidying up the temporary build files now, one moment ; rm -rf ~";

#[test]
fn a_key_that_is_too_long_or_hides_characters_is_refused() {
    let long = "k".repeat(KEY_CHARS + 1);
    let why = refusal(&json!({ long: "x" }), None, false).expect("(a) a 41-character name is refused");
    assert!(why.contains("41 characters"), "{why}");
    assert!(refusal(&json!({ "k".repeat(KEY_CHARS): "x" }), None, false).is_none(), "40 is allowed");
    for hidden in ["pid\nUndo", "p\u{202E}id", "pi\u{200B}d", "a\\b"] {
        let why = refusal(&json!({ hidden: "1" }), None, false).unwrap_or_else(|| panic!("(a) {hidden:?} is refused"));
        assert!(why.contains("escapes"), "{why}");
    }
}

#[test]
fn an_argument_the_app_does_not_publish_is_refused() {
    let published = vec!["pid".to_string(), "force".to_string()];
    let why = refusal(&json!({ "pid": 1, "note": "x" }), Some(&published), false).expect("(b) refused");
    assert!(why.contains("`note`"), "{why}");
    assert!(refusal(&json!({ "pid": 1, "force": true }), Some(&published), false).is_none());
    // An app that publishes no parameters for the action takes none.
    assert!(refusal(&json!({ "pid": 1 }), Some(&[]), false).is_some());
    // Where the parameters are not to hand (the shell's own surface), the other rules still hold.
    assert!(refusal(&json!({ "pid": 1 }), None, false).is_none());
}

#[test]
fn rows_too_long_to_draw_are_refused() {
    // Eight values of control characters, each escaped nine-fold: past the budget.
    let args: serde_json::Map<String, serde_json::Value> =
        (0..8).map(|i| (format!("a{i}"), json!("\u{E0041}".repeat(60)))).collect();
    let why = refusal(&serde_json::Value::Object(args), None, false).expect("(c) refused");
    assert!(why.contains("characters as the card would draw them"), "{why}");
    // At the budget, allowed: eight rows with forty-character names and control characters in
    // every value, which with the seven "; " between them come to exactly TOTAL_CHARS drawn.
    let width_of = |i: usize| if i == 7 { 80 } else { 78 };
    let args: serde_json::Map<String, serde_json::Value> = (0..8)
        .map(|i| {
            let key = format!("{i}{}", "k".repeat(KEY_CHARS - 1));
            let value = "\u{1}".repeat(3) + &"v".repeat(width_of(i) - KEY_CHARS - 2 - 3 * 8);
            (key, json!(value))
        })
        .collect();
    let rows = approvals::args_rows(&serde_json::Value::Object(args.clone()));
    assert_eq!(joined(&rows).chars().count(), TOTAL_CHARS, "{rows:?}");
    assert!(refusal(&serde_json::Value::Object(args.clone()), None, false).is_none(), "exactly the budget is shown");
    // One character more is refused.
    let mut over = args;
    over.insert("0".repeat(1), json!("x"));
    assert!(refusal(&serde_json::Value::Object(over), None, false).is_some());
    // Eight ordinary sixty-character values under short names are asked about.
    let ordinary: serde_json::Map<String, serde_json::Value> =
        (0..8).map(|i| (format!("arg{i}"), json!("v".repeat(approvals::ARG_VALUE_CHARS)))).collect();
    assert!(refusal(&serde_json::Value::Object(ordinary), None, false).is_none());
}

/// (d), and the pre-existing hole it closes: `echo … ; rm -rf ~` padded past the sixty characters
/// a card shows of a value hid its end everywhere on the card. A card whose action cannot be taken
/// back or runs whatever it is given shows a value whole — up to 240 characters drawn, so ordinary
/// file names, container ids and event titles are asked about — and refuses anything longer.
#[test]
fn a_card_that_must_show_its_arguments_whole_shows_them_whole_or_is_not_asked() {
    assert!(PADDED_RM.chars().count() > approvals::ARG_VALUE_CHARS);
    // Whole: asked about, and the end is on the card.
    assert!(refusal(&json!({ "command": PADDED_RM }), None, true).is_none());
    let rows = approvals::args_rows_with(&json!({ "command": PADDED_RM }), approvals::WHOLE_VALUE_CHARS);
    assert!(rows[0].ends_with("rm -rf ~") && !rows[0].contains("characters in full"), "{rows:?}");
    // A 64-character container id, the kind sixty characters refused.
    assert!(refusal(&json!({ "container": "f".repeat(64) }), None, true).is_none());
    // Padded past what a card shows whole: refused, never cut.
    let padded = format!("echo {} ; rm -rf ~", "x".repeat(approvals::WHOLE_VALUE_CHARS));
    let why = refusal(&json!({ "command": padded }), None, true).expect("(d) refused");
    assert!(why.contains("shown whole or not asked about"), "{why}");
    // Escapes count: 121 newlines are 242 characters drawn.
    assert!(refusal(&json!({ "command": "\n".repeat(121) }), None, true).is_some());
    assert!(refusal(&json!({ "command": "\n".repeat(120) }), None, true).is_none());
    // No "and N more" on such a card: more than eight arguments is refused.
    let nine: serde_json::Map<String, serde_json::Value> = (0..9).map(|i| (format!("a{i}"), json!("x"))).collect();
    assert!(refusal(&serde_json::Value::Object(nine.clone()), None, true).expect("refused").contains("every one of them or none"));
    assert!(refusal(&serde_json::Value::Object(nine), None, false).is_none(), "an ordinary card summarises the ninth");
    // The padded value on an ordinary card is asked about, and shown cut under its marker.
    assert!(refusal(&json!({ "command": PADDED_RM }), None, false).is_none());
    let rows = approvals::args_rows(&json!({ "command": PADDED_RM }));
    assert!(rows[0].contains("characters in full)") && !rows[0].contains("rm -rf ~"), "{rows:?}");
    let c = crate::approval_wording::consequences("agent_run", crate::approval_wording::Published::app("Run it."), "Run it.", "", &rows);
    assert!(c.exactly.contains("characters in full)"), "the marker reaches the card: {}", c.exactly);
    // And the refusal says why, in one sentence.
    assert!(refused("x").starts_with("this request is too long to put in front of a person in full, so nothing was asked: "));
}

/// Fifth review of #639: the "; " between rows count. A request shown whole at exactly the budget
/// is drawn whole and uncut — the card's line is the very string the refusal measured — and one
/// character more is refused rather than cut on the card.
#[test]
fn the_separators_count_and_a_whole_request_at_the_limit_is_drawn_uncut() {
    // Three rows "a: …", "b: …", "c: …" and two separators: 3 + 3 + 3 + 4 = 13 around the values.
    let at_limit = |third: usize| {
        json!({
            "a": "x".repeat(approvals::WHOLE_VALUE_CHARS),
            "b": "y".repeat(approvals::WHOLE_VALUE_CHARS),
            "c": "z".repeat(third),
        })
    };
    let third = TOTAL_CHARS - 13 - 2 * approvals::WHOLE_VALUE_CHARS;
    let args = at_limit(third);
    let rows = approvals::args_rows_with(&args, approvals::WHOLE_VALUE_CHARS);
    assert_eq!(joined(&rows).chars().count(), TOTAL_CHARS);
    assert!(refusal(&args, None, true).is_none(), "exactly at the limit is asked about");
    let c = crate::approval_wording::consequences("x", crate::approval_wording::Published::app(""), "", "", &rows);
    assert!(!c.exactly.contains("characters in full"), "and drawn uncut: {}", c.exactly);
    assert!(c.exactly.ends_with(&"z".repeat(third)), "its last argument whole");
    assert_eq!(c.exactly, joined(&rows), "the card draws the string the refusal measured");
    // One more character, in the last value: refused, never cut.
    assert!(refusal(&at_limit(third + 1), None, true).is_some(), "one over is refused");
}

/// The recipe executor's hand_off card is raised by the desktop itself and never meets the
/// refusal; its task — up to 200 characters, the thing being asked — is shown whole, not cut at
/// sixty (fifth review of #639).
#[test]
fn a_hand_off_the_desktop_raises_shows_its_task_whole() {
    let task = format!("Review the release notes for 0.4 and {} then say what is missing", "check every section ".repeat(6));
    assert!(task.chars().count() > approvals::ARG_VALUE_CHARS && task.chars().count() <= 200);
    let mut store = approvals::Store::new();
    let now = std::time::Instant::now();
    let verified = approvals::Verified { raised_by_desktop: true, ..Default::default() };
    store
        .request("Council recipe", verified, "shell", "hand_off", json!({ "role": "reviewer", "task": task }), "sensitive", "Hand work to the Reviewer.", "", "", now, "10:00")
        .expect("raised");
    let card = store.cards(now).pop().expect("the card");
    assert!(card.args.iter().any(|r| r.ends_with("then say what is missing")), "{:?}", card.args);
    assert!(!card.args.iter().any(|r| r.contains("characters in full")), "{:?}", card.args);
}

/// Which cards must show their arguments whole: the red button's cards, and open-ended ones —
/// `agent_run` is graded `sensitive` and was neither (review of #639, should-fix 1).
#[test]
fn open_ended_cards_show_their_arguments_whole_too() {
    let open = format!("Run a command. {}", yantrik_ipc_contracts::control_surface::OPEN_ENDED);
    assert!(approvals::shown_whole("sensitive", &open));
    assert!(approvals::shown_whole("dangerous", "End a process."));
    assert!(approvals::shown_whole("sensitive", "Take an event off the calendar. It is not recoverable"));
    assert!(!approvals::shown_whole("sensitive", "Move files."));
}

/// Defence in depth: whatever reaches the card is cut after escaping, keys included, so its worst
/// case is a fixed size.
#[test]
fn what_is_drawn_is_cut_after_escaping() {
    let huge = format!("{}: {}", "k".repeat(500), "\u{E0041}".repeat(60));
    let row = clip_row(&visible(&huge));
    assert!(row.chars().count() <= ROW_CHARS + 40, "{}", row.chars().count());
    assert!(row.contains("characters in full)"));
    let rows: Vec<String> = (0..9).map(|_| huge.clone()).collect();
    let c = crate::approval_wording::consequences("x", crate::approval_wording::Published::app(""), "", "", &rows);
    assert!(c.exactly.chars().count() <= TOTAL_CHARS + 40, "{}", c.exactly.chars().count());
}
