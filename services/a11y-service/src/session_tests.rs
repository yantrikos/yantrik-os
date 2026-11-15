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
