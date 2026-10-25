//! The events in hand, and what is read off them: which are on a day, how a row says when an
//! event runs, how many fall in the month, and the month grid's cells.
//!
//! Pure. The month grid, the agenda, the time grids, the rail and `describe` are all derived from
//! these functions, so the screen and the Mind cannot be told two different things about one day.

use chrono::{Datelike, NaiveDate};
use yantrik_ipc_contracts::calendar::{month_grid, CalendarEvent, DayCell};

use crate::views::{self, EventRef, SourceEvent, WeekStart};

/// One stored event, as this window holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct CalEvent {
    pub id: String,
    pub title: String,
    pub start: String,
    pub end: String,
    pub notes: String,
    pub is_all_day: bool,
    /// Who created this event, as the store recorded it at creation — `None` for everything
    /// stored before the record existed and everything a window's own form made. Read by
    /// `describe` and by the own-creation rule in `ownership`; never written from this app.
    pub creator: Option<String>,
    /// Position in the palette, not a colour: the same event has to be coloured the same in the
    /// agenda list and on the week grid, and `views` -- which has no Slint in it -- carries this
    /// through the derivation. `view.rs` holds the five colours.
    pub color_index: usize,
}

/// The store's events, in the order it returned them, each coloured by its position.
pub fn from_store(events: &[CalendarEvent]) -> Vec<CalEvent> {
    events
        .iter()
        .enumerate()
        .map(|(i, e)| CalEvent {
            id: e.id.clone(),
            title: e.title.clone(),
            start: e.start.clone(),
            end: e.end.clone(),
            notes: e.description.clone(),
            is_all_day: e.is_all_day,
            creator: e.creator.clone(),
            color_index: i,
        })
        .collect()
}

/// The day of the month the marker goes on in the month shown, if today is in it.
pub fn today_in(year: i32, month: u32, today: NaiveDate) -> Option<u32> {
    (today.year() == year && today.month() == month).then(|| today.day())
}

/// The month grid's cells. The grid itself is the contract's `month_grid`, shared with the
/// shell's Today panel: two copies of "which column does the 1st land in" is how two calendars
/// end up disagreeing.
pub fn month_cells(
    year: i32,
    month: u32,
    events: &[CalEvent],
    today_day: Option<u32>,
    week_start: WeekStart,
) -> Vec<DayCell> {
    let today = today_day.and_then(|d| NaiveDate::from_ymd_opt(year, month, d));
    let counts = |d: u32| {
        let prefix = format!("{:04}-{:02}-{:02}", year, month, d);
        events.iter().filter(|e| e.start.starts_with(&prefix)).count() as i32
    };
    month_grid(year, month, &counts, today, week_start)
}

/// What is on one day, in the order the store returned it.
///
/// One list, read by three things that must not disagree: the agenda rows the sidebar draws, the
/// row index the trash icon hands to `delete-event`, and the events `describe` reports for the
/// selected day. They were derived separately, and the index was the thing that came apart.
pub fn events_on_day(events: &[CalEvent], year: i32, month: u32, day: i32) -> Vec<&CalEvent> {
    let prefix = format!("{:04}-{:02}-{:02}", year, month, day);
    events.iter().filter(|e| e.start.starts_with(&prefix)).collect()
}

/// When an event runs, as a person reads it.
///
/// Hours and minutes. The seconds are in the store because the store keeps ISO timestamps, and
/// nobody reading their own day needs "14:00:00 - 15:00:00".
pub fn time_text(event: &CalEvent) -> String {
    if event.is_all_day {
        return "All day".to_string();
    }
    let clock = |iso: &str| {
        iso.split('T').nth(1).unwrap_or("").split(':').take(2).collect::<Vec<_>>().join(":")
    };
    format!("{} – {}", clock(&event.start), clock(&event.end))
}

/// The events in hand, in the shape the pure view code works in.
pub fn source_events(events: &[CalEvent]) -> Vec<SourceEvent> {
    events
        .iter()
        .map(|e| SourceEvent {
            id: e.id.clone(),
            title: e.title.clone(),
            start: e.start.clone(),
            end: e.end.clone(),
            is_all_day: e.is_all_day,
            color_index: e.color_index,
        })
        .collect()
}

/// The events in hand, in the shape naming one by title and date works in.
pub fn event_refs(events: &[CalEvent]) -> Vec<EventRef> {
    events
        .iter()
        .map(|e| EventRef {
            id: e.id.clone(),
            title: e.title.clone(),
            start: e.start.clone(),
            end: e.end.clone(),
            is_all_day: e.is_all_day,
        })
        .collect()
}

/// How many of the events in hand fall in the month on screen.
///
/// Not `events.len()`: the week view widens what is fetched past the month's edges, and counting
/// everything fetched would report events the month grid is not drawing.
pub fn events_in_month(events: &[CalEvent], year: i32, month: u32) -> usize {
    let prefix = format!("{:04}-{:02}", year, month);
    events.iter().filter(|e| e.start.starts_with(&prefix)).count()
}

/// The year, month and day a `YYYY-MM-DD…` string starts with (a date, or an event's start).
pub fn ymd(date: &str) -> Option<(i32, u32, i32)> {
    let mut parts = date.get(..10)?.split('-');
    let year = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: i32 = parts.next()?.parse().ok()?;
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some((year, month, day))
}

pub fn count_phrase(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The month heading: "September 2026".
pub fn month_title(year: i32, month: u32) -> String {
    format!("{} {}", views::month_name(month), year)
}

#[cfg(test)]
mod tests {
    use super::ymd;

    #[test]
    fn a_date_or_an_events_start_names_the_day_to_show() {
        assert_eq!(ymd("2026-09-30"), Some((2026, 9, 30)));
        assert_eq!(ymd("2026-10-15T16:30:00-05:00"), Some((2026, 10, 15)), "an event's start");
        assert_eq!(ymd("2026-13-01"), None, "no month 13");
        assert_eq!(ymd("2026-09-32"), None);
        assert_eq!(ymd("30 Sep"), None, "not a date: the window stays where it is");
        assert_eq!(ymd(""), None);
    }
}
