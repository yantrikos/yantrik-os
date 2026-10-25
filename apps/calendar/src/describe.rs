//! What the Mind reads: the same keys, values and sentences Calendar has always given.
//!
//! "What is on my calendar today" should never be answered by photographing a month grid and
//! asking a vision model to read the numbers. Everything here is derived from the events in hand
//! through the same functions the window is drawn with (`events.rs`, `views`), so this is that
//! drawing read back in words rather than a second account of it. It is derived rather than read
//! off the Slint rows because the rows carry no id, and an event a caller cannot name is an event
//! it cannot move or remove.

use serde_json::{json, Value};
use yantrik_app::control::View;

use crate::events::{self, CalEvent};
use crate::follow;
use crate::model::Model;
use crate::ownership;
use crate::views::{self, ViewMode};
use crate::who::{Caller, UNIDENTIFIED};

pub fn view(m: &Model) -> View {
    // Before answering, not after (`follow.rs`).
    let fresh = follow::before_describe(m);
    let events: &[CalEvent] = fresh.as_deref().unwrap_or(&m.events);

    // Who this caller was verified to be, and what that is worth against each event's creator
    // record: the answer `delete_own_event` would give, before it is asked (#201). Per describe,
    // because the caller changes per describe, and from the same two facts the action reads.
    let me = m.machine.who.identify(&Caller::now());

    let month = m.month_title();
    let day = m.selected_day;
    let selected = m.selected_date();
    let (week_start, week_end) = m.week_bounds();

    // Which days of the month have anything on them. Six numbers instead of a picture of a grid,
    // and it is what a caller planning around the month actually needs.
    let today_day = events::today_in(m.year, m.month, m.today.date());
    let busy: Vec<Value> = events::month_cells(m.year, m.month, events, today_day, m.week_start)
        .into_iter()
        .filter(|c| c.is_current_month && c.event_count > 0)
        .map(|c| json!({ "day": c.day as i32, "events": c.event_count }))
        .collect();

    let on_day: Vec<Value> = events::events_on_day(events, m.year, m.month, day)
        .into_iter()
        .map(|e| {
            json!({
                // The id the store gave it, which is what `delete_event` and `update_event` take.
                // A caller told an event's title and time and not its id can read this calendar
                // and cannot act on it.
                "id": e.id,
                "title": e.title,
                "date": format!("{:04}-{:02}-{:02}", m.year, m.month, day),
                "time": events::time_text(e),
                "all_day": e.is_all_day,
                // Whether this caller created this event, which is whether `delete_own_event`
                // will take it off without anybody being asked.
                "may_delete_unasked": ownership::may_delete_unasked(e.creator.as_deref(), me.as_deref()),
            })
        })
        .collect();

    // A mind asking what is on the calendar used to be told about the month whichever view was
    // up, because the month was the only view with anything in it. Week and Day answer as
    // themselves: the week says its range and what each of its seven days holds; the day says its
    // date and its events in order.
    let source = events::source_events(events);
    let week = views::week_view(&source, selected, m.week_start);
    let day_view = views::day_view(&source, selected);

    let summary = match m.view {
        ViewMode::Week => format!(
            "Calendar — week of {week_start} to {week_end}, {} on the grid",
            events::count_phrase(week.events.len(), "event", "events")
        ),
        ViewMode::Day => format!(
            "Calendar — {}, {} on the grid",
            day_view.title,
            events::count_phrase(day_view.events.len(), "event", "events")
        ),
        ViewMode::Month if on_day.is_empty() => format!("Calendar — {month}, nothing on day {day}"),
        ViewMode::Month if on_day.len() == 1 => format!(
            "Calendar — {month}, one thing on day {day}: {}",
            on_day[0]["title"].as_str().unwrap_or_default()
        ),
        ViewMode::Month => format!("Calendar — {month}, {} things on day {day}", on_day.len()),
    };

    // What each event id in hand names, as a person would say it. The id is the reliable way to
    // point at an event — `delete_event` recommends it and the approval grant is bound to it
    // exactly — but a card that asks permission with a uuid alone asks a question nobody can
    // answer (#54), and the shell reads this from the same `app.describe` it already reads the
    // grade and the purpose from. Every event in hand, not just the selected day's, and bounded:
    // only the loaded visible range, never the whole store, and at most `views::NAMING_CAP`
    // entries, oldest dropped (see `views::naming_index`).
    let naming: serde_json::Map<String, Value> = views::naming_index(&events::event_refs(events))
        .into_iter()
        .map(|(id, name)| (id, Value::String(name)))
        .collect();

    let out = View::new(summary)
        .with("month", month)
        .with("year", m.year)
        .with("month_number", m.month as i64)
        .with("selected_day", day)
        // Which day today is — "2026-09-23 Wednesday". The month and the selected day above say
        // where the screen is looking, not which day the machine is living in, and a mind that
        // needed the second one ran `date` through the shell: sensitive, so learning the day
        // raised an approval card (#207).
        .with("today", views::today_line(m.machine.clock.now().date()))
        .with("view", m.view.as_str())
        // The first column of the month grid and the week view, so a caller asking for "the week
        // of Monday 28 September" knows what the window's week is. To read a range without
        // caring, use `events_between`.
        .with("week_starts_on", m.week_start.as_str())
        .with("events_on_selected_day", Value::Array(on_day))
        .with("naming", Value::Object(naming))
        .with("days_with_events", Value::Array(busy))
        .with("events_this_month", events::events_in_month(events, m.year, m.month) as i64)
        // What the person is being told went wrong, if anything. A caller that just failed to
        // save should be able to read the reason rather than infer it.
        .with("notice", m.notice.text.clone())
        // The identity every `may_delete_unasked` in this describe was answered against, so a
        // caller that expected to own something and sees `false` can tell whether the event has
        // no record or the record names somebody else — and how it itself was recorded.
        .with("you", me.clone().unwrap_or_else(|| UNIDENTIFIED.to_string()));

    match m.view {
        ViewMode::Week => {
            let per_day: Vec<Value> = week
                .labels
                .iter()
                .enumerate()
                .map(|(column, label)| {
                    let blocks: Vec<Value> = week
                        .events
                        .iter()
                        .filter(|b| b.day_index == column as i32)
                        .map(|b| block_json(b, events, me.as_deref()))
                        .collect();
                    json!({ "day": label, "events": blocks })
                })
                .collect();
            out.with("week_start", week_start.to_string())
                .with("week_end", week_end.to_string())
                .with("week", Value::Array(per_day))
        }
        ViewMode::Day => {
            let blocks: Vec<Value> = day_view.events.iter().map(|b| block_json(b, events, me.as_deref())).collect();
            out.with("day_shown", day_view.title.clone()).with("events_on_day_grid", Value::Array(blocks))
        }
        ViewMode::Month => out,
    }
}

/// One block of a time grid as a caller reads it: which event, what, when, and for how long.
///
/// The id is here so that a mind reading the week can name an event back to `delete_event` or
/// `update_event`. It could see one and not say which, which is how a surface comes to be
/// readable and not actable. An event running past midnight is two blocks carrying one id.
///
/// The grids carry ids, so they carry the same mark the day list does: whether this caller
/// created the event and may take it off through `delete_own_event` without anybody being asked
/// (#201). The creator is looked up in the events in hand rather than carried through `views` —
/// the grid arithmetic has no business knowing who made anything.
fn block_json(block: &views::TimeEvent, events: &[CalEvent], me: Option<&str>) -> Value {
    let creator = events.iter().find(|e| e.id == block.id).and_then(|e| e.creator.as_deref());
    json!({
        "id": block.id,
        "title": block.title,
        "at": format!("{:02}:{:02}", block.start_hour, block.start_min),
        "minutes": block.duration_min,
        "may_delete_unasked": ownership::may_delete_unasked(creator, me),
    })
}
