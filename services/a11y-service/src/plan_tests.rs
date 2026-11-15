//! The rules for what a signal needs, from signals and from what is known.

use super::*;

struct World {
    with_windows: &'static [&'static str],
    named: &'static [&'static str],
}

impl Known for World {
    fn has_window(&self, connection: &str) -> bool {
        self.with_windows.contains(&connection)
    }
    fn named(&self, connection: &str) -> bool {
        self.named.contains(&connection)
    }
}

const REGISTRY: &str = ":1.3";
const KNOWN: World = World { with_windows: &[":1.42"], named: &[":1.50"] };

fn on(connection: &str, n: u32) -> Key {
    (connection.to_string(), format!("/org/a11y/atspi/accessible/{n}"))
}

fn root(connection: &str) -> Key {
    (connection.to_string(), "/org/a11y/atspi/accessible/root".to_string())
}

#[test]
fn what_a_known_connection_announces_is_asked_about() {
    for connection in [":1.42", ":1.50"] {
        assert_eq!(plan(Raw::Created(on(connection, 7)), REGISTRY, &KNOWN), Step::Learn(on(connection, 7)), "{connection}");
        assert_eq!(plan(Raw::Focused(on(connection, 7)), REGISTRY, &KNOWN), Step::Focus(on(connection, 7)), "{connection}");
    }
    assert_eq!(plan(Raw::Activated(on(":1.42", 7)), REGISTRY, &KNOWN), Step::LearnAndActivate(on(":1.42", 7)));
}

/// #1257 review, N2: 25 connections that each sent one focus signal for an object that never
/// answers cost the watcher 25 limits.
#[test]
fn a_focus_from_a_connection_nobody_knows_is_not_asked_about() {
    assert_eq!(plan(Raw::Focused(on(":1.77", 7)), REGISTRY, &KNOWN), Step::Nothing);
}

/// #1257 review, N2: a stranger's announcement of a window is a reason to list, not to ask it.
#[test]
fn a_window_a_connection_nobody_knows_announces_wants_a_listing_not_a_question() {
    assert_eq!(plan(Raw::Created(on(":1.77", 7)), REGISTRY, &KNOWN), Step::Relist);
    assert_eq!(plan(Raw::Activated(on(":1.77", 7)), REGISTRY, &KNOWN), Step::RelistThenActivate(on(":1.77", 7)));
    let added = Raw::Children { source: root(":1.77"), added: true, child: Some(on(":1.77", 9)) };
    assert_eq!(plan(added, REGISTRY, &KNOWN), Step::Relist);
    let known_added = Raw::Children { source: root(":1.42"), added: true, child: Some(on(":1.42", 9)) };
    assert_eq!(plan(known_added, REGISTRY, &KNOWN), Step::Learn(on(":1.42", 9)));
}

#[test]
fn what_needs_no_question_is_told_to_the_model_as_it_is() {
    assert_eq!(plan(Raw::Destroyed(on(":1.77", 7)), REGISTRY, &KNOWN), Step::Apply(Fact::Closed(on(":1.77", 7))));
    assert_eq!(plan(Raw::Deactivated(on(":1.77", 7)), REGISTRY, &KNOWN), Step::Apply(Fact::Deactivated(on(":1.77", 7))));
    let forged_close = Raw::Children { source: root(":1.66"), added: false, child: Some(on(":1.42", 9)) };
    assert_eq!(plan(forged_close, REGISTRY, &KNOWN), Step::Nothing, "a window is closed only by the connection that owns it");
}

#[test]
fn an_application_that_left_wants_a_listing_only_if_the_model_had_a_window_of_it() {
    assert_eq!(plan(Raw::Vanished(":1.42".into()), REGISTRY, &KNOWN), Step::Relist);
    assert_eq!(plan(Raw::Vanished(":1.77".into()), REGISTRY, &KNOWN), Step::Nothing);
    let left = Raw::Children { source: root(REGISTRY), added: false, child: Some(root(":1.42")) };
    assert_eq!(plan(left, REGISTRY, &KNOWN), Step::Relist);
}

/// #1257 review, N1: a focus whose role could not be learned is told as one with an unknown
/// role, which the model does not tell text changes for.
#[test]
fn a_focus_whose_role_could_not_be_learned_is_told_as_unknown_not_dropped() {
    let source = on(":1.42", 41);
    let told = focus_fact(source.clone(), None, None);
    assert!(matches!(&told, Fact::Focus { role, name, source: s, .. } if role == UNKNOWN_ROLE && name.is_empty() && *s == source), "{told:?}");
    let learned = focus_fact(source, None, Some(("password text".into(), "Password".into())));
    assert!(matches!(&learned, Fact::Focus { role, .. } if role == "password text"));
}

/// #1257 review, N7: 24 hops of 17 ms ran out of their shared limit and marked a quick
/// application slow.
#[test]
fn a_long_climb_of_quick_answers_does_not_blame_the_application() {
    let limit = Duration::from_millis(400);
    assert!(!climb_blames_application(Duration::from_millis(17), limit));
    assert!(!climb_blames_application(Duration::from_millis(200), limit), "half the limit is not more than half");
    assert!(climb_blames_application(Duration::from_millis(201), limit));
}
