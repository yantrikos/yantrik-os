//! Calendar wire module — month navigation, day selection, event CRUD.
//!
//! Syncs with Google Calendar API when configured (OAuth2 via email account).
//! Falls back to local-only events when calendar is not configured.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, CalendarDay, CalendarEvent};
use yantrikdb_companion::calendar;
use yantrikdb_companion::config::{CompanionConfig, EmailAccountConfig};
use yantrikdb_companion::email;

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

fn update_ui(ui: &App, state: &CalState) {
    ui.set_cal_month_title(state.month_title().into());
    let days = state.build_days();
    ui.set_cal_days(ModelRc::new(VecModel::from(days)));
    ui.set_cal_selected_day(state.selected_day as i32);

    let events = state.events_for_day(state.selected_day);
    ui.set_cal_events_today(ModelRc::new(VecModel::from(events)));
}

/// Try to load config and find an OAuth2 email account for Google Calendar.
fn load_calendar_account(config_path: &Option<std::path::PathBuf>) -> Option<(EmailAccountConfig, String)> {
    let path = config_path.as_ref()?;
    let config = CompanionConfig::from_yaml(path).ok()?;
    if !config.calendar.enabled {
        return None;
    }

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

    account.map(|a| (a, path.to_string_lossy().to_string()))
}

/// Fetch Google Calendar events for a month in a background thread.
fn fetch_month_events(
    account: EmailAccountConfig,
    config_path: String,
    year: i32,
    month: u32,
    state: Arc<Mutex<CalState>>,
    ui_weak: slint::Weak<App>,
) {
    std::thread::spawn(move || {
        let mut account = account;
        let token = match calendar::get_access_token(&mut account, Some(&config_path)) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Calendar OAuth2 token failed: {}", e);
                return;
            }
        };

        let month_len = days_in_month(year, month);
        let time_min = format!("{:04}-{:02}-01T00:00:00Z", year, month);
        let time_max = format!("{:04}-{:02}-{:02}T23:59:59Z", year, month, month_len);

        match calendar::list_events(&token, None, Some(&time_min), Some(&time_max), 100, None) {
            Ok(events) => {
                tracing::info!("Fetched {} calendar events for {:04}-{:02}", events.len(), year, month);
                if let Ok(mut s) = state.lock() {
                    s.load_google_events(events);
                    // Capture data needed for UI update
                    let month_title: slint::SharedString = s.month_title().into();
                    let days = s.build_days();
                    let selected_day = s.selected_day;
                    let events_for_day = s.events_for_day(selected_day);

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_cal_month_title(month_title);
                            ui.set_cal_days(ModelRc::new(VecModel::from(days)));
                            ui.set_cal_selected_day(selected_day as i32);
                            ui.set_cal_events_today(ModelRc::new(VecModel::from(events_for_day)));
                            ui.set_cal_is_loading(false);
                        }
                    });
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch calendar events: {}", e);
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
    let cal_account = load_calendar_account(&ctx.config_path);
    let has_google = cal_account.is_some();

    // If Google Calendar is configured, fetch events for current month
    if let Some((account, config_path)) = cal_account.clone() {
        ui.set_cal_is_loading(true);
        let year = state.lock().map(|s| s.year).unwrap_or(2026);
        let month = state.lock().map(|s| s.month).unwrap_or(3);
        fetch_month_events(account, config_path, year, month, state.clone(), ui.as_weak());
    }

    // ── Prev month ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        let cal_account = cal_account.clone();
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

                    if let Some((ref account, ref config_path)) = cal_account {
                        ui.set_cal_is_loading(true);
                        fetch_month_events(
                            account.clone(), config_path.clone(),
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

                    if let Some((ref account, ref config_path)) = cal_account {
                        ui.set_cal_is_loading(true);
                        fetch_month_events(
                            account.clone(), config_path.clone(),
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
                        if let Some((ref account, ref config_path)) = cal_account {
                            ui.set_cal_is_loading(true);
                            fetch_month_events(
                                account.clone(), config_path.clone(),
                                s.year, s.month, st.clone(), ui.as_weak(),
                            );
                        }
                    }
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

            // Create on Google Calendar in background
            if let (Some(local_id), Some((ref account, ref config_path))) = (local_id, &cal_account) {
                let account = account.clone();
                let config_path = config_path.clone();
                let st2 = st.clone();

                std::thread::spawn(move || {
                    let mut account = account;
                    let token = match calendar::get_access_token(&mut account, Some(&config_path)) {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("Calendar create failed (token): {}", e);
                            return;
                        }
                    };

                    let is_all_day = time_str.is_empty();
                    let (start, end) = if is_all_day {
                        // All-day event: use date format
                        (date_str.clone(), date_str.clone())
                    } else {
                        // Parse "HH:MM - HH:MM" format
                        let parts: Vec<&str> = time_str.split(" - ").collect();
                        let start_time = parts.first().unwrap_or(&"09:00");
                        let end_time = parts.get(1).unwrap_or(&"10:00");
                        (
                            format!("{}T{}:00", date_str, start_time),
                            format!("{}T{}:00", date_str, end_time),
                        )
                    };

                    let desc = if notes_str.is_empty() { None } else { Some(notes_str.as_str()) };

                    match calendar::create_event(&token, None, &title_str, &start, &end, desc, None, is_all_day) {
                        Ok(event) => {
                            tracing::info!("Created Google Calendar event: {} ({})", event.summary, event.id);
                            // Update the local event with the Google ID
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

            // Delete from Google Calendar in background
            if let (Some(gid), Some((ref account, ref config_path))) = (google_id, &cal_account) {
                let account = account.clone();
                let config_path = config_path.clone();
                std::thread::spawn(move || {
                    let mut account = account;
                    let token = match calendar::get_access_token(&mut account, Some(&config_path)) {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("Calendar delete failed (token): {}", e);
                            return;
                        }
                    };
                    if let Err(e) = calendar::delete_event(&token, None, &gid) {
                        tracing::warn!("Failed to delete Google Calendar event {}: {}", gid, e);
                    } else {
                        tracing::info!("Deleted Google Calendar event: {}", gid);
                    }
                });
            }
        });
    }

    // ── Periodic sync timer (every 10 minutes) ──
    if let Some((account, config_path)) = cal_account {
        let sync_timer = Timer::default();
        let st = state.clone();
        let ui_weak = ui.as_weak();
        sync_timer.start(TimerMode::Repeated, std::time::Duration::from_secs(600), move || {
            if let Ok(s) = st.try_lock() {
                let year = s.year;
                let month = s.month;
                fetch_month_events(
                    account.clone(), config_path.clone(),
                    year, month, st.clone(), ui_weak.clone(),
                );
            }
        });
        // Keep timer alive — leak it since it's a long-lived app
        std::mem::forget(sync_timer);
    }
}
