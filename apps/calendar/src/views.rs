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
//! a desktop or a service -- `tests/calendar-core` includes this file directly. `main.rs` converts
//! these plain structs into `CalendarTimeEvent` rows and nothing else.
//!
//! The week starts on Sunday. That is not a preference: the month grid's own header row in
//! `calendar.slint` reads Sun..Sat and `day_of_week_for_date` fills it with
//! `num_days_from_sunday`, so a week view starting anywhere else would put the same date in two
//! different columns in two views of the same calendar.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

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
    /// The Sunday the week starts on.
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

fn weekday_short(date: NaiveDate) -> &'static str {
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        [date.weekday().num_days_from_sunday() as usize]
}

fn month_short(month: u32) -> &'static str {
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        [month as usize - 1]
}

fn weekday_long(date: NaiveDate) -> &'static str {
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

/// The Sunday on or before `date`.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_sunday() as i64)
}

/// The Sunday and the Saturday of the week containing `date`, both inclusive.
pub fn week_bounds(date: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = week_start(date);
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
) -> (NaiveDate, NaiveDate) {
    let first = NaiveDate::from_ymd_opt(year, month, 1)
        .unwrap_or_else(|| selected_date(year, month, 1));
    let last = NaiveDate::from_ymd_opt(year, month, last_day_of_month(year, month))
        .unwrap_or(first);
    match view {
        ViewMode::Month => (first, last),
        ViewMode::Week => {
            let start = week_start(selected_date(year, month, selected_day));
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

fn parse_clock(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S"))
        .ok()
}

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

/// The events on `date` whose title is `title`, compared without case or surrounding space.
///
/// `date` is matched as a prefix of the stored start, which is what the rest of this app does with
/// a day; it has to arrive as a full `YYYY-MM-DD` or it will match a range of days instead of one,
/// and the action that calls this checks that before asking.
///
/// Exact titles only. A substring match would make "delete standup" remove "standup with the
/// platform team" on a day holding both, and the caller who meant the other one would have no way
/// to tell that from the answer.
pub fn named_on(events: &[EventRef], title: &str, date: &str) -> Named {
    let wanted = title.trim().to_lowercase();
    let day = date.trim();
    let mut found: Vec<EventRef> = events
        .iter()
        .filter(|e| e.start.starts_with(day) && e.title.trim().to_lowercase() == wanted)
        .cloned()
        .collect();
    match found.len() {
        0 => Named::None,
        1 => Named::One(found.remove(0)),
        _ => Named::Ambiguous(found),
    }
}

/// One event the way a person would name it: "Dentist, Fri 25 Sep 13:00".
///
/// [`named_on`] answers name and date to id; this answers id back, because the other way an
/// action names an event is the store's uuid — the reliable one, which `delete_event` recommends
/// and which an approval grant is bound to byte for byte. A card that asks permission with a
/// uuid alone asks a question nobody can answer (#54), and the app is the only thing on the
/// machine that knows what its own id stands for. It publishes this through `describe` as an
/// index, and the shell puts the hit beside the arguments on the card.
///
/// An all-day event says so rather than reading out the midnight its row is stored at. A start
/// that will not parse contributes no time at all — the title alone is short, and a made-up
/// time in the sentence a person decides on is worse than that.
pub fn name_event(event: &EventRef) -> String {
    let Some(start) = parse_datetime(&event.start) else {
        return event.title.clone();
    };
    let day = start.date();
    let head = format!("{}, {} {} {}", event.title, weekday_short(day), day.day(), month_short(day.month()));
    if event.is_all_day {
        format!("{head}, all day")
    } else {
        format!("{head} {}", start.format("%H:%M"))
    }
}

/// How many entries the naming index publishes at most.
///
/// `describe` is read by agents, and an unbounded table inside one reply is exactly what those
/// replies are told to avoid: a year of a busy calendar would ride along with every "what is on
/// today?" A fixed cap keeps the reply bounded whatever the loaded range holds, and 200 is far
/// more than a visible range of weeks carries — a caller that hit it is a caller whose window has
/// been sitting on a very full month, and even then it keeps the newest names, which are the ones
/// an action taken now was read from.
pub const NAMING_CAP: usize = 200;

/// The id→name index `describe` publishes, from the events the app has in hand (#54).
///
/// Bounded both ways, deliberately. Only the events passed in get a name — the app passes the
/// loaded visible range, the same events the rest of the describe reply is already derived from,
/// so the index never widens what `describe` covers. And at most [`NAMING_CAP`] entries: past
/// that the oldest starts are dropped, because the id an action carries was read from a recent
/// view. A start that will not parse counts as oldest of all — a garbage row should not displace
/// a real event from the index, and [`name_event`] still names it by title if it survives.
pub fn naming_index(events: &[EventRef]) -> Vec<(String, String)> {
    let mut by_start: Vec<&EventRef> = events.iter().collect();
    // `None` sorts before every `Some`, so unparsable starts sit at the old end. Stable, so
    // events the store returned in one order keep it within a shared (or absent) start.
    by_start.sort_by_key(|e| parse_datetime(&e.start));
    if by_start.len() > NAMING_CAP {
        by_start = by_start.split_off(by_start.len() - NAMING_CAP);
    }
    by_start
        .into_iter()
        .map(|e| (e.id.clone(), name_event(e)))
        .collect()
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

// ── Placing events on a grid ─────────────────────────────────────────

/// The blocks one event contributes to the days `first..=last`, one block per day it covers.
///
/// An event running past midnight is clipped at the day boundary and continues as a second block
/// on the next day, which is the only way a 22:00-01:00 meeting can be on a grid of hours at all:
/// a single block would have to be drawn 180 minutes tall in a column that ends at 1440.
///
/// Refused here rather than drawn wrong: an all-day event, which has no hour to be placed at; an
/// event whose start or end will not parse; an event that ends before it starts. All three exist
/// only in hand-written files -- the store refuses to write them -- and none of them is worth a
/// panic in a redraw.
fn segments(event: &SourceEvent, first: NaiveDate, last: NaiveDate) -> Vec<TimeEvent> {
    if event.is_all_day {
        return Vec::new();
    }
    let (Some(start), Some(end)) = (parse_datetime(&event.start), parse_datetime(&event.end))
    else {
        return Vec::new();
    };
    if end < start {
        return Vec::new();
    }
    // An event with no duration is a moment, and a moment is worth a marker. A zero-length tail
    // left over from clipping -- 22:00 to exactly midnight -- is not, and is dropped below.
    let is_moment = end == start;

    let mut out = Vec::new();
    let mut day = start.date().max(first);
    let through = end.date().min(last);
    while day <= through {
        let (Some(day_start), Some(next_day_start)) =
            (day.and_hms_opt(0, 0, 0), (day + Duration::days(1)).and_hms_opt(0, 0, 0))
        else {
            break;
        };
        let from = start.max(day_start);
        let to = end.min(next_day_start);
        let minutes = (to - from).num_minutes() as i32;
        if minutes > 0 || is_moment {
            out.push(TimeEvent {
                id: event.id.clone(),
                title: event.title.clone(),
                start_hour: from.hour() as i32,
                start_min: from.minute() as i32,
                duration_min: minutes.max(0),
                day_index: (day - first).num_days() as i32,
                color_index: event.color_index,
            });
        }
        day = day + Duration::days(1);
    }
    out
}

/// The columns, within `first..=last`, an all-day event sits on.
fn all_day_columns(event: &SourceEvent, first: NaiveDate, last: NaiveDate) -> Vec<i32> {
    let Some(start) = parse_datetime(&event.start) else {
        return Vec::new();
    };
    let end = parse_datetime(&event.end).map(|e| e.date()).unwrap_or(start.date());
    let mut out = Vec::new();
    let mut day = start.date().max(first);
    let through = end.max(start.date()).min(last);
    while day <= through {
        out.push((day - first).num_days() as i32);
        day = day + Duration::days(1);
    }
    out
}

fn ordered(mut events: Vec<TimeEvent>) -> Vec<TimeEvent> {
    events.sort_by_key(|e| (e.day_index, e.start_hour, e.start_min, e.title.clone()));
    events
}

/// The week containing `selected`, Sunday through Saturday.
///
/// Events outside it are not in the result at all: the columns are the week, and something on the
/// following Monday has nowhere to go.
pub fn week_view(events: &[SourceEvent], selected: NaiveDate) -> WeekView {
    let (start, end) = week_bounds(selected);

    let mut blocks = Vec::new();
    let mut all_day: Vec<Vec<String>> = vec![Vec::new(); 7];
    for event in events {
        if event.is_all_day {
            for column in all_day_columns(event, start, end) {
                if let Some(slot) = all_day.get_mut(column as usize) {
                    slot.push(event.title.clone());
                }
            }
        } else {
            blocks.extend(segments(event, start, end));
        }
    }

    // The column header carries the all-day count because the grid cannot: an all-day event has
    // no hour, and inventing one -- 00:00, or the top of the working day -- would put a thing on
    // the calendar at a time nobody chose. The agenda in the sidebar already spells out the
    // all-day events of the selected day; this is how the other six days say they have one.
    let labels = (0..7)
        .map(|i| {
            let date = start + Duration::days(i as i64);
            let count = all_day[i].len();
            if count == 0 {
                format!("{} {}", weekday_short(date), date.day())
            } else {
                format!("{} {} · {} all day", weekday_short(date), date.day(), count)
            }
        })
        .collect();

    WeekView { start, end, labels, events: ordered(blocks), all_day }
}

/// One day, on its own grid. Column 0 is the only column.
pub fn day_view(events: &[SourceEvent], selected: NaiveDate) -> DayView {
    let mut blocks = Vec::new();
    let mut all_day = Vec::new();
    for event in events {
        if event.is_all_day {
            if !all_day_columns(event, selected, selected).is_empty() {
                all_day.push(event.title.clone());
            }
        } else {
            blocks.extend(segments(event, selected, selected));
        }
    }

    let mut title = format!(
        "{}, {} {} {}",
        weekday_long(selected),
        selected.day(),
        month_name(selected.month()),
        selected.year()
    );
    if !all_day.is_empty() {
        title.push_str(&format!(" · {} all day", all_day.len()));
    }

    DayView { date: selected, title, events: ordered(blocks), all_day }
}
