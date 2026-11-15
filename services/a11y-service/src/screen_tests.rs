//! The screen model's rules, each one stated as what a reader is told.

use super::*;

fn key(n: u32) -> Key {
    (":1.7".to_string(), format!("/org/a11y/atspi/accessible/{n}"))
}

fn window(n: u32, title: &str) -> WindowInfo {
    WindowInfo { id: format!("w{n}"), app: "gedit".into(), title: title.into(), role: "frame".into(), pid: 4242 }
}

fn touched(n: u32, what: What) -> Fact {
    Fact::Touched { window: Some(key(n)), app: "gedit".into(), what }
}

/// A screen with windows 1 and 2 open and window 1 in front.
fn two_windows(now: Instant) -> Screen {
    let mut screen = Screen::default();
    screen.apply(Fact::Listing(vec![(key(1), window(1, "notes.md")), (key(2), window(2, "todo.md"))]), now);
    screen.apply(Fact::Activated(key(1)), now);
    screen
}

#[test]
fn a_window_that_opens_is_told_once_with_its_handle() {
    let now = Instant::now();
    let mut screen = Screen::default();
    let told = screen.apply(Fact::Opened(key(1), window(1, "notes.md")), now);
    assert_eq!(told, [Kind::WindowOpened { window: window(1, "notes.md") }]);
    assert!(screen.apply(Fact::Opened(key(1), window(1, "notes.md")), now).is_empty(), "announced twice, told once");
    assert_eq!(screen.windows().count(), 1);
}

#[test]
fn a_window_that_closes_is_told_and_leaves_the_model() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    let told = screen.apply(Fact::Closed(key(2)), now);
    assert_eq!(told, [Kind::WindowClosed { window: window(2, "todo.md") }]);
    assert!(!screen.knows(&key(2)));
    assert!(screen.apply(Fact::Closed(key(2)), now).is_empty(), "a window that is not known closes silently");
}

#[test]
fn a_switch_between_two_windows_is_one_change_not_two() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    assert!(screen.apply(Fact::Deactivated(key(1)), now).is_empty(), "the loss waits for a taker");
    let told = screen.apply(Fact::Activated(key(2)), now + Duration::from_millis(5));
    assert_eq!(told, [Kind::FrontChanged { window: Some(window(2, "todo.md")) }]);
    assert!(screen.flush(now + FRONT_GRACE * 2).is_empty(), "nothing left to say about the front");
    assert_eq!(screen.front().map(|w| w.id.as_str()), Some("w2"));
}

#[test]
fn a_front_nobody_takes_is_told_as_none_after_the_grace() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(Fact::Deactivated(key(1)), now);
    assert!(screen.flush(now + FRONT_GRACE / 2).is_empty(), "still inside the grace");
    assert_eq!(screen.next_due(), Some(now + FRONT_GRACE));
    assert_eq!(screen.flush(now + FRONT_GRACE), [Kind::FrontChanged { window: None }]);
    assert!(screen.front().is_none());
}

#[test]
fn a_window_that_takes_the_front_back_inside_the_grace_is_not_news() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(Fact::Deactivated(key(1)), now);
    assert!(screen.apply(Fact::Activated(key(1)), now + Duration::from_millis(10)).is_empty());
    assert!(screen.flush(now + FRONT_GRACE * 2).is_empty());
    assert_eq!(screen.front().map(|w| w.id.as_str()), Some("w1"));
}

#[test]
fn closing_the_front_window_hands_the_front_to_whoever_takes_it() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(Fact::Closed(key(1)), now);
    assert!(screen.front().is_none(), "a closed window is not in front");
    let told = screen.apply(Fact::Activated(key(2)), now + Duration::from_millis(3));
    assert_eq!(told, [Kind::FrontChanged { window: Some(window(2, "todo.md")) }]);
}

#[test]
fn the_first_change_in_a_quiet_window_is_told_at_once() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    let told = screen.apply(touched(1, What::Text), now);
    assert_eq!(told, [Kind::ContentChanged { window: Some("w1".into()), app: "gedit".into(), what: vec!["text"], signals: 1 }]);
}

#[test]
fn a_burst_in_one_window_is_one_more_event_that_says_how_many() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    assert_eq!(screen.apply(touched(1, What::Text), now).len(), 1, "the first, at once");
    for n in 1..=500u64 {
        let what = if n % 2 == 0 { What::Text } else { What::Children };
        assert!(screen.apply(touched(1, what), now + Duration::from_micros(n * 100)).is_empty(), "held");
    }
    assert_eq!(screen.next_due(), Some(now + COALESCE));
    assert!(screen.flush(now + COALESCE / 2).is_empty(), "not due yet");
    let told = screen.flush(now + COALESCE);
    assert_eq!(told, [Kind::ContentChanged { window: Some("w1".into()), app: "gedit".into(), what: vec!["text", "children"], signals: 500 }]);
    assert_eq!(screen.counts().folded, 499);
    assert!(screen.next_due().is_none(), "nothing waiting");
}

