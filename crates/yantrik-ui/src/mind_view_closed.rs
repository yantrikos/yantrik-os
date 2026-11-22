//! Who ended Mind View: the person, or the shell (#1347).
//!
//! Only the person's own close sends the next start aside ([`crate::mind_view_show`]). The shell
//! stopping a Mind View an earlier shell left, or a Mind View that died, is not the person
//! closing it, so neither is noted. The person's close is seen as the window going away while
//! its compositor keeps running: the host window's close button ends only the window, so the
//! nested compositor lives on with no window to show it, and the next start would say it had
//! started and show nothing. [`judge`] tells that from a window that has not mapped yet; the
//! caller then stops the compositor and starts a fresh one.

use std::time::Duration;

/// How long a started Mind View may go without its window being seen before the missing window is
/// taken for a close (the person's own: the map takes a moment, not this long).
pub(crate) const MAP_GRACE: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Verdict {
    /// The window is there, or there is no knowing it is not: leave it running.
    Shown,
    /// The window was there and is gone, or never came: the person closed it.
    Closed,
}

/// `window` is the compositor's answer to "is Mind View's window there" (`None`: it did not
/// say). `seen` is whether it has ever been seen; the returned flag is the new value of it.
pub(crate) fn judge(seen: bool, since_start: Duration, window: Option<bool>) -> (Verdict, bool) {
    match window {
        Some(true) => (Verdict::Shown, true),
        Some(false) if seen || since_start > MAP_GRACE => (Verdict::Closed, seen),
        _ => (Verdict::Shown, seen),
    }
}

/// Only a compositor that is still running with its window gone is the person's close: one that
/// has exited died or was stopped by the shell, and neither sends the next start aside.
pub(crate) fn closed_by_person(alive: bool, verdict: Verdict) -> bool {
    alive && verdict == Verdict::Closed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_that_was_seen_and_is_gone_was_closed_by_the_person() {
        assert_eq!(judge(true, Duration::from_secs(60), Some(false)).0, Verdict::Closed);
        assert_eq!(judge(false, MAP_GRACE + Duration::from_secs(1), Some(false)).0, Verdict::Closed, "never came");
    }

    /// A Mind View that died, or that the shell stopped (an earlier shell's, found at a start),
    /// has no running compositor: it is never the person's close, whatever the window list says.
    #[test]
    fn a_mind_view_that_died_or_was_stopped_is_not_the_persons_close() {
        assert!(!closed_by_person(false, Verdict::Closed));
        assert!(!closed_by_person(false, Verdict::Shown));
        assert!(!closed_by_person(true, Verdict::Shown));
        assert!(closed_by_person(true, Verdict::Closed));
    }

    #[test]
    fn a_window_still_mapping_or_a_compositor_that_did_not_answer_is_not_a_close() {
        assert_eq!(judge(false, Duration::from_secs(1), Some(false)).0, Verdict::Shown, "not mapped yet");
        assert_eq!(judge(true, Duration::from_secs(60), None).0, Verdict::Shown, "no answer is no evidence");
        assert_eq!(judge(false, Duration::from_secs(1), Some(true)), (Verdict::Shown, true), "seen now");
    }
}
