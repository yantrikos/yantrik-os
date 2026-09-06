//! Yantrik Calendar — standalone app binary.
//!
//! Communicates with `calendar-service` via JSON-RPC IPC.
//! Falls back to local event storage when service is unavailable.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_transport::SyncRpcClient;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-calendar");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("calendar") else { return };

    let app = CalendarApp::new().unwrap();

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    wire(&app);
    app.run().unwrap();
}

// ── Calendar state ───────────────────────────────────────────────────

#[derive(Clone)]
struct CalState {
    year: i32,
    month: u32,
    events: Vec<CalEvent>,
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
    color: slint::Color,
}

// ── Service wrappers ─────────────────────────────────────────────────

fn fetch_events_via_service(year: i32, month: u32) -> Result<Vec<CalEvent>, String> {
    let client = SyncRpcClient::for_service("calendar");
    let start = format!("{:04}-{:02}-01T00:00:00", year, month);
    let last_day = last_day_of_month(year, month);
    let end = format!("{:04}-{:02}-{:02}T23:59:59", year, month, last_day);
    let result = client
        .call("calendar.events", serde_json::json!({ "start": start, "end": end }))
        .map_err(|e| e.message)?;
    let svc_events: Vec<yantrik_ipc_contracts::calendar::CalendarEvent> =
        serde_json::from_value(result).map_err(|e| e.to_string())?;

    let colors = [
        slint::Color::from_rgb_u8(0x4E, 0x79, 0xA7),
        slint::Color::from_rgb_u8(0xF2, 0x8E, 0x2C),
        slint::Color::from_rgb_u8(0xE1, 0x57, 0x59),
        slint::Color::from_rgb_u8(0x76, 0xB7, 0xB2),
        slint::Color::from_rgb_u8(0x59, 0xA1, 0x4F),
    ];

    Ok(svc_events.iter().enumerate().map(|(i, e)| CalEvent {
        id: e.id.clone(),
        title: e.title.clone(),
        start: e.start.clone(),
        end: e.end.clone(),
        notes: e.description.clone(),
        is_all_day: e.is_all_day,
        color: colors[i % colors.len()],
    }).collect())
}

fn create_event_via_service(title: &str, start: &str, end: &str, notes: &str) -> Result<String, String> {
    let client = SyncRpcClient::for_service("calendar");
    let result = client
        .call("calendar.create_event", serde_json::json!({
            "title": title, "start": start, "end": end, "description": notes,
        }))
        .map_err(|e| e.message)?;
    // Return the event ID
    let event: yantrik_ipc_contracts::calendar::CalendarEvent =
        serde_json::from_value(result).map_err(|e| e.to_string())?;
    Ok(event.id)
}

fn delete_event_via_service(event_id: &str) -> Result<(), String> {
    let client = SyncRpcClient::for_service("calendar");
    client
        .call("calendar.delete_event", serde_json::json!({ "event_id": event_id }))
        .map_err(|e| e.message)?;
    Ok(())
}

// ── Date helpers ─────────────────────────────────────────────────────

fn last_day_of_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 { 29 } else { 28 }
        }
        _ => 30,
    }
}

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

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January", 2 => "February", 3 => "March", 4 => "April",
        5 => "May", 6 => "June", 7 => "July", 8 => "August",
        9 => "September", 10 => "October", 11 => "November", 12 => "December",
        _ => "?",
    }
}

fn today() -> (i32, u32, u32) {
    let now = chrono::Local::now();
    (now.year(), now.month(), now.day())
}

use chrono::Datelike;

