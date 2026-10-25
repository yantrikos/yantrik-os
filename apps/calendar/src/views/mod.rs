//! What the week and day views are drawing, derived from the events the month already holds.
//!
//! Week and Day were an empty drawing. `week-events`, `day-events`, `week-day-labels`,
//! `current-hour` and `day-view-title` are `in` properties of the calendar screen and nothing in
//! the repo ever set one of them, so pressing Week or Day moved to a grid with no events, blank
//! column headers and a "now" line pinned to midnight -- while `set_view` answered
//! `{"view": "week"}` as though it had shown something. The events were already in the app's
//! hands; only the arithmetic that turns a stored timestamp into a column and a height was
//! missing.
//!
//! It lives in its own module, with no Slint types in it, so the arithmetic can be tested without
//! a desktop or a service (`tests/`). `main.rs` converts these plain structs into
//! `CalendarTimeEvent` rows and nothing else.
//!
//! The week starts on the day `WeekStart` says (the settings file, then the locale). The month
//! grid, the week view and `show_date` all take the same value: a week view starting on one day
//! while the month grid's header started on another would put the same date in two different
//! columns of the same calendar, and a week view fixed to Sunday could not show a mind the
//! Monday-to-Sunday week it asked for.
//!
//! The module is split by concept: placing events on the time grids (`grid`), the times a
//! saved or moved event gets (`times`), naming an event by title and date and back (`naming`),
//! and reading a range straight from the store (`range`).

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
pub use yantrik_ipc_contracts::calendar::WeekStart;

mod grid;
mod naming;
mod range;
mod times;
#[cfg(test)]
mod tests;

pub use grid::{day_view, week_view};
pub use naming::{name_event, named_on, naming_index, NAMING_CAP};
pub use range::{events_between, parse_range, MAX_RANGE_DAYS};
pub use times::{added_clock, all_day_bounds, rescheduled, start_and_end};

/// Which of the three views the screen is showing. Mirrors `view-mode` in `calendar.slint`,
/// where the value is an untyped int.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Month,
    Week,
    Day,
}

impl ViewMode {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => ViewMode::Week,
            2 => ViewMode::Day,
            _ => ViewMode::Month,
        }
    }

    /// The screen's number for it: the inverse of [`ViewMode::from_index`].
    pub fn index(self) -> i32 {
        match self {
            ViewMode::Month => 0,
            ViewMode::Week => 1,
            ViewMode::Day => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ViewMode::Month => "month",
            ViewMode::Week => "week",
            ViewMode::Day => "day",
        }
    }
}

/// A stored event, reduced to what placing it on a time grid needs.
///
/// `color_index` rather than a colour: the palette is a Slint concern and this module has no
/// Slint in it. `main.rs` holds the five colours and looks the index back up.
///
/// `id` is the store's own, carried through because `describe` is derived from these and a caller
/// that is told an event's title and time and not its id has no way to name it back. Until today
/// the only id anywhere near the screen was a row position, which is the whole of the trash-icon
/// bug 617dac9 fixed.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceEvent {
    pub id: String,
    pub title: String,
    /// ISO 8601 as the store keeps it, `YYYY-MM-DDTHH:MM:SS`.
    pub start: String,
    pub end: String,
    pub is_all_day: bool,
    pub color_index: usize,
}

/// One block on a time grid, in the shape `CalendarTimeEvent` wants.
///
/// An event running past midnight becomes two blocks carrying the same `id`, which is the truth:
/// it is one appointment drawn twice because a grid of hours has nowhere else to put it.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeEvent {
    pub id: String,
    pub title: String,
    pub start_hour: i32,
    pub start_min: i32,
    pub duration_min: i32,
    /// Column, 0 = the first day of the range shown. In the day view there is one column.
    pub day_index: i32,
    pub color_index: usize,
}

/// A stored event as an action that names one has to see it.
///
/// Separate from [`SourceEvent`] because naming an event and drawing it are different jobs: this
/// one carries no colour and is never placed on a grid, and it exists so that resolving a name can
/// be tested without a desktop.
#[derive(Clone, Debug, PartialEq)]
pub struct EventRef {
    /// The id the store gave the event.
    pub id: String,
    pub title: String,
    /// ISO 8601 as the store keeps it.
    pub start: String,
    pub end: String,
    pub is_all_day: bool,
}

/// What a `title` and a `date` named.
///
/// `Ambiguous` exists because the alternative is a guess, and a guess in a delete removes the
/// wrong appointment — the same shape as the bug 617dac9 fixed from the other side, where every
/// row of a day carried the id 0 and the trash icon on any of them deleted the first. Two events
/// called "standup" on one Tuesday is an ordinary thing for a calendar to hold, and the only
/// honest answer is to hand both back with their ids and let the caller say which.
#[derive(Clone, Debug, PartialEq)]
pub enum Named {
    /// Exactly one event answers to that name on that day.
    One(EventRef),
    /// Nothing does.
    None,
    /// More than one does, in the order the store returned them.
    Ambiguous(Vec<EventRef>),
}

