//! Calendar wire module — month navigation, day selection, event CRUD.
//!
//! Syncs with Google Calendar API when configured (OAuth2 via email account).
//! Falls back to local-only events when calendar is not configured.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, CalendarAttendee, CalendarDay, CalendarEvent, CalendarTemplate, CalendarTimeEvent};
use yantrik_companion::calendar;
use yantrik_companion::config::{CompanionConfig, EmailAccountConfig};
use yantrik_companion::email;
use yantrik_ipc_contracts::calendar as cal_contract;

/// Calendar state — tracks current month/year and events.
struct CalState {
    year: i32,
    month: u32, // 1-12
    selected_day: u32,
    events: Vec<EventRecord>,
    next_id: i32,
}

#[derive(Clone)]
struct EventRecord {
    id: i32,
    google_id: Option<String>, // Google Calendar event ID
    title: String,
    year: i32,
    month: u32,
    day: u32,
    time_text: String,
    notes: String,
    is_all_day: bool,
}

impl CalState {
    fn new() -> Self {
        let now = current_date();
        Self {
            year: now.0,
            month: now.1,
            selected_day: now.2,
            events: Vec::new(),
            next_id: 1,
        }
    }

    fn month_title(&self) -> String {
        let month_name = match self.month {
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
        };
        format!("{} {}", month_name, self.year)
    }

    fn build_days(&self) -> Vec<CalendarDay> {
        let today = current_date();
        let first_dow = day_of_week(self.year, self.month, 1);
        let month_len = days_in_month(self.year, self.month);

        let prev_month_days = if self.month == 1 {
            days_in_month(self.year - 1, 12)
        } else {
            days_in_month(self.year, self.month - 1)
        };

        let mut cells = Vec::with_capacity(42);

        for i in 0..first_dow {
            let day = prev_month_days - first_dow + i + 1;
            cells.push(CalendarDay {
                day_number: day as i32,
                is_today: false,
                is_selected: false,
                is_current_month: false,
                has_events: false,
                event_count: 0,
            });
        }

        for d in 1..=month_len {
            let is_today = self.year == today.0 && self.month == today.1 && d == today.2;
            let is_selected = d == self.selected_day;
            let event_count = self
                .events
                .iter()
                .filter(|e| e.year == self.year && e.month == self.month && e.day == d)
                .count() as i32;

            cells.push(CalendarDay {
                day_number: d as i32,
                is_today,
                is_selected,
                is_current_month: true,
                has_events: event_count > 0,
                event_count,
            });
        }

        let remaining = 42 - cells.len();
        for d in 1..=remaining {
            cells.push(CalendarDay {
                day_number: d as i32,
                is_today: false,
                is_selected: false,
                is_current_month: false,
                has_events: false,
                event_count: 0,
            });
        }

        cells
    }

    fn events_for_day(&self, day: u32) -> Vec<CalendarEvent> {
        let colors = [
            slint::Color::from_argb_u8(255, 90, 200, 212),  // cyan
            slint::Color::from_argb_u8(255, 212, 165, 116),  // amber
            slint::Color::from_argb_u8(255, 160, 120, 200),  // purple
            slint::Color::from_argb_u8(255, 120, 200, 140),  // green
        ];

        self.events
            .iter()
            .filter(|e| e.year == self.year && e.month == self.month && e.day == day)
            .enumerate()
            .map(|(i, e)| CalendarEvent {
                id: e.id,
                title: e.title.clone().into(),
                date_text: format!("{} {}, {}", month_short(self.month), e.day, self.year).into(),
                time_text: if e.is_all_day {
                    "All day".into()
                } else {
                    e.time_text.clone().into()
                },
                color: colors[i % colors.len()],
                is_all_day: e.is_all_day,
            })
            .collect()
    }

    /// Build week-day-labels for the week containing `selected_day`.
    /// Returns (labels, week_start_day, week_start_month, week_start_year).
    fn week_info(&self) -> (Vec<String>, Vec<(i32, u32, u32)>) {
        let dow = day_of_week(self.year, self.month, self.selected_day);
        // dow: 0=Sun, 1=Mon, ... 6=Sat. Week starts on Sunday.
        let day_names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

        let mut labels = Vec::with_capacity(7);
        let mut dates = Vec::with_capacity(7);

        for i in 0..7u32 {
            let offset = i as i32 - dow as i32;
            let (y, m, d) = offset_date(self.year, self.month, self.selected_day, offset);
            labels.push(format!("{} {}", day_names[i as usize], d));
            dates.push((y, m, d));
        }

        (labels, dates)
    }

