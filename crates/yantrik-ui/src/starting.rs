//! Apps the shell has launched that have not shown a window yet.
//!
//! A launch is not a window (see `running`). Between the two there is a stretch the shell used to
//! say nothing about: `open_app email` answered `launching` at once, the Email window came up
//! eight seconds later, and in between the window that was already in front stayed there with no
//! sign anything had happened — so it looked as if the wrong app had opened, or none. The dock,
//! START and the taskbar now say "Starting…" for that stretch, and `open_app` says the app is
//! starting rather than implying it is open.
//!
//! "Has a window" is the compositor's last reading (`windows`), so the mark is only as fresh as
//! that reading; `windows` takes a new one on the taskbar's poll while anything is starting.

use std::time::{SystemTime, UNIX_EPOCH};

/// The word the shell's surfaces draw beside an app that is starting.
pub const MARK: &str = "Starting\u{2026}";

/// How long a launch is called "starting" when no window has been seen for it.
///
/// A bound, because the compositor cannot always be read (no `wlrctl` on the image): without it
/// every app launched on such a machine would say "Starting…" for as long as it ran. Twenty
/// seconds covers a slow first start, a service being brought up for it included, and is short
/// enough that a mark the shell cannot clear does not outstay its welcome.
pub const BUDGET_SECS: u64 = 20;

pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Whether a launch made at `since_unix` is still starting at `now_unix`.
pub fn still_starting(since_unix: u64, now_unix: u64, has_window: bool) -> bool {
    !has_window && now_unix.saturating_sub(since_unix) < BUDGET_SECS
}

/// Whether the shell launched `app_id` and has not seen its window yet.
pub fn is_starting(app_id: &str) -> bool {
    crate::windows::starting_apps().iter().any(|id| id == app_id)
}

/// What `open_app` says about a launch, in the person's words: whether the app is starting or was
/// already open. `state` is the same thing for a caller that reads fields.
pub fn launch_answer(app_id: &str, had_window: bool) -> (&'static str, String) {
    let name = crate::windows::app_display_name(app_id);
    if had_window {
        ("open", format!("{name} is already open"))
    } else {
        ("starting", format!("{name} is starting; its window is not up yet"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_launch_without_a_window_is_starting() {
        assert!(still_starting(100, 105, false));
    }

    #[test]
    fn a_window_ends_starting() {
        assert!(!still_starting(100, 101, true));
    }

    /// A machine whose compositor cannot be read never sees the window; the mark still goes.
    #[test]
    fn starting_runs_out_when_no_window_is_ever_seen() {
        assert!(!still_starting(100, 100 + BUDGET_SECS, false));
    }

    /// A clock that stepped backwards reads as a launch made just now, not as an error.
    #[test]
    fn a_clock_that_stepped_back_reads_as_just_launched() {
        assert!(still_starting(200, 100, false));
    }

    #[test]
    fn the_answer_says_starting_until_there_is_a_window() {
        let (state, said) = launch_answer("email", false);
        assert_eq!(state, "starting");
        assert_eq!(said, "Email is starting; its window is not up yet");
        assert_eq!(launch_answer("email", true).0, "open");
    }
}
