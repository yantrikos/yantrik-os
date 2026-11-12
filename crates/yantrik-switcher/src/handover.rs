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
//! Acking early gives nothing away: if this program dies before its frame lands, labwc sees its
//! connection close and draws the list (`handle_resource_destroy` in the patch).

use std::time::Duration;

/// A quick Alt+Tab is over before this, and draws nothing.
pub const SHOW_AFTER: Duration = Duration::from_millis(100);
/// How long labwc waits for an ack before it draws the session itself.
pub const LABWC_WAITS: Duration = Duration::from_millis(300);
/// Kept back from labwc's wait: this program's clock for a session starts when it reads
/// `session_start`, which is after labwc started its own, and the ack still has to travel.
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
        let line = format!("#define ACK_TIMEOUT_MS {}", LABWC_WAITS.as_millis());
        assert!(patch.contains(&line), "packaging/labwc's patch no longer says `{line}`");
    }
}