    /// Build CalendarTimeEvent list for the week containing `selected_day`.
    fn week_time_events(&self) -> Vec<CalendarTimeEvent> {
        let colors = [
            slint::Color::from_argb_u8(255, 90, 200, 212),
            slint::Color::from_argb_u8(255, 212, 165, 116),
            slint::Color::from_argb_u8(255, 160, 120, 200),
            slint::Color::from_argb_u8(255, 120, 200, 140),
        ];

        let (_, dates) = self.week_info();
        let mut result = Vec::new();
        let mut color_idx = 0;

        for (day_index, &(y, m, d)) in dates.iter().enumerate() {
            for e in &self.events {
                if e.year == y && e.month == m && e.day == d {
                    let (start_hour, start_min, duration) = parse_time_range(&e.time_text, e.is_all_day);
                    result.push(CalendarTimeEvent {
                        title: e.title.clone().into(),
                        start_hour,
                        start_min,
                        duration_min: duration,
                        day_index: day_index as i32,
                        color: colors[color_idx % colors.len()],
                    });
                    color_idx += 1;
                }
            }
        }
        result
    }

    /// Build CalendarTimeEvent list for a single selected day.
    fn day_time_events(&self) -> Vec<CalendarTimeEvent> {
        let colors = [
            slint::Color::from_argb_u8(255, 90, 200, 212),
            slint::Color::from_argb_u8(255, 212, 165, 116),
            slint::Color::from_argb_u8(255, 160, 120, 200),
            slint::Color::from_argb_u8(255, 120, 200, 140),
        ];

        self.events
            .iter()
            .filter(|e| e.year == self.year && e.month == self.month && e.day == self.selected_day)
            .enumerate()
            .map(|(i, e)| {
                let (start_hour, start_min, duration) = parse_time_range(&e.time_text, e.is_all_day);
                CalendarTimeEvent {
                    title: e.title.clone().into(),
                    start_hour,
                    start_min,
                    duration_min: duration,
                    day_index: 0,
                    color: colors[i % colors.len()],
                }
            })
            .collect()
    }

    /// Day view title like "Monday, March 9".
    fn day_view_title(&self) -> String {
        let dow = day_of_week(self.year, self.month, self.selected_day);
        let day_name = match dow {
            0 => "Sunday",
            1 => "Monday",
            2 => "Tuesday",
            3 => "Wednesday",
            4 => "Thursday",
            5 => "Friday",
            6 => "Saturday",
            _ => "?",
        };
        let month_name = match self.month {
            1 => "January", 2 => "February", 3 => "March", 4 => "April",
            5 => "May", 6 => "June", 7 => "July", 8 => "August",
            9 => "September", 10 => "October", 11 => "November", 12 => "December",
            _ => "?",
        };
        format!("{}, {} {}", day_name, month_name, self.selected_day)
    }

    /// Replace all events with Google Calendar data for the current month.
    fn load_google_events(&mut self, events: Vec<calendar::CalEvent>) {
        // Remove existing Google Calendar events for this month
        self.events.retain(|e| e.google_id.is_none() || e.year != self.year || e.month != self.month);

        for ge in events {
            let (year, month, day) = parse_event_date(&ge.start);
            let time_text = if ge.is_all_day {
                String::new()
            } else {
                format_event_time(&ge.start, &ge.end)
            };

            let id = self.next_id;
            self.next_id += 1;
            self.events.push(EventRecord {
                id,
                google_id: Some(ge.id),
                title: ge.summary,
                year,
                month,
                day,
                time_text,
                notes: ge.description.unwrap_or_default(),
                is_all_day: ge.is_all_day,
            });
        }
    }
}

/// Parse ISO8601 date/datetime into (year, month, day).
fn parse_event_date(s: &str) -> (i32, u32, u32) {
    // "2026-03-05" or "2026-03-05T14:00:00+05:30"
    let date_part = if s.contains('T') {
        s.split('T').next().unwrap_or(s)
    } else {
        s
    };

    let parts: Vec<&str> = date_part.split('-').collect();
    if parts.len() >= 3 {
        let y = parts[0].parse::<i32>().unwrap_or(2026);
        let m = parts[1].parse::<u32>().unwrap_or(1);
        let d = parts[2].parse::<u32>().unwrap_or(1);
        (y, m, d)
    } else {
        let now = current_date();
        (now.0, now.1, now.2)
    }
}

/// Format event start/end times into a readable string like "14:00 - 15:00".
fn format_event_time(start: &str, end: &str) -> String {
    let start_time = extract_time(start);
    let end_time = extract_time(end);
    if start_time.is_empty() && end_time.is_empty() {
        return String::new();
    }
    format!("{} - {}", start_time, end_time)
}

/// Extract time portion from ISO8601 datetime, e.g., "14:00" from "2026-03-05T14:00:00+05:30".
fn extract_time(s: &str) -> String {
    if let Some(time_part) = s.split('T').nth(1) {
        // Take HH:MM from "14:00:00+05:30" or "14:00:00Z"
        let clean = time_part.split('+').next().unwrap_or(time_part);
        let clean = clean.split('-').next().unwrap_or(clean);
        let clean = clean.trim_end_matches('Z');
        // Take just HH:MM
        let parts: Vec<&str> = clean.split(':').collect();
        if parts.len() >= 2 {
            return format!("{}:{}", parts[0], parts[1]);
        }
    }
    String::new()
}

