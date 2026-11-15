//! What reaches the ring, and what a reader gets back.

use super::*;
use crate::screen::WindowInfo;

fn window(id: &str) -> WindowInfo {
    WindowInfo { app: "gedit".into(), title: "notes.md".into(), role: "frame".into(), pid: 7, id: id.into() }
}

#[test]
fn climbs_are_rationed_by_the_second() {
    let start = Instant::now();
    let mut walks = Walks { left: WALKS_PER_SECOND, refills_at: start + Duration::from_secs(1) };
    let allowed = (0..WALKS_PER_SECOND * 3).filter(|_| walks.take(start)).count();
    assert_eq!(allowed as u32, WALKS_PER_SECOND);
    assert!(walks.take(start + Duration::from_secs(1)), "a new second, a new ration");
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
