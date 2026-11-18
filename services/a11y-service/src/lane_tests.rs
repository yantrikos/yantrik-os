//! What typing is told as, and what is never told, and whose windows a full model closes (#1257
//! review, M2 and M3).

use super::tests::{focus_on, key, text_changed, touched, two_windows};
use super::*;
use crate::throttle::TEXT_COALESCE;
use crate::typing::UNKNOWN_ROLE;

fn from(connection: &str, element: u32, in_window: u32, what: What) -> Fact {
    Fact::Touched { source: (connection.to_string(), format!("/org/a11y/atspi/accessible/{element}")), window: Some(key(in_window)), app: "gedit".into(), what }
}

// ---- M2: when typing is told ----

/// #1257 review, M2 (RX1 inverted): a reader woken on `a11y.since` noted the clock, and got the
/// first keystroke of each interval to the millisecond. Text falls due on a grid.
#[test]
fn first_keys_inside_one_step_fall_due_at_the_same_instant() {
    let t0 = Instant::now();
    let (mut a, mut b) = (two_windows(t0), two_windows(t0));
    a.apply(touched(1, What::Text), t0 + Duration::from_millis(300));
    b.apply(touched(1, What::Text), t0 + Duration::from_millis(700));
    assert_eq!(a.next_due(), b.next_due(), "400 ms apart, due together");
    assert_eq!(a.next_due(), Some(t0 + TEXT_COALESCE * 2), "on the grid, at least a step after the key");
}

#[test]
fn every_windows_text_falls_due_at_the_same_instant() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    screen.apply(touched(1, What::Text), t0 + Duration::from_millis(100));
    screen.apply(touched(2, What::Text), t0 + Duration::from_millis(1900));
    assert!(screen.flush(t0 + TEXT_COALESCE * 2 - Duration::from_millis(1)).is_empty(), "not before the instant");
    let told = screen.flush(t0 + TEXT_COALESCE * 2);
    assert_eq!(told.len(), 2, "both, at once: {told:?}");
    assert!(screen.next_due().is_none());
}

#[test]
fn the_grid_does_not_move_with_the_typing() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    let mut due = Vec::new();
    for second in [0u64, 5, 11] {
        screen.apply(touched(1, What::Text), t0 + Duration::from_millis(second * 1000 + 777));
        due.push(screen.next_due().map(|at| at.duration_since(t0).as_millis() % 2000));
        screen.flush(t0 + Duration::from_secs(second + 4));
    }
    assert_eq!(due, [Some(0), Some(0), Some(0)], "always a multiple of the step from the start");
}

/// #1257 review, M2 (b): rich editors announce a keystroke as children being added, which went
/// through the quick lane, key by key, unrounded.
#[test]
fn children_changes_from_a_focused_editor_take_the_text_lane() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    screen.apply(focus_on(40, 1, "paragraph", "Message"), t0);
    for n in 0..20u64 {
        let at = t0 + Duration::from_millis(n * 150);
        assert!(screen.apply(touched(1, What::Children), at).is_empty(), "key {n}: not told at once");
        assert!(screen.apply(touched(1, What::Name), at).is_empty());
        screen.flush(at);
    }
    let told = screen.flush(t0 + Duration::from_secs(10));
    assert!(told.len() <= 4, "{told:?}");
    assert!(told.iter().all(|kind| matches!(kind, Kind::ContentChanged { what, .. } if what.as_slice() == ["text"])), "told as text: {told:?}");
}

#[test]
fn children_changes_from_an_application_with_nothing_to_type_in_are_still_quick() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    screen.apply(focus_on(40, 1, "push button", "Save"), t0);
    assert_eq!(screen.apply(touched(1, What::Children), t0).len(), 1, "told at once");
    screen.apply(focus_on(41, 1, "text", "Search"), t0);
    let elsewhere = from(":1.9", 5, 1, What::Children);
    assert_eq!(screen.apply(elsewhere, t0 + Duration::from_secs(1)).len(), 1, "an editor is focused in another application: not this one's typing");
}

/// #1257 review, M2 (RX2 inverted): a text change from a child of the focused field was told.
#[test]
fn text_from_inside_a_focused_password_field_is_not_told() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    screen.apply(focus_on(50, 1, "password text", "Password"), t0);
    for n in 0..12u64 {
        screen.apply(from(":1.7", 51, 1, What::Text), t0 + Duration::from_millis(n * 300));
    }
    assert!(screen.flush(t0 + Duration::from_secs(10)).is_empty(), "nothing told, nothing held");
    assert!(screen.next_due().is_none());
}

