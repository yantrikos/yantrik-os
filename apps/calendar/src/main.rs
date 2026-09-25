//! Yantrik Calendar — standalone app binary.
//!
//! Communicates with `calendar-service` via JSON-RPC IPC.
//! Falls back to local event storage when service is unavailable.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_contracts::calendar::{
    method, CalendarRevision, CreateEventParams, DeleteEventParams, EventsParams, GetEventParams,
    UpdateEventParams,
};
use yantrik_ipc_transport::{peer_identity, reach};

mod ownership;
mod views;
use views::ViewMode;

slint::include_modules!();

/// How long an event runs when nothing says otherwise.
const DEFAULT_EVENT_MINUTES: i32 = 60;

/// How often an open window asks the store whether anything has changed under it.
///
/// Twenty seconds, and only while the window is on screen. This is not a poll of the calendar —
/// it is a poll of two numbers (`calendar.revision`), and the month is re-listed only when they
/// move. A calendar is written at human speed by a person and at tool speed by a mind, and both
/// of those already redraw this window through the path that wrote; the timer is for the third
/// writer, which is somebody else's process, and twenty seconds of lag on an appointment nobody
/// in this window made is not worth a second of CPU.
const STORE_WATCH: std::time::Duration = std::time::Duration::from_secs(20);

/// Fill the agent rail from the day on screen.
///
/// Events come off the model the app already renders; memory comes from the companion when it
/// is reachable. Nothing is added to fill the column -- a day with nothing on it and no shell
/// behind it gets an empty rail, and the calendar is wider for it.
fn refresh_agent_rail(ui: &CalendarApp) {
    let mut context: Vec<AgentContextItem> = Vec::new();
    for e in ui.get_events_today().iter() {
        context.push(AgentContextItem {
            id: format!("event:{}", e.id).into(),
            label: e.title.clone(),
            detail: e.time_text.clone(),
            source: "calendar".into(),
        });
    }
    ui.set_agent_context(ModelRc::new(VecModel::from(context)));

    let reach = companion::reach();
    let mut next: Vec<AgentSuggestion> = Vec::new();
    if reach == companion::Reach::Ready {
        next.push(AgentSuggestion {
            id: "explain".into(),
            label: "What does this day look like?".into(),
            detail: "shape of the day, and what to prepare".into(),
            icon: "spark".into(),
            running: ui.get_ai_is_working(),
            // An answer to read; nothing is moved, so nothing is proposed.
            proposes: false,
        });
    }
    if ui.get_selected_day() > 0 {
        next.push(AgentSuggestion {
            id: "today".into(),
            label: "Back to today".into(),
            detail: SharedString::new(),
            icon: "search".into(),
            running: false,
            proposes: false,
        });
    }
    ui.set_agent_suggestions(ModelRc::new(VecModel::from(next)));

    ui.set_agent_unavailable(match reach.hint() {
        Some(hint) => hint.into(),
        None => SharedString::new(),
    });
}

fn main() {
    init_tracing("yantrik-calendar");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("calendar") else { return };

    let app = CalendarApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    // Held here rather than dropped at the end of `wire`. A Slint timer stops when it is dropped,
    // and `let _keep = ...` inside the function that starts it is how System Monitor came to take
    // one reading of the machine and show it for the life of the window (b291cb2).
    let _store_watch = wire(&app);

    // The "now" line on the week and day grids, kept at now.
    //
    // Those grids are one pixel to the minute, so the line has to be told the minute as well as
    // the hour or it stands up to an hour away from where the person is; and a window left open
    // past midnight would go on drawing yesterday's line. A minute is as often as a line that
    // moves a pixel a minute can usefully be redrawn.
    let clock = slint::Timer::default();
    {
        let weak = app.as_weak();
        clock.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(60),
            move || {
                if let Some(ui) = weak.upgrade() {
                    set_clock(&ui);
                }
            },
        );
    }

    run_until_closed(&app, "yantrik-calendar");
}

/// Where "now" is, and which clock it is on.
///
/// The timezone strip in the new-event form says the offset and no more, because the offset is
/// all `chrono::Local` carries -- there is no zone database in this dependency set. It was an
/// empty string nothing ever wrote, so the form never said which clock a time typed into it
/// belonged to.
fn set_clock(ui: &CalendarApp) {
    let now = chrono::Local::now();
    ui.set_current_hour(now.hour() as i32);
    ui.set_current_minute(now.minute() as i32);
    ui.set_cal_timezone_display(views::timezone_label(now.offset().local_minus_utc()).into());
}

// ── Calendar state ───────────────────────────────────────────────────

#[derive(Clone)]
struct CalState {
    year: i32,
    month: u32,
    events: Vec<CalEvent>,
    /// The date range `events` was read for, or `None` when the last read failed and the next
    /// redraw should ask again rather than show an empty calendar for the life of the process.
    range: Option<(chrono::NaiveDate, chrono::NaiveDate)>,
    /// What the store was at when `events` was read, or `None` when that could not be asked.
    ///
    /// The window is not the only writer any more — the mind's own tools and Google sync go
    /// through the same service — so "the range has not changed" stopped being a reason to
    /// believe the events in hand are the events on disk. See `reread`.
    revision: Option<CalendarRevision>,
    /// How long the event the form is about should run.
    ///
    /// The form has no duration field -- it asks for a title, a date, a time and notes -- so a
    /// template's answer to "how long" waits here until Save. `DEFAULT_EVENT_MINUTES` otherwise.
    new_event_duration_min: i32,
}

#[derive(Clone, Debug)]
struct CalEvent {
    id: String,
    title: String,
    start: String,
    end: String,
    #[allow(dead_code)]
    notes: String,
    is_all_day: bool,
    /// Who created this event, as the store recorded it at creation — `None` for everything
    /// stored before the record existed and everything a window's own form made. Read by
    /// `describe` and by the own-creation rule in `ownership`; never written from this app.
    creator: Option<String>,
    /// Position in `PALETTE`, not a colour: the same event has to be coloured the same in the
    /// agenda list and on the week grid, and `views` -- which has no Slint in it -- carries this
    /// through the derivation.
    color_index: usize,
}

/// The colours an event is drawn in, by its position in what the store returned.
const PALETTE: [(u8, u8, u8); 5] = [
    (0x4E, 0x79, 0xA7),
    (0xF2, 0x8E, 0x2C),
    (0xE1, 0x57, 0x59),
    (0x76, 0xB7, 0xB2),
    (0x59, 0xA1, 0x4F),
];

fn palette(index: usize) -> slint::Color {
    let (r, g, b) = PALETTE[index % PALETTE.len()];
    slint::Color::from_rgb_u8(r, g, b)
}

// ── Service wrappers ─────────────────────────────────────────────────