fn month_short(m: u32) -> &'static str {
    match m {
        1 => "Jan", 2 => "Feb", 3 => "Mar", 4 => "Apr",
        5 => "May", 6 => "Jun", 7 => "Jul", 8 => "Aug",
        9 => "Sep", 10 => "Oct", 11 => "Nov", 12 => "Dec",
        _ => "???",
    }
}

fn current_date() -> (i32, u32, u32) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let days = (secs / 86400) as i32;
    let (y, m, d) = civil_from_days(days);
    (y, m as u32, d as u32)
}

fn civil_from_days(z: i32) -> (i32, i32, i32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as i32, d as i32)
}

fn day_of_week(year: i32, month: u32, day: u32) -> u32 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let m = month as usize;
    ((y + y / 4 - y / 100 + y / 400 + t[m - 1] + day as i32) % 7) as u32
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 29 } else { 28 }
        }
        _ => 30,
    }
}

/// Offset a date by `offset` days (can be negative).
fn offset_date(year: i32, month: u32, day: u32, offset: i32) -> (i32, u32, u32) {
    let mut d = day as i32 + offset;
    let mut m = month as i32;
    let mut y = year;

    while d < 1 {
        m -= 1;
        if m < 1 {
            m = 12;
            y -= 1;
        }
        d += days_in_month(y, m as u32) as i32;
    }

    loop {
        let dim = days_in_month(y, m as u32) as i32;
        if d <= dim {
            break;
        }
        d -= dim;
        m += 1;
        if m > 12 {
            m = 1;
            y += 1;
        }
    }

    (y, m as u32, d as u32)
}

/// Parse a time range string like "14:00 - 15:00" into (start_hour, start_min, duration_min).
/// For all-day events, returns (0, 0, 60) as a 1-hour block at midnight.
fn parse_time_range(time_text: &str, is_all_day: bool) -> (i32, i32, i32) {
    if is_all_day || time_text.is_empty() {
        return (0, 0, 60);
    }

    let parts: Vec<&str> = time_text.split(" - ").collect();
    let start = parts.first().unwrap_or(&"09:00");
    let end = parts.get(1).unwrap_or(&"10:00");

    let (sh, sm) = parse_hhmm(start);
    let (eh, em) = parse_hhmm(end);

    let start_mins = sh * 60 + sm;
    let end_mins = eh * 60 + em;
    let duration = if end_mins > start_mins { end_mins - start_mins } else { 60 };

    (sh, sm, duration)
}

/// Parse "HH:MM" into (hour, minute).
fn parse_hhmm(s: &str) -> (i32, i32) {
    let parts: Vec<&str> = s.trim().split(':').collect();
    let h = parts.first().and_then(|p| p.parse::<i32>().ok()).unwrap_or(0);
    let m = parts.get(1).and_then(|p| p.parse::<i32>().ok()).unwrap_or(0);
    (h.clamp(0, 23), m.clamp(0, 59))
}

/// Get current hour (0-23) from system time.
fn current_hour() -> i32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    // This gives UTC hour; good enough for now
    ((secs % 86400) / 3600) as i32
}

fn update_ui(ui: &App, state: &CalState) {
    ui.set_cal_month_title(state.month_title().into());
    let days = state.build_days();
    ui.set_cal_days(ModelRc::new(VecModel::from(days)));
    ui.set_cal_selected_day(state.selected_day as i32);

    let events = state.events_for_day(state.selected_day);
    ui.set_cal_events_today(ModelRc::new(VecModel::from(events)));

    // Update current hour
    ui.set_cal_current_hour(current_hour());

    // Update view-mode-specific data
    update_view_data(ui, state);
}

fn update_view_data(ui: &App, state: &CalState) {
    let mode = ui.get_cal_view_mode();

    if mode == 1 {
        // Week view
        let (labels, _) = state.week_info();
        let labels_shared: Vec<slint::SharedString> = labels.into_iter().map(|s| s.into()).collect();
        ui.set_cal_week_day_labels(ModelRc::new(VecModel::from(labels_shared)));
        let week_evts = state.week_time_events();
        ui.set_cal_week_events(ModelRc::new(VecModel::from(week_evts)));
    } else if mode == 2 {
        // Day view
        let day_evts = state.day_time_events();
        ui.set_cal_day_events(ModelRc::new(VecModel::from(day_evts)));
        ui.set_cal_day_view_title(state.day_view_title().into());
    }
}

