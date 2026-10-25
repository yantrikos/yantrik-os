//! The times an event gets: from the form or an `add_event` (a date, a clock and a length), for a
//! whole day, and when an update moves it.

use chrono::{Duration, NaiveDate, NaiveDateTime};

use super::{parse_clock, parse_datetime};

/// The ISO start and end an event gets when it is saved from the form.
///
/// The end used to be built as `hour + 1`, so 23:30 produced `T24:30:00`, which is not a time and
/// which the store refuses outright. Minutes, added properly, and clamped to the last minute of
/// the same day: an event that would run past midnight ends at 23:59 rather than becoming
/// unstorable. `None` when the date or the time cannot be read, so the app can say which.
pub fn start_and_end(date: &str, time: &str, duration_min: i32) -> Option<(String, String)> {
    let day = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").ok()?;
    let clock = parse_clock(time.trim())?;
    let start = day.and_time(clock);
    let day_end = day.and_hms_opt(23, 59, 0)?;
    let mut end = start + Duration::minutes(duration_min.max(0) as i64);
    if end > day_end {
        end = day_end;
    }
    let iso = |t: NaiveDateTime| t.format("%Y-%m-%dT%H:%M:%S").to_string();
    Some((iso(start), iso(end)))
}

/// The ISO start and end of a whole day.
///
/// An all-day event has no hour, so it is stored spanning its day: midnight to the last minute of
/// it, which is the same clamp [`start_and_end`] puts on a timed event that would otherwise run
/// past midnight. Nothing reads a clock off either end — `all_day_columns` reads the two dates —
/// so what matters is only that the pair is a real day and that it parses.
pub fn all_day_bounds(date: &str) -> Option<(String, String)> {
    let day = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").ok()?;
    let iso = |t: NaiveDateTime| t.format("%Y-%m-%dT%H:%M:%S").to_string();
    Some((iso(day.and_hms_opt(0, 0, 0)?), iso(day.and_hms_opt(23, 59, 0)?)))
}

/// The clock a new event gets: required unless the event fills the day.
///
/// `add_event` declares `time` optional because `all_day` has no use for one, which leaves this
/// handler deciding what an absent `time` means. A whole-day event needs none — the answer is
/// empty, and `store_event` does not consult it — and a timed event cannot be placed at an hour
/// nobody gave, so it is refused in a sentence naming `time` and pointing at `all_day`. Until
/// today the opposite held: `time` was required on the surface, so the all-day add sent exactly
/// as the describe told it to be sent — without `time` — never reached this code at all, and was
/// refused as "needs argument `time`" by the argument check (#297). A `time` that did arrive but
/// cannot hold a clock keeps the format refusal it always had.
pub fn added_clock(time: Option<&str>, all_day: bool) -> Result<String, String> {
    if all_day {
        return Ok(String::new());
    }
    let Some(time) = time.map(str::trim).filter(|t| !t.is_empty()) else {
        return Err("`time` is needed for an event at a particular hour; set `all_day` for a \
                    whole day"
            .into());
    };
    if !time.contains(':') {
        return Err(format!("`time` should look like 14:30, not `{time}`"));
    }
    Ok(time.to_string())
}

/// Where an update moves an event to: its new start and end, or the reason it cannot go there.
///
/// `Ok(None)` means the update said nothing about when the event is, so its stored times are left
/// exactly as they are — the contract's rule for every optional field, applied to the three fields
/// that together describe one thing.
///
/// Moving a meeting keeps how long it runs. "Move the standup to 10:00" says nothing about length,
/// and an update that quietly reset it to the default hour would be changing something nobody
/// asked about, which is the same mistake as dropping an argument, one size smaller.
///
/// An all-day event has no time to move and no length to change, so a `time` or a `duration_min`
/// given for one is refused rather than applied to a clock it does not have. Applying it would
/// silently turn a day-long event into a timed one, and the caller would be told it had moved
/// something.
pub fn rescheduled(
    current_start: &str,
    current_end: &str,
    is_all_day: bool,
    date: Option<&str>,
    time: Option<&str>,
    duration_min: Option<i32>,
) -> Result<Option<(String, String)>, String> {
    if is_all_day {
        if time.is_some() || duration_min.is_some() {
            return Err(
                "that event is all day: it has no time to move and no length to change, so only \
                 its `date` can be given"
                    .into(),
            );
        }
        let Some(date) = date else { return Ok(None) };
        return all_day_bounds(date)
            .map(Some)
            .ok_or_else(|| format!("`{}` is not a date", date.trim()));
    }

    if date.is_none() && time.is_none() && duration_min.is_none() {
        return Ok(None);
    }

    let start = parse_datetime(current_start).ok_or_else(|| {
        format!("the stored event starts at `{current_start}`, which is not a date and a time")
    })?;
    let duration = match duration_min {
        Some(minutes) if minutes < 0 => {
            return Err(format!("`duration_min` cannot be negative, and was {minutes}"))
        }
        Some(minutes) => minutes,
        // Read off the event rather than defaulted, which is the whole point: this is how long
        // the appointment already runs, and the update did not ask to change it.
        None => {
            let end = parse_datetime(current_end).ok_or_else(|| {
                format!(
                    "the stored event ends at `{current_end}`, which is not a date and a time, so \
                     how long it runs cannot be worked out — give `duration_min` as well"
                )
            })?;
            (end - start).num_minutes().max(0) as i32
        }
    };

    let day = date
        .map(|d| d.trim().to_string())
        .unwrap_or_else(|| start.format("%Y-%m-%d").to_string());
    let clock = time
        .map(|t| t.trim().to_string())
        .unwrap_or_else(|| start.format("%H:%M").to_string());
    start_and_end(&day, &clock, duration)
        .map(Some)
        .ok_or_else(|| format!("`{day} {clock}` is not a date and a time"))
}
