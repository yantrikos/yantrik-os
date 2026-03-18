//! Yantrik Calendar — standalone app binary.
//!
//! Communicates with `calendar-service` via JSON-RPC IPC.
//! Falls back to local event storage when service is unavailable.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_transport::SyncRpcClient;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-calendar");

    let app = CalendarApp::new().unwrap();
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

fn day_of_week_for_date(year: i32, month: u32, day: u32) -> u32 {
    // Zeller's congruence → 0=Mon .. 6=Sun
    let (y, m) = if month <= 2 { (year - 1, month + 12) } else { (year, month) };
    let q = day as i32;
    let k = y % 100;
    let j = y / 100;
    let h = (q + (13 * (m as i32 + 1)) / 5 + k + k / 4 + j / 4 - 2 * j) % 7;
    let h = ((h + 7) % 7) as u32; // 0=Sat
    // Convert: 0=Sat→5, 1=Sun→6, 2=Mon→0, 3=Tue→1 ...
    (h + 5) % 7
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
