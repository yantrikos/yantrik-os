//! The recipes' clock, read in the machine's own zone (#187).
//!
//! What a recipe keeps is an absolute instant — a unix time — wherever it keeps one: when a timer
//! wakes (`_wait.until`), when a trigger last fired. What a person writes is a time on their own
//! clock: "wait until 09:00", "every morning at 8" (`0 8 * * *`), "after 18:00". Those are read in
//! the machine's local zone — /etc/localtime, which the shell keeps (`wire::location`) — and turned
//! into the instant they name there, so 09:00 is nine on the clock in the status bar, before and
//! after a daylight-saving change. The engine used to read every one of them in UTC, and the
//! Recipes screen said so ("09:00 UTC").
//!
//! Two nights a year a zone makes a wall-clock time awkward:
//!
//! - A spring-forward gap (02:30 on the night the clocks go from 02:00 to 03:00) never happens.
//!   A time in it is taken as the first minute after the jump: 03:00.
//! - A fall-back repeat (01:30 on the night the clocks go back from 02:00 to 01:00) happens twice.
//!   A time in it is the first of the two, and a schedule fires once, not twice.
//!
//! Every function takes the zone (`…_in`), so a test can hand it one with a change in it; the
//! recipe engine hands it [`chrono::Local`].

use chrono::{Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};

use crate::cron_mini::CronSpec;

/// The instant a wall-clock time names in `tz`, in unix seconds: the earlier of two in a
/// fall-back repeat, and the first minute after the jump for one in a spring-forward gap.
pub fn instant_of<Tz: TimeZone>(tz: &Tz, local: NaiveDateTime) -> Option<i64> {
    let at = |r: LocalResult<chrono::DateTime<Tz>>| match r {
        LocalResult::Single(t) => Some(t.timestamp()),
        LocalResult::Ambiguous(a, b) => Some(a.timestamp().min(b.timestamp())),
        LocalResult::None => None,
    };
    // A gap is at most a few hours; the first minute after it is the one the clock lands on.
    at(tz.from_local_datetime(&local))
        .or_else(|| (1..=24 * 60).find_map(|m| at(tz.from_local_datetime(&(local + Duration::minutes(m))))))
}

/// The wall-clock time an instant reads as in `tz`.
pub fn local_of<Tz: TimeZone>(tz: &Tz, ts: f64) -> Option<NaiveDateTime> {
    tz.timestamp_opt(ts.floor() as i64, 0).single().map(|t| t.naive_local())
}

/// When a wait for `hour:minute` on the local clock, begun at `from`, wakes: that time later
/// today, or tomorrow's once today's has gone by. None when `from` is inside that very minute —
/// its time has come — and when it names no time of day (an hour past 23, a minute past 59).
pub fn next_time_of_day_in<Tz: TimeZone>(tz: &Tz, hour: u8, minute: u8, from: f64) -> Option<f64> {
    let time = NaiveTime::from_hms_opt(hour.into(), minute.into(), 0)?;
    let today = local_of(tz, from)?.date();
    let at = |date: NaiveDate| instant_of(tz, date.and_time(time)).map(|t| t as f64);
    let first = at(today)?;
    if from < first {
        return Some(first);
    }
    if from < first + 60.0 {
        return None;
    }
    // Tomorrow's. The day after is looked at too only because a zone's rules are data.
    let mut day = today;
    for _ in 0..3 {
        day = day.succ_opt()?;
        if let Some(t) = at(day).filter(|t| *t > from) {
            return Some(t);
        }
    }
    None
}

/// The hour and minute an instant reads as on the local clock of `tz`.
pub fn hour_minute_in<Tz: TimeZone>(tz: &Tz, ts: f64) -> (u8, u8) {
    local_of(tz, ts).map(|t| (t.hour() as u8, t.minute() as u8)).unwrap_or((0, 0))
}

