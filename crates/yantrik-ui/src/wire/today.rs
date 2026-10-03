//! Today: what the clock's popover shows, read when it opens.
//!
//! Opening is not wired here. A click on the clock, Super+N and `act shell open_today` all set
//! `today-open`, and the one hook they reach is `shell-overlay-opened` (see `shell_overlays`), which
//! calls [`refresh_on_open`]. Nothing here runs while the popover is shut: no timer, no poll.
//!
//! The date and the month grid are drawn from the clock at once, so the panel never opens empty.
//! The calendar is the calendar service's, and the shell does not call a service on its UI thread
//! (a service that is down costs its whole timeout): the month's events are asked for on a worker
//! with a short timeout and handed back with `upgrade_in_event_loop`. Until they arrive the panel says
//! "Reading the calendar…"; if the service does not answer it says so, which is a different thing
//! from a day with nothing on it. The notifications are not read here: the poll keeps
//! `TodayState.notifications` current (`notifications::sync_to_ui`).
//!
//! One rule decides which day an event belongs to, and it is the Calendar app's: the day its start
//! falls on. The grid's dots and the list under it come from the same count, so they cannot disagree
//! with each other or with the app.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate};
use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_ipc_contracts::calendar::{method, month_grid, CalendarEvent, EventsParams};

use crate::{App, CalendarDay, TodayEvent, TodayState};

/// How long the calendar gets to answer. The panel is open and waiting: past this it says the
/// calendar did not answer, and the next open asks again.
const CALENDAR_TIMEOUT: Duration = Duration::from_secs(2);

/// Which open the answer on the wire belongs to. An answer for an earlier open is dropped, so a slow
/// calendar cannot paint last month over this one.
static OPEN: AtomicU64 = AtomicU64::new(0);

/// A fresh open: the date and the month now, the events when the calendar answers.
pub(super) fn refresh_on_open(ui: &App) {
    let now = Local::now();
    let today = now.date_naive();
    let state = ui.global::<TodayState>();
    state.set_weekday(now.format("%A").to_string().into());
    state.set_date_line(now.format("%-d %B %Y").to_string().into());
    state.set_month_title(now.format("%B %Y").to_string().into());
    state.set_days(days_model(today, &[]));
    state.set_events_state("loading".into());
    state.set_events(ModelRc::default());

    let open = OPEN.fetch_add(1, Ordering::SeqCst) + 1;
    let weak = ui.as_weak();
    let spawned = std::thread::Builder::new().name("yos-today-calendar".into()).spawn(move || {
        let answer = fetch_month(today);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if OPEN.load(Ordering::SeqCst) != open {
                return;
            }
            let state = ui.global::<TodayState>();
            match answer {
                Ok(events) => {
                    state.set_days(days_model(today, &events));
                    state.set_events(ModelRc::new(VecModel::from(
                        todays_events(&events, today)
                            .into_iter()
                            .map(|(title, time_text)| TodayEvent { title: title.into(), time_text: time_text.into() })
                            .collect::<Vec<_>>(),
                    )));
                    state.set_events_state("ok".into());
                }
                Err(why) => {
                    tracing::warn!(%why, "Today could not read the calendar");
                    state.set_events_state("unavailable".into());
                }
            }
        });
    });
    if let Err(why) = spawned {
        tracing::warn!(%why, "Today could not start its calendar read");
        ui.global::<TodayState>().set_events_state("unavailable".into());
    }
}

/// The month's events, from the calendar service. Runs on a worker.
fn fetch_month(today: NaiveDate) -> Result<Vec<CalendarEvent>, String> {
    let first = today.with_day(1).ok_or("no first of the month")?;
    let last = month_last(today);
    let client = yantrik_app_runtime::service::client("calendar")?.with_timeout(CALENDAR_TIMEOUT);
    let params = EventsParams {
        start_date: format!("{}T00:00:00", first.format("%Y-%m-%d")),
        end_date: format!("{}T23:59:59", last.format("%Y-%m-%d")),
    };
    let reply = client
        .call(method::EVENTS, serde_json::to_value(params).map_err(|e| e.to_string())?)
        .map_err(|e| e.message)?;
    serde_json::from_value(reply).map_err(|e| e.to_string())
}

fn month_last(day: NaiveDate) -> NaiveDate {
    let (y, m) = if day.month() == 12 { (day.year() + 1, 1) } else { (day.year(), day.month() + 1) };
    NaiveDate::from_ymd_opt(y, m, 1).and_then(|d| d.pred_opt()).unwrap_or(day)
}

/// The grid for `today`'s month, with a dot under each day that has an event starting on it.
fn days_model(today: NaiveDate, events: &[CalendarEvent]) -> ModelRc<CalendarDay> {
    let counts = |d: u32| {
        let prefix = format!("{:04}-{:02}-{:02}", today.year(), today.month(), d);
        events.iter().filter(|e| e.start.starts_with(&prefix)).count() as i32
    };
    ModelRc::new(VecModel::from(
        month_grid(today.year(), today.month(), &counts, Some(today))
            .into_iter()
            .map(|c| CalendarDay {
                day_number: c.day as i32,
                is_today: c.is_today,
                is_selected: false,
                is_current_month: c.is_current_month,
                has_events: c.event_count > 0,
                event_count: c.event_count,
            })
            .collect::<Vec<_>>(),
    ))
}

