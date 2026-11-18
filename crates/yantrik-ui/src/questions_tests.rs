//! The four questions as the shell publishes them (#1311): each at its grade, each refused by its
//! handler, and the shell's own decision table reading them as the issue says. The same reading
//! as `control_mail`'s tests: the grade and sentence come from the declaration itself.

use std::time::Instant;

use yantrik_companion::tools::asks::{self, Question};
use yantrik_ipc_transport::gate::{self, Authority, Mode as GateMode};

use super::*;
use crate::mind_mode::{Decision, Mode, Modes};

fn authority(mode: &str) -> Authority {
    Authority { ceiling: "dangerous".into(), mode: GateMode::named(mode), granted: false, asks_above: None }
}

fn decide(q: &Published, mode: Mode) -> Decision {
    let cannot_be_undone = gate::unrecoverable(q.sentence);
    Modes::new(mode).decide(q.grade, "shell", q.name, cannot_be_undone, "dangerous", Instant::now())
}

fn asked(d: &Decision) -> bool {
    matches!(d, Decision::Ask)
}

fn question(name: &str) -> &'static Published {
    QUESTIONS.iter().find(|q| q.name == name).expect("a published question")
}

#[test]
fn each_question_is_published_at_its_grade_and_the_companion_agrees() {
    let published: Vec<_> = QUESTIONS.iter().map(|q| (q.name, q.grade)).collect();
    assert_eq!(
        published,
        [("delete_for_good", "sensitive"), ("send_message", "sensitive"), ("plan_work", "sensitive"), ("change_item", "standard")]
    );
    // The companion asks by the same names and grades, and draws each card as the shell does.
    for (q, ask) in QUESTIONS.iter().zip(Question::ALL) {
        assert_eq!((q.name, q.grade), (ask.action(), ask.grade()));
        assert!(ask.shown_whole() && crate::approval_whole::shown_whole("shell", q.name, q.grade, q.sentence), "{}: whole", q.name);
    }
}

#[test]
fn what_cannot_be_undone_says_so_and_the_other_two_do_not() {
    for name in ["delete_for_good", "send_message"] {
        assert!(gate::unrecoverable(question(name).sentence), "{name}");
    }
    for name in ["plan_work", "change_item"] {
        assert!(!gate::unrecoverable(question(name).sentence), "{name} claims what it should not");
    }
}

/// A question is a question: nothing it is asked changes anything, and it says so.
#[test]
fn every_handler_refuses() {
    let production = include_str!("control_questions.rs").split("#[cfg(test)]").next().unwrap();
    assert!(production.contains("surface.action(publish(question), move |_| Err(refusal(name)))"), "the one handler is a refusal");
    assert_eq!(production.matches("surface.action(").count(), 1, "no question has a handler of its own");
    for q in &QUESTIONS {
        let said = refusal(q.name);
        assert!(said.contains(q.name) && said.contains("Nothing was changed"), "{said}");
    }
}

#[test]
fn every_question_takes_the_whole_vocabulary_and_each_field_is_optional() {
    let fields = asks::fields();
    assert!(fields.iter().any(|(k, _)| k == "tool") && fields.iter().any(|(k, _)| k == "step_12"));
    let mut seen = std::collections::HashSet::new();
    assert!(fields.iter().all(|(k, _)| seen.insert(k.clone())), "no field is named twice");
    // The declaration really goes through the one helper: the action carries every field.
    for q in &QUESTIONS {
        let action = publish(q);
        assert_eq!((action.name.as_str(), action.permission, action.description.as_str()), (q.name, q.grade, q.sentence));
        let names: Vec<&str> = action.params.iter().map(|p| p.name.as_str()).collect();
        let expected: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, expected, "{} publishes the whole vocabulary", q.name);
        assert!(action.params.iter().all(|p| !p.required), "{}: every field is optional", q.name);
    }
}

#[test]
fn the_shell_asks_for_what_cannot_be_undone_in_ask_auto_and_bypass_and_refuses_in_plan() {
    for name in ["delete_for_good", "send_message"] {
        let q = question(name);
        for mode in [Mode::Ask, Mode::Auto, Mode::Bypass] {
            assert!(asked(&decide(q, mode)), "{name} in {mode:?}");
        }
        assert!(matches!(decide(q, Mode::Plan), Decision::Refuse { .. }), "{name} in plan");
        // The wire gate agrees, and only full bypass or a grant runs it.
        for mode in ["ask", "auto", "bypass", "plan"] {
            assert!(gate::decide(&authority(mode), "shell", name, q.grade, q.sentence).is_err(), "{name} on the wire in {mode}");
        }
        assert!(gate::decide(&authority("bypass_all"), "shell", name, q.grade, q.sentence).is_ok(), "{name} in full bypass");
        let granted = Authority { granted: true, ..authority("auto") };
        assert!(gate::decide(&granted, "shell", name, q.grade, q.sentence).is_ok(), "{name} with a grant");
    }
}

/// Planning is sensitive, and not something that "cannot be undone": `ask` shows a card, plan mode
/// refuses, and the modes above `ask` run it unasked (the shell's existing rule for the grade),
/// which the companion then writes down (`Permit::finish`) rather than the shell asking.
#[test]
fn planning_work_is_a_card_in_ask_and_a_refusal_in_plan() {
    let q = question("plan_work");
    assert!(asked(&decide(q, Mode::Ask)));
    assert!(matches!(decide(q, Mode::Plan), Decision::Refuse { .. }));
    assert!(!asked(&decide(q, Mode::Auto)), "auto runs `sensitive` unasked: only what cannot be undone is asked there");
    assert!(gate::decide(&authority("plan"), "shell", q.name, q.grade, q.sentence).is_err());
    assert!(gate::decide(&authority("ask"), "shell", q.name, q.grade, q.sentence).is_err(), "ask needs a grant");
}

/// `change_item` is `standard`, and is read as its grade says: plan mode refuses it, and every
/// other mode runs it with no card (`Modes::decide` asks from `sensitive` up, or for what cannot be
/// undone). The companion records the run; this is the grade the issue chose, kept as published.
#[test]
fn changing_an_item_is_read_as_the_standard_grade_it_is() {
    let q = question("change_item");
    assert_eq!(q.grade, "standard");
    assert!(matches!(decide(q, Mode::Plan), Decision::Refuse { .. }), "plan refuses it");
    for mode in [Mode::Ask, Mode::Auto, Mode::Bypass] {
        assert!(!asked(&decide(q, mode)), "{mode:?}: a standard action is below what the shell raises a card for");
    }
}

/// The mail send is unchanged beside them.
#[test]
fn send_mail_is_still_published_with_the_questions() {
    let line = include_str!("control.rs")
        .lines()
        .find(|l| l.contains("control_mail::actions("))
        .expect("the registration line");
    assert!(line.contains("control_questions::actions(crate::control_mail::actions("), "{line}");
}