/// Every event overlapping `from..=to`.
///
/// The range is a parameter because the week view needs one. The app asked for exactly the month
/// on screen, and a week straddling a month boundary is the common case at both ends of every
/// month: the first week of October would have been drawn with nothing on 28, 29 or 30 September
/// and said nothing about it. `views::visible_range` decides what to ask for.
fn fetch_events_in_range(
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> Result<Vec<CalEvent>, String> {
    let client = service::client("calendar")?;
    let params = EventsParams {
        start_date: format!("{}T00:00:00", from.format("%Y-%m-%d")),
        end_date: format!("{}T23:59:59", to.format("%Y-%m-%d")),
    };
    let result = client
        .call(method::EVENTS, serde_json::to_value(params).map_err(|e| e.to_string())?)
        .map_err(|e| e.message)?;
    let svc_events: Vec<yantrik_ipc_contracts::calendar::CalendarEvent> =
        serde_json::from_value(result).map_err(|e| e.to_string())?;

    Ok(svc_events.iter().enumerate().map(|(i, e)| CalEvent {
        id: e.id.clone(),
        title: e.title.clone(),
        start: e.start.clone(),
        end: e.end.clone(),
        notes: e.description.clone(),
        is_all_day: e.is_all_day,
        creator: e.creator.clone(),
        color_index: i,
    }).collect())
}

/// Everything stored on one day, read now rather than taken off the screen.
///
/// An action that names an event by title and date may well name a day the window is not showing,
/// and the events in hand are only the visible range. So the day is asked for.
fn fetch_events_on(date: chrono::NaiveDate) -> Result<Vec<CalEvent>, String> {
    fetch_events_in_range(date, date)
}

/// What the store is at, or `None` when the question could not be put.
///
/// Only asked of a service that is already listening. `service::client` starts the calendar if it
/// is down, and a window sitting idle must never be the reason a service comes up — that is the
/// lesson `meeting_prep` learned on the companion's side, where a playbook evaluated every think
/// cycle would have started the calendar because it ran. It also keeps this off the transport's
/// circuit breaker: a failed connect trips it for the next few seconds, and a check nobody asked
/// for should not be able to make the next real call fail.
fn fetch_revision() -> Option<CalendarRevision> {
    if !service::is_up("calendar") {
        return None;
    }
    SyncRpcClient::for_service("calendar")
        .call_typed(method::REVISION, &serde_json::json!({}))
        .ok()
}

/// Who is asking, in the one spelling the store records it under.
///
/// The agent an agent token belongs to, where the call carried one and the shell's reach file
/// knows it; else the program the kernel's peer credentials lead to — `peer_identity`'s walk,
/// which steps over our own `yos`/`yos-mcp` plumbing to the first thing a person would
/// recognise (#221). `None` when nothing could be established: a call from the window's own
/// form, a TCP dev connection, a `/proc` that said nothing.
///
/// Nothing here reads the request's arguments — a caller cannot say who it is, only be
/// recognised — and nothing is invented when the machine cannot tell: an event stored with no
/// creator is one nobody may delete unasked, and a caller with no identity deletes nothing
/// unasked. `ownership::may_delete_unasked` is the rule those two facts are worth.
fn requester() -> Option<String> {
    if let Some(token) = control::agent_token() {
        // A token the reach file does not know is no agent — the shell refuses a token whose
        // reach it cannot read before the call gets here at all — so this falls through to
        // the program rather than refusing again.
        if let Ok(Some(reach)) = reach::reach_of(&token) {
            return Some(ownership::agent_identity(&reach.agent));
        }
    }
    let who = control::caller()?;
    let name = peer_identity::resolve(Some(who.pid)).name();
    if name.is_empty() { None } else { Some(name) }
}

fn create_event_via_service(
    title: &str,
    start: &str,
    end: &str,
    notes: &str,
    is_all_day: bool,
) -> Result<String, String> {
    let client = service::client("calendar")?;
    let params = CreateEventParams {
        title: title.to_string(),
        start: start.to_string(),
        end: end.to_string(),
        description: notes.to_string(),
        location: None,
        color: String::new(),
        // The form has no field for either, and this is not the place to add one. `is_all_day`
        // comes from the caller because the contract carries it and the surface now offers it;
        // attendees have no way in from this app and are not invented here.
        is_all_day,
        attendees: Vec::new(),
        // Who is asking, verified, and the record a later `delete_own_event` is checked
        // against (#201). Inside a surface dispatch this is the caller the kernel and the
        // reach file establish; from the window's own form there is no caller on the socket
        // and nothing is recorded, so an event a person typed into the form is nobody's
        // "own" but a person's.
        creator: requester(),
    };
    let result = client
        .call(method::CREATE_EVENT, serde_json::to_value(params).map_err(|e| e.to_string())?)
        .map_err(|e| e.message)?;
    // The stored event, with the id the store gave it — the proof it landed, not a hope.
    let event: yantrik_ipc_contracts::calendar::CalendarEvent =
        serde_json::from_value(result).map_err(|e| e.to_string())?;
    Ok(event.id)
}

fn delete_event_via_service(event_id: &str) -> Result<(), String> {
    let client = service::client("calendar")?;
    let params = DeleteEventParams { id: event_id.to_string() };
    client
        .call(method::DELETE_EVENT, serde_json::to_value(params).map_err(|e| e.to_string())?)
        .map_err(|e| e.message)?;
    Ok(())
}

fn get_event_via_service(
    event_id: &str,
) -> Result<yantrik_ipc_contracts::calendar::CalendarEvent, String> {
    let client = service::client("calendar")?;
    client
        .call_typed(method::GET_EVENT, &GetEventParams { id: event_id.to_string() })
        .map_err(|e| e.message)
}

/// Whether the store still holds this event: `Ok(true)`, `Ok(false)`, or `Err` when the question
/// could not be put at all.
///
/// The distinction matters exactly once, immediately after a delete. A service that has gone away
/// answers a read with an error too, and reading that as "it is gone" would be a delete reporting
/// an outcome it never observed — the fabrication the whole of `design/calendar-2026-09-20.md` is
/// about, pointing the other way. The store refuses an unknown id with `-32602`, which is an
/// answer; a transport failure is `-32000`, which is not.
fn event_still_there(event_id: &str) -> Result<bool, String> {
    let client = service::client("calendar")?;
    let params =
        serde_json::to_value(GetEventParams { id: event_id.to_string() }).map_err(|e| e.to_string())?;
    match client.call(method::GET_EVENT, params) {
        Ok(_) => Ok(true),
        Err(e) if e.code == -32602 => Ok(false),
        Err(e) => Err(e.message),
    }
}

fn update_event_via_service(
    params: &UpdateEventParams,
) -> Result<yantrik_ipc_contracts::calendar::CalendarEvent, String> {
    let client = service::client("calendar")?;
    client.call_typed(method::UPDATE_EVENT, params).map_err(|e| e.message)
}

// ── Date helpers ─────────────────────────────────────────────────────

/// Column index of a date in the month grid: 0=Sunday .. 6=Saturday.
///
/// This MUST match the header row in calendar.slint, which is Sun-first. The previous
/// hand-rolled Zeller returned a Monday-first index and the grid used it as the number of
/// leading blanks, so every month was drawn one column to the left of the truth.
fn day_of_week_for_date(year: i32, month: u32, day: u32) -> u32 {
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .map(|d| d.weekday().num_days_from_sunday())
        .unwrap_or(0)
}

fn today() -> (i32, u32, u32) {
    let now = chrono::Local::now();
    (now.year(), now.month(), now.day())
}

use chrono::{Datelike, Timelike};

fn build_month_grid(year: i32, month: u32, events: &[CalEvent], today_day: Option<u32>) -> Vec<CalendarDay> {
    let first_dow = day_of_week_for_date(year, month, 1);
    let last = views::last_day_of_month(year, month);
    let mut cells = Vec::with_capacity(42);

    // Empty cells before month start
    for _ in 0..first_dow {
        cells.push(CalendarDay {
            day_number: 0,
            is_today: false,
            is_selected: false,
            is_current_month: false,
            has_events: false,
            event_count: 0,
        });
    }

    for d in 1..=last as i32 {
        let day_str = format!("{:04}-{:02}-{:02}", year, month, d);
        let ev_count = events.iter().filter(|e| e.start.starts_with(&day_str)).count() as i32;
        cells.push(CalendarDay {
            day_number: d,
            is_today: today_day == Some(d as u32),
            is_selected: false,
            is_current_month: true,
            has_events: ev_count > 0,
            event_count: ev_count,
        });
    }

    // Pad to 42 cells (6 weeks)
    while cells.len() < 42 {
        cells.push(CalendarDay {
            day_number: 0,
            is_today: false,
            is_selected: false,
            is_current_month: false,
            has_events: false,
            event_count: 0,
        });
    }
    cells
}

/// What is on one day, in the order the store returned it.
///
/// One list, read by three things that must not disagree: the agenda rows the sidebar draws, the
/// row index the trash icon hands to `delete-event`, and the events `describe` reports for the
/// selected day. They were derived separately, and the index was the thing that came apart.
fn events_on_day<'a>(
    events: &'a [CalEvent],
    year: i32,
    month: u32,
    day: i32,
) -> Vec<&'a CalEvent> {
    let prefix = format!("{:04}-{:02}-{:02}", year, month, day);
    events.iter().filter(|e| e.start.starts_with(&prefix)).collect()
}

/// When an event runs, as a person reads it.
///
/// Hours and minutes. The seconds are in the store because the store keeps ISO timestamps, and
/// nobody reading their own day needs "14:00:00 - 15:00:00".
fn time_text(event: &CalEvent) -> String {
    if event.is_all_day {
        return "All day".to_string();
    }
    let clock = |iso: &str| {
        iso.split('T').nth(1).unwrap_or("").split(':').take(2).collect::<Vec<_>>().join(":")
    };
    format!("{} – {}", clock(&event.start), clock(&event.end))
}

