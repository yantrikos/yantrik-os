//! Which day is today, kept true for as long as the window stays open.
//!
//! VM 520, 00:21 on Monday 5 October: a Calendar opened on Wednesday 30 September still said
//! "September 2026 · DAY 30", with the marker on the 30th. Nothing had navigated there. `wire`
//! read the clock once, to choose the month and the day the window opens on, and the marker was
//! worked out again only when something else happened to redraw; nothing ever told the window
//! that the date had changed under it. So the window now asks, at midnight and whenever it is
//! brought back to the front (`main`), and this module answers the two questions that asking
//! raises: has the day turned since it last looked, and where should the window be now.
//!
//! The clock is a parameter ([`Clock`]) so a test can cross midnight without waiting for one.

use chrono::{Datelike, NaiveDate, NaiveDateTime};

/// Where "now" comes from.
pub trait Clock {
    fn now(&self) -> NaiveDateTime;
}

/// The machine's own clock, in its local time: the date a person reads off their taskbar.
pub struct LocalClock;

impl Clock for LocalClock {
    fn now(&self) -> NaiveDateTime {
        chrono::Local::now().naive_local()
    }
}

/// The day the window last saw as today.
#[derive(Clone, Copy, Debug)]
pub struct Today {
    seen: NaiveDate,
}

impl Today {
    pub fn new(clock: &impl Clock) -> Self {
        Today { seen: clock.now().date() }
    }

    /// Today, as of the last [`check`](Self::check).
    pub fn date(&self) -> NaiveDate {
        self.seen
    }

    /// Read the clock. `None` while it is still the day last seen; otherwise the day it was, and
    /// the new one is remembered. A clock set backwards is a change of day like any other.
    pub fn check(&mut self, clock: &impl Clock) -> Option<NaiveDate> {
        let now = clock.now().date();
        if now == self.seen {
            return None;
        }
        Some(std::mem::replace(&mut self.seen, now))
    }
}

/// What the window is showing: the month, and the day picked in it (0 when none is).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Shown {
    pub fn on(date: NaiveDate) -> Self {
        Shown { year: date.year(), month: date.month(), day: date.day() }
    }
}

/// Where the window goes when the day turns from `was` to `now`.
///
/// A window that was showing the old today — its month, with that day picked — was showing
/// "today", and follows it to the new one, into the next month if that is where it falls. A
/// window anywhere else is where the person put it, and stays: the marker moves under it, the
/// view does not.
pub fn follow(shown: Shown, was: NaiveDate, now: NaiveDate) -> Shown {
    if shown == Shown::on(was) {
        Shown::on(now)
    } else {
        shown
    }
}

/// How long from `now` to the next midnight, and a second past it, so a timer set for this wakes
/// in the new day rather than in the last instant of the old one.
pub fn until_midnight(now: NaiveDateTime) -> std::time::Duration {
    let next = now.date().succ_opt().and_then(|d| d.and_hms_opt(0, 0, 1));
    let wait = next.map(|n| n - now).unwrap_or_else(|| chrono::Duration::hours(1));
    // Never a zero or negative wait: that would spin.
    wait.to_std().unwrap_or(std::time::Duration::from_secs(1)).max(std::time::Duration::from_secs(1))
}

/// A clock that says whatever the test last set it to.
#[cfg(test)]
pub(crate) struct FakeClock(std::cell::Cell<NaiveDateTime>);

#[cfg(test)]
impl FakeClock {
    pub(crate) fn at(text: &str) -> Self {
        FakeClock(std::cell::Cell::new(NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()))
    }
    pub(crate) fn advance(&self, seconds: i64) {
        self.0.set(self.0.get() + chrono::Duration::seconds(seconds));
    }
}

#[cfg(test)]
impl Clock for FakeClock {
    fn now(&self) -> NaiveDateTime {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    /// The sign-off's case, from the other side: a window left open on today, the last minute of
    /// September, follows the date into October; one the person moved elsewhere stays put.
    #[test]
    fn crossing_midnight_moves_today_and_a_window_that_was_on_it() {
        let clock = FakeClock::at("2026-09-30 23:59:30");
        let mut today = Today::new(&clock);
        assert_eq!(today.date(), date("2026-09-30"));
        assert_eq!(until_midnight(clock.now()), std::time::Duration::from_secs(31));

        clock.advance(20);
        assert_eq!(today.check(&clock), None, "23:59:50 is still the 30th");

        clock.advance(30);
        let was = today.check(&clock).expect("00:00:20 is a new day");
        assert_eq!(was, date("2026-09-30"));
        assert_eq!(today.date(), date("2026-10-01"));
        assert_eq!(today.check(&clock), None, "and it turns once");

        let on_today = Shown { year: 2026, month: 9, day: 30 };
        assert_eq!(follow(on_today, was, today.date()), Shown { year: 2026, month: 10, day: 1 });

        // Navigation the person did stands.
        for elsewhere in [
            Shown { year: 2026, month: 9, day: 12 },
            Shown { year: 2026, month: 9, day: 0 },
            Shown { year: 2026, month: 12, day: 30 },
        ] {
            assert_eq!(follow(elsewhere, was, today.date()), elsewhere);
        }
    }

    /// Days the window slept through (a suspended machine, a window left open for the weekend)
    /// are one turn, from the day last seen to the day it is.
    #[test]
    fn a_window_asleep_for_days_lands_on_the_real_date() {
        let clock = FakeClock::at("2026-09-30 18:00:00");
        let mut today = Today::new(&clock);
        clock.advance(4 * 86_400 + 6 * 3_600 + 21 * 60);
        assert_eq!(clock.now().to_string(), "2026-10-05 00:21:00");
        let was = today.check(&clock).unwrap();
        assert_eq!(follow(Shown::on(was), was, today.date()), Shown { year: 2026, month: 10, day: 5 });
        assert_eq!(until_midnight(clock.now()), std::time::Duration::from_secs(23 * 3_600 + 39 * 60 + 1));
    }

    #[test]
    fn the_wait_is_never_zero() {
        assert_eq!(until_midnight(NaiveDateTime::parse_from_str("2026-10-05 23:59:59", "%Y-%m-%d %H:%M:%S").unwrap()),
            std::time::Duration::from_secs(2));
        assert_eq!(until_midnight(NaiveDateTime::parse_from_str("2026-10-05 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap()),
            std::time::Duration::from_secs(86_401));
    }
}
