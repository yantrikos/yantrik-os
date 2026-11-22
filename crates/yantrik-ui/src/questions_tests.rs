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
        [("delete_for_good", "sensitive"), ("send_chat_message", "sensitive"), ("plan_work", "sensitive"), ("change_item", "standard")]
    );
    // The companion asks by the same names and grades, and draws each card as the shell does.
    for (q, ask) in QUESTIONS.iter().zip(Question::ALL) {
        assert_eq!((q.name, q.grade), (ask.action(), ask.grade()));
        assert!(ask.shown_whole() && crate::approval_whole::shown_whole("shell", q.name, q.grade, q.sentence), "{}: whole", q.name);
    }
}

#[test]
fn what_cannot_be_undone_says_so_and_the_other_two_do_not() {
    for name in ["delete_for_good", "send_chat_message"] {
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

/// Round 4 of #1313, item 1: the effect is approved, not the route to it. The four questions
/// draw a card in EVERY mode (full bypass included), and no session rule and no standing rule
/// answers one; plan mode still refuses. Nothing else about which mode asks moves.
fn in_mode(mode: Mode, now: Instant) -> Modes {
    let mut modes = Modes::new(Mode::Ask);
    if mode != Mode::Ask {
        modes.person_set_mode(mode, crate::mind_mode::Bypass::Hour, now, 0);
    }
    modes
}

fn decide_at(modes: &Modes, q: &Published, now: Instant) -> Decision {
    let cannot_be_undone = gate::unrecoverable(q.sentence);
    modes.decide(q.grade, "shell", q.name, cannot_be_undone, "dangerous", now)
}

#[test]
fn every_question_draws_a_card_in_every_mode_and_plan_refuses() {
    let now = Instant::now();
    for q in &QUESTIONS {
        for mode in [Mode::Ask, Mode::Auto, Mode::Bypass] {
            assert!(asked(&decide_at(&in_mode(mode, now), q, now)), "{} in {mode:?}", q.name);
        }
        // Full bypass asks nothing (#1353, decision D): it runs, and the shell audits it.
        assert!(!asked(&decide_at(&in_mode(Mode::BypassAll, now), q, now)), "{} in full bypass", q.name);
        assert!(matches!(decide(q, Mode::Plan), Decision::Refuse { .. }), "{} in plan", q.name);
        // The wire gate agrees: ungranted is refused in every mode but full bypass, a person's grant runs it.
        for mode in ["plan", "ask", "auto", "bypass"] {
            assert!(gate::decide(&authority(mode), "shell", q.name, q.grade, q.sentence).is_err(), "{} on the wire in {mode}", q.name);
        }
        assert!(gate::decide(&authority("bypass_all"), "shell", q.name, q.grade, q.sentence).is_ok(), "{} on the wire in full bypass", q.name);
        let granted = Authority { granted: true, ..authority("bypass_all") };
        assert!(gate::decide(&granted, "shell", q.name, q.grade, q.sentence).is_ok(), "{} with a grant", q.name);
    }
}

#[test]
fn no_session_rule_and_no_standing_rule_answers_a_question() {
    let now = Instant::now();
    for q in &QUESTIONS {
        // A rule cannot even be stored for it...
        let mut modes = in_mode(Mode::Auto, now);
        let stored = modes.person_add_rule_at("shell", q.name, q.grade, "Do it.", now);
        assert!(stored.unwrap_err().contains("four questions"), "{}", q.name);
        // ...a standing file naming it is refused whole, as for a bad line...
        let file = format!("shell agent_run\nshell {}\n", q.name);
        assert!(crate::mind_mode::parse_standing_rules(&file).unwrap_err().contains("four questions"), "{}", q.name);
        // ...and a rule that is somehow in force (a gate over one of the stored ones) answers nothing.
        for mode in [Mode::Ask, Mode::Auto, Mode::Bypass] {
            let modes = in_mode(mode, now);
            assert!(asked(&decide_at(&modes, q, now)), "{} in {mode:?}", q.name);
        }
        let mut auto = authority("auto");
        auto.mode.session_rules.push(("shell".into(), q.name.into()));
        auto.mode.standing_rules.push(("shell".into(), q.name.into()));
        assert!(gate::decide(&auto, "shell", q.name, q.grade, q.sentence).is_err(), "{}: the wire gate ignores a rule", q.name);
    }
    // Another app's action of the same name is not one of them, and a rule for it is as before.
    let mut modes = in_mode(Mode::Auto, now);
    assert!(modes.person_add_rule_at("notes", "plan_work", "sensitive", "Make a note.", now).is_ok());
}

/// The four and `ALWAYS_ASKED` agree, so a fifth question cannot be published without being asked.
#[test]
fn the_always_asked_list_holds_exactly_the_four_questions_and_the_three_consents() {
    use yantrik_ipc_transport::always_asked::{always_asked, ALWAYS_ASKED};
    for q in &QUESTIONS {
        assert!(always_asked("shell", q.name), "{}", q.name);
    }
    assert_eq!(ALWAYS_ASKED.len(), QUESTIONS.len() + 4, "plus the three consents and send_mail");
    assert!(ALWAYS_ASKED.iter().all(|(app, action)| *app == "shell" && (crate::approval_whole::is_question(app, action) || ["allow_location_lookup", "allow_background_research", "report_problem", "send_mail"].contains(action))));
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
