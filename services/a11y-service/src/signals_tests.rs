//! How signals are sorted.

use super::*;
use zbus::zvariant::Structure;

const WINDOW: &str = "org.a11y.atspi.Event.Window";
const OBJECT: &str = "org.a11y.atspi.Event.Object";

fn source() -> Key {
    (":1.42".to_string(), "/org/a11y/atspi/accessible/7".to_string())
}

fn root() -> Key {
    (":1.42".to_string(), "/org/a11y/atspi/accessible/root".to_string())
}

fn nothing() -> Value<'static> {
    Value::from(0i32)
}

fn child(path: &'static str) -> Value<'static> {
    let path = ObjectPath::try_from(path).expect("a valid path");
    Value::from(Structure::from((":1.42", path)))
}

#[test]
fn window_signals_are_sorted_by_what_happened_to_the_window() {
    for (member, expected) in [
        ("Create", Raw::Created(source())),
        ("Activate", Raw::Activated(source())),
        ("Deactivate", Raw::Deactivated(source())),
        ("Destroy", Raw::Destroyed(source())),
        ("Close", Raw::Destroyed(source())),
    ] {
        assert_eq!(sort(WINDOW, member, source(), "", 0, &nothing()), Some(expected), "{member}");
    }
    assert_eq!(sort(WINDOW, "Move", source(), "", 0, &nothing()), None, "a move is not a change the model keeps");
}

#[test]
fn only_gaining_the_focus_is_a_focus_change() {
    assert_eq!(sort(OBJECT, "StateChanged", source(), "focused", 1, &nothing()), Some(Raw::Focused(source())));
    assert_eq!(sort(OBJECT, "StateChanged", source(), "focused", 0, &nothing()), None, "losing it is not");
    assert_eq!(sort(OBJECT, "StateChanged", source(), "showing", 1, &nothing()), None, "another state is not");
}

#[test]
fn a_children_change_carries_the_child_it_is_about() {
    let added = sort(OBJECT, "ChildrenChanged", root(), "add", 0, &child("/org/a11y/atspi/accessible/9"));
    let expected = Some((":1.42".to_string(), "/org/a11y/atspi/accessible/9".to_string()));
    assert_eq!(added, Some(Raw::Children { source: root(), added: true, child: expected.clone() }));
    let removed = sort(OBJECT, "ChildrenChanged", root(), "remove/system", 0, &child("/org/a11y/atspi/accessible/9"));
    assert_eq!(removed, Some(Raw::Children { source: root(), added: false, child: expected }));
    let bare = sort(OBJECT, "ChildrenChanged", root(), "add", 0, &nothing());
    assert_eq!(bare, Some(Raw::Children { source: root(), added: true, child: None }), "no reference, still a change");
}

#[test]
fn a_reference_wrapped_in_a_variant_is_still_read() {
    let wrapped = Value::Value(Box::new(child("/org/a11y/atspi/accessible/9")));
    assert_eq!(reference_in(&wrapped), Some((":1.42".to_string(), "/org/a11y/atspi/accessible/9".to_string())));
    assert_eq!(reference_in(&Value::from("not a reference")), None);
}

/// The signal brings the typed text with it. The sorted signal has nowhere to put it.
#[test]
fn a_text_change_keeps_where_it_happened_and_none_of_the_text() {
    let typed = Value::from("hunter2 SENTINEL-1256");
    let sorted = sort(OBJECT, "TextChanged", source(), "insert", 0, &typed).expect("a text change");
    assert_eq!(sorted, Raw::Text(source()));
    assert!(!format!("{sorted:?}").contains("SENTINEL"), "{sorted:?}");
}

#[test]
fn a_new_name_is_read_from_the_signal_when_it_is_there() {
    let renamed = sort(OBJECT, "PropertyChange", source(), "accessible-name", 0, &Value::from("todo.md"));
    assert_eq!(renamed, Some(Raw::Name { source: source(), name: Some("todo.md".into()) }));
    let unnamed = sort(OBJECT, "PropertyChange", source(), "accessible-name", 0, &nothing());
    assert_eq!(unnamed, Some(Raw::Name { source: source(), name: None }), "the watcher asks for it instead");
    assert_eq!(sort(OBJECT, "PropertyChange", source(), "accessible-value", 0, &nothing()), None);
}

#[test]
fn the_root_is_told_from_an_ordinary_object_by_its_path() {
    assert!(is_root("/org/a11y/atspi/accessible/root"));
    assert!(!is_root("/org/a11y/atspi/accessible/7"));
    assert!(!is_root("/org/a11y/atspi/accessible/rootless"));
}