#[test]
fn children_and_names_from_a_focused_password_field_are_not_told_either() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    screen.apply(focus_on(50, 1, "password text", "Password"), t0);
    for (n, what) in [What::Children, What::Name, What::Children, What::Name].into_iter().enumerate() {
        assert!(screen.apply(from(":1.7", 50 + n as u32, 1, what), t0 + Duration::from_secs(n as u64)).is_empty());
    }
    assert!(screen.flush(t0 + Duration::from_secs(20)).is_empty());
}

#[test]
fn a_field_of_unknown_role_is_secret_for_every_lane_of_its_application_and_for_that_one_only() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    screen.apply(focus_on(50, 1, UNKNOWN_ROLE, ""), t0);
    assert!(screen.apply(from(":1.7", 99, 1, What::Children), t0).is_empty() && screen.next_due().is_none());
    assert_eq!(screen.apply(from(":1.9", 99, 1, What::Children), t0).len(), 1, "another application is not the field's");
}

/// #1257 review, M2 (RX2b inverted): after a gap the old focus was kept, and the password field's
/// focus signal may be the one that was lost.
#[test]
fn after_a_gap_no_text_is_told_until_the_focus_is_known_again() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    screen.apply(focus_on(40, 1, "text", "Search"), t0);
    screen.apply(Fact::Gap, t0);
    for n in 0..12u64 {
        screen.apply(from(":1.7", 50, 1, What::Text), t0 + Duration::from_millis(n * 300));
        screen.apply(from(":1.9", 50, 1, What::Text), t0 + Duration::from_millis(n * 300));
    }
    assert!(screen.flush(t0 + Duration::from_secs(10)).is_empty(), "for any application");
    screen.apply(focus_on(41, 1, "text", "Search"), t0 + Duration::from_secs(11));
    screen.apply(from(":1.7", 41, 1, What::Text), t0 + Duration::from_secs(11));
    assert_eq!(screen.flush(t0 + Duration::from_secs(16)), [text_changed("w1")], "and again once the focus is known");
}

#[test]
fn after_the_private_time_no_text_is_told_until_the_focus_is_known_again() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    screen.apply(focus_on(40, 1, "password text", "Password"), t0);
    screen.private_began();
    screen.private_ended();
    screen.apply(from(":1.7", 40, 1, What::Text), t0);
    assert!(screen.next_due().is_none(), "the password field may still have the focus");
    screen.apply(focus_on(41, 1, "text", "Search"), t0);
    screen.apply(from(":1.7", 41, 1, What::Text), t0);
    assert!(screen.next_due().is_some());
}

/// A focus that is simply gone (its window closed) is not a secret one.
#[test]
fn a_focus_that_went_with_its_window_is_not_a_secret() {
    let t0 = Instant::now();
    let mut screen = two_windows(t0);
    screen.apply(focus_on(40, 2, "text", "Search"), t0);
    screen.apply(Fact::Closed(key(2)), t0);
    screen.apply(from(":1.7", 41, 1, What::Text), t0);
    assert!(screen.next_due().is_some());
}

// ---- M3: whose windows a full model closes ----

fn win(id: u32, app: &str, pid: u32) -> WindowInfo {
    WindowInfo { id: format!("w{id}"), app: app.into(), title: "t".into(), role: "frame".into(), pid }
}

fn at(connection: &str, n: u32) -> Key {
    (connection.to_string(), format!("/org/a11y/atspi/accessible/{n}"))
}

/// An honest connection with two windows, and 510 connections of one hostile program with one
/// each: the model is full.
fn full_of_small_connections(now: Instant) -> Screen {
    let mut screen = Screen::default();
    screen.apply(Fact::Opened(at(":1.9", 1), win(1, "mail", 100)), now);
    screen.apply(Fact::Opened(at(":1.9", 2), win(2, "mail", 100)), now);
    for n in 0..510u32 {
        screen.apply(Fact::Opened(at(&format!(":2.{n}"), 1), win(100 + n, "spam", 666)), now);
    }
    assert_eq!(screen.windows().count(), MAX_WINDOWS);
    screen
}

/// #1257 review, M3 (RX3 inverted): one more hostile connection opened a window by closing the
/// honest application's newest, and then the honest dialog was refused.
#[test]
fn many_small_connections_do_not_close_an_honest_applications_windows() {
    let now = Instant::now();
    let mut screen = full_of_small_connections(now);
    let told = screen.apply(Fact::Opened(at(":2.999", 1), win(999, "spam", 666)), now);
    assert!(told.is_empty(), "the hostile program holds its share and more: refused, and nobody loses a window: {told:?}");
    let dialog = win(5000, "mail", 100);
    let told = screen.apply(Fact::Opened(at(":1.9", 3), dialog.clone()), now);
    assert_eq!(told.len(), 2, "the dialog is told, and one window made room: {told:?}");
    assert!(matches!(&told[0], Kind::WindowClosed { window } if window.app == "spam"), "a hostile window was closed, not the honest one: {:?}", told[0]);
    assert_eq!(told[1], Kind::WindowOpened { window: dialog });
    assert!(screen.knows(&at(":1.9", 1)) && screen.knows(&at(":1.9", 2)));
}

