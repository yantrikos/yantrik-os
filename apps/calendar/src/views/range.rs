//! `events_between`: a range of days read straight from the store, whatever the window shows.

use chrono::NaiveDate;

/// The most days `events_between` will answer for. A mind planning a week or a month fits; a
/// request for a year is a mistake, and the service would be listing every file for it.
pub const MAX_RANGE_DAYS: i64 = 62;

/// Read `from` and `to` of `events_between`: ISO dates, inclusive, `to` not before `from`, at most
/// [`MAX_RANGE_DAYS`] days in all.
pub fn parse_range(from: &str, to: &str) -> Result<(NaiveDate, NaiveDate), String> {
    let date = |name: &str, text: &str| {
        NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
            .map_err(|_| format!("`{name}` should look like 2026-09-28, not `{text}`"))
    };
    let (from, to) = (date("from", from)?, date("to", to)?);
    if to < from {
        return Err(format!("`to` ({to}) is before `from` ({from})"));
    }
    let days = (to - from).num_days() + 1;
    if days > MAX_RANGE_DAYS {
        return Err(format!(
            "{days} days is too many: ask for at most {MAX_RANGE_DAYS} days at a time"
        ));
    }
    Ok((from, to))
}

/// The events that fall in `from..=to`, as `{date, start, end, title, location}`, by start time.
///
/// An event is in the range when its start day is. (An event that began before `from` and runs
/// into it is not listed: the same rule `fetch_events_in_range`'s callers use to count a day.)
/// `location` is `null` when the event has none. This reads what it is given and touches no view:
/// a mind reading a range must not have to steer the grid to do it.
pub fn events_between(
    events: &[yantrik_ipc_contracts::calendar::CalendarEvent],
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<serde_json::Value> {
    let mut hits: Vec<(NaiveDate, &yantrik_ipc_contracts::calendar::CalendarEvent)> = events
        .iter()
        .filter_map(|e| {
            let day = e.start.get(..10).and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())?;
            (from..=to).contains(&day).then_some((day, e))
        })
        .collect();
    hits.sort_by(|a, b| (a.0, &a.1.start).cmp(&(b.0, &b.1.start)));
    hits.into_iter()
        .map(|(day, e)| {
            serde_json::json!({
                "date": day.to_string(),
                "start": e.start,
                "end": e.end,
                "title": e.title,
                "location": e.location,
            })
        })
        .collect()
}