/// Everything the week view draws, for one week.
#[derive(Clone, Debug, PartialEq)]
pub struct WeekView {
    /// The day the week starts on.
    pub start: NaiveDate,
    /// The Saturday it ends on, inclusive.
    pub end: NaiveDate,
    /// Seven column headers, "Sun 20".
    pub labels: Vec<String>,
    /// Blocks, ordered by column then by time, so reading them is reading the week.
    pub events: Vec<TimeEvent>,
    /// The titles of the all-day events, one list per column. They are not on the grid -- see
    /// `segments` -- and this is what says so.
    pub all_day: Vec<Vec<String>>,
}

/// Everything the day view draws, for one day.
#[derive(Clone, Debug, PartialEq)]
pub struct DayView {
    pub date: NaiveDate,
    /// "Sunday, 20 September 2026", with the all-day count appended when there is one.
    pub title: String,
    pub events: Vec<TimeEvent>,
    pub all_day: Vec<String>,
}

// ── Dates ────────────────────────────────────────────────────────────

pub fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "?",
    }
}

pub(crate) fn weekday_short(date: NaiveDate) -> &'static str {
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        [date.weekday().num_days_from_sunday() as usize]
}

pub(crate) fn month_short(month: u32) -> &'static str {
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        [month as usize - 1]
}

pub(crate) fn weekday_long(date: NaiveDate) -> &'static str {
    ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]
        [date.weekday().num_days_from_sunday() as usize]
}

pub fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month >= 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|first_of_next| first_of_next.pred_opt())
        .map(|d| d.day())
        .unwrap_or(30)
}

/// The first day of the week containing `date`.
pub fn week_start(date: NaiveDate, first: WeekStart) -> NaiveDate {
    first.week_start_of(date)
}

/// The first and last day of the week containing `date`, both inclusive.
pub fn week_bounds(date: NaiveDate, first: WeekStart) -> (NaiveDate, NaiveDate) {
    let start = week_start(date, first);
    (start, start + Duration::days(6))
}

/// The date the week and day views are about.
///
/// `selected-day` is 0 when the person has moved to another month without picking a day in it,
/// so the week and day views fall back to the first of the month shown rather than to nothing.
/// A day past the end of a short month is clamped, which is what happens when the 31st is
/// selected and the month is stepped back into September.
pub fn selected_date(year: i32, month: u32, selected_day: i32) -> NaiveDate {
    let last = last_day_of_month(year, month);
    let day = if selected_day < 1 { 1 } else { (selected_day as u32).min(last) };
    NaiveDate::from_ymd_opt(year, month, day)
        .or_else(|| NaiveDate::from_ymd_opt(year, month, 1))
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).expect("1970-01-01 is a date"))
}

/// The dates the store has to be asked for, for the views that are on screen.
///
/// The month is always in it: the month grid and `describe`'s `days_with_events` are kept current
/// whichever view is showing. The week is added because the app used to fetch exactly one month,
/// so a week straddling a month boundary silently dropped every event on the far side of it --
/// the first week of October would have shown nothing from 28..30 September and said nothing
/// about it either.
pub fn visible_range(
    year: i32,
    month: u32,
    view: ViewMode,
    selected_day: i32,
    week: WeekStart,
) -> (NaiveDate, NaiveDate) {
    let first = NaiveDate::from_ymd_opt(year, month, 1)
        .unwrap_or_else(|| selected_date(year, month, 1));
    let last = NaiveDate::from_ymd_opt(year, month, last_day_of_month(year, month))
        .unwrap_or(first);
    match view {
        ViewMode::Month => (first, last),
        ViewMode::Week => {
            let start = week_start(selected_date(year, month, selected_day), week);
            (first.min(start), last.max(start + Duration::days(6)))
        }
        ViewMode::Day => {
            let day = selected_date(year, month, selected_day);
            (first.min(day), last.max(day))
        }
    }
}

/// Parse what the store keeps, and nothing else.
///
/// `YYYY-MM-DDTHH:MM:SS` is what the service writes; the minute form and the bare date are
/// accepted because a file on disk can be edited by hand. Anything else is `None`, and an event
/// whose time cannot be read is left off the grid rather than guessed at or panicked over.
pub fn parse_datetime(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M"))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })
}

pub(crate) fn parse_clock(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S"))
        .ok()
}

/// How the timezone strip in the new-event form reads.
///
/// The offset is all the app can honestly know: `chrono::Local` carries no zone name, and there
/// is no time-zone database in this dependency set. So it says the offset and stops -- the field
/// used to be an empty string nothing ever wrote, and the form said nothing about which clock
/// the time typed into it belongs to.
pub fn timezone_label(utc_offset_seconds: i32) -> String {
    let sign = if utc_offset_seconds < 0 { '-' } else { '+' };
    let magnitude = utc_offset_seconds.abs();
    format!(
        "Times are local, UTC{}{:02}:{:02}",
        sign,
        magnitude / 3600,
        (magnitude % 3600) / 60
    )
}

/// The date this machine calls today, as `describe` says it: "2026-09-23 Wednesday".
///
/// The describe already carried the month, the year and the selected day, but nothing that
/// said which day today is — and "what is on my calendar today" is the question a calendar
/// exists to answer. The mind that asked it used to run `date` through the shell, which is
/// graded sensitive, so learning the day raised an approval card for the person (#207). The
/// shell's describe carries the whole line as `now`; this is the part of it a caller of the
/// calendar needs.
pub fn today_line(date: NaiveDate) -> String {
    format!("{} {}", date.format("%Y-%m-%d"), weekday_long(date))
}