/// The agenda in the sidebar.
///
/// The row's `id` is its position in this list, because that is what `delete-event` hands back
/// and what `on_delete_event` indexes with. It was the literal 0 on every row, so the trash icon
/// on any row of a day deleted the first one. The store's own id is not on the row because the
/// Slint struct has no field for one; the handler resolves the index against `events_on_day`,
/// which is the same list these rows were built from.
fn events_for_day(events: &[CalEvent], year: i32, month: u32, day: i32) -> Vec<CalendarEvent> {
    let prefix = format!("{:04}-{:02}-{:02}", year, month, day);
    events_on_day(events, year, month, day)
        .into_iter()
        .enumerate()
        .map(|(row, e)| CalendarEvent {
            id: row as i32,
            title: e.title.clone().into(),
            date_text: prefix.clone().into(),
            time_text: time_text(e).into(),
            color: palette(e.color_index),
            is_all_day: e.is_all_day,
        })
        .collect()
}

/// The events in hand, in the shape the pure view code works in.
fn source_events(events: &[CalEvent]) -> Vec<views::SourceEvent> {
    events
        .iter()
        .map(|e| views::SourceEvent {
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
fn event_refs(events: &[CalEvent]) -> Vec<views::EventRef> {
    events
        .iter()
        .map(|e| views::EventRef {
            id: e.id.clone(),
            title: e.title.clone(),
            start: e.start.clone(),
            end: e.end.clone(),
            is_all_day: e.is_all_day,
        })
        .collect()
}

/// Derived blocks, turned into the rows the two time grids draw.
///
/// The id is dropped on the way: `CalendarTimeEvent` has no field for one, and a grid block is
/// drawn, not named. `describe` reports the ids from the same derivation before it gets here.
fn time_events(blocks: &[views::TimeEvent]) -> Vec<CalendarTimeEvent> {
    blocks
        .iter()
        .map(|b| CalendarTimeEvent {
            title: b.title.clone().into(),
            start_hour: b.start_hour,
            start_min: b.start_min,
            duration_min: b.duration_min,
            day_index: b.day_index,
            color: palette(b.color_index),
        })
        .collect()
}

/// How many of the events in hand fall in the month on screen.
///
/// Not `events.len()`: the week view widens what is fetched past the month's edges, and counting
/// everything fetched would report events the month grid is not drawing.
fn events_in_month(events: &[CalEvent], year: i32, month: u32) -> usize {
    let prefix = format!("{:04}-{:02}", year, month);
    events.iter().filter(|e| e.start.starts_with(&prefix)).count()
}

// ── Wire all callbacks ───────────────────────────────────────────────

// ── The control surface ──────────────────────────────────────────────
//
// "What is on my calendar today" should never be answered by photographing a month grid and
// asking a vision model to read the numbers. See `yantrik_app_runtime::control`.

/// One block of a time grid as a caller reads it: which event, what, when, and for how long.
///
/// The id is here so that a mind reading the week can name an event back to `delete_event` or
/// `update_event`. It could see one and not say which, which is how a surface comes to be
/// readable and not actable. An event running past midnight is two blocks carrying one id.
///
/// The grids carry ids, so they carry the same mark the day list does: whether this caller
/// created the event and may take it off through `delete_own_event` without anybody being
/// asked (#201). The creator is looked up in the events the window holds rather than carried
/// through `views` — the grid arithmetic has no business knowing who made anything.
fn block_json(
    block: &views::TimeEvent,
    events: &[CalEvent],
    me: Option<&str>,
) -> serde_json::Value {
    let creator = events.iter().find(|e| e.id == block.id).and_then(|e| e.creator.as_deref());
    serde_json::json!({
        "id": block.id,
        "title": block.title,
        "at": format!("{:02}:{:02}", block.start_hour, block.start_min),
        "minutes": block.duration_min,
        "may_delete_unasked": ownership::may_delete_unasked(creator, me),
    })
}

fn count_phrase(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// Everything on screen, re-derived from the events the app is holding.
///
/// One function rather than a redraw written out again in each callback, because the week and the
/// day were left out of every one of them: `on_switch_view` set `view-mode` and nothing else, so
/// pressing Week moved to a grid holding whatever it had been given last, which was nothing at
/// all. Anything that changes the month, the selected day, the view, or what is stored ends here.
fn render(ui: &CalendarApp, state: &Rc<RefCell<CalState>>) {
    let s = state.borrow();
    let (this_year, this_month, this_day) = today();
    let today_day =
        if s.year == this_year && s.month == this_month { Some(this_day) } else { None };
    let day = ui.get_selected_day();

    ui.set_month_title(format!("{} {}", views::month_name(s.month), s.year).into());
    ui.set_days(ModelRc::new(VecModel::from(build_month_grid(
        s.year,
        s.month,
        &s.events,
        today_day,
    ))));
    ui.set_events_today(ModelRc::new(VecModel::from(events_for_day(
        &s.events, s.year, s.month, day,
    ))));

    // The same events, placed on the hour grids. `views` decides where each one goes; this side
    // only turns the answer into Slint rows.
    let source = source_events(&s.events);
    let selected = views::selected_date(s.year, s.month, day);

    let week = views::week_view(&source, selected);
    let labels: Vec<SharedString> =
        week.labels.iter().map(|l| SharedString::from(l.as_str())).collect();
    ui.set_week_day_labels(ModelRc::new(VecModel::from(labels)));
    ui.set_week_events(ModelRc::new(VecModel::from(time_events(&week.events))));

    let day_view = views::day_view(&source, selected);
    ui.set_day_events(ModelRc::new(VecModel::from(time_events(&day_view.events))));
    ui.set_day_view_title(day_view.title.into());

    drop(s);
    set_clock(ui);
    refresh_agent_rail(ui);
}

/// The dates the views on screen need.
fn visible_range(
    ui: &CalendarApp,
    state: &Rc<RefCell<CalState>>,
) -> (chrono::NaiveDate, chrono::NaiveDate) {
    let s = state.borrow();
    views::visible_range(
        s.year,
        s.month,
        ViewMode::from_index(ui.get_view_mode()),
        ui.get_selected_day(),
    )
}

/// Read the store again if the range on screen has moved, if the store has, or if `force` says
/// so. Returns whether the events in hand were replaced.
///
/// This used to be the range test alone, which was right while this window was the only writer.
/// It is not: the mind's own calendar tools and Google sync go through the same service, and a
/// second caller can add something through this app's own surface while the person is on another
/// day. So the range test gained a second question — `calendar.revision`, two numbers and one
/// round trip to a local socket — and the month is re-listed when either says to.
///
/// The revision is asked *before* the listing, never after. A write landing between the two
/// leaves this holding the older token, so the next check re-reads once more than it had to; the
/// other order would record a token for events that did not include that write and never look
/// again. Being wrong in the cheap direction is the point.
fn reread(
    state: &Rc<RefCell<CalState>>,
    wanted: (chrono::NaiveDate, chrono::NaiveDate),
    force: bool,
) -> bool {
    let (held_range, held_revision) = {
        let s = state.borrow();
        (s.range, s.revision.clone())
    };
    let now = fetch_revision();
    let moved = match (&now, &held_revision) {
        (Some(now), Some(held)) => now != held,
        // Nothing to compare against: a first read, or one that failed and left no token behind.
        (Some(_), None) => true,
        // The question could not be put at all — the service is not listening, or did not answer.
        // Keep what is in hand rather than throwing a month away because nothing is there to ask.
        (None, _) => false,
    };
    if !force && held_range == Some(wanted) && !moved {
        return false;
    }
    match fetch_events_in_range(wanted.0, wanted.1) {
        Ok(events) => {
            let mut s = state.borrow_mut();
            s.events = events;
            s.range = Some(wanted);
            s.revision = now;
            true
        }
        Err(e) => {
            // The range and the token are left unrecorded on purpose: the next redraw asks again,
            // instead of a calendar that failed one read once staying empty until it is restarted.
            tracing::warn!(error = %e, "could not read the calendar");
            let mut s = state.borrow_mut();
            s.events.clear();
            s.range = None;
            s.revision = None;
            true
        }
    }
}

/// Redraw, reading the store again first if anything says to.
///
/// Where every callback that moves the month, the day or the view ends.
fn refresh(ui: &CalendarApp, state: &Rc<RefCell<CalState>>) {
    let wanted = visible_range(ui, state);
    reread(state, wanted, false);
    render(ui, state);
}

/// Redraw, reading the store again whatever the range. For after something has been written.
fn reload(ui: &CalendarApp, state: &Rc<RefCell<CalState>>) {
    let wanted = visible_range(ui, state);
    reread(state, wanted, true);
    render(ui, state);
}

/// Follow the store: re-read and redraw, but only if something under it has actually changed.
///
/// The difference from `refresh` is that nothing is redrawn when nothing moved. This one is
/// called by the watch timer and by `describe`, neither of which has any other reason to touch
/// the screen, and replacing a model that did not change is work for nobody — which is the rule
/// `design/performance-2026-09-20.md` spent a day applying to the shell.
///
/// Returns whether anything was replaced.
fn follow_store(ui: &CalendarApp, state: &Rc<RefCell<CalState>>) -> bool {
    let wanted = visible_range(ui, state);
    if !reread(state, wanted, false) {
        return false;
    }
    render(ui, state);
    true
}

/// Put an event on the calendar and show it, or say why not.
///
/// The single path behind the form's Save button and the `add_event` action, so neither can
/// report an outcome it did not get. The action used to answer `{"added": ...}` the moment it
/// had handed the title to the window, and the window's own save dropped the service's error on
/// the floor — which is how a calendar that was storing nothing told every caller it had.
fn store_event(
    ui: &CalendarApp,
    state: &Rc<RefCell<CalState>>,
    title: &str,
    date: &str,
    time: &str,
    notes: &str,
    duration_min: Option<i32>,
    all_day: bool,
) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("an event needs a title".into());
    }
    // How long it runs is the caller's if it said, and the state's otherwise: the form has no
    // duration field, and a template pressed a moment ago has already said 15 or 30 or 120. The
    // arithmetic that turns a date, a clock time and a length into two ISO timestamps is in
    // `views` so it can be tested -- it is where 23:30 plus an hour used to become "24:30", which
    // is not a time.
    let (start, end) = if all_day {
        // A whole day has no clock, so the time is not consulted and not silently applied.
        views::all_day_bounds(date).ok_or_else(|| format!("`{date}` is not a date"))?
    } else {
        let duration = duration_min.unwrap_or_else(|| state.borrow().new_event_duration_min);
        if duration < 0 {
            return Err(format!("`duration_min` cannot be negative, and was {duration}"));
        }
        views::start_and_end(date, time, duration)
            .ok_or_else(|| format!("`{date} {time}` is not a date and a time"))?
    };

    let id = create_event_via_service(title, &start, &end, notes, all_day)?;
    state.borrow_mut().new_event_duration_min = DEFAULT_EVENT_MINUTES;
    reload(ui, state);
    Ok(id)
}

/// Take something off the calendar, and show that it is gone — or say why it is not.
///
/// The one delete path. The trash icon on a row and the `delete_event` action both end here, so
/// neither can report an outcome the other would not, and there is only one place where the
/// service's answer is read.
///
/// Three round trips rather than one, all of them to a local socket and all of them necessary.
/// The event is read first so the answer can name what was removed rather than repeat an id back
/// at the caller. It is read again afterwards because "the service did not refuse" and "the event
/// is gone" are different sentences, and a calendar that answered the first while meaning the
/// second is the whole of `design/calendar-2026-09-20.md`. A read that cannot be made at all is
/// not read as "gone" — see `event_still_there`.
fn remove_event(
    ui: &CalendarApp,
    state: &Rc<RefCell<CalState>>,
    event_id: &str,
) -> Result<String, String> {
    let outcome = delete_through_service(event_id);
    // Whatever happened, the window is redrawn from the store: a delete that failed may still
    // have been preceded by somebody else's successful one.
    reload(ui, state);
    outcome
}

fn delete_through_service(event_id: &str) -> Result<String, String> {
    let event = get_event_via_service(event_id)?;
    delete_event_via_service(event_id)?;
    if event_still_there(event_id)? {
        return Err(format!(
            "the calendar still holds “{}” under id {event_id} after deleting it",
            event.title
        ));
    }
    Ok(event.title)
}

/// Which event a delete names: by id, or by exactly one title on one date. Never a guess.
///
/// The resolution both delete actions share, lifted out of the handlers so the two cannot
/// drift — `delete_event` and `delete_own_event` must answer with the same event for the same
/// words, and differ only in the rule that runs afterwards. The answer is the store's id and
/// the name to use in a notice.
fn named_event(args: &serde_json::Value) -> Result<(String, String), String> {
    let given = |key: &str| {
        args[key].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
    };
    match (given("id"), given("title"), given("date")) {
        (Some(id), _, _) => Ok((id.clone(), id)),
        (None, Some(title), Some(date)) => {
            if date.len() != 10 || date.matches('-').count() != 2 {
                return Err(format!("`date` should look like 2026-09-06, not `{date}`"));
            }
            let day = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map_err(|_| format!("`{date}` is not a date"))?;
            // Read from the store now, not off the screen: the day named may not be the day
            // the window is showing, and the events in hand are only the visible range.
            let on_that_day = fetch_events_on(day)?;
            match views::named_on(&event_refs(&on_that_day), &title, &date) {
                views::Named::One(event) => Ok((event.id, format!("“{title}”"))),
                views::Named::None => Err(format!(
                    "nothing called “{title}” on {date}; the day holds: {}",
                    if on_that_day.is_empty() {
                        "nothing".to_string()
                    } else {
                        on_that_day
                            .iter()
                            .map(|e| format!("“{}”", e.title))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )),
                // Never a guess. Two events of one name on one day is an ordinary thing for a
                // calendar to hold, and picking one would remove the wrong appointment and
                // report success — which is the trash-icon bug 617dac9 fixed, rebuilt on the
                // surface.
                views::Named::Ambiguous(candidates) => Err(format!(
                    "{} events on {date} are called “{title}”; say which by id: {}",
                    candidates.len(),
                    candidates
                        .iter()
                        .map(|e| format!(
                            "{} at {}",
                            e.id,
                            e.start.split('T').nth(1).unwrap_or(&e.start)
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
        _ => Err("name the event by `id`, or by `title` and `date` together".into()),
    }
}

/// Change an appointment, and show what it became — or say why it did not.
///
/// The one update path, on the same terms as the delete above: what comes back is read from the
/// store afterwards, not taken from the reply the update handed us. `update_event` answers with
/// the event it built, and this calendar's history is of answers built from intentions.
fn change_event(
    ui: &CalendarApp,
    state: &Rc<RefCell<CalState>>,
    event_id: &str,
    title: Option<&str>,
    date: Option<&str>,
    time: Option<&str>,
    duration_min: Option<i32>,
    notes: Option<&str>,
) -> Result<yantrik_ipc_contracts::calendar::CalendarEvent, String> {
    let outcome = update_through_service(event_id, title, date, time, duration_min, notes);
    reload(ui, state);
    outcome
}

fn update_through_service(
    event_id: &str,
    title: Option<&str>,
    date: Option<&str>,
    time: Option<&str>,
    duration_min: Option<i32>,
    notes: Option<&str>,
) -> Result<yantrik_ipc_contracts::calendar::CalendarEvent, String> {
    // Read before written, because moving an event has to know how long it already runs. The
    // caller said "10:00", not "10:00 for an hour".
    let current = get_event_via_service(event_id)?;
    let times = views::rescheduled(
        &current.start,
        &current.end,
        current.is_all_day,
        date,
        time,
        duration_min,
    )
    .map_err(|e| format!("“{}”: {e}", current.title))?;

    if let Some(t) = title {
        if t.trim().is_empty() {
            return Err("an event needs a title".into());
        }
    }
    if title.is_none() && times.is_none() && notes.is_none() {
        return Err(
            "nothing to change: give a `title`, a `date`, a `time`, a `duration_min` or `notes`"
                .into(),
        );
    }

    let params = UpdateEventParams {
        id: event_id.to_string(),
        title: title.map(|t| t.trim().to_string()),
        start: times.as_ref().map(|(start, _)| start.clone()),
        end: times.as_ref().map(|(_, end)| end.clone()),
        description: notes.map(|n| n.to_string()),
        ..Default::default()
    };
    update_event_via_service(&params)?;

    // Observed, from the store, after the write.
    let stored = get_event_via_service(event_id)?;
    if let Some((start, end)) = &times {
        if &stored.start != start || &stored.end != end {
            return Err(format!(
                "asked the calendar for {start} to {end} and it kept {} to {}",
                stored.start, stored.end
            ));
        }
    }
    if let Some(t) = title {
        if stored.title != t.trim() {
            return Err(format!(
                "asked the calendar to call it “{}” and it kept “{}”",
                t.trim(),
                stored.title
            ));
        }
    }
    Ok(stored)
}

fn publish_control(app: &CalendarApp, state: Rc<RefCell<CalState>>) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let weak = app.as_weak();
        let st = state.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Calendar — closing");
            };

            // Before answering, not after. A mind reads `describe` immediately after acting — on
            // this calendar, or just as often through its own tools, which write to the same
            // store — and that is exactly the moment a month read some navigations ago lies. The
            // question is two numbers over a local socket, it is only put to a service that is
            // already listening, and the month is re-listed only when the answer says it moved.
            // Well inside the three seconds `control.rs` gives an action on the UI thread.
            follow_store(&ui, &st);

            let month = ui.get_month_title().to_string();
            let day = ui.get_selected_day();

            // Which days of the month have anything on them. Six numbers instead of a picture of
            // a grid, and it is what a caller planning around the month actually needs.
            let grid = ui.get_days();
            let busy: Vec<serde_json::Value> = (0..grid.row_count())
                .filter_map(|i| grid.row_data(i))
                .filter(|d| d.is_current_month && d.has_events)
                .map(|d| serde_json::json!({ "day": d.day_number, "events": d.event_count }))
                .collect();

            // Who this caller was verified to be, and what that is worth against each event's
            // creator record: the answer `delete_own_event` would give, before it is asked
            // (#201). Per describe, because the caller changes per describe, and from the same
            // two facts the action itself reads.
            let me = requester();

            let s = st.borrow();
            let view = ViewMode::from_index(ui.get_view_mode());
            let selected = views::selected_date(s.year, s.month, day);
            let (week_start, week_end) = views::week_bounds(selected);

            let today: Vec<serde_json::Value> = events_on_day(&s.events, s.year, s.month, day)
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        // The id the store gave it, which is what `delete_event` and
                        // `update_event` take. A caller told an event's title and time and not
                        // its id can read this calendar and cannot act on it.
                        "id": e.id,
                        "title": e.title,
                        "date": format!("{:04}-{:02}-{:02}", s.year, s.month, day),
                        "time": time_text(e),
                        "all_day": e.is_all_day,
                        // Whether this caller created this event, which is whether
                        // `delete_own_event` will take it off without anybody being asked.
                        "may_delete_unasked": ownership::may_delete_unasked(
                            e.creator.as_deref(),
                            me.as_deref(),
                        ),
                    })
                })
                .collect();

            // What the view on screen is showing, derived from the events the models on screen
            // were built from a moment ago by `render` — through the same functions, so this is
            // that drawing read back in words rather than a second account of it. It is derived
            // rather than read off the Slint rows because the rows carry no id, and an event a
            // caller cannot name is an event it cannot move or remove.
            //
            // A mind asking what is on the calendar used to be told about the month whichever
            // view was up, because the month was the only view with anything in it. Week and Day
            // now answer as themselves: the week says its range and what each of its seven days
            // holds; the day says its date and its events in order.
            let source = source_events(&s.events);
            let week = views::week_view(&source, selected);
            let day_view = views::day_view(&source, selected);

            let summary = match view {
                ViewMode::Week => format!(
                    "Calendar — week of {week_start} to {week_end}, {} on the grid",
                    count_phrase(week.events.len(), "event", "events")
                ),
                ViewMode::Day => format!(
                    "Calendar — {}, {} on the grid",
                    day_view.title,
                    count_phrase(day_view.events.len(), "event", "events")
                ),
                ViewMode::Month if today.is_empty() => {
                    format!("Calendar — {month}, nothing on day {day}")
                }
                ViewMode::Month if today.len() == 1 => format!(
                    "Calendar — {month}, one thing on day {day}: {}",
                    today[0]["title"].as_str().unwrap_or_default()
                ),
                ViewMode::Month => {
                    format!("Calendar — {month}, {} things on day {day}", today.len())
                }
            };

            // What each event id in hand names, as a person would say it. The id is the
            // reliable way to point at an event — `delete_event` recommends it and the
            // approval grant is bound to it exactly — but a card that asks permission with a
            // uuid alone asks a question nobody can answer (#54), and the shell reads this
            // from the same `app.describe` it already reads the grade and the purpose from.
            // Every event in hand, not just the selected day's: an id in an action was read
            // from some view of the visible range, and the card cannot say what it has not
            // been given. Bounded, because `describe` is read by agents and a full table in
            // a reply is what describe replies are told to avoid: only the loaded visible
            // range — the same events every other key below is derived from, never the whole
            // store — and a fixed cap of `views::NAMING_CAP` entries, oldest dropped (see
            // `views::naming_index`).
            let naming: serde_json::Map<String, serde_json::Value> =
                views::naming_index(&event_refs(&s.events))
                    .into_iter()
                    .map(|(id, name)| (id, serde_json::Value::String(name)))
                    .collect();

            let mut out = View::new(summary)
                .with("month", month)
                .with("year", s.year)
                .with("month_number", s.month as i64)
                .with("selected_day", day)
                // Which day today is — "2026-09-23 Wednesday". The month and the selected day
                // above say where the screen is looking, not which day the machine is living
                // in, and a mind that needed the second one ran `date` through the shell:
                // sensitive, so learning the day raised an approval card (#207).
                .with("today", views::today_line(chrono::Local::now().date_naive()))
                .with("view", view.as_str())
                .with("events_on_selected_day", serde_json::Value::Array(today))
                .with("naming", serde_json::Value::Object(naming))
                .with("days_with_events", serde_json::Value::Array(busy))
                .with("events_this_month", events_in_month(&s.events, s.year, s.month) as i64)
                // What the person is being told went wrong, if anything. A caller that just
                // failed to save should be able to read the reason rather than infer it.
                .with("notice", ui.get_notice().to_string())
                // The identity every `may_delete_unasked` in this describe was answered
                // against, so a caller that expected to own something and sees `false` can
                // tell whether the event has no record or the record names somebody else —
                // and how it itself was recorded. The transport's one spelling for "nothing
                // could be established" when there is no identity to show.
                .with(
                    "you",
                    me.clone().unwrap_or_else(|| peer_identity::UNIDENTIFIED.to_string()),
                );

            match view {
                ViewMode::Week => {
                    let per_day: Vec<serde_json::Value> = week
                        .labels
                        .iter()
                        .enumerate()
                        .map(|(column, label)| {
                            let events: Vec<serde_json::Value> = week
                                .events
                                .iter()
                                .filter(|b| b.day_index == column as i32)
                                .map(|b| block_json(b, &s.events, me.as_deref()))
                                .collect();
                            serde_json::json!({ "day": label, "events": events })
                        })
                        .collect();
                    out = out
                        .with("week_start", week_start.to_string())
                        .with("week_end", week_end.to_string())
                        .with("week", serde_json::Value::Array(per_day));
                }
                ViewMode::Day => {
                    let events: Vec<serde_json::Value> = day_view
                        .events
                        .iter()
                        .map(|b| block_json(b, &s.events, me.as_deref()))
                        .collect();
                    out = out
                        .with("day_shown", day_view.title.clone())
                        .with("events_on_day_grid", serde_json::Value::Array(events));
                }
                ViewMode::Month => {}
            }
            out
        }
    };

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "Calendar window is gone".to_string());

    let add_state = state.clone();
    let view_state = state.clone();
    let delete_state = state.clone();
    let own_delete_state = state.clone();
    let update_state = state.clone();
    let day_ui = ui_for.clone();
    let move_ui = ui_for.clone();
    let today_ui = ui_for.clone();
    let add_ui = ui_for.clone();
    let delete_ui = ui_for.clone();
    let own_delete_ui = ui_for.clone();
    let update_ui = ui_for.clone();
    let view_ui = ui_for;

    App::new("calendar")
        .describe(describe)
        .action(
            Action::new("select_day", "Show what is on one day of the month shown")
                .arg(Param::integer("day").describe("Day of the month, 1-31")),
            move |args| {
                let ui = day_ui()?;
                let day = args["day"].as_i64().ok_or("`day` must be a number")? as i32;
                if !(1..=31).contains(&day) {
                    return Err(format!("{day} is not a day of the month"));
                }
                ui.invoke_day_clicked(day);
                let model = ui.get_events_today();
                let titles: Vec<String> = (0..model.row_count())
                    .filter_map(|i| model.row_data(i))
                    .map(|e| e.title.to_string())
                    .collect();
                Ok(serde_json::json!({ "day": day, "events": titles }))
            },
        )
        .action(
            Action::new("show_month", "Move to the next or previous month")
                .arg(Param::text("direction").describe("next | previous")),
            move |args| {
                let ui = move_ui()?;
                match args["direction"].as_str().unwrap_or_default().to_lowercase().as_str() {
                    "next" | "forward" => ui.invoke_next_month(),
                    "previous" | "prev" | "back" => ui.invoke_prev_month(),
                    other => return Err(format!("`direction` is next or previous, not `{other}`")),
                }
                Ok(serde_json::json!({ "showing": ui.get_month_title().to_string() }))
            },
        )
        .action(Action::new("go_to_today", "Jump back to the current month and day"), move |_| {
            let ui = today_ui()?;
            ui.invoke_today_pressed();
            Ok(serde_json::json!({
                "showing": ui.get_month_title().to_string(),
                "day": ui.get_selected_day(),
            }))
        })
        .action(
            Action::new("add_event", "Put something on the calendar")
                .arg(Param::text("title"))
                .arg(Param::text("date").describe("YYYY-MM-DD"))
                .arg(Param::text("time")
                    .describe("HH:MM, 24-hour; needed unless `all_day` is set")
                    .optional())
                .arg(Param::text("notes").optional())
                // The form has no duration field and the template path already carries minutes,
                // so the one caller that could say how long a thing runs was the one that could
                // not: a mind asking for a fifteen-minute call got an hour and was not told.
                .arg(Param::integer("duration_min")
                    .describe("How long it runs, in minutes; an hour when not given")
                    .optional())
                // The contract carries `is_all_day` and nothing on this surface could set it, so
                // a whole-day event asked for here was stored as a one-hour appointment at
                // whatever time happened to be passed.
                .arg(Param::flag("all_day")
                    .describe("A whole day rather than a time; `time` and `duration_min` are \
                               not used with it")
                    .optional()),
            move |args| {
                let ui = add_ui()?;
                let title = args["title"].as_str().unwrap_or_default().trim().to_string();
                let date = args["date"].as_str().unwrap_or_default().trim().to_string();
                let all_day = args["all_day"].as_bool().unwrap_or(false);
                if title.is_empty() {
                    return Err("`title` is empty".into());
                }
                // Checked here rather than let the service reject a malformed timestamp: the
                // error a caller can act on names the format it should have used.
                if date.len() != 10 || date.matches('-').count() != 2 {
                    return Err(format!("`date` should look like 2026-09-06, not `{date}`"));
                }
                // Whether this call needed a `time` at all is decided here, not by the
                // declaration: `time` is optional on the surface because an all-day event has
                // no clock to give, but a timed one is nothing without it.
                let time = views::added_clock(args["time"].as_str(), all_day)?;
                let duration_min = match args.get("duration_min") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(v) => Some(
                        v.as_i64().ok_or("`duration_min` must be a number of minutes")? as i32,
                    ),
                };
                if all_day && duration_min.is_some() {
                    return Err("an all-day event has no length to set; drop `duration_min` or \
                                drop `all_day`"
                        .into());
                }
                let notes = args["notes"].as_str().unwrap_or_default().to_string();
                // Stored before answering, and the answer carries the id it was stored under,
                // so "added" cannot be a guess about what the window did next. A failure is put
                // on screen as well as returned: when a mind tries to put something on the
                // calendar and cannot, the person watching the window is owed the reason too.
                let id = match store_event(
                    &ui, &add_state, &title, &date, &time, &notes, duration_min, all_day,
                ) {
                    Ok(id) => id,
                    Err(e) => {
                        ui.set_notice(format!("Could not save “{title}”: {e}").into());
                        return Err(e);
                    }
                };
                ui.set_notice(SharedString::new());
                let on = if all_day { date.clone() } else { format!("{date} {time}") };
                Ok(serde_json::json!({
                    "added": title, "on": on, "id": id, "all_day": all_day,
                }))
            },
        )
        .action(
            // Graded `sensitive`, and the reason is that there is no trash. A delete here removes
            // the file the event lives in; nothing on this machine keeps a copy, so an appointment
            // a person or a mind put on the calendar is gone and its time with it. That is a
            // different thing from `add_event`, which is `standard` because it is undone by this
            // action, and from `update_event`, which moves something that still exists. It sits
            // below `dangerous` because it destroys one named thing the caller asked for by name,
            // not a range and not a directory — the range delete Google sync used to do, which
            // took every local event in the window with it, would have been the other grade.
            Action::new("delete_event", "Take an event off the calendar. It is not recoverable")
                .risk("sensitive")
                .arg(Param::text("id")
                    .describe("The id the store gave the event — `add_event` answers with it and \
                               `describe` lists it for every event it shows")
                    .optional())
                .arg(Param::text("title")
                    .describe("The event's exact title, given with `date`, when the id is not known")
                    .optional())
                .arg(Param::text("date")
                    .describe("YYYY-MM-DD, given with `title`")
                    .optional()),
            move |args| {
                let ui = delete_ui()?;
                let (id, named) = named_event(&args)?;

                match remove_event(&ui, &delete_state, &id) {
                    Ok(title) => {
                        ui.set_notice(SharedString::new());
                        Ok(serde_json::json!({ "deleted": title, "id": id }))
                    }
                    // On screen as well as in the answer. A delete that did not happen leaves a
                    // row where it was, and a row that stayed put has to say which of the two
                    // things it means.
                    Err(e) => {
                        ui.set_notice(format!("Could not delete {named}: {e}").into());
                        Err(e)
                    }
                }
            },
        )
        .action(
            // Graded `standard`, and the only events it can reach are the ones the store
            // records as created by this very caller: `requester` reads the kernel's account
            // of the call — the peer walk of #221, or the agent an agent token belongs to —
            // at creation, the service keeps that in the event's own file, and the rule in
            // `ownership` compares the two when a delete arrives. Nothing the request itself
            // says is consulted on either side. For the caller that made it, this is the
            // inverse of its own `add_event`, which is `standard` too: an unattended harness
            // can put an event on and take it off again without a person being asked, which
            // is the whole of issue #201. Everything else — a person's event, another
            // caller's, an event stored before the record existed — is refused here and stays
            // with `delete_event` above: same grade, same card, exactly as it was.
            Action::new(
                "delete_own_event",
                "Take an event this caller created itself off the calendar. Anything else is \
                 `delete_event`, which asks a person first",
            )
            .arg(Param::text("id")
                .describe("The id the store gave the event — `add_event` answers with it and \
                           `describe` lists it, marked with whether this caller may delete it \
                           unasked")
                .optional())
            .arg(Param::text("title")
                .describe("The event's exact title, given with `date`, when the id is not known")
                .optional())
            .arg(Param::text("date")
                .describe("YYYY-MM-DD, given with `title`")
                .optional()),
            move |args| {
                let ui = own_delete_ui()?;
                let (id, named) = named_event(&args)?;
                // The record the service kept at creation, read back from the event's own
                // file — so the check survives this app restarting between the create and
                // this delete, which the arena's reset can (#201).
                let event = get_event_via_service(&id)?;
                let me = requester();
                if !ownership::may_delete_unasked(event.creator.as_deref(), me.as_deref()) {
                    let who =
                        me.unwrap_or_else(|| "nobody this machine could identify".to_string());
                    let why = match event.creator.as_deref() {
                        Some(made_by) => format!(
                            "{named} was created by {made_by} and this call is {who}: only the \
                             caller that created an event may delete it without a person being \
                             asked"
                        ),
                        None => format!(
                            "{named} has no creator on record — it is older than the record, or \
                             a person made it in the window — and this call is {who}: only the \
                             caller that created an event may delete it without a person being \
                             asked"
                        ),
                    };
                    let why = format!(
                        "{why}. Any event comes off through `delete_event`, which asks first"
                    );
                    ui.set_notice(format!("Could not delete {named}: {why}").into());
                    return Err(why);
                }

                match remove_event(&ui, &own_delete_state, &id) {
                    Ok(title) => {
                        ui.set_notice(SharedString::new());
                        Ok(serde_json::json!({ "deleted": title, "id": id }))
                    }
                    Err(e) => {
                        ui.set_notice(format!("Could not delete {named}: {e}").into());
                        Err(e)
                    }
                }
            },
        )
        .action(
            // Graded `standard`, unlike the delete above, and the difference is what survives.
            // Moving a meeting leaves the appointment on the calendar under the same id, where
            // the person can see where it went and this same action can put it back; the previous
            // time is the only thing lost, and the caller is told the new one. Nothing is
            // destroyed, so this is the grade `add_event` carries — a change to an appointment
            // the window is showing, which a person watching can see and undo.
            Action::new("update_event", "Move or rename an event that is already on the calendar")
                .arg(Param::text("id").describe("The id the store gave the event"))
                .arg(Param::text("title").describe("A new title").optional())
                .arg(Param::text("date").describe("Move it to this day, YYYY-MM-DD").optional())
                .arg(Param::text("time").describe("Move it to this time, HH:MM").optional())
                .arg(Param::integer("duration_min")
                    .describe("How long it runs, in minutes; unchanged when not given")
                    .optional())
                .arg(Param::text("notes").optional()),
            move |args| {
                let ui = update_ui()?;
                let given = |key: &str| {
                    args[key].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
                };
                let id = given("id").ok_or("`id` is empty")?;
                let date = given("date");
                let time = given("time");
                if let Some(date) = &date {
                    if date.len() != 10 || date.matches('-').count() != 2 {
                        return Err(format!("`date` should look like 2026-09-06, not `{date}`"));
                    }
                }
                if let Some(time) = &time {
                    if !time.contains(':') {
                        return Err(format!("`time` should look like 14:30, not `{time}`"));
                    }
                }
                let duration_min = match args.get("duration_min") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(v) => Some(
                        v.as_i64().ok_or("`duration_min` must be a number of minutes")? as i32,
                    ),
                };
                // `notes` is taken as given, empty string included: clearing the notes on an
                // event is a thing a caller may mean, and `given` would read that as "not said".
                let notes = args.get("notes").and_then(|v| v.as_str()).map(str::to_string);

                match change_event(
                    &ui,
                    &update_state,
                    &id,
                    given("title").as_deref(),
                    date.as_deref(),
                    time.as_deref(),
                    duration_min,
                    notes.as_deref(),
                ) {
                    Ok(event) => {
                        ui.set_notice(SharedString::new());
                        // What the store holds now, read back after the write — not the fields
                        // that were asked for.
                        Ok(serde_json::json!({
                            "id": event.id,
                            "title": event.title,
                            "start": event.start,
                            "end": event.end,
                            "all_day": event.is_all_day,
                        }))
                    }
                    Err(e) => {
                        ui.set_notice(format!("Could not change the event {id}: {e}").into());
                        Err(e)
                    }
                }
            },
        )
        .action(
            Action::new("set_view", "Switch between the month, week and day views")
                .arg(Param::text("view").describe("month | week | day")),
            move |args| {
                let ui = view_ui()?;
                let mode = match args["view"].as_str().unwrap_or_default().to_lowercase().as_str() {
                    "month" => 0,
                    "week" => 1,
                    "day" => 2,
                    other => return Err(format!("unknown view `{other}`; use month, week or day")),
                };
                ui.invoke_switch_view(mode);
                // What is on screen now, not the word that was asked for. This answered
                // `{"view": "week"}` while the week grid was empty of everything -- no events, no
                // column headers, a "now" line on midnight -- which reads as a view that was
                // shown. It is the same fabrication `add_event` was making in September, one
                // layer up.
                Ok(match ViewMode::from_index(mode) {
                    ViewMode::Week => {
                        let s = view_state.borrow();
                        let (start, end) = views::week_bounds(views::selected_date(
                            s.year,
                            s.month,
                            ui.get_selected_day(),
                        ));
                        serde_json::json!({
                            "view": "week",
                            "week_start": start.to_string(),
                            "week_end": end.to_string(),
                            "events": ui.get_week_events().row_count(),
                        })
                    }
                    ViewMode::Day => serde_json::json!({
                        "view": "day",
                        "day": ui.get_day_view_title().to_string(),
                        "events": ui.get_day_events().row_count(),
                    }),
                    ViewMode::Month => {
                        let s = view_state.borrow();
                        serde_json::json!({
                            "view": "month",
                            "showing": ui.get_month_title().to_string(),
                            "events": events_in_month(&s.events, s.year, s.month),
                        })
                    }
                })
            },
        )
        .serve();
}