/// Try to load config and find an OAuth2 email account for Google Calendar.
/// Returns (account, config_path, db_path).
fn load_calendar_account(config_path: &Option<std::path::PathBuf>) -> Option<(EmailAccountConfig, String, String)> {
    let path = config_path.as_ref()?;
    let config = CompanionConfig::from_yaml(path).ok()?;
    if !config.calendar.enabled {
        return None;
    }

    let db_path = config.yantrikdb.db_path.clone();
    let preferred = config.calendar.account.as_deref();

    // Find OAuth2 account
    let account = if let Some(name) = preferred {
        let lower = name.to_lowercase();
        config.email.accounts.iter()
            .find(|a| a.auth_method.as_deref() == Some("oauth2")
                && (a.name.to_lowercase().contains(&lower) || a.email.to_lowercase().contains(&lower)))
            .cloned()
    } else {
        config.email.accounts.iter()
            .find(|a| a.auth_method.as_deref() == Some("oauth2"))
            .cloned()
    };

    account.map(|a| (a, path.to_string_lossy().to_string(), db_path))
}

/// Try fetching calendar events for a month via the calendar-service JSON-RPC.
/// Returns `Ok(events)` converted to `calendar::CalEvent` format, or `Err` if the service
/// is unreachable or returns an error.
fn fetch_events_via_service(year: i32, month: u32) -> Result<Vec<calendar::CalEvent>, String> {
    use yantrik_ipc_transport::SyncRpcClient;

    let client = SyncRpcClient::for_service("calendar");
    let month_len = days_in_month(year, month);
    let start_date = format!("{:04}-{:02}-01", year, month);
    let end_date = format!("{:04}-{:02}-{:02}", year, month, month_len);

    let result = client
        .call(
            "calendar.events",
            serde_json::json!({ "start_date": start_date, "end_date": end_date }),
        )
        .map_err(|e| e.message)?;

    let svc_events: Vec<cal_contract::CalendarEvent> =
        serde_json::from_value(result).map_err(|e| e.to_string())?;

    // Convert contract events → companion CalEvent for load_google_events compatibility.
    let events = svc_events
        .into_iter()
        .map(|e| calendar::CalEvent {
            id: e.id,
            summary: e.title,
            description: if e.description.is_empty() { None } else { Some(e.description) },
            location: e.location,
            start: e.start,
            end: e.end,
            is_all_day: e.is_all_day,
            status: "confirmed".to_string(),
            html_link: None,
        })
        .collect();

    Ok(events)
}

/// Try creating a calendar event via the calendar-service JSON-RPC.
/// Returns `Ok(service_event_id)` on success, `Err` if unreachable.
fn create_event_via_service(
    title: &str,
    start: &str,
    end: &str,
    description: &str,
) -> Result<String, String> {
    use yantrik_ipc_transport::SyncRpcClient;

    let client = SyncRpcClient::for_service("calendar");
    let mut params = serde_json::json!({
        "title": title,
        "start": start,
        "end": end,
    });
    if !description.is_empty() {
        params["description"] = serde_json::Value::String(description.to_string());
    }

    let result = client
        .call("calendar.create_event", params)
        .map_err(|e| e.message)?;

    let created: cal_contract::CalendarEvent =
        serde_json::from_value(result).map_err(|e| e.to_string())?;
    Ok(created.id)
}

/// Try deleting a calendar event via the calendar-service JSON-RPC.
fn delete_event_via_service(event_id: &str) -> Result<(), String> {
    use yantrik_ipc_transport::SyncRpcClient;

    let client = SyncRpcClient::for_service("calendar");
    client
        .call(
            "calendar.delete_event",
            serde_json::json!({ "id": event_id }),
        )
        .map_err(|e| e.message)?;
    Ok(())
}

/// Fetch Google Calendar events for a month in a background thread.
/// Tries the calendar-service via JSON-RPC first, falling back to direct
/// Google Calendar API + local cache.
fn fetch_month_events(
    account: EmailAccountConfig,
    config_path: String,
    year: i32,
    month: u32,
    state: Arc<Mutex<CalState>>,
    ui_weak: slint::Weak<App>,
    db_path: Option<String>,
) {
    std::thread::spawn(move || {
        // ── Try calendar-service first ──
        match fetch_events_via_service(year, month) {
            Ok(events) => {
                tracing::info!(
                    "Fetched {} calendar events via service for {:04}-{:02}",
                    events.len(), year, month,
                );
                apply_fetched_events(events, &state, &ui_weak);
                return;
            }
            Err(e) => {
                tracing::debug!("Calendar service unavailable, falling back to direct: {}", e);
            }
        }

        // ── Fallback: direct Google Calendar API ──
        let mut account = account;
        let month_len = days_in_month(year, month);
        let time_min = format!("{:04}-{:02}-01T00:00:00Z", year, month);
        let time_max = format!("{:04}-{:02}-{:02}T23:59:59Z", year, month, month_len);

        // Try online first, fall back to cached events on any failure
        let events = match calendar::get_access_token(&mut account, Some(&config_path))
            .and_then(|token| calendar::list_events(&token, None, Some(&time_min), Some(&time_max), 100, None))
        {
            Ok(events) => {
                tracing::info!("Fetched {} calendar events for {:04}-{:02}", events.len(), year, month);
                if let Some(ref path) = db_path {
                    if let Ok(conn) = rusqlite::Connection::open(path) {
                        calendar::ensure_table(&conn);
                        calendar::cache_events(&conn, &events, &time_min, &time_max);
                        tracing::info!("Cached {} calendar events to local DB", events.len());
                    }
                }
                events
            }
            Err(e) => {
                tracing::warn!("Failed to fetch calendar events: {} — falling back to local cache", e);
                if let Some(ref path) = db_path {
                    if let Ok(conn) = rusqlite::Connection::open(path) {
                        let cached = calendar::get_cached_events(&conn, &time_min, &time_max);
                        if !cached.is_empty() {
                            tracing::info!("Loaded {} cached calendar events", cached.len());
                        }
                        cached
                    } else { vec![] }
                } else { vec![] }
            }
        };

        apply_fetched_events(events, &state, &ui_weak);
    });
}