/// Today's events as `(title, when)`: all-day ones first, then by start time.
pub(crate) fn todays_events(events: &[CalendarEvent], today: NaiveDate) -> Vec<(String, String)> {
    let prefix = today.format("%Y-%m-%d").to_string();
    let mut mine: Vec<&CalendarEvent> = events.iter().filter(|e| e.start.starts_with(&prefix)).collect();
    mine.sort_by(|a, b| (!a.is_all_day, &a.start).cmp(&(!b.is_all_day, &b.start)));
    mine.into_iter().map(|e| (e.title.clone(), when(e))).collect()
}

/// When an event runs, as a person reads it: "All day", or "09:30 – 10:00". Hours and minutes; the
/// store keeps seconds because it keeps ISO timestamps, and nobody reads their day in them.
fn when(event: &CalendarEvent) -> String {
    if event.is_all_day {
        return "All day".to_string();
    }
    let clock = |iso: &str| iso.split('T').nth(1).unwrap_or("").split(':').take(2).collect::<Vec<_>>().join(":");
    let (start, end) = (clock(&event.start), clock(&event.end));
    // An end on another day is not "10:00" today, so it is left off rather than shown wrong.
    if end.is_empty() || event.end.get(..10) != event.start.get(..10) {
        start
    } else {
        format!("{start} – {end}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(title: &str, start: &str, end: &str, all_day: bool) -> CalendarEvent {
        CalendarEvent {
            id: title.into(),
            title: title.into(),
            description: String::new(),
            start: start.into(),
            end: end.into(),
            is_all_day: all_day,
            location: None,
            attendees: Vec::new(),
            recurrence: None,
            calendar_id: "default".into(),
            remote_id: None,
            creator: None,
            reminder_minutes: 10,
        }
    }

    fn oct2() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
    }

    /// Only events that start today, all-day first and then in the order they happen.
    #[test]
    fn today_lists_its_own_events_in_the_order_they_happen() {
        let events = [
            event("Dentist", "2026-10-02T16:00:00", "2026-10-02T17:00:00", false),
            event("Tomorrow", "2026-10-03T09:00:00", "2026-10-03T10:00:00", false),
            event("Stand-up", "2026-10-02T09:30:00", "2026-10-02T09:45:00", false),
            event("Rent due", "2026-10-02T00:00:00", "2026-10-02T23:59:59", true),
        ];
        let got = todays_events(&events, oct2());
        let titles: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(titles, ["Rent due", "Stand-up", "Dentist"]);
        assert_eq!(got[0].1, "All day");
        assert_eq!(got[1].1, "09:30 – 09:45");
    }

    /// An event that ends on another day does not get an end time that belongs to that day.
    #[test]
    fn an_event_that_runs_past_midnight_shows_only_when_it_starts() {
        let night = event("Deploy", "2026-10-02T23:00:00", "2026-10-03T01:00:00", false);
        assert_eq!(when(&night), "23:00");
    }

    /// The grid's dots and the list are one count: a day with an event starting on it has a dot.
    #[test]
    fn the_grid_dots_the_days_events_start_on() {
        let events = [event("Stand-up", "2026-10-02T09:30:00", "2026-10-02T09:45:00", false)];
        let model = days_model(oct2(), &events);
        let cells: Vec<CalendarDay> = slint::Model::iter(&model).collect();
        let with: Vec<i32> = cells.iter().filter(|c| c.has_events).map(|c| c.day_number).collect();
        assert_eq!(with, [2]);
        assert_eq!(cells.iter().filter(|c| c.is_today).count(), 1);
    }

    #[test]
    fn the_last_day_of_a_month_is_found_across_a_year_end() {
        assert_eq!(month_last(NaiveDate::from_ymd_opt(2026, 12, 9).unwrap()), NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
        assert_eq!(month_last(NaiveDate::from_ymd_opt(2028, 2, 1).unwrap()), NaiveDate::from_ymd_opt(2028, 2, 29).unwrap());
    }

    /// The calendar is asked on a worker with a timeout, never on the UI thread.
    #[test]
    fn the_calendar_is_read_off_the_ui_thread_with_a_timeout() {
        let src = include_str!("today.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let open = &code[code.find("pub(super) fn refresh_on_open").unwrap()..code.find("fn fetch_month").unwrap()];
        assert!(open.contains("std::thread::Builder"), "the read runs on a worker");
        assert!(open.contains("upgrade_in_event_loop"), "and is handed back to the UI thread");
        assert!(open.find("spawn").unwrap() < open.find("fetch_month(today)").unwrap(), "fetched inside the worker, not before it");
        assert!(code.contains("with_timeout(CALENDAR_TIMEOUT)"), "with the short timeout");
        assert!(!code.contains("Timer"), "Today runs no timer");
    }
}