#[test]
fn a_busy_window_does_not_delay_a_quiet_one() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(touched(1, What::Text), now);
    screen.apply(touched(1, What::Text), now + Duration::from_millis(1));
    let told = screen.apply(touched(2, What::Children), now + Duration::from_millis(2));
    assert_eq!(told.len(), 1, "window 2 has its own throttle");
    assert!(matches!(&told[0], Kind::ContentChanged { window: Some(id), .. } if id == "w2"));
}

#[test]
fn a_new_title_is_in_the_model_at_once_and_told_through_the_throttle() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    let told = screen.apply(Fact::Renamed(key(1), "notes.md (edited)".into()), now);
    assert_eq!(told, [Kind::WindowRenamed { window: window(1, "notes.md (edited)") }]);
    // A window that retitles itself every frame: the model follows, a reader gets one event.
    for n in 1..=30u64 {
        assert!(screen.apply(Fact::Renamed(key(1), format!("frame {n}")), now + Duration::from_millis(n)).is_empty());
    }
    assert_eq!(screen.windows().next().map(|w| w.title.as_str()), Some("frame 30"));
    assert_eq!(screen.flush(now + COALESCE), [Kind::WindowRenamed { window: window(1, "frame 30") }]);
    assert!(screen.apply(Fact::Renamed(key(1), "frame 30".into()), now + COALESCE * 3).is_empty(), "the same title is not news");
}

#[test]
fn focus_is_told_when_it_moves_and_names_its_window_by_handle() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    let fact = || Fact::Focus { window: Some(key(2)), role: "push button".into(), name: "Save".into() };
    let told = screen.apply(fact(), now);
    assert_eq!(told, [Kind::FocusMoved { window: Some("w2".into()), role: "push button".into(), name: "Save".into() }]);
    assert!(screen.apply(fact(), now).is_empty(), "the same focus announced again is not news");
    screen.apply(Fact::Closed(key(2)), now);
    assert!(screen.focus().is_none(), "focus does not outlive its window");
}

#[test]
fn a_listing_reconciles_the_model_and_tells_each_difference() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    let told = screen.apply(Fact::Listing(vec![(key(2), window(2, "todo.md (saved)")), (key(3), window(3, "new.md"))]), now);
    assert_eq!(
        told,
        [
            Kind::WindowClosed { window: window(1, "notes.md") },
            Kind::WindowRenamed { window: window(2, "todo.md (saved)") },
            Kind::WindowOpened { window: window(3, "new.md") },
        ]
    );
    assert!(screen.apply(Fact::Listing(vec![(key(2), window(2, "todo.md (saved)")), (key(3), window(3, "new.md"))]), now).is_empty(), "an equal listing is silent");
    assert_eq!(screen.counts().listings, 3);
}

#[test]
fn a_change_in_a_window_that_is_not_known_is_told_for_its_application() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    let told = screen.apply(Fact::Touched { window: Some(key(9)), app: "gedit".into(), what: What::Children }, now);
    assert_eq!(told, [Kind::ContentChanged { window: None, app: "gedit".into(), what: vec!["children"], signals: 1 }]);
}

#[test]
fn a_change_held_for_a_window_that_then_closes_is_dropped_not_misattributed() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(touched(1, What::Text), now);
    screen.apply(touched(1, What::Text), now + Duration::from_millis(1));
    screen.apply(Fact::Closed(key(1)), now + Duration::from_millis(2));
    let told = screen.flush(now + COALESCE * 2);
    assert_eq!(told, [Kind::FrontChanged { window: None }], "only the front it left behind");
}

#[test]
fn a_gap_is_told_so_a_reader_knows_to_read_the_whole_screen_again() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    assert_eq!(screen.apply(Fact::Gap, now), [Kind::Gap]);
    assert_eq!(screen.counts().gaps, 1);
}

#[test]
fn an_event_carries_no_text_and_serialises_flat() {
    let kind = Kind::ContentChanged { window: Some("w1".into()), app: "gedit".into(), what: vec!["text"], signals: 3 };
    let json = serde_json::to_value(&kind).expect("an event is plain data");
    assert_eq!(json, serde_json::json!({ "kind": "content_changed", "window": "w1", "app": "gedit", "what": ["text"], "signals": 3 }));
    assert_eq!(serde_json::to_value(Kind::Gap).expect("plain data"), serde_json::json!({ "kind": "gap" }));
}

#[test]
fn many_applications_touching_do_not_grow_the_throttle_table_without_bound() {
    let now = Instant::now();
    let mut screen = Screen::default();
    for n in 0..5_000u64 {
        screen.apply(Fact::Touched { window: None, app: format!("app{n}"), what: What::Text }, now + Duration::from_millis(n));
    }
    assert!(screen.throttles.len() <= MAX_THROTTLED, "{} throttles", screen.throttles.len());
}