/// Apply fetched events to the calendar state and update the UI.
/// Shared by both the service path and the direct Google Calendar path.
fn apply_fetched_events(
    events: Vec<calendar::CalEvent>,
    state: &Arc<Mutex<CalState>>,
    ui_weak: &slint::Weak<App>,
) {
    if let Ok(mut s) = state.lock() {
        s.load_google_events(events);
        let month_title: slint::SharedString = s.month_title().into();
        let days = s.build_days();
        let selected_day = s.selected_day;
        let events_for_day = s.events_for_day(selected_day);
        let week_labels: Vec<slint::SharedString> = s.week_info().0.into_iter().map(|l| l.into()).collect();
        let week_evts = s.week_time_events();
        let day_evts = s.day_time_events();
        let day_title: slint::SharedString = s.day_view_title().into();
        let cur_hour = current_hour();

        let ui_weak = ui_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_cal_month_title(month_title);
                ui.set_cal_days(ModelRc::new(VecModel::from(days)));
                ui.set_cal_selected_day(selected_day as i32);
                ui.set_cal_events_today(ModelRc::new(VecModel::from(events_for_day)));
                ui.set_cal_is_loading(false);
                ui.set_cal_current_hour(cur_hour);
                ui.set_cal_week_day_labels(ModelRc::new(VecModel::from(week_labels)));
                ui.set_cal_week_events(ModelRc::new(VecModel::from(week_evts)));
                ui.set_cal_day_events(ModelRc::new(VecModel::from(day_evts)));
                ui.set_cal_day_view_title(day_title);
            }
        });
    }
}

/// Service-only fetch — used when no Google Calendar account is configured.
/// Tries the calendar-service; if unavailable, clears loading state.
fn fetch_month_events_service_only(
    year: i32,
    month: u32,
    state: Arc<Mutex<CalState>>,
    ui_weak: slint::Weak<App>,
) {
    std::thread::spawn(move || {
        match fetch_events_via_service(year, month) {
            Ok(events) => {
                tracing::info!(
                    "Fetched {} calendar events via service for {:04}-{:02}",
                    events.len(), year, month,
                );
                apply_fetched_events(events, &state, &ui_weak);
            }
            Err(e) => {
                tracing::debug!("Calendar service unavailable (no Google fallback): {}", e);
                // Clear loading state
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_cal_is_loading(false);
                    }
                });
            }
        }
    });
}

