//! How long ago something happened, in the one way the OS says it.
//!
//! Before this module the OS said it eleven ways: "4m ago" in the Mind panel, "4 min ago" on a
//! Files tile, "4m" in the Files list, "4 minutes ago" in its details, "95 min ago" in Weather,
//! "10s ago" in the Agents workroom. Now there is one rule in two lengths:
//!
//! | elapsed            | [`ago`], in lists and tiles | [`ago_long`], in a details pane |
//! |--------------------|-----------------------------|---------------------------------|
//! | under a minute     | `just now`                  | `just now`                      |
//! | under an hour      | `4 min ago`                 | `4 minutes ago`                 |
//! | under a day        | `3 h ago`                   | `3 hours ago`                   |
//! | under 30 days      | `2 d ago`                   | `2 days ago`                    |
//! | under a year       | `5 mo ago`                  | `5 months ago`                  |
//! | a year or more     | `2 y ago`                   | `2 years ago`                   |
//!
//! Whole numbers only, always rounded down: a thing saved 59 minutes ago was not saved "1 h ago".
//! A time ahead of the clock (a clock that moved) is "just now", never a negative age.

use std::time::{Duration, SystemTime};

const MIN: u64 = 60;
const HOUR: u64 = 60 * MIN;
const DAY: u64 = 24 * HOUR;
const MONTH: u64 = 30 * DAY;
const YEAR: u64 = 365 * DAY;

/// The band an age falls in: how many of which unit. `None` under a minute.
fn band(elapsed: Duration) -> Option<(u64, Unit)> {
    let s = elapsed.as_secs();
    Some(match s {
        s if s < MIN => return None,
        s if s < HOUR => (s / MIN, Unit::Minute),
        s if s < DAY => (s / HOUR, Unit::Hour),
        s if s < MONTH => (s / DAY, Unit::Day),
        s if s < YEAR => (s / MONTH, Unit::Month),
        s => (s / YEAR, Unit::Year),
    })
}

#[derive(Clone, Copy)]
enum Unit {
    Minute,
    Hour,
    Day,
    Month,
    Year,
}

impl Unit {
    fn short(self) -> &'static str {
        match self {
            Unit::Minute => "min",
            Unit::Hour => "h",
            Unit::Day => "d",
            Unit::Month => "mo",
            Unit::Year => "y",
        }
    }

    fn long(self, n: u64) -> &'static str {
        match (self, n) {
            (Unit::Minute, 1) => "minute",
            (Unit::Minute, _) => "minutes",
            (Unit::Hour, 1) => "hour",
            (Unit::Hour, _) => "hours",
            (Unit::Day, 1) => "day",
            (Unit::Day, _) => "days",
            (Unit::Month, 1) => "month",
            (Unit::Month, _) => "months",
            (Unit::Year, 1) => "year",
            (Unit::Year, _) => "years",
        }
    }
}

/// How long ago, short enough for a list or a tile: "4 min ago".
pub fn ago(elapsed: Duration) -> String {
    match band(elapsed) {
        None => "just now".into(),
        Some((n, unit)) => format!("{n} {} ago", unit.short()),
    }
}

/// How long ago, in full words for a details pane: "4 minutes ago".
pub fn ago_long(elapsed: Duration) -> String {
    match band(elapsed) {
        None => "just now".into(),
        Some((n, unit)) => format!("{n} {} ago", unit.long(n)),
    }
}

/// How long ago to the minute, for a time line where the minutes matter (Alt+Tab): "4 min ago",
/// "1 h ago", "2 h 15 min ago". From a day on it is [`ago`]'s "2 d ago".
pub fn ago_precise(elapsed: Duration) -> String {
    let s = elapsed.as_secs();
    if s < HOUR || s >= DAY {
        return ago(elapsed);
    }
    match (s / HOUR, (s % HOUR) / MIN) {
        (h, 0) => format!("{h} h ago"),
        (h, m) => format!("{h} h {m} min ago"),
    }
}