fn build_month_grid(year: i32, month: u32, events: &[CalEvent], today_day: Option<u32>) -> Vec<CalendarDay> {
    let first_dow = day_of_week_for_date(year, month, 1);
    let last = last_day_of_month(year, month);
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

fn events_for_day(events: &[CalEvent], year: i32, month: u32, day: i32) -> Vec<CalendarEvent> {
    let prefix = format!("{:04}-{:02}-{:02}", year, month, day);
    events.iter().filter(|e| e.start.starts_with(&prefix)).map(|e| {
        let time_text = if e.is_all_day {
            "All day".to_string()
        } else {
            let start_time = e.start.split('T').nth(1).unwrap_or("").to_string();
            let end_time = e.end.split('T').nth(1).unwrap_or("").to_string();
            format!("{} - {}", start_time, end_time)
        };
        CalendarEvent {
            id: 0,
            title: e.title.clone().into(),
            date_text: prefix.clone().into(),
            time_text: time_text.into(),
            color: e.color,
            is_all_day: e.is_all_day,
        }
    }).collect()
}

// ── Wire all callbacks ───────────────────────────────────────────────

// ── The control surface ──────────────────────────────────────────────
//
// "What is on my calendar today" should never be answered by photographing a month grid and
// asking a vision model to read the numbers. See `yantrik_app_runtime::control`.

fn publish_control(app: &CalendarApp, state: Rc<RefCell<CalState>>) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let weak = app.as_weak();
        let st = state.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Calendar — closing");
            };
            let month = ui.get_month_title().to_string();
            let day = ui.get_selected_day();

            let today_model = ui.get_events_today();
            let today: Vec<serde_json::Value> = (0..today_model.row_count())
                .filter_map(|i| today_model.row_data(i))
                .map(|e| {
                    serde_json::json!({
                        "title": e.title.to_string(),
                        "date": e.date_text.to_string(),
                        "time": e.time_text.to_string(),
                        "all_day": e.is_all_day,
                    })
                })
                .collect();

            // Which days of the month have anything on them. Six numbers instead of a picture of
            // a grid, and it is what a caller planning around the month actually needs.
            let grid = ui.get_days();
            let busy: Vec<serde_json::Value> = (0..grid.row_count())
                .filter_map(|i| grid.row_data(i))
                .filter(|d| d.is_current_month && d.has_events)
                .map(|d| serde_json::json!({ "day": d.day_number, "events": d.event_count }))
                .collect();

            let summary = if today.is_empty() {
                format!("Calendar — {month}, nothing on day {day}")
            } else if today.len() == 1 {
                format!(
                    "Calendar — {month}, one thing on day {day}: {}",
                    today[0]["title"].as_str().unwrap_or_default()
                )
            } else {
                format!("Calendar — {month}, {} things on day {day}", today.len())
            };

            let s = st.borrow();
            View::new(summary)
                .with("month", month)
                .with("year", s.year)
                .with("month_number", s.month as i64)
                .with("selected_day", day)
                .with("view", match ui.get_view_mode() {
                    1 => "week",
                    2 => "day",
                    _ => "month",
                })
                .with("events_on_selected_day", serde_json::Value::Array(today))
                .with("days_with_events", serde_json::Value::Array(busy))
                .with("events_this_month", s.events.len() as i64)
        }
    };

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "Calendar window is gone".to_string());

    let day_ui = ui_for.clone();
    let move_ui = ui_for.clone();
    let today_ui = ui_for.clone();
    let add_ui = ui_for.clone();
    let view_ui = ui_for;

    App::new("calendar")
        .describe(describe)
        .action(
            Action::new("select_day", "Show what is on one day of the month shown")
                .arg(Param::number("day").describe("Day of the month, 1-31")),
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
                .arg(Param::text("time").describe("HH:MM, 24-hour"))
                .arg(Param::text("notes").optional()),
            move |args| {
                let ui = add_ui()?;
                let title = args["title"].as_str().unwrap_or_default().trim().to_string();
                let date = args["date"].as_str().unwrap_or_default().trim().to_string();
                let time = args["time"].as_str().unwrap_or_default().trim().to_string();
                if title.is_empty() {
                    return Err("`title` is empty".into());
                }
                // Checked here rather than let the service reject a malformed timestamp: the
                // error a caller can act on names the format it should have used.
                if date.len() != 10 || date.matches('-').count() != 2 {
                    return Err(format!("`date` should look like 2026-09-06, not `{date}`"));
                }
                if !time.contains(':') {
                    return Err(format!("`time` should look like 14:30, not `{time}`"));
                }
                let notes = args["notes"].as_str().unwrap_or_default().to_string();
                ui.invoke_save_event(title.clone().into(), date.clone().into(), time.clone().into(), notes.into());
                Ok(serde_json::json!({ "added": title, "on": format!("{date} {time}") }))
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
                Ok(serde_json::json!({ "view": args["view"].as_str().unwrap_or_default() }))
            },
        )
        .serve();
}

