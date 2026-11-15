//! How signals are sorted, from real bus messages.

use super::*;
use zbus::zvariant::{DynamicType, Structure};

type NoProperties = HashMap<&'static str, Value<'static>>;

fn source() -> Key {
    (":1.42".to_string(), "/org/a11y/atspi/accessible/7".to_string())
}

fn root_of(connection: &str) -> Key {
    (connection.to_string(), "/org/a11y/atspi/accessible/root".to_string())
}

fn nothing() -> Value<'static> {
    Value::from(0i32)
}

fn child(connection: &'static str, path: &'static str) -> Value<'static> {
    let path = ObjectPath::try_from(path).expect("a valid path");
    Value::from(Structure::from((connection, path)))
}

/// A signal as it comes off the bus, from `sender`, on the object every test uses.
fn signal<B: serde::Serialize + DynamicType>(sender: &str, interface: &str, member: &str, body: &B) -> zbus::Message {
    zbus::Message::signal("/org/a11y/atspi/accessible/7", interface, member)
        .and_then(|builder| builder.sender(sender))
        .and_then(|builder| builder.build(body))
        .expect("a well-formed signal")
}

/// An object event in the shape GTK, Chromium and Slint send.
fn event(member: &str, detail: &str, detail1: i32, any: Value<'static>) -> zbus::Message {
    signal(":1.42", OBJECT_EVENTS, member, &(detail, detail1, 0i32, any, NoProperties::new()))
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
        assert_eq!(parse(&signal(":1.42", WINDOW_EVENTS, member, &())), Some(expected), "{member}");
    }
    assert_eq!(parse(&signal(":1.42", WINDOW_EVENTS, "Move", &())), None, "a move is not a change the model keeps");
}

#[test]
fn only_gaining_the_focus_is_a_focus_change() {
    assert_eq!(parse(&event("StateChanged", "focused", 1, nothing())), Some(Raw::Focused(source())));
    assert_eq!(parse(&event("StateChanged", "focused", 0, nothing())), None, "losing it is not");
    assert_eq!(parse(&event("StateChanged", "showing", 1, nothing())), None, "another state is not");
}

/// Qt still sends the older shape, with a reference where the others send properties.
#[test]
fn the_older_signal_shape_is_read_too() {
    let application = ObjectPath::try_from("/org/a11y/atspi/accessible/root").expect("a valid path");
    let older = signal(":1.42", OBJECT_EVENTS, "StateChanged", &("focused", 1i32, 0i32, nothing(), (":1.42", application)));
    assert_eq!(parse(&older), Some(Raw::Focused(source())));
}

#[test]
fn a_children_change_carries_the_child_it_is_about() {
    let nine = Some((":1.42".to_string(), "/org/a11y/atspi/accessible/9".to_string()));
    let added = parse(&event("ChildrenChanged", "add", 0, child(":1.42", "/org/a11y/atspi/accessible/9")));
    assert_eq!(added, Some(Raw::Children { source: source(), added: true, child: nine.clone() }));
    let removed = parse(&event("ChildrenChanged", "remove/system", 0, child(":1.42", "/org/a11y/atspi/accessible/9")));
    assert_eq!(removed, Some(Raw::Children { source: source(), added: false, child: nine }));
    let bare = parse(&event("ChildrenChanged", "add", 0, nothing()));
    assert_eq!(bare, Some(Raw::Children { source: source(), added: true, child: None }), "no reference, still a change");
}

#[test]
fn a_reference_wrapped_in_a_variant_is_still_read() {
    let wrapped = Value::Value(Box::new(child(":1.42", "/org/a11y/atspi/accessible/9")));
    assert_eq!(reference_in(&wrapped), Some((":1.42".to_string(), "/org/a11y/atspi/accessible/9".to_string())));
    assert_eq!(reference_in(&Value::from("not a reference")), None);
}

/// #1257 review, M5: the body of a text change is what was typed. This one's body could not be
/// read as an event if anything tried, and it is sorted all the same.
#[test]
fn a_text_change_is_sorted_without_reading_its_body() {
    let unreadable = signal(":1.42", OBJECT_EVENTS, "TextChanged", &("hunter2 SENTINEL-1256",));
    let sorted = parse(&unreadable).expect("sorted by its name alone");
    assert_eq!(sorted, Raw::Text(source()));
    assert!(!format!("{sorted:?}").contains("SENTINEL"), "{sorted:?}");
    // The same body under a member whose body *is* read is refused, which shows the body above
    // was never looked at.
    assert_eq!(parse(&signal(":1.42", OBJECT_EVENTS, "StateChanged", &("hunter2 SENTINEL-1256",))), None);
}

