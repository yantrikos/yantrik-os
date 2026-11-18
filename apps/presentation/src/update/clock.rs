//! The presenter's clock: one repeating second that exists only while the clock runs, so a deck
//! at rest has no timer armed and draws nothing.

use std::time::Duration;

use yantrik_app::effects::Effects;
use yantrik_app::TimerId;

use crate::command::Command;
use crate::model::Model;
use crate::YPresent;

/// The timer a running clock is ticked by.
pub const TICK: TimerId = TimerId("clock");

/// Start the clock if it is stopped, stop it if it runs.
pub fn toggle(m: &mut Model, fx: &mut Effects<YPresent>) {
    m.clock.running = !m.clock.running;
    if m.clock.running {
        fx.every(TICK, Duration::from_secs(1), Command::Tick);
    } else {
        fx.stop_timer(TICK);
    }
}

/// Stop the clock and put it back to zero.
pub fn reset(m: &mut Model, fx: &mut Effects<YPresent>) {
    m.clock.running = false;
    m.clock.seconds = 0;
    fx.stop_timer(TICK);
}

/// The clock as the screen reads it: `MM:SS`, and `H:MM:SS` past the hour.
pub fn text(seconds: i64) -> String {
    let s = seconds.max(0);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::text;

    #[test]
    fn the_clock_reads_minutes_and_seconds_and_hours_only_when_it_must() {
        assert_eq!(text(0), "00:00");
        assert_eq!(text(75), "01:15");
        assert_eq!(text(3599), "59:59");
        assert_eq!(text(3600), "1:00:00");
        assert_eq!(text(-5), "00:00");
    }
}
