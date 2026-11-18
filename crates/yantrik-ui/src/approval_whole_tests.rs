//! The four questions are drawn whole, by the question and not by its sentence (#1311, round 2).

use serde_json::json;

use super::*;

fn long(n: usize) -> String {
    "x".repeat(n)
}

#[test]
fn the_four_questions_are_whole_whatever_their_sentence_says() {
    for q in &crate::control_questions::QUESTIONS {
        assert!(is_question("shell", q.name), "{}", q.name);
        // Not the sentence: an empty one, and a mild grade, change nothing.
        assert!(shown_whole("shell", q.name, "standard", ""), "{}", q.name);
    }
    assert!(!is_question("shell", "send_mail") && !is_question("notes", "plan_work"), "only the shell's own four");
    assert!(!shown_whole("notes", "plan_work", "standard", "Make a note."), "another app's action of the same name is not one");
}

/// The cases the round exists for: a mail subject of sixty-one characters, and a recipe of more
/// than eight rows, are asked about and drawn in full.
#[test]
fn a_long_subject_and_a_long_plan_are_asked_and_drawn_whole() {
    let subject = json!({ "tool": "email_move", "subject": long(61), "to_folder": "Archive" });
    assert_eq!(refusal("shell", "change_item", &subject, None, true), None);
    let drawn = rows("shell", "change_item", "standard", "", &subject, false);
    assert!(drawn.contains(&format!("subject: {}", long(61))), "{drawn:?}");

    let mut plan = serde_json::Map::new();
    plan.insert("tool".into(), "create_recipe".into());
    plan.insert("name".into(), "Morning".into());
    plan.insert("trigger".into(), "every day at 07:00".into());
    for i in 1..=12 {
        plan.insert(format!("step_{i}"), format!("run get_weather with step {i}").into());
    }
    let plan = serde_json::Value::Object(plan);
    assert_eq!(refusal("shell", "plan_work", &plan, None, true), None, "fifteen rows are asked");
    assert_eq!(rows("shell", "plan_work", "sensitive", "", &plan, false).len(), 15, "and every one is drawn, with no 'and N more'");
}

#[test]
fn what_cannot_be_drawn_whole_is_refused_not_cut() {
    let too_wide = json!({ "tool": "t", "text": long(241) });
    assert!(refusal("shell", "plan_work", &too_wide, None, true).unwrap().contains("shown whole or not asked about"));
    let mut many = serde_json::Map::new();
    for i in 0..=ROWS {
        many.insert(format!("k{i}"), "v".into());
    }
    assert!(refusal("shell", "change_item", &serde_json::Value::Object(many), None, true).unwrap().contains("at most"));
    let mut heavy = serde_json::Map::new();
    for i in 0..12 {
        heavy.insert(format!("step_{i}"), long(230).into());
    }
    assert!(refusal("shell", "plan_work", &serde_json::Value::Object(heavy), None, true).unwrap().contains("past the"));
    let unpublished = json!({ "tool": "t", "sneaky": "v" });
    let published = vec!["tool".to_string()];
    assert!(refusal("shell", "plan_work", &unpublished, Some(&published), true).unwrap().contains("not an argument"));
}

#[test]
fn everything_else_is_bounded_as_before() {
    let ordinary = json!({ "text": long(100) });
    assert!(refusal("notes", "plan_work", &ordinary, None, false).is_none(), "an ordinary card cuts under its marker");
    assert!(refusal("notes", "x", &json!({ "text": long(300) }), None, true).is_some(), "a whole card of another app keeps its 240");
    assert_eq!(rows("notes", "x", "standard", "", &ordinary, false).len(), 1);
}

#[test]
fn the_companion_checks_against_these_bounds() {
    use yantrik_companion::tools::asks;
    assert_eq!((asks::WHOLE_ROWS, asks::WHOLE_TOTAL_CHARS, asks::WHOLE_VALUE_CHARS), (ROWS, TOTAL_CHARS, crate::approvals::WHOLE_VALUE_CHARS));
}