#[test]
fn a_new_name_is_read_from_the_signal_when_it_is_there() {
    let renamed = parse(&event("PropertyChange", "accessible-name", 0, Value::from("todo.md")));
    assert_eq!(renamed, Some(Raw::Name { source: source(), name: Some("todo.md".into()) }));
    let unnamed = parse(&event("PropertyChange", "accessible-name", 0, nothing()));
    assert_eq!(unnamed, Some(Raw::Name { source: source(), name: None }), "the watcher asks for it instead");
    assert_eq!(parse(&event("PropertyChange", "accessible-value", 0, nothing())), None);
}

/// #1257 review, H4: a name is copied into the model and into every event that carries it.
#[test]
fn a_name_of_a_megabyte_is_cut_as_it_is_read() {
    let huge = "t".repeat(1 << 20);
    let Some(Raw::Name { name: Some(name), .. }) = parse(&event("PropertyChange", "accessible-name", 0, Value::from(huge))) else {
        panic!("a name change");
    };
    assert_eq!(name.len(), limits::NAME_MAX);
}

/// #1257 review, H2: anyone on the bus can send these words. Only the bus is believed.
#[test]
fn only_the_bus_is_believed_about_who_left() {
    let gone = (":1.42", ":1.42", "");
    assert_eq!(parse(&signal(BUS, BUS, "NameOwnerChanged", &gone)), Some(Raw::Vanished(":1.42".into())));
    assert_eq!(parse(&signal(":1.66", BUS, "NameOwnerChanged", &gone)), None, "the same words from an ordinary connection");
    assert_eq!(parse(&signal(BUS, BUS, "NameOwnerChanged", &(":1.42", "", ":1.42"))), None, "arriving is not leaving");
    assert_eq!(parse(&signal(BUS, BUS, "NameAcquired", &(":1.42",))), None);
}

/// #1257 review, M3: a registry that restarted has forgotten what it was asked to announce.
#[test]
fn a_new_owner_of_the_registry_is_noticed() {
    assert_eq!(parse(&signal(BUS, BUS, "NameOwnerChanged", &(REGISTRY, ":1.3", ":1.90"))), Some(Raw::RegistryRestarted));
    assert_eq!(parse(&signal(BUS, BUS, "NameOwnerChanged", &(REGISTRY, ":1.3", ""))), None, "gone is not back yet");
    assert_eq!(parse(&signal(":1.66", BUS, "NameOwnerChanged", &(REGISTRY, ":1.3", ":1.66"))), None, "nor is a claim to be it");
}

#[test]
fn a_message_that_is_not_a_signal_is_not_sorted() {
    let call = zbus::Message::method_call("/org/a11y/atspi/accessible/7", "Activate")
        .and_then(|builder| builder.interface(WINDOW_EVENTS))
        .and_then(|builder| builder.sender(":1.42"))
        .and_then(|builder| builder.build(&()))
        .expect("a well-formed call");
    assert_eq!(parse(&call), None);
}

#[test]
fn the_root_is_told_from_an_ordinary_object_by_its_path() {
    assert!(is_root("/org/a11y/atspi/accessible/root"));
    assert!(!is_root("/org/a11y/atspi/accessible/7"));
    assert!(!is_root("/org/a11y/atspi/accessible/rootless"));
}

const REGISTRY_NOW: &str = ":1.3";

#[test]
fn that_an_application_left_is_believed_only_from_the_registry() {
    let application = Some(root_of(":1.42"));
    assert_eq!(root_change(&root_of(REGISTRY_NOW), false, application.clone(), REGISTRY_NOW), RootChange::Relist);
    assert_eq!(root_change(&root_of(":1.66"), false, application.clone(), REGISTRY_NOW), RootChange::Ignore, "an application saying another left");
    assert_eq!(root_change(&root_of(REGISTRY_NOW), true, application, REGISTRY_NOW), RootChange::Ignore, "one that came announces its own windows");
    assert_eq!(root_change(&root_of(REGISTRY_NOW), false, None, REGISTRY_NOW), RootChange::Relist, "the registry, naming nobody");
    assert_eq!(root_change(&root_of(":1.66"), false, None, REGISTRY_NOW), RootChange::Ignore);
}

/// #1257 review, M2: one application could close another's window in the model with one signal.
#[test]
fn a_window_is_opened_or_closed_only_by_the_connection_that_owns_it() {
    let own = (":1.42".to_string(), "/org/a11y/atspi/accessible/9".to_string());
    assert_eq!(root_change(&root_of(":1.42"), true, Some(own.clone()), REGISTRY_NOW), RootChange::Learn(own.clone()));
    assert_eq!(root_change(&root_of(":1.42"), false, Some(own.clone()), REGISTRY_NOW), RootChange::Closed(own.clone()));
    assert_eq!(root_change(&root_of(":1.66"), false, Some(own.clone()), REGISTRY_NOW), RootChange::Ignore, "a forged close");
    assert_eq!(root_change(&root_of(":1.66"), true, Some(own), REGISTRY_NOW), RootChange::Ignore, "a forged open");
}
