//! The screen model's rules, each one stated as what a reader is told.

use super::*;

fn key(n: u32) -> Key {
    (":1.7".to_string(), format!("/org/a11y/atspi/accessible/{n}"))
}

fn window(n: u32, title: &str) -> WindowInfo {
    WindowInfo { id: format!("w{n}"), app: "gedit".into(), title: title.into(), role: "frame".into(), pid: 4242 }
}

fn listing(windows: Vec<(Key, WindowInfo)>) -> Fact {
    Fact::Listing { windows, unanswered: Vec::new() }
}

/// A change in element 900 + `n`, which is in window `n`.
fn touched(n: u32, what: What) -> Fact {
    Fact::Touched { source: key(900 + n), window: Some(key(n)), app: "gedit".into(), what }
}

fn focus_on(element: u32, in_window: u32, role: &str, name: &str) -> Fact {
    Fact::Focus { source: key(element), window: Some(key(in_window)), role: role.into(), name: name.into() }
}

fn text_changed(window: &str) -> Kind {
    Kind::ContentChanged { window: Some(window.into()), app: "gedit".into(), what: vec!["text"] }
}

/// A screen with windows 1 and 2 open and window 1 in front.
fn two_windows(now: Instant) -> Screen {
    let mut screen = Screen::default();
    screen.apply(listing(vec![(key(1), window(1, "notes.md")), (key(2), window(2, "todo.md"))]), now);
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
    assert!(screen.has_connection(":1.7"), "the application still has a window");
    screen.apply(Fact::Closed(key(1)), now);
    assert!(!screen.has_connection(":1.7") && screen.app_on(":1.7").is_none(), "and now it has none");
}

#[test]
fn windows_are_listed_in_the_order_they_were_learned() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(Fact::Opened(key(3), window(3, "new.md")), now);
    screen.apply(Fact::Closed(key(2)), now);
    let ids: Vec<&str> = screen.windows().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["w1", "w3"]);
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
    assert_eq!(screen.apply(touched(1, What::Text), now), [text_changed("w1")]);
}

#[test]
fn a_burst_in_one_window_is_one_more_event() {
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
    assert_eq!(told, [Kind::ContentChanged { window: Some("w1".into()), app: "gedit".into(), what: vec!["text", "children"] }]);
    assert!(screen.next_due().is_none(), "nothing waiting");
}

/// #1257 review, H5: one text-changed signal is one keystroke, so a count per event would be the
/// length of whatever was typed. Twelve keystrokes and two thousand read the same.
#[test]
fn a_burst_of_typing_is_not_counted_by_keystroke() {
    let typed = |keystrokes: u64| {
        let now = Instant::now();
        let mut screen = two_windows(now);
        let mut told = screen.apply(touched(1, What::Text), now);
        for n in 1..keystrokes {
            told.extend(screen.apply(touched(1, What::Text), now + Duration::from_micros(n * 50)));
        }
        told.extend(screen.flush(now + COALESCE));
        (serde_json::to_string(&told).expect("events are plain data"), serde_json::to_string(screen.counts()).expect("counts are plain data"))
    };
    let (twelve, twelve_counts) = typed(12);
    let (many, many_counts) = typed(2000);
    assert_eq!(twelve, many, "the events say the same whatever was typed");
    assert_eq!(twelve_counts, many_counts, "and so do the counters");
}