/// Hook the window up, and hand back the timer that keeps it following the store.
///
/// The timer is returned rather than kept here because a Slint timer stops when it is dropped,
/// and a binding inside this function is dropped the moment it returns — see the comment at its
/// declaration below, and `main`.
fn wire(app: &CalendarApp) -> slint::Timer {
    let (ty, tm, td) = today();
    let state = Rc::new(RefCell::new(CalState {
        year: ty,
        month: tm,
        events: Vec::new(),
        range: None,
        revision: None,
        new_event_duration_min: DEFAULT_EVENT_MINUTES,
    }));

    // Initial load. The selected day goes on first because the range the week view needs is
    // decided by it.
    app.set_selected_day(td as i32);
    refresh(app, &state);

    // Published once the first read is done, so the first `app.describe` reports real events.
    publish_control(app, state.clone());

    // ── Prev month ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_prev_month(move || {
            let Some(ui) = weak.upgrade() else { return };
            {
                let mut s = st.borrow_mut();
                if s.month == 1 {
                    s.month = 12;
                    s.year -= 1;
                } else {
                    s.month -= 1;
                }
            }
            // Nothing is picked in the month just arrived at; the week and day views read that
            // as the first of it. See `views::selected_date`.
            ui.set_selected_day(0);
            refresh(&ui, &st);
        });
    }

    // ── Next month ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_next_month(move || {
            let Some(ui) = weak.upgrade() else { return };
            {
                let mut s = st.borrow_mut();
                if s.month == 12 {
                    s.month = 1;
                    s.year += 1;
                } else {
                    s.month += 1;
                }
            }
            ui.set_selected_day(0);
            refresh(&ui, &st);
        });
    }

    // ── Day clicked ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_day_clicked(move |day| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_selected_day(day);
            // Not just the agenda: picking a day moves the day view onto it and the week view
            // onto the week around it, and a day in the first or last week of the month needs
            // days the month fetch did not ask for.
            refresh(&ui, &st);
        });
    }

    // ── Add event (open form) ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_add_event(move || {
            let Some(ui) = weak.upgrade() else { return };
            let date = {
                let mut s = st.borrow_mut();
                // A blank form is an hour long; a template says otherwise and is honoured once.
                s.new_event_duration_min = DEFAULT_EVENT_MINUTES;
                views::selected_date(s.year, s.month, ui.get_selected_day())
            };
            ui.set_event_date(date.format("%Y-%m-%d").to_string().into());
            ui.set_event_time("09:00".into());
            ui.set_event_title(SharedString::default());
            ui.set_event_notes(SharedString::default());
            ui.set_show_event_form(true);
        });
    }

    // ── A template pressed ──
    //
    // The four templates belong to the screen; pressing one hands over the two things the app
    // needs, the title to start from and how long the thing runs. That is the whole of what
    // "use a template" can honestly mean while the form has a title, a date, a time and notes
    // and nothing else: the title is filled in, and the minutes wait in the state until Save.
    // The handler used to log "Use template (standalone mode)" and return.
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_cal_use_template(move |title, duration_min| {
            let Some(ui) = weak.upgrade() else { return };
            let date = {
                let mut s = st.borrow_mut();
                s.new_event_duration_min =
                    if duration_min > 0 { duration_min } else { DEFAULT_EVENT_MINUTES };
                views::selected_date(s.year, s.month, ui.get_selected_day())
            };
            ui.set_event_date(date.format("%Y-%m-%d").to_string().into());
            ui.set_event_time("09:00".into());
            ui.set_event_title(title);
            ui.set_event_notes(SharedString::default());
            ui.set_notice(SharedString::new());
            ui.set_show_event_form(true);
        });
    }

    // ── Save event ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_save_event(move |title, date, time, notes| {
            let Some(ui) = weak.upgrade() else { return };
            // The form has no duration field and no all-day switch, so it says nothing about
            // either: the duration comes from the state, where a template left it, and an event
            // typed into this form is a timed one.
            match store_event(&ui, &st, &title, &date, &time, &notes, None, false) {
                Ok(_) => {
                    ui.set_notice(SharedString::new());
                    ui.set_show_event_form(false);
                }
                // The form stays open holding what was typed. Closing it on a failed save threw
                // the event away twice: once from the store, once from the screen.
                Err(e) => ui.set_notice(format!("Could not save “{}”: {e}", title.trim()).into()),
            }
        });
    }

    // ── Delete event ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_delete_event(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            let (event_id, event_title) = {
                let s = st.borrow();
                let day = ui.get_selected_day();
                let day_events = events_on_day(&s.events, s.year, s.month, day);
                let idx = idx as usize;
                // The row index is a position in the list the agenda was drawn from, which is
                // this one. Every row of a day used to carry the id 0, so the trash icon on any
                // of them deleted the first event on that day (617dac9).
                let Some(event) = day_events.get(idx) else { return };
                (event.id.clone(), event.title.clone())
            };

            // The same path the `delete_event` action takes, so the trash icon cannot succeed
            // where the action would fail or report something the action would not.
            match remove_event(&ui, &st, &event_id) {
                Ok(_) => ui.set_notice(SharedString::new()),
                // A row that stayed on screen after a delete used to mean either "it is still
                // there" or "the store never heard"; now it means the first, and says the second.
                Err(e) => ui.set_notice(format!("Could not delete “{event_title}”: {e}").into()),
            }
        });
    }

    // ── Cancel event form ──
    {
        let weak = app.as_weak();
        app.on_cancel_event_form(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_show_event_form(false);
            }
        });
    }

    // ── Today pressed ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_today_pressed(move || {
            let Some(ui) = weak.upgrade() else { return };
            let (ny, nm, nd) = today();
            {
                let mut s = st.borrow_mut();
                s.year = ny;
                s.month = nm;
            }
            ui.set_selected_day(nd as i32);
            refresh(&ui, &st);
        });
    }

    // ── Switch view ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_switch_view(move |mode| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_view_mode(mode);
            // This used to be the whole handler. Week and Day are drawn from the same events the
            // month is, but nothing derived them and nothing fetched the days a week needs when
            // it runs past the month's edge, so both views were an empty drawing.
            refresh(&ui, &st);
        });
    }

    // ── The agent layer ──
    //
    // Calendar had no companion connection at all: its AI button logged a line and returned,
    // which is the state fifteen of the sixteen apps were in. The rail and the card follow the
    // same rule Notes does -- every row is something this app or the companion actually holds,
    // and says where it came from.
    {
        let weak = app.as_weak();
        app.on_ai_explain_pressed(move || {
            let Some(ui) = weak.upgrade() else { return };
            let day = if ui.get_selected_day() > 0 {
                format!("{} {}", ui.get_month_title(), ui.get_selected_day())
            } else {
                ui.get_month_title().to_string()
            };

            // What the day actually holds, read off the model rather than described to the
            // model second-hand.
            let events: Vec<String> = ui
                .get_events_today()
                .iter()
                .map(|e| format!("- {} ({})", e.title, e.time_text))
                .collect();

            ui.set_ai_is_working(true);
            ui.set_proposal_working(true);
            ui.set_proposal(AgentProposal {
                title: "Your day".into(),
                source: format!("from {day}").into(),
                ..Default::default()
            });

            let prompt = if events.is_empty() {
                format!(
                    "My calendar for {day} is empty. In two sentences, say so plainly and \
                     suggest one useful thing to do with an open day. Do not invent \
                     appointments."
                )
            } else {
                format!(
                    "Here is my calendar for {day}:\n{}\n\nIn at most four short lines, tell \
                     me what the shape of this day is and what to prepare. Use only what is \
                     listed; invent nothing.",
                    events.join("\n")
                )
            };

            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&prompt);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_ai_is_working(false);
                    ui.set_proposal_working(false);
                    match outcome {
                        Ok(text) => ui.set_proposal(AgentProposal {
                            title: "Your day".into(),
                            body: text.into(),
                            source: format!("from {day}").into(),
                            // Reading, not changing. The card gives this one button.
                            impact: SharedString::new(),
                            destructive: false,
                            verb: "Close".into(),
                        }),
                        Err(e) => {
                            tracing::warn!(error = %e, "Companion call failed");
                            ui.set_proposal(AgentProposal {
                                title: "The companion did not answer".into(),
                                body: e.to_string().into(),
                                verb: "Close".into(),
                                ..Default::default()
                            });
                        }
                    }
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        app.on_proposal_dismissed(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_proposal(AgentProposal::default());
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_agent_suggestion_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            match id.as_str() {
                // One code path: the rail presses the same button the toolbar does.
                "explain" => ui.invoke_ai_explain_pressed(),
                "today" => ui.invoke_today_pressed(),
                other => tracing::warn!(id = other, "unknown rail suggestion"),
            }
        });
    }
    // A row in the rail is an event on the day already on screen, so there is nowhere for a
    // click to go. It stays empty rather than being given something to do for the sake of it.
    app.on_agent_context_activated(|_| {});
    {
        let weak = app.as_weak();
        app.on_ai_dismiss(move || {
            if let Some(ui) = weak.upgrade() {
                // The header's AI button is a toggle and the second press has to put away what
                // the first press put up. The answer is in the proposal card; this handler was
                // empty, so pressing AI again closed nothing and left the card on the grid.
                ui.set_proposal_working(false);
                ui.set_proposal(AgentProposal::default());
            }
        });
    }

    // ── Following the store ──────────────────────────────────────────
    //
    // This window is no longer the only writer. `refresh` re-read only when the visible date
    // range changed, so an event the mind created through its own tools, or one Google sync
    // pulled in, or one a second caller added through this app's own surface, did not appear in
    // an open Calendar until the person navigated away and back. `one-calendar.py` had to step a
    // month forward and back before every read to get past exactly this, which is a probe
    // measuring the app's cache and knowing it.
    //
    // A slow timer rather than a fast poll, and a cheap question rather than a re-list: the tick
    // asks `calendar.revision` — two numbers — and re-lists the month only when they have moved.
    // It is skipped entirely while the window is not visible or is minimized, and it never starts
    // the calendar service (see `fetch_revision`), so a window nobody is looking at costs
    // nothing. The other half of the mechanism is in `describe`, which asks the same question
    // before answering, because a mind reads `describe` straight after acting and that is exactly
    // when twenty seconds of lag would be a lie.
    //
    // Held by the caller, not by this function: a Slint timer stops when it is dropped, and
    // `let _keep = ...` at the end of `wire` is how System Monitor came to take one reading of
    // the machine and show it for the life of the window (b291cb2).
    let watch = slint::Timer::default();
    {
        let weak = app.as_weak();
        let st = state.clone();
        watch.start(slint::TimerMode::Repeated, STORE_WATCH, move || {
            let Some(ui) = weak.upgrade() else { return };
            if !ui.window().is_visible() || ui.window().is_minimized() {
                return;
            }
            follow_store(&ui, &st);
        });
    }
    watch
}
