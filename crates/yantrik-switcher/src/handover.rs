//! Who draws a session: this program or labwc.
//!
//! labwc waits `LABWC_WAITS` from the start of a session for an ack (ACK_TIMEOUT_MS in
//! packaging/labwc's patch). With none it draws its own list, ignores an ack that comes later,
//! and does not say that it did. So a session is acked the moment this program decides to draw
//! it, not when its first frame lands. Under load a first frame can take longer than labwc
//! waits, and an ack sent after it left both on the screen at once: labwc's list over the stack
//! (seen on VM 520, 12 Nov 2026). A session that is already too old when that moment comes is
//! left to labwc alone, for the same reason.
//!
//! What acking early gives up: after the ack only this program's death hands the session back
//! (labwc sees its connection close and draws the list, `handle_resource_destroy` in the patch).
//! A switcher that is alive but gets no frame on the screen, because it hangs after the ack, its
//! first frame is very slow or a buffer cannot be made, shows nothing until the frame comes, where
//! labwc's list would have come up at `LABWC_WAITS`. Alt coming up still switches. A layer surface
//! the compositor closes during a session is the one such case this program acts on: it exits,
//! so labwc draws.
//!
//! What is still open: labwc cannot say that it fell back, and this program's clock for a session
//! starts when it reads `session_start`. Starved for long enough before that read (about 200 ms
//! for a session it then decides on time), it can take an old session for a young one, ack too
//! late and draw over labwc's list. Closing that takes a word from labwc (#1193).

use std::time::Duration;

/// A quick Alt+Tab is over before this, and draws nothing.
pub const SHOW_AFTER: Duration = Duration::from_millis(100);
/// How long labwc waits for an ack before it draws the session itself.
pub const LABWC_WAITS: Duration = Duration::from_millis(300);
/// Kept back from labwc's wait: this program's clock for a session starts when it reads
/// `session_start`, which is after labwc started its own, and the ack still has to travel.
/// A guess, not a measurement: an on-time session acks with about 200 ms to spare, so this only
/// places the cutoff. `YANTRIK_SWITCHER_DEBUG=1` logs each session's age when it is decided.
const MARGIN: Duration = Duration::from_millis(80);

/// Who draws the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handover {
    /// This program: ack now, then draw.
    Draw,
    /// labwc: it has drawn its own list or is about to. Neither ack nor draw.
    LeaveToLabwc,
}

/// Decided once per session, when the stack is about to go up, from how long ago this program
/// read the session's start.
pub fn decide(since_start: Duration) -> Handover {
    if since_start + MARGIN < LABWC_WAITS {
        Handover::Draw
    } else {
        Handover::LeaveToLabwc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_shown_on_time_is_this_programs() {
        assert_eq!(decide(SHOW_AFTER), Handover::Draw);
        assert_eq!(decide(SHOW_AFTER + Duration::from_millis(60)), Handover::Draw);
    }

    #[test]
    fn a_session_too_old_to_ack_in_time_is_left_to_labwc() {
        assert_eq!(decide(LABWC_WAITS - MARGIN), Handover::LeaveToLabwc);
        assert_eq!(decide(LABWC_WAITS), Handover::LeaveToLabwc);
        assert_eq!(decide(Duration::from_secs(2)), Handover::LeaveToLabwc);
    }

    #[test]
    fn the_wait_here_is_the_wait_in_labwcs_patch() {
        let patch = include_str!("../../../packaging/labwc/debian-patches/yantrik-window-switcher-renderer.patch");
        // The whole line as the patch adds it: `300` must not pass for `3000`.
        let line = format!("\n+#define ACK_TIMEOUT_MS {}\n", LABWC_WAITS.as_millis());
        assert!(patch.contains(&line), "packaging/labwc's patch no longer says {line:?}");
    }
}