/// [`ago`] for `then`, seen at `now`. A `then` after `now` is "just now".
pub fn ago_between(then: SystemTime, now: SystemTime) -> String {
    ago(now.duration_since(then).unwrap_or_default())
}

/// [`ago`] for an age in seconds, as unix timestamps are subtracted. A negative or non-finite age
/// is "just now"; an absurdly large one is held to a million years, where `Duration` would panic.
pub fn ago_secs(secs: f64) -> String {
    let secs = if secs.is_finite() && secs > 0.0 {
        secs.min(1e6 * YEAR as f64)
    } else {
        0.0
    };
    ago(Duration::from_secs_f64(secs))
}

/// [`ago`] for a unix timestamp in seconds, seen now.
pub fn ago_since_unix(then_secs: f64) -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    ago_secs(now - then_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> String {
        ago(Duration::from_secs(s))
    }

    #[test]
    fn every_band_reads_as_words() {
        assert_eq!(secs(0), "just now");
        assert_eq!(secs(59), "just now");
        assert_eq!(secs(60), "1 min ago");
        assert_eq!(secs(4 * MIN + 30), "4 min ago");
        assert_eq!(
            secs(HOUR - 1),
            "59 min ago",
            "rounded down, never up to an hour"
        );
        assert_eq!(secs(HOUR), "1 h ago");
        assert_eq!(secs(3 * HOUR + 10), "3 h ago");
        assert_eq!(secs(DAY), "1 d ago");
        assert_eq!(secs(29 * DAY), "29 d ago");
        assert_eq!(secs(MONTH), "1 mo ago");
        assert_eq!(secs(YEAR - 1), "12 mo ago");
        assert_eq!(secs(2 * YEAR), "2 y ago");
    }

    #[test]
    fn the_long_form_has_the_same_bands_in_full_words() {
        let long = |s: u64| ago_long(Duration::from_secs(s));
        assert_eq!(long(30), "just now");
        assert_eq!(long(MIN), "1 minute ago");
        assert_eq!(long(12 * MIN), "12 minutes ago");
        assert_eq!(long(HOUR), "1 hour ago");
        assert_eq!(long(5 * HOUR), "5 hours ago");
        assert_eq!(long(DAY + 5), "1 day ago");
        assert_eq!(long(3 * DAY), "3 days ago");
        assert_eq!(long(MONTH), "1 month ago");
        assert_eq!(long(7 * MONTH), "7 months ago");
        assert_eq!(long(YEAR), "1 year ago");
        assert_eq!(long(3 * YEAR), "3 years ago");
    }

    #[test]
    fn the_precise_form_keeps_the_minutes_within_a_day() {
        let p = |s: u64| ago_precise(Duration::from_secs(s));
        assert_eq!(p(5), "just now");
        assert_eq!(p(4 * MIN), "4 min ago");
        assert_eq!(p(HOUR), "1 h ago");
        assert_eq!(p(2 * HOUR + 15 * MIN + 30), "2 h 15 min ago");
        assert_eq!(p(DAY + HOUR), "1 d ago");
    }

    #[test]
    fn a_time_ahead_of_the_clock_is_just_now() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert_eq!(ago_between(now + Duration::from_secs(500), now), "just now");
        assert_eq!(
            ago_between(now - Duration::from_secs(7_200), now),
            "2 h ago"
        );
        assert_eq!(ago_secs(-30.0), "just now");
        assert_eq!(ago_secs(f64::NAN), "just now");
        assert_eq!(ago_secs(f64::INFINITY), "just now");
        assert_eq!(
            ago_secs(1e300),
            "1000000 y ago",
            "a huge age does not panic"
        );
        assert_eq!(ago_secs(125.9), "2 min ago");
    }

    #[test]
    fn now_is_just_now() {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        assert_eq!(ago_since_unix(now), "just now");
        assert_eq!(ago_since_unix(now - 3.0 * DAY as f64 - 5.0), "3 d ago");
    }
}