pub fn wire(ui: &App, ctx: &AppContext) {
    // Use Arc<Mutex> for thread-safe state sharing
    let state = Arc::new(Mutex::new(CalState::new()));

    // Initial render
    if let Ok(s) = state.lock() {
        update_ui(ui, &s);
    }

    // Try to load Google Calendar config
    let cal_info = load_calendar_account(&ctx.config_path);
    // Split into (account, config_path) tuple + separate db_path for caching
    let db_path: Option<String> = cal_info.as_ref().map(|(_, _, dp)| dp.clone());
    let cal_account: Option<(EmailAccountConfig, String)> = cal_info.map(|(a, cp, _)| (a, cp));

    // Fetch events for current month — try service first, then Google Calendar
    {
        ui.set_cal_is_loading(true);
        let year = state.lock().map(|s| s.year).unwrap_or(2026);
        let month = state.lock().map(|s| s.month).unwrap_or(3);
        if let Some((account, config_path)) = cal_account.clone() {
            fetch_month_events(account, config_path, year, month, state.clone(), ui.as_weak(), db_path.clone());
        } else {
            fetch_month_events_service_only(year, month, state.clone(), ui.as_weak());
        }
    }

    // ── Prev month ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        let cal_account = cal_account.clone();
        let db_path = db_path.clone();
        ui.on_cal_prev_month(move || {
            if let Ok(mut s) = st.try_lock() {
                if s.month == 1 {
                    s.month = 12;
                    s.year -= 1;
                } else {
                    s.month -= 1;
                }
                s.selected_day = 1;
                if let Some(ui) = ui_weak.upgrade() {
                    update_ui(&ui, &s);
                    ui.set_cal_is_loading(true);

                    if let Some((ref account, ref config_path)) = cal_account {
                        fetch_month_events(
                            account.clone(), config_path.clone(),
                            s.year, s.month, st.clone(), ui.as_weak(), db_path.clone(),
                        );
                    } else {
                        fetch_month_events_service_only(
                            s.year, s.month, st.clone(), ui.as_weak(),
                        );
                    }
                }
            }
        });
    }

    // ── Next month ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        let cal_account = cal_account.clone();
        let db_path = db_path.clone();
        ui.on_cal_next_month(move || {
            if let Ok(mut s) = st.try_lock() {
                if s.month == 12 {
                    s.month = 1;
                    s.year += 1;
                } else {
                    s.month += 1;
                }
                s.selected_day = 1;
                if let Some(ui) = ui_weak.upgrade() {
                    update_ui(&ui, &s);
                    ui.set_cal_is_loading(true);

                    if let Some((ref account, ref config_path)) = cal_account {
                        fetch_month_events(
                            account.clone(), config_path.clone(),
                            s.year, s.month, st.clone(), ui.as_weak(), db_path.clone(),
                        );
                    } else {
                        fetch_month_events_service_only(
                            s.year, s.month, st.clone(), ui.as_weak(),
                        );
                    }
                }
            }
        });
    }

    // ── Day clicked ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_cal_day_clicked(move |day| {
            if let Ok(mut s) = st.try_lock() {
                s.selected_day = day as u32;
                if let Some(ui) = ui_weak.upgrade() {
                    update_ui(&ui, &s);
                }
            }
        });
    }

    // ── Today ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        let cal_account = cal_account.clone();
        let db_path = db_path.clone();
        ui.on_cal_today_pressed(move || {
            if let Ok(mut s) = st.try_lock() {
                let today = current_date();
                let month_changed = s.year != today.0 || s.month != today.1;
                s.year = today.0;
                s.month = today.1;
                s.selected_day = today.2;
                if let Some(ui) = ui_weak.upgrade() {
                    update_ui(&ui, &s);

                    if month_changed {
                        ui.set_cal_is_loading(true);
                        if let Some((ref account, ref config_path)) = cal_account {
                            fetch_month_events(
                                account.clone(), config_path.clone(),
                                s.year, s.month, st.clone(), ui.as_weak(), db_path.clone(),
                            );
                        } else {
                            fetch_month_events_service_only(
                                s.year, s.month, st.clone(), ui.as_weak(),
                            );
                        }
                    }
                }
            }
        });
    }

    // ── Switch view mode ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_cal_switch_view(move |mode| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_cal_view_mode(mode);
                ui.set_cal_current_hour(current_hour());
                if let Ok(s) = st.try_lock() {
                    update_view_data(&ui, &s);
                }
            }
        });
    }

    // ── Add event (open form) ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_cal_add_event(move || {
            if let Ok(s) = st.try_lock() {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_cal_event_title("".into());
                    ui.set_cal_event_date(
                        format!("{:04}-{:02}-{:02}", s.year, s.month, s.selected_day).into(),
                    );
                    ui.set_cal_event_time("".into());
                    ui.set_cal_event_notes("".into());
                    ui.set_cal_show_event_form(true);
                }
            }
        });
    }

    // ── Cancel event form ──
    {
        let ui_weak = ui.as_weak();
        ui.on_cal_cancel_event_form(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_cal_show_event_form(false);
            }
        });
    }

    // ── Save event ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        let cal_account = cal_account.clone();
        ui.on_cal_save_event(move |title, date, time, notes| {
            if title.is_empty() {
                return;
            }
            let title_str = title.to_string();
            let date_str = date.to_string();
            let time_str = time.to_string();
            let notes_str = notes.to_string();

            let local_id = if let Ok(mut s) = st.try_lock() {
                let id = s.next_id;
                s.next_id += 1;
                let year = s.year;
                let month = s.month;
                let day = s.selected_day;

                s.events.push(EventRecord {
                    id,
                    google_id: None,
                    title: title_str.clone(),
                    year,
                    month,
                    day,
                    time_text: time_str.clone(),
                    notes: notes_str.clone(),
                    is_all_day: time_str.is_empty(),
                });

                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_cal_show_event_form(false);
                    update_ui(&ui, &s);
                }
                Some(id)
            } else {
                None
            };

            // Create event remotely in background — try service first, then Google Calendar
            if let Some(local_id) = local_id {
                let cal_account = cal_account.clone();
                let st2 = st.clone();

                std::thread::spawn(move || {
                    let is_all_day = time_str.is_empty();
                    let (start, end) = if is_all_day {
                        (date_str.clone(), date_str.clone())
                    } else {
                        let parts: Vec<&str> = time_str.split(" - ").collect();
                        let start_time = parts.first().unwrap_or(&"09:00");
                        let end_time = parts.get(1).unwrap_or(&"10:00");
                        (
                            format!("{}T{}:00", date_str, start_time),
                            format!("{}T{}:00", date_str, end_time),
                        )
                    };

                    // ── Try calendar-service first ──
                    match create_event_via_service(&title_str, &start, &end, &notes_str) {
                        Ok(svc_id) => {
                            tracing::info!("Created event via calendar-service: {}", svc_id);
                            if let Ok(mut s) = st2.lock() {
                                if let Some(rec) = s.events.iter_mut().find(|e| e.id == local_id) {
                                    rec.google_id = Some(svc_id);
                                }
                            }
                            return;
                        }
                        Err(e) => {
                            tracing::debug!("Calendar service create unavailable, falling back: {}", e);
                        }
                    }

                    // ── Fallback: Google Calendar API ──
                    if let Some((ref account, ref config_path)) = cal_account {
                        let mut account = account.clone();
                        let token = match calendar::get_access_token(&mut account, Some(config_path)) {
                            Ok(t) => t,
                            Err(e) => {
                                tracing::warn!("Calendar create failed (token): {}", e);
                                return;
                            }
                        };

                        let desc = if notes_str.is_empty() { None } else { Some(notes_str.as_str()) };

                        match calendar::create_event(&token, None, &title_str, &start, &end, desc, None, is_all_day) {
                            Ok(event) => {
                                tracing::info!("Created Google Calendar event: {} ({})", event.summary, event.id);
                                if let Ok(mut s) = st2.lock() {
                                    if let Some(rec) = s.events.iter_mut().find(|e| e.id == local_id) {
                                        rec.google_id = Some(event.id);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to create Google Calendar event: {}", e);
                            }
                        }
                    }
                });
            }
        });
    }

    // ── Delete event ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        let cal_account = cal_account.clone();
        ui.on_cal_delete_event(move |id| {
            let google_id = if let Ok(mut s) = st.try_lock() {
                let gid = s.events.iter()
                    .find(|e| e.id == id)
                    .and_then(|e| e.google_id.clone());
                s.events.retain(|e| e.id != id);
                if let Some(ui) = ui_weak.upgrade() {
                    update_ui(&ui, &s);
                }
                gid
            } else {
                None
            };

            // Delete event remotely — try service first, then Google Calendar
            if let Some(gid) = google_id {
                let cal_account = cal_account.clone();
                let gid_clone = gid.clone();
                std::thread::spawn(move || {
                    // ── Try calendar-service first ──
                    match delete_event_via_service(&gid_clone) {
                        Ok(()) => {
                            tracing::info!("Deleted event via calendar-service: {}", gid_clone);
                            return;
                        }
                        Err(e) => {
                            tracing::debug!("Calendar service delete unavailable, falling back: {}", e);
                        }
                    }

                    // ── Fallback: Google Calendar API ──
                    if let Some((ref account, ref config_path)) = cal_account {
                        let mut account = account.clone();
                        let token = match calendar::get_access_token(&mut account, Some(config_path)) {
                            Ok(t) => t,
                            Err(e) => {
                                tracing::warn!("Calendar delete failed (token): {}", e);
                                return;
                            }
                        };
                        if let Err(e) = calendar::delete_event(&token, None, &gid_clone) {
                            tracing::warn!("Failed to delete Google Calendar event {}: {}", gid_clone, e);
                        } else {
                            tracing::info!("Deleted Google Calendar event: {}", gid_clone);
                        }
                    }
                });
            }
        });
    }

    // ── Template panel toggle ──
    {
        let ui_weak = ui.as_weak();
        ui.on_cal_toggle_template_panel(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let open = ui.get_cal_template_panel_open();
                ui.set_cal_template_panel_open(!open);
            }
        });
    }

    // ── Use template ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_cal_use_template(move |idx| {
            if let Some(ui) = ui_weak.upgrade() {
                let templates = ui.get_cal_templates();
                if let Some(tpl) = templates.row_data(idx as usize) {
                    ui.set_cal_event_title(tpl.title_template.clone());

                    // Compute end time from duration
                    if let Ok(s) = st.try_lock() {
                        let date_str = format!("{:04}-{:02}-{:02}", s.year, s.month, s.selected_day);
                        ui.set_cal_event_date(date_str.into());
                    }

                    // Set a default start time and compute end from duration
                    let duration = tpl.duration_min;
                    let start_hour = 9; // default 9:00 AM
                    let end_total = start_hour * 60 + duration;
                    let end_hour = (end_total / 60).min(23);
                    let end_min = end_total % 60;
                    let time_str = format!(
                        "{:02}:{:02} - {:02}:{:02}",
                        start_hour, 0, end_hour, end_min
                    );
                    ui.set_cal_event_time(time_str.into());
                    ui.set_cal_event_notes("".into());
                    ui.set_cal_show_event_form(true);
                    ui.set_cal_template_panel_open(false);
                }
            }
        });
    }

    // ── Attendee management ──
    let attendees: Rc<RefCell<Vec<(String, String, String)>>> = Rc::new(RefCell::new(Vec::new()));

    {
        let ui_weak = ui.as_weak();
        let attendees = attendees.clone();
        ui.on_cal_add_attendee(move |name, email_addr| {
            let name_str = name.to_string();
            let email_str = email_addr.to_string();
            if email_str.is_empty() {
                return;
            }
            attendees.borrow_mut().push((name_str, email_str, "pending".to_string()));
            if let Some(ui) = ui_weak.upgrade() {
                let model: Vec<CalendarAttendee> = attendees
                    .borrow()
                    .iter()
                    .map(|(n, e, r)| CalendarAttendee {
                        name: n.clone().into(),
                        email: e.clone().into(),
                        rsvp_status: r.clone().into(),
                    })
                    .collect();
                ui.set_cal_event_attendees(ModelRc::new(VecModel::from(model)));
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let attendees = attendees.clone();
        ui.on_cal_remove_attendee(move |idx| {
            let idx = idx as usize;
            let mut list = attendees.borrow_mut();
            if idx < list.len() {
                list.remove(idx);
            }
            if let Some(ui) = ui_weak.upgrade() {
                let model: Vec<CalendarAttendee> = list
                    .iter()
                    .map(|(n, e, r)| CalendarAttendee {
                        name: n.clone().into(),
                        email: e.clone().into(),
                        rsvp_status: r.clone().into(),
                    })
                    .collect();
                ui.set_cal_event_attendees(ModelRc::new(VecModel::from(model)));
            }
        });
    }

    // ── Reminder ──
    {
        let ui_weak = ui.as_weak();
        let reminder_mins: Rc<RefCell<i32>> = Rc::new(RefCell::new(15));
        ui.on_cal_set_reminder(move |mins| {
            *reminder_mins.borrow_mut() = mins;
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_cal_reminder_minutes(mins);
            }
        });
    }

    // Set timezone display
    {
        // Try to detect timezone from system
        let tz_display = detect_timezone();
        ui.set_cal_timezone_display(tz_display.into());
    }

    // ── Periodic sync timer (every 10 minutes) ──
    {
        let sync_timer = Timer::default();
        let st = state.clone();
        let ui_weak = ui.as_weak();
        let db_path = db_path.clone();
        let cal_account = cal_account.clone();
        sync_timer.start(TimerMode::Repeated, std::time::Duration::from_secs(600), move || {
            if let Ok(s) = st.try_lock() {
                let year = s.year;
                let month = s.month;
                if let Some((ref account, ref config_path)) = cal_account {
                    fetch_month_events(
                        account.clone(), config_path.clone(),
                        year, month, st.clone(), ui_weak.clone(), db_path.clone(),
                    );
                } else {
                    fetch_month_events_service_only(
                        year, month, st.clone(), ui_weak.clone(),
                    );
                }
            }
        });
        // Keep timer alive — leak it since it's a long-lived app
        std::mem::forget(sync_timer);
    }

    // ── AI Insights callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_cal_ai_explain(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let month = ui.get_cal_month_title().to_string();
        let events = ui.get_cal_events_today();
        let selected = ui.get_cal_selected_day();

        let mut context = format!("Month: {}, Selected day: {}\n", month, selected);
        if events.row_count() > 0 {
            context.push_str("Today's events:\n");
            for i in 0..events.row_count().min(10) {
                if let Some(e) = events.row_data(i) {
                    context.push_str(&format!("  - {} ({})\n", e.title, e.time_text));
                }
            }
        } else {
            context.push_str("No events for selected day.\n");
        }

        let prompt = super::ai_assist::calendar_insights_prompt(&context);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_cal_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_cal_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_cal_ai_response().to_string()),
            },
        );
    });

    let ui_weak = ui.as_weak();
    ui.on_cal_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_cal_ai_panel_open(false);
        }
    });
}

/// Detect system timezone for display.
fn detect_timezone() -> String {
    // Try TZ env var first
    if let Ok(tz) = std::env::var("TZ") {
        if !tz.is_empty() {
            return tz;
        }
    }

    // On Linux, read /etc/timezone
    #[cfg(target_os = "linux")]
    {
        if let Ok(tz) = std::fs::read_to_string("/etc/timezone") {
            let tz = tz.trim().to_string();
            if !tz.is_empty() {
                return tz;
            }
        }
    }

    // Fallback: compute UTC offset
    use std::time::{SystemTime, UNIX_EPOCH};
    let _secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    "UTC".to_string()
}