/// #1257 review, M3: a person is looking at the front window.
#[test]
fn the_front_window_is_never_closed_to_make_room() {
    let now = Instant::now();
    let mut screen = Screen::default();
    for connection in 0..8u32 {
        for n in 0..MAX_PER_CONNECTION as u32 {
            screen.apply(Fact::Opened(at(&format!(":1.{connection}"), n), win(connection * 100 + n, "busy", 666)), now);
        }
    }
    let newest = at(":1.7", MAX_PER_CONNECTION as u32 - 1);
    screen.apply(Fact::Activated(newest.clone()), now);
    let told = screen.apply(Fact::Opened(at(":1.99", 1), win(9000, "bank", 9)), now);
    assert_eq!(told.len(), 2, "{told:?}");
    assert!(screen.knows(&newest), "the front window stayed");
    assert_eq!(screen.front().map(|w| w.id.as_str()), Some("w763"));
    assert!(matches!(&told[0], Kind::WindowClosed { window } if window.id == "w762"), "the next newest of that connection went: {:?}", told[0]);
}

#[test]
fn the_focused_window_is_never_closed_to_make_room() {
    let now = Instant::now();
    let mut screen = Screen::default();
    for connection in 0..8u32 {
        for n in 0..MAX_PER_CONNECTION as u32 {
            screen.apply(Fact::Opened(at(&format!(":1.{connection}"), n), win(connection * 100 + n, "busy", 666)), now);
        }
    }
    let focused = at(":1.7", MAX_PER_CONNECTION as u32 - 1);
    screen.apply(Fact::Focus { source: at(":1.7", 5000), window: Some(focused.clone()), role: "text".into(), name: "x".into() }, now);
    screen.apply(Fact::Opened(at(":1.99", 1), win(9000, "bank", 9)), now);
    assert!(screen.knows(&focused));
}

/// Of equal holders, the connection learned last loses a window, not the one that was there first.
#[test]
fn of_equal_connections_the_one_learned_last_loses_its_window() {
    let now = Instant::now();
    let mut screen = Screen::default();
    // 8 processes of 64 windows: the model is full, and every process holds the same.
    for process in 0..8u32 {
        for n in 0..MAX_PER_CONNECTION as u32 {
            screen.apply(Fact::Opened(at(&format!(":1.{}", 7 - process), n), win(process * 100 + n, "busy", 10 + process)), now);
        }
    }
    let told = screen.apply(Fact::Opened(at(":1.99", 1), win(9000, "bank", 9)), now);
    assert!(matches!(&told[0], Kind::WindowClosed { window } if window.id == "w763"), "process 7's, the last learned: {:?}", told[0]);
}

/// A full model in which every one of 256 processes holds two windows, and the one learned last
/// is the one that would lose a window: `:9.255`, with `w0` and `w1`, `w1` the newer.
fn full_of_pairs(now: Instant) -> Screen {
    let mut screen = Screen::default();
    for process in 0..256u32 {
        for n in 0..2u32 {
            screen.apply(Fact::Opened(at(&format!(":9.{process}"), n), win(process * 10 + n, "pair", 1000 + process)), now);
        }
    }
    assert_eq!(screen.windows().count(), MAX_WINDOWS);
    screen
}

/// #1257 review, M3: the front window of the process that would lose a window is not the one
/// closed, nor its newer one: nobody loses a window, and the newcomer is refused.
#[test]
fn the_front_window_and_the_newest_of_its_application_are_never_closed_to_make_room() {
    let now = Instant::now();
    let mut screen = full_of_pairs(now);
    screen.apply(Fact::Activated(at(":9.255", 0)), now);
    let told = screen.apply(Fact::Opened(at(":1.99", 1), win(9000, "bank", 9)), now);
    assert!(told.is_empty(), "{told:?}");
    assert!(screen.knows(&at(":9.255", 0)) && screen.knows(&at(":9.255", 1)));
}

#[test]
fn the_focused_window_and_the_newest_of_its_application_are_never_closed_to_make_room() {
    let now = Instant::now();
    let mut screen = full_of_pairs(now);
    screen.apply(Fact::Focus { source: at(":9.255", 77), window: Some(at(":9.255", 0)), role: "text".into(), name: "x".into() }, now);
    let told = screen.apply(Fact::Opened(at(":1.99", 1), win(9000, "bank", 9)), now);
    assert!(told.is_empty(), "{told:?}");
    assert!(screen.knows(&at(":9.255", 0)) && screen.knows(&at(":9.255", 1)));
}