/// #1257 review, H5: neither that a password was typed nor when is told.
#[test]
fn text_changes_in_a_focused_password_field_are_not_told() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(focus_on(41, 1, "password text", "Password"), now);
    let typing = || Fact::Touched { source: key(41), window: Some(key(1)), app: "gedit".into(), what: What::Text };
    for n in 0..12u64 {
        assert!(screen.apply(typing(), now + Duration::from_millis(n * 90)).is_empty(), "keystroke {n}");
    }
    assert!(screen.flush(now + Duration::from_secs(5)).is_empty(), "and nothing was held to be told later");
    // Another field in the same window is told as ever, and so is the password field once the
    // focus is on something that is not one.
    assert_eq!(screen.apply(touched(1, What::Text), now + Duration::from_secs(6)), [text_changed("w1")]);
    screen.apply(focus_on(42, 1, "text", "Search"), now + Duration::from_secs(7));
    assert_eq!(screen.apply(typing(), now + Duration::from_secs(8)), [text_changed("w1")]);
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
    let told = screen.apply(focus_on(40, 2, "push button", "Save"), now);
    assert_eq!(told, [Kind::FocusMoved { window: Some("w2".into()), role: "push button".into(), name: "Save".into() }]);
    assert!(screen.apply(focus_on(40, 2, "push button", "Save"), now).is_empty(), "the same focus announced again is not news");
    screen.apply(Fact::Closed(key(2)), now);
    assert!(screen.focus().is_none(), "focus does not outlive its window");
}

/// #1257 review, L4: focus in an element whose window was never found used to stay in the model
/// after its application had gone.
#[test]
fn focus_does_not_outlive_an_application_with_no_window_left() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(Fact::Focus { source: key(40), window: None, role: "menu item".into(), name: "Quit".into() }, now);
    screen.apply(Fact::Closed(key(1)), now);
    assert!(screen.focus().is_some(), "the application still has a window");
    screen.apply(Fact::Closed(key(2)), now);
    assert!(screen.focus().is_none(), "its last window closed, so it holds no focus");
}

#[test]
fn a_listing_reconciles_the_model_and_tells_each_difference() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    let after = || listing(vec![(key(2), window(2, "todo.md (saved)")), (key(3), window(3, "new.md"))]);
    let told = screen.apply(after(), now);
    assert_eq!(
        told,
        [
            Kind::WindowClosed { window: window(1, "notes.md") },
            Kind::WindowRenamed { window: window(2, "todo.md (saved)") },
            Kind::WindowOpened { window: window(3, "new.md") },
        ]
    );
    assert!(screen.apply(after(), now).is_empty(), "an equal listing is silent");
    assert_eq!(screen.counts().listings, 3);
}

/// #1257 review, H3: an application that did not answer the listing in time has not closed its
/// windows, and a reader must not be told that it has.
#[test]
fn an_application_that_did_not_answer_keeps_its_windows_through_a_listing() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    let other: Key = (":1.9".to_string(), "/org/a11y/atspi/accessible/1".to_string());
    let calc = WindowInfo { id: "w50".into(), app: "calc".into(), title: "Calculator".into(), role: "frame".into(), pid: 9 };
    let told = screen.apply(Fact::Listing { windows: vec![(other, calc.clone())], unanswered: vec![":1.7".into()] }, now);
    assert_eq!(told, [Kind::WindowOpened { window: calc }], "nothing of the frozen application is told as closed");
    assert!(screen.knows(&key(1)) && screen.knows(&key(2)));
}

#[test]
fn a_change_in_a_window_that_is_not_known_is_told_for_its_application() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    let told = screen.apply(Fact::Touched { source: key(77), window: Some(key(9)), app: "gedit".into(), what: What::Children }, now);
    assert_eq!(told, [Kind::ContentChanged { window: None, app: "gedit".into(), what: vec!["children"] }]);
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
fn an_event_carries_no_text_and_no_count_and_serialises_flat() {
    let json = serde_json::to_value(text_changed("w1")).expect("an event is plain data");
    assert_eq!(json, serde_json::json!({ "kind": "content_changed", "window": "w1", "app": "gedit", "what": ["text"] }));
    assert_eq!(serde_json::to_value(Kind::Gap).expect("plain data"), serde_json::json!({ "kind": "gap" }));
}

/// #1257 review, H4: any program can announce any number of windows.
#[test]
fn a_connection_cannot_open_more_than_its_share_of_windows() {
    let now = Instant::now();
    let mut screen = Screen::default();
    for n in 0..100_000u32 {
        screen.apply(Fact::Opened(key(n), window(n, "spam")), now);
    }
    assert_eq!(screen.windows().count(), MAX_PER_CONNECTION);
    assert_eq!(screen.counts().refused_windows, 100_000 - MAX_PER_CONNECTION as u64);
    let other: Key = (":1.9".to_string(), "/org/a11y/atspi/accessible/1".to_string());
    let told = screen.apply(Fact::Opened(other, window(1, "calc")), now);
    assert_eq!(told.len(), 1, "another application still has its share");
}

