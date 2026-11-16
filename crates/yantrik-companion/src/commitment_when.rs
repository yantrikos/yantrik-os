//! When "Thursday" is: the next Thursday, at five in the afternoon, on the person's own clock
//! (#1184). The end of the working day was added to the start of a UTC day, so for anyone away
//! from UTC the deadline, and with it the prep window, landed hours off.

use chrono::{Datelike, Duration, TimeZone};

/// The hour a weekday deadline falls on, local time: the end of the working day.
const WORKDAY_END_HOUR: u32 = 17;

/// Days of the week as the extractor lists them, Monday first.
pub const WEEKDAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

/// The next `weekday` (0 is Monday) after the day `now` falls on in `tz`, at 17:00 there; a
/// weekday that is today means next week's. `None` only for a moment `tz` cannot place.
pub fn next_weekday_at_five<Tz: TimeZone>(now: f64, weekday: usize, tz: &Tz) -> Option<f64> {
    let today = tz.timestamp_opt(now as i64, 0).single()?.date_naive();
    let current = today.weekday().num_days_from_monday() as i64;
    let target = weekday as i64;
    let ahead = if target > current { target - current } else { 7 + target - current };
    let at = (today + Duration::days(ahead)).and_hms_opt(WORKDAY_END_HOUR, 0, 0)?;
    tz.from_local_datetime(&at).earliest().map(|t| t.timestamp() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

    /// Monday 10:00 UTC.
    const MONDAY_10_UTC: f64 = 20738.0 * 86400.0 + 10.0 * 3600.0;

    #[test]
    fn thursday_is_five_in_the_afternoon_where_the_person_is() {
        let utc = next_weekday_at_five(MONDAY_10_UTC, 3, &Utc).unwrap();
        assert_eq!(utc, 20738.0 * 86400.0 + 3.0 * 86400.0 + 17.0 * 3600.0);
        let india = FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
        let local = next_weekday_at_five(MONDAY_10_UTC, 3, &india).unwrap();
        assert_eq!(local, utc - (5.0 * 3600.0 + 1800.0), "17:00 in UTC+5:30 is 11:30 UTC");
        let new_york = FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(next_weekday_at_five(MONDAY_10_UTC, 3, &new_york).unwrap(), utc + 5.0 * 3600.0);
    }

    /// 23:00 UTC on Monday is already Tuesday in UTC+5:30: "Thursday" is counted from the local day.
    #[test]
    fn the_day_is_counted_from_the_local_date() {
        let india = FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
        let late_monday_utc = 20738.0 * 86400.0 + 23.0 * 3600.0;
        let thursday = next_weekday_at_five(late_monday_utc, 3, &india).unwrap();
        assert_eq!(thursday, 20738.0 * 86400.0 + 3.0 * 86400.0 + 17.0 * 3600.0 - (5.0 * 3600.0 + 1800.0));
        assert_eq!(next_weekday_at_five(late_monday_utc, 1, &india).unwrap() > late_monday_utc, true);
    }

    #[test]
    fn today_means_next_week() {
        assert_eq!(next_weekday_at_five(MONDAY_10_UTC, 0, &Utc).unwrap(), 20738.0 * 86400.0 + 7.0 * 86400.0 + 17.0 * 3600.0);
    }
}