/// An instant as the local clock of `tz` shows it: "08:15".
pub fn clock_text_in<Tz: TimeZone>(tz: &Tz, ts: f64) -> String {
    local_of(tz, ts).map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

/// An instant as the local clock shows it, said against another (`since`): "08:15" on the same
/// day, "09:00 tomorrow" on the next, "09:00 on Thu 24 Sep" after that.
pub fn clock_text_from_in<Tz: TimeZone>(tz: &Tz, ts: f64, since: f64) -> String {
    let (Some(at), Some(from)) = (local_of(tz, ts), local_of(tz, since)) else { return String::new() };
    let clock = at.format("%H:%M").to_string();
    let days = (at.date() - from.date()).num_days();
    match days {
        0 => clock,
        1 => format!("{clock} tomorrow"),
        _ => format!("{clock} on {}", at.format("%a %-d %b")),
    }
}

/// Whether a wall-clock minute is one a schedule names.
fn names(spec: &CronSpec, t: &NaiveDateTime) -> bool {
    spec.matches(t.minute(), t.hour(), t.day(), t.month(), t.weekday().num_days_from_sunday())
}

/// The first time a cron schedule fires in (`from`, `until`], read on the local clock of `tz`, as
/// a unix time. Minute by minute, so keep the span short: the trigger clock asks about the last
/// few seconds, and at most [`crate::recipe_triggers::CATCH_UP_SECS`] after a sleep.
///
/// A minute the clock jumps over in a spring-forward gap fires as the clock lands after it; a
/// minute it comes back to in a fall-back repeat does not fire a second time.
pub fn cron_first_in<Tz: TimeZone>(tz: &Tz, spec: &CronSpec, from: f64, until: f64) -> Option<f64> {
    let minute = |t: NaiveDateTime| t.with_second(0).unwrap_or(t);
    // The latest wall-clock minute already seen: one the clock comes back to is not new.
    let mut high = minute(local_of(tz, from)?);
    let mut prev = high;
    let mut ts = (from.floor() as i64).div_euclid(60) * 60 + 60;
    let end = until.floor() as i64;
    while ts <= end {
        let local = minute(local_of(tz, ts as f64)?);
        // Minutes the clock jumped over: one the schedule names fires now, as the clock lands.
        let mut skipped = prev + Duration::minutes(1);
        while skipped < local && local - skipped <= Duration::hours(3) {
            if names(spec, &skipped) {
                return Some(ts as f64);
            }
            skipped += Duration::minutes(1);
        }
        if local > high {
            if names(spec, &local) {
                return Some(ts as f64);
            }
            high = local;
        }
        prev = local;
        ts += 60;
    }
    None
}

// ── On the machine's clock ──

/// [`next_time_of_day_in`] on the machine's clock.
pub fn next_time_of_day(hour: u8, minute: u8, from: f64) -> Option<f64> {
    next_time_of_day_in(&chrono::Local, hour, minute, from)
}

/// [`hour_minute_in`] on the machine's clock.
pub fn hour_minute(ts: f64) -> (u8, u8) {
    hour_minute_in(&chrono::Local, ts)
}

/// [`clock_text_in`] on the machine's clock.
pub fn clock_text(ts: f64) -> String {
    clock_text_in(&chrono::Local, ts)
}

/// [`clock_text_from_in`] on the machine's clock.
pub fn clock_text_from(ts: f64, since: f64) -> String {
    clock_text_from_in(&chrono::Local, ts, since)
}

/// [`cron_first_in`] on the machine's clock.
pub fn cron_first(spec: &CronSpec, from: f64, until: f64) -> Option<f64> {
    cron_first_in(&chrono::Local, spec, from, until)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset, Utc};

    /// US Central in 2026, as a zone a test can hold still: CST (−6) until 2026-03-08 08:00 UTC,
    /// when 02:00 becomes 03:00; CDT (−5) until 2026-11-01 07:00 UTC, when 02:00 becomes 01:00.
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct Central;

    pub(crate) fn spring() -> i64 {
        Utc.with_ymd_and_hms(2026, 3, 8, 8, 0, 0).unwrap().timestamp()
    }
    pub(crate) fn fall() -> i64 {
        Utc.with_ymd_and_hms(2026, 11, 1, 7, 0, 0).unwrap().timestamp()
    }
    fn cst() -> FixedOffset {
        FixedOffset::west_opt(6 * 3600).unwrap()
    }
    fn cdt() -> FixedOffset {
        FixedOffset::west_opt(5 * 3600).unwrap()
    }

    impl TimeZone for Central {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Self {
            Central
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            let t = utc.and_utc().timestamp();
            if (spring()..fall()).contains(&t) { cdt() } else { cst() }
        }
        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
            let wall = local.and_utc().timestamp();
            // Read as each offset, and kept where the zone agrees it is that offset then.
            let holds = |off: FixedOffset| {
                let utc = DateTime::from_timestamp(wall - i64::from(off.local_minus_utc()), 0).unwrap().naive_utc();
                self.offset_from_utc_datetime(&utc) == off
            };
            match (holds(cdt()), holds(cst())) {
                (true, true) => LocalResult::Ambiguous(cdt(), cst()),
                (true, false) => LocalResult::Single(cdt()),
                (false, true) => LocalResult::Single(cst()),
                (false, false) => LocalResult::None,
            }
        }
        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
        }
    }

    /// A wall-clock time in Central, as the instant it names.
    pub(crate) fn central(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> f64 {
        instant_of(&Central, NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()).unwrap() as f64
    }

    /// "Wait until 09:00" means nine on the local clock — CDT in summer, CST in winter — and the
    /// wait across a change is the hours the clock says, not 24 of UTC's.
    #[test]
    fn a_time_of_day_is_the_local_clocks_on_both_sides_of_a_change() {
        // Summer: 09:00 CDT is 14:00 UTC.
        let at = next_time_of_day_in(&Central, 9, 0, central(2026, 7, 1, 8, 0)).unwrap();
        assert_eq!(at, Utc.with_ymd_and_hms(2026, 7, 1, 14, 0, 0).unwrap().timestamp() as f64);
        assert_eq!(clock_text_in(&Central, at), "09:00");
        // Winter: 09:00 CST is 15:00 UTC.
        let at = next_time_of_day_in(&Central, 9, 0, central(2026, 1, 15, 8, 0)).unwrap();
        assert_eq!(at, Utc.with_ymd_and_hms(2026, 1, 15, 15, 0, 0).unwrap().timestamp() as f64);

        // Set at 22:00 the night the clocks go forward: 09:00 is ten hours on, not eleven.
        let evening = central(2026, 3, 7, 22, 0);
        let nine = next_time_of_day_in(&Central, 9, 0, evening).unwrap();
        assert_eq!(nine - evening, 10.0 * 3600.0);
        assert_eq!(clock_text_in(&Central, nine), "09:00");
        assert_eq!(clock_text_from_in(&Central, nine, evening), "09:00 tomorrow");
        // And the night they go back: twelve hours on, not eleven.
        let evening = central(2026, 10, 31, 22, 0);
        let nine = next_time_of_day_in(&Central, 9, 0, evening).unwrap();
        assert_eq!(nine - evening, 12.0 * 3600.0);
        assert_eq!(clock_text_in(&Central, nine), "09:00");
    }

    /// The two awkward nights: 02:30 does not happen when the clocks go forward, so a wait for it
    /// wakes as they land at 03:00; 01:30 happens twice when they go back, and a wait for it wakes
    /// at the first — a wait begun inside the repeat waits for tomorrow's.
    #[test]
    fn a_time_in_a_gap_wakes_as_the_clock_lands_and_a_repeat_wakes_once() {
        let before = central(2026, 3, 8, 1, 0);
        let gap = next_time_of_day_in(&Central, 2, 30, before).unwrap();
        assert_eq!(gap, spring() as f64, "the instant the clock jumps to 03:00");
        assert_eq!(clock_text_in(&Central, gap), "03:00");

        let before = central(2026, 11, 1, 0, 30);
        let first = next_time_of_day_in(&Central, 1, 30, before).unwrap();
        assert_eq!(first, Utc.with_ymd_and_hms(2026, 11, 1, 6, 30, 0).unwrap().timestamp() as f64, "01:30 CDT");
        // 01:45 on the second pass through the hour (CST): today's 01:30 has gone by.
        let second_pass = Utc.with_ymd_and_hms(2026, 11, 1, 7, 45, 0).unwrap().timestamp() as f64;
        assert_eq!(clock_text_in(&Central, second_pass), "01:45");
        let next = next_time_of_day_in(&Central, 1, 30, second_pass).unwrap();
        assert_eq!(next, central(2026, 11, 2, 1, 30), "tomorrow's, not the repeat");

        // Inside the very minute: no wait. No such time: no wait either.
        assert_eq!(next_time_of_day_in(&Central, 9, 0, central(2026, 7, 1, 9, 0) + 30.0), None);
        assert_eq!(next_time_of_day_in(&Central, 24, 0, central(2026, 7, 1, 9, 0)), None);
    }

    /// A schedule on the local clock: `0 8 * * *` fires at 08:00 CST in winter and 08:00 CDT in
    /// summer; a time in the spring gap fires as the clock lands; a time in the fall repeat fires
    /// once.
    #[test]
    fn a_schedule_fires_on_the_local_clock_across_a_change() {
        let eight = CronSpec::parse("0 8 * * *").unwrap();
        let fired = cron_first_in(&Central, &eight, central(2026, 3, 7, 12, 0), central(2026, 3, 9, 0, 0)).unwrap();
        assert_eq!(fired, central(2026, 3, 8, 8, 0), "the morning after the change: 08:00 CDT");
        assert_eq!(fired, Utc.with_ymd_and_hms(2026, 3, 8, 13, 0, 0).unwrap().timestamp() as f64);
        assert_eq!(
            cron_first_in(&Central, &eight, central(2026, 3, 8, 8, 0), central(2026, 3, 8, 23, 0)),
            None,
            "once a day: not again the same day"
        );
        assert_eq!(cron_first_in(&Central, &eight, central(2026, 1, 1, 0, 0), central(2026, 1, 1, 7, 59)), None, "not early");

        let half_two = CronSpec::parse("30 2 * * *").unwrap();
        let fired = cron_first_in(&Central, &half_two, central(2026, 3, 8, 0, 0), central(2026, 3, 8, 12, 0)).unwrap();
        assert_eq!(fired, spring() as f64, "02:30 never happens: it fires as the clock lands at 03:00");

        let half_one = CronSpec::parse("30 1 * * *").unwrap();
        let from = central(2026, 11, 1, 0, 0);
        let first = cron_first_in(&Central, &half_one, from, central(2026, 11, 1, 12, 0)).unwrap();
        assert_eq!(first, Utc.with_ymd_and_hms(2026, 11, 1, 6, 30, 0).unwrap().timestamp() as f64, "01:30 CDT");
        assert_eq!(
            cron_first_in(&Central, &half_one, first, central(2026, 11, 1, 12, 0)),
            None,
            "the clock comes back to 01:30 an hour later, and it does not fire again"
        );
        assert!(CronSpec::parse("61 * * * *").is_none());
    }
}