#[test]
fn the_model_keeps_no_more_than_its_cap_in_all() {
    let now = Instant::now();
    let mut screen = Screen::default();
    for connection in 0..200u32 {
        for n in 0..10u32 {
            let key: Key = (format!(":1.{connection}"), format!("/org/a11y/atspi/accessible/{n}"));
            screen.apply(Fact::Opened(key, window(connection * 10 + n, "many")), now);
        }
    }
    assert_eq!(screen.windows().count(), MAX_WINDOWS);
    assert_eq!(screen.counts().refused_windows, 2000 - MAX_WINDOWS as u64);
}

/// #1257 review, M1(a): a change held in a throttle when the person went private used to be
/// told by the first flush after it ended.
#[test]
fn a_change_held_when_private_begins_is_never_told() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(touched(1, What::Text), now);
    screen.apply(touched(1, What::Text), now + Duration::from_millis(5));
    screen.apply(Fact::Renamed(key(2), "private.md".into()), now + Duration::from_millis(6));
    screen.apply(Fact::Renamed(key(2), "private-2.md".into()), now + Duration::from_millis(7));
    screen.apply(Fact::Deactivated(key(1)), now + Duration::from_millis(8));
    screen.private_began();
    assert!(screen.next_due().is_none(), "nothing is waiting");
    assert!(screen.flush(now + Duration::from_secs(60)).is_empty(), "and nothing is told, however long after");
}

/// #1257 review, M1(b): the first `a11y.now` after private ended used to name the last element
/// focused while private.
#[test]
fn focus_from_the_private_time_is_not_answered_after_it() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.apply(focus_on(40, 1, "text", "Search"), now);
    screen.private_began();
    assert!(screen.focus().is_none(), "the focus is not kept into the private time");
    screen.apply_quietly(focus_on(41, 1, "list item", "Re: the offer"), now);
    screen.apply_quietly(touched(1, What::Text), now);
    assert!(screen.focus().is_none());
    screen.private_ended();
    assert!(screen.focus().is_none(), "nor out of it");
    assert!(screen.flush(now + Duration::from_secs(60)).is_empty(), "and the typing was never held");
}

/// #1257 review, M1(c): a reader that follows the stream had no way to know its picture was
/// from before the private time.
#[test]
fn leaving_private_mode_tells_a_gap() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.private_began();
    assert_eq!(screen.private_ended(), [Kind::Gap]);
    assert_eq!(screen.counts().gaps, 1);
}

#[test]
fn while_private_the_model_still_follows_which_windows_exist() {
    let now = Instant::now();
    let mut screen = two_windows(now);
    screen.private_began();
    screen.apply_quietly(Fact::Opened(key(3), window(3, "bank.html")), now);
    screen.apply_quietly(Fact::Closed(key(2)), now);
    screen.apply_quietly(Fact::Renamed(key(1), "notes.md (edited)".into()), now);
    screen.apply_quietly(Fact::Activated(key(3)), now);
    screen.private_ended();
    let titles: Vec<&str> = screen.windows().map(|w| w.title.as_str()).collect();
    assert_eq!(titles, ["notes.md (edited)", "bank.html"], "what is open now, as a listing would say");
    assert_eq!(screen.front().map(|w| w.id.as_str()), Some("w3"));
    assert_eq!(screen.counts().told, 4, "the two windows and the front before it, and the gap: nothing from the private time");
}

#[test]
fn many_applications_touching_do_not_grow_the_throttle_table_without_bound() {
    let now = Instant::now();
    let mut screen = Screen::default();
    for n in 0..5_000u64 {
        screen.apply(Fact::Touched { source: key(1), window: None, app: format!("app{n}"), what: What::Text }, now + Duration::from_millis(n));
    }
    assert!(screen.throttles.len() <= MAX_THROTTLED, "{} throttles", screen.throttles.len());
}