fn wire(app: &CalendarApp) {
    let (ty, tm, td) = today();
    let state = Rc::new(RefCell::new(CalState {
        year: ty,
        month: tm,
        events: Vec::new(),
    }));

    // Initial load
    {
        let mut st = state.borrow_mut();
        st.events = fetch_events_via_service(ty, tm).unwrap_or_default();
        let grid = build_month_grid(ty, tm, &st.events, Some(td));
        let day_events = events_for_day(&st.events, ty, tm, td as i32);
        app.set_month_title(format!("{} {}", month_name(tm), ty).into());
        app.set_days(ModelRc::new(VecModel::from(grid)));
        app.set_events_today(ModelRc::new(VecModel::from(day_events)));
        app.set_selected_day(td as i32);
    }

    // Published once the month is loaded, so the first `app.describe` reports real events.
    publish_control(app, state.clone());

    // ── Prev month ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_prev_month(move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = st.borrow_mut();
            if s.month == 1 {
                s.month = 12;
                s.year -= 1;
            } else {
                s.month -= 1;
            }
            s.events = fetch_events_via_service(s.year, s.month).unwrap_or_default();
            let (_, _, td_now) = today();
            let td_opt = if s.year == ty && s.month == tm { Some(td_now) } else { None };
            let grid = build_month_grid(s.year, s.month, &s.events, td_opt);
            ui.set_month_title(format!("{} {}", month_name(s.month), s.year).into());
            ui.set_days(ModelRc::new(VecModel::from(grid)));
            ui.set_selected_day(0);
            ui.set_events_today(ModelRc::new(VecModel::from(Vec::<CalendarEvent>::new())));
        });
    }

    // ── Next month ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_next_month(move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = st.borrow_mut();
            if s.month == 12 {
                s.month = 1;
                s.year += 1;
            } else {
                s.month += 1;
            }
            s.events = fetch_events_via_service(s.year, s.month).unwrap_or_default();
            let (_, _, td_now) = today();
            let td_opt = if s.year == ty && s.month == tm { Some(td_now) } else { None };
            let grid = build_month_grid(s.year, s.month, &s.events, td_opt);
            ui.set_month_title(format!("{} {}", month_name(s.month), s.year).into());
            ui.set_days(ModelRc::new(VecModel::from(grid)));
            ui.set_selected_day(0);
            ui.set_events_today(ModelRc::new(VecModel::from(Vec::<CalendarEvent>::new())));
        });
    }

    // ── Day clicked ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_day_clicked(move |day| {
            let Some(ui) = weak.upgrade() else { return };
            let s = st.borrow();
            ui.set_selected_day(day);
            let day_events = events_for_day(&s.events, s.year, s.month, day);
            ui.set_events_today(ModelRc::new(VecModel::from(day_events)));
        });
    }

    // ── Add event (open form) ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_add_event(move || {
            let Some(ui) = weak.upgrade() else { return };
            let s = st.borrow();
            let day = ui.get_selected_day();
            let day = if day <= 0 { 1 } else { day };
            ui.set_event_date(format!("{:04}-{:02}-{:02}", s.year, s.month, day).into());
            ui.set_event_time("09:00".into());
            ui.set_event_title(SharedString::default());
            ui.set_event_notes(SharedString::default());
            ui.set_show_event_form(true);
        });
    }

    // ── Save event ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_save_event(move |title, date, time, notes| {
            let Some(ui) = weak.upgrade() else { return };
            let start = format!("{}T{}:00", date, time);
            // Default 1 hour duration
            let hour: i32 = time.split(':').next().unwrap_or("9").parse().unwrap_or(9);
            let end = format!("{}T{:02}:{}:00", date, hour + 1,
                time.split(':').nth(1).unwrap_or("00"));

            if create_event_via_service(&title, &start, &end, &notes).is_ok() {
                // Refresh
                let mut s = st.borrow_mut();
                s.events = fetch_events_via_service(s.year, s.month).unwrap_or_default();
                let (_, _, td_now) = today();
                let td_opt = if s.year == ty && s.month == tm { Some(td_now) } else { None };
                let grid = build_month_grid(s.year, s.month, &s.events, td_opt);
                let day = ui.get_selected_day();
                let day_events = events_for_day(&s.events, s.year, s.month, day);
                ui.set_days(ModelRc::new(VecModel::from(grid)));
                ui.set_events_today(ModelRc::new(VecModel::from(day_events)));
            }
            ui.set_show_event_form(false);
        });
    }

    // ── Delete event ──
    {
        let weak = app.as_weak();
        let st = state.clone();
        app.on_delete_event(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            let s = st.borrow();
            let day = ui.get_selected_day();
            let prefix = format!("{:04}-{:02}-{:02}", s.year, s.month, day);
            let day_events: Vec<&CalEvent> = s.events.iter()
                .filter(|e| e.start.starts_with(&prefix)).collect();
            let idx = idx as usize;
            if idx >= day_events.len() { return; }
            let event_id = day_events[idx].id.clone();
            drop(s);

            if delete_event_via_service(&event_id).is_ok() {
                let mut s = st.borrow_mut();
                s.events = fetch_events_via_service(s.year, s.month).unwrap_or_default();
                let (_, _, td_now) = today();
                let td_opt = if s.year == ty && s.month == tm { Some(td_now) } else { None };
                let grid = build_month_grid(s.year, s.month, &s.events, td_opt);
                let day_events = events_for_day(&s.events, s.year, s.month, day);
                ui.set_days(ModelRc::new(VecModel::from(grid)));
                ui.set_events_today(ModelRc::new(VecModel::from(day_events)));
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
            let mut s = st.borrow_mut();
            s.year = ny;
            s.month = nm;
            s.events = fetch_events_via_service(ny, nm).unwrap_or_default();
            let grid = build_month_grid(ny, nm, &s.events, Some(nd));
            let day_events = events_for_day(&s.events, ny, nm, nd as i32);
            ui.set_month_title(format!("{} {}", month_name(nm), ny).into());
            ui.set_days(ModelRc::new(VecModel::from(grid)));
            ui.set_selected_day(nd as i32);
            ui.set_events_today(ModelRc::new(VecModel::from(day_events)));
        });
    }

    // ── Switch view ──
    {
        let weak = app.as_weak();
        app.on_switch_view(move |mode| {
            if let Some(ui) = weak.upgrade() {
                ui.set_view_mode(mode);
            }
        });
    }

    // ── Stubs for AI / enterprise features ──
    app.on_ai_explain_pressed(|| { tracing::info!("AI explain (standalone mode)"); });
    app.on_ai_dismiss(|| {});
    app.on_cal_add_attendee(|_, _| { tracing::info!("Add attendee (standalone mode)"); });
    app.on_cal_remove_attendee(|_| { tracing::info!("Remove attendee (standalone mode)"); });
    app.on_cal_set_reminder(|_| { tracing::info!("Set reminder (standalone mode)"); });
    app.on_cal_use_template(|_| { tracing::info!("Use template (standalone mode)"); });
    app.on_cal_toggle_template_panel(|| {});
}
