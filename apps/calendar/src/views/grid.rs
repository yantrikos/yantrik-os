//! Placing events on the week and day grids: one block per day an event covers, the all-day
//! events beside the grid, and the column headers.

use chrono::{Datelike, Duration, NaiveDate, Timelike};

use super::{
    month_name, parse_datetime, week_bounds, weekday_long, weekday_short, DayView, SourceEvent,
    TimeEvent, WeekStart, WeekView,
};

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
        day += Duration::days(1);
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
        day += Duration::days(1);
    }
    out
}

fn ordered(mut events: Vec<TimeEvent>) -> Vec<TimeEvent> {
    events.sort_by_key(|e| (e.day_index, e.start_hour, e.start_min, e.title.clone()));
    events
}

/// The week containing `selected`, seven days from the first day of the week.
///
/// Events outside it are not in the result at all: the columns are the week, and something on the
/// following Monday has nowhere to go.
pub fn week_view(events: &[SourceEvent], selected: NaiveDate, first: WeekStart) -> WeekView {
    let (start, end) = week_bounds(selected, first);

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
