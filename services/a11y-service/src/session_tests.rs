//! The watcher's own small rules.

use super::*;

/// #1257 review, N8: with a wedged bus one more connection was left behind on every retry.
#[test]
fn connections_that_would_not_close_are_kept_but_not_without_limit() {
    let mut stuck = Vec::new();
    for connection in 0..10 {
        keep_newest(&mut stuck, connection, MAX_STUCK);
    }
    assert_eq!(stuck, [6, 7, 8, 9]);
}

/// #1257 review, N5 (M13): a gap told while private is dropped, so that the person was private is
/// kept, and the next session tells the gap when the private time is over.
#[test]
fn a_bus_that_goes_away_while_the_person_is_private_leaves_that_for_the_next_session() {
    let (sight, handles) = (Sight::idle(), handles::Shared::default());
    sight.set_listening(true);
    bus_went_away(&sight, &handles, true);
    assert!(sight.private_seen(), "the next session starts from private");
    assert!(sight.since(0, Duration::ZERO)["events"].as_array().is_some_and(Vec::is_empty), "and the gap is not told now");
    assert!(!sight.set_listening(false), "it is no longer listening");
}

#[test]
fn a_bus_that_goes_away_is_told_as_a_gap_once() {
    let (sight, handles) = (Sight::idle(), handles::Shared::default());
    sight.set_listening(true);
    bus_went_away(&sight, &handles, false);
    bus_went_away(&sight, &handles, false);
    assert_eq!(sight.since(0, Duration::ZERO)["events"].as_array().map(Vec::len), Some(1));
    assert!(!sight.private_seen());
}
