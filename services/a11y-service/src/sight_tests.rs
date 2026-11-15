//! What reaches the ring, and what a reader gets back.

use super::*;
use crate::screen::WindowInfo;
use zbus::zvariant::ObjectPath;

fn window(id: &str) -> WindowInfo {
    WindowInfo { app: "gedit".into(), title: "notes.md".into(), role: "frame".into(), pid: 7, id: id.into() }
}

#[test]
fn what_the_model_says_reaches_the_ring_in_order_with_a_cursor() {
    let sight = Sight::idle();
    let handles = handles::Shared::default();
    sight.record(&handles, false, vec![Kind::WindowOpened { window: window("w1") }, Kind::FrontChanged { window: Some(window("w1")) }]);
    let page = sight.since(0, Duration::ZERO);
    assert_eq!(page["next_seq"], 2);
    assert_eq!(page["missed"], 0);
    assert_eq!(page["events"][0]["kind"], "window_opened");
    assert_eq!(page["events"][0]["window"]["id"], "w1");
    assert_eq!(page["events"][1]["kind"], "front_changed");
    assert_eq!(page["events"][1]["seq"], 1);
    assert!(sight.since(2, Duration::ZERO)["events"].as_array().is_some_and(Vec::is_empty), "nothing after the cursor");
}

/// Nothing the person did while private is in the ring afterwards, and it is counted.
#[test]
fn while_the_person_is_private_nothing_is_recorded_and_the_drop_is_counted() {
    let sight = Sight::idle();
    let handles = handles::Shared::default();
    sight.record(&handles, true, vec![Kind::WindowOpened { window: window("w1") }, Kind::Gap]);
    assert!(sight.since(0, Duration::ZERO)["events"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(sight.now()["counts"]["dropped_private"], 2);
    sight.record(&handles, false, vec![Kind::Gap]);
    assert_eq!(sight.since(0, Duration::ZERO)["events"].as_array().map(Vec::len), Some(1), "recording resumes");
}

#[test]
fn a_reader_far_behind_gets_a_page_and_the_cursor_for_the_next() {
    let sight = Sight::idle();
    let handles = handles::Shared::default();
    sight.record(&handles, false, (0..MAX_PAGE + 10).map(|_| Kind::Gap).collect());
    let first = sight.since(0, Duration::ZERO);
    assert_eq!(first["events"].as_array().map(Vec::len), Some(MAX_PAGE));
    assert_eq!(first["more"], true);
    assert_eq!(first["next_seq"], MAX_PAGE as u64);
    let rest = sight.since(MAX_PAGE as u64, Duration::ZERO);
    assert_eq!(rest["events"].as_array().map(Vec::len), Some(10));
    assert_eq!(rest["more"], false);
}

/// A reader whose cursor is from a service that has since restarted is told, not left waiting.
#[test]
fn a_reader_with_a_cursor_from_another_life_is_told_to_start_again() {
    let sight = Sight::idle();
    let page = sight.since(5_000, Duration::ZERO);
    assert_eq!(page["reset"], true);
    assert_eq!(page["next_seq"], 0);
    assert_eq!(sight.since(0, Duration::ZERO)["reset"], false);
}

#[test]
fn a_window_keeps_its_handle_while_open_and_loses_it_when_it_closes() {
    let sight = Sight::idle();
    let handles = handles::Shared::default();
    let path = ObjectPath::try_from("/org/a11y/atspi/accessible/7").expect("a valid path");
    let id = handles::hand_out(&handles, (":1.42".to_string(), path.into()));
    sight.record(&handles, true, vec![Kind::WindowOpened { window: window(&id) }]);
    assert!(handles::resolve(&handles, &id).is_some(), "pinned, private or not");
    sight.record(&handles, true, vec![Kind::WindowClosed { window: window(&id) }]);
    assert!(handles::resolve(&handles, &id).is_none(), "a closed window's handle resolves to nothing");
}

#[test]
fn a_model_nothing_feeds_says_it_is_not_listening() {
    let now = Sight::idle().now();
    assert_eq!(now["listening"], false);
    assert_eq!(now["seq"], 0);
    assert!(now["front"].is_null() && now["focus"].is_null());
    assert!(now["windows"].as_array().is_some_and(Vec::is_empty));
}

/// #1257 review, H5: a counter that moved once per signal would say how much was typed to
/// anyone who read `a11y.now` twice.
#[test]
fn no_counter_a_reader_can_see_counts_signals() {
    let sorted_keys = |value: &serde_json::Value| {
        let mut keys: Vec<String> = value.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
        keys.sort();
        keys
    };
    let counts = Sight::idle().now()["counts"].clone();
    assert_eq!(sorted_keys(&counts), ["dropped_private", "model", "overwritten", "signals_dropped"]);
    assert_eq!(sorted_keys(&counts["model"]), ["gaps", "listings", "refused_windows", "told"]);
}
