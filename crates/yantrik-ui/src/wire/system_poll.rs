//! System poll wiring — 3-second timer that drains system events,
//! runs proactive features, handles keybinds, updates status bar,
//! and injects system context into the LLM prompt.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Timer, TimerMode};

use slint::{ModelRc, VecModel};

use crate::app_context::{self, AppContext};
use crate::{cards, features, lock, system_context, windows, App, DockItem, ProcessData, Tr, WindowItem};

/// Maximum number of data points in the chart history ring buffer.
const CHART_HISTORY_LEN: usize = 60;

/// Wire the system poll timer.
pub fn wire(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let observer = ctx.observer.clone();
    let registry = ctx.feature_registry.clone();
    let scorer = ctx.scorer.clone();
    let snapshot = ctx.system_snapshot.clone();
    let bridge = ctx.bridge.clone();
    let accumulator = ctx.accumulator.clone();
    let card_mgr = ctx.card_manager.clone();
    let notification_store = ctx.notification_store.clone();

    // Dedup cache: prevents recording the same system event to memory more than
    // once per 5 minutes. Key = event text, Value = last recorded time.
    let event_dedup: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
    const DEDUP_WINDOW: Duration = Duration::from_secs(300); // 5 minutes

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(3), move || {
        // 0. Sync interruptibility with focus mode state
        if let Some(ui) = ui_weak.upgrade() {
            let target = if ui.get_focus_mode() { 0.1 } else { 1.0 };
            scorer.borrow_mut().set_interruptibility(target);
        }

        // 1. Drain all pending system events
        let events = observer.drain();
        if events.is_empty() {
            // Still tick features (for time-based logic like FocusFlow)
            let snap = snapshot.borrow();
            let ctx = features::FeatureContext {
                system: &snap,
                clock: std::time::SystemTime::now(),
                bond_level: bridge.bond_level_cached(),
            };
            let tick_urges = registry.borrow_mut().tick(&ctx);
            if !tick_urges.is_empty() {
                let scored = scorer.borrow_mut().score(tick_urges);
                if !scored.is_empty() {
                    cards::push_whisper_cards(&card_mgr, &ui_weak, &scored);
                }
            }
            return;
        }

        // 1b. Handle keybind events (UI actions, not features)
        for event in &events {
            if let yantrik_os::SystemEvent::KeybindTriggered { action } = event {
                if let Some(ui) = ui_weak.upgrade() {
                    handle_keybind(&ui, action);
                }
            }
        }

        // 1c. Capture notifications into store
        for event in &events {
            if let yantrik_os::SystemEvent::NotificationReceived {
                app,
                summary,
                body,
                urgency,
            } = event
            {
                notification_store
                    .borrow_mut()
                    .push(app.clone(), summary.clone(), body.clone(), *urgency);
                // Update badge unless in focus mode (notifications still stored, badge deferred)
                if let Some(ui) = ui_weak.upgrade() {
                    if !ui.get_focus_mode() {
                        ui.set_notification_unread_count(
                            notification_store.borrow().unread_count() as i32,
                        );
                    }
                }
                // Push toast banner (only on desktop screen, not in focus mode, respects DND)
                if let Some(ui_ref) = ui_weak.upgrade() {
                    if !ui_ref.get_focus_mode() && ui_ref.get_current_screen() == 1 {
                        // DND check: skip toast if DND is on, UNLESS critical (urgency == 2)
                        let dnd = ui_ref.get_dnd_mode();
                        if !dnd || *urgency == 2 {
                            super::toast::push_toast(&ui_weak, app, summary, body, *urgency);
                        }
                    }
                }
            }
        }

        // 2. Process each event through features
        let mut all_urges = Vec::new();
        for event in &events {
            snapshot.borrow_mut().apply(event);
            let snap = snapshot.borrow();
            let ctx = features::FeatureContext {
                system: &snap,
                clock: std::time::SystemTime::now(),
                bond_level: bridge.bond_level_cached(),
            };
            let event_urges = registry.borrow_mut().process_event(event, &ctx);
            all_urges.extend(event_urges);
        }

        // Tick features too
        {
            let snap = snapshot.borrow();
            let ctx = features::FeatureContext {
                system: &snap,
                clock: std::time::SystemTime::now(),
                bond_level: bridge.bond_level_cached(),
            };
            all_urges.extend(registry.borrow_mut().tick(&ctx));
        }

        // 2b. Feed events into activity accumulator + detect issues
        {
            let mut acc = accumulator.borrow_mut();
            let snap = snapshot.borrow();
            for event in &events {
                acc.ingest(event);
                if let Some(issue) = acc.detect_issue(event, &snap) {
                    bridge.record_issue(issue.text, issue.importance, issue.decay);
                }
            }
        }

        // 3. Forward significant events to companion memory (with dedup)
        {
            let now = Instant::now();
            let mut cache = event_dedup.borrow_mut();

            // Periodic cleanup: remove expired entries every ~30 seconds
            if cache.len() > 100 {
                cache.retain(|_, ts| now.duration_since(*ts) < DEDUP_WINDOW);
            }

            for event in &events {
                if let Some((text, domain, importance)) = system_context::event_to_memory(event) {
                    // Skip if same event text was recorded within the dedup window
                    if let Some(last) = cache.get(&text) {
                        if now.duration_since(*last) < DEDUP_WINDOW {
                            continue;
                        }
                    }
                    cache.insert(text.clone(), now);
                    bridge.record_system_event(text, domain, importance);
                }
            }
        }

        // 4. Update status bar from snapshot
        let snap = snapshot.borrow();
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_battery_level(snap.battery_level as i32);
            ui.set_battery_charging(snap.battery_charging);
            ui.set_wifi_connected(snap.network_connected);

            // Ambient Intelligence: push sentiment, cognitive load, time-of-day
            let (sentiment, cognitive_load) = bridge.ambient_state();
            let time_of_day = crate::ambient::AmbientState::time_of_day();
            ui.set_particle_sentiment(sentiment);
            ui.set_particle_cognitive_load(cognitive_load);
            ui.set_particle_time_of_day(time_of_day);

            // Auto-lock on idle (only from desktop screen, 0 = disabled)
            let lock_timeout = ui.get_settings_auto_lock_secs() as u64;
            if lock_timeout > 0
                && snap.user_idle
                && snap.idle_seconds >= lock_timeout
                && ui.get_current_screen() == 1
            {
                ui.set_current_screen(3);
                ui.set_lock_error("".into());
                ui.set_lock_date_text(app_context::current_date_text().into());
                ui.set_lock_greeting(ui.get_greeting_text());
                tracing::info!(idle_secs = snap.idle_seconds, "Auto-locked due to idle");
            }
        }

        // 4b. Live-update System Dashboard (screen 10) from snapshot
        if let Some(ui) = ui_weak.upgrade() {
            if ui.get_current_screen() == 10 {
                ui.set_sys_cpu_usage(snap.cpu_usage_percent);
                ui.set_sys_memory_usage(snap.memory_usage_percent());

                let total = snap.memory_total_bytes;
                let used = snap.memory_used_bytes;
                let cached = snap.memory_cached_bytes;
                let free = snap.memory_free_bytes;

                // Overall memory text (used / total)
                let mem_text = format!(
                    "{} / {}",
                    format_bytes(used),
                    format_bytes(total)
                );
                ui.set_sys_memory_text(mem_text.into());

                // Memory breakdown percentages
                if total > 0 {
                    let used_pct = (used as f64 / total as f64 * 100.0) as f32;
                    let cached_pct = (cached as f64 / total as f64 * 100.0) as f32;
                    ui.set_sys_memory_used_percent(used_pct);
                    ui.set_sys_memory_cached_percent(cached_pct);
                }
                ui.set_sys_memory_used_text(format_bytes(used).into());
                ui.set_sys_memory_cached_text(format_bytes(cached).into());
                // "Free" label shows the actual free (not cached) memory
                ui.set_sys_memory_total_text(format_bytes(free).into());

                // Swap
                let swap_total = snap.swap_total_bytes;
                let swap_used = snap.swap_used_bytes;
                if swap_total > 0 {
                    let swap_pct = (swap_used as f64 / swap_total as f64 * 100.0) as f32;
                    ui.set_sys_swap_usage(swap_pct);
                    ui.set_sys_swap_text(
                        format!("{} / {}", format_bytes(swap_used), format_bytes(swap_total)).into(),
                    );
                } else {
                    ui.set_sys_swap_usage(0.0);
                    ui.set_sys_swap_text("N/A".into());
                }

                ui.set_sys_wifi_ssid(
                    snap.network_ssid.clone().unwrap_or_default().into(),
                );
                ui.set_sys_wifi_signal(snap.network_signal.unwrap_or(0) as i32);

                let procs: Vec<ProcessData> = snap
                    .running_processes
                    .iter()
                    .take(15)
                    .map(|p| ProcessData {
                        name: p.name.clone().into(),
                        pid: p.pid as i32,
                        cpu_percent: p.cpu_percent,
                    })
                    .collect();
                ui.set_sys_top_processes(ModelRc::new(VecModel::from(procs)));
            }
        }

        // 4c. Update dock running indicators + window list
        if let Some(ui) = ui_weak.upgrade() {
            if ui.get_current_screen() == 1 {
                let wins = windows::list_windows();

                // Update dock items with running state (labels from Tr global for i18n)
                let tr = ui.global::<Tr>();
                let dock_defs: &[(&str, fn(&Tr) -> slint::SharedString, &str)] = &[
                    ("terminal",      |t| t.get_dock_terminal(), ">_"),
                    ("browser",       |t| t.get_dock_browser(),  "W"),
                    ("files",         |t| t.get_dock_files(),    "F"),
                    ("email",         |t| t.get_dock_email(),    "@"),
                    ("notes",         |t| t.get_dock_notes(),    "\u{270E}"),
                    ("editor",        |t| t.get_dock_editor(),   "\u{2261}"),
                    ("memory",        |t| t.get_dock_memory(),   "\u{25C8}"),
                    ("notifications", |t| t.get_dock_alerts(),   "N"),
                    ("system",        |t| t.get_dock_system(),   "\u{25C9}"),
                    ("media",         |t| t.get_dock_media(),    "\u{266A}"),
                    ("calendar",      |t| t.get_dock_calendar(), "\u{25A6}"),
                    ("launchpad",     |t| t.get_dock_apps(),     "\u{229E}"),
                    ("settings",      |t| t.get_dock_settings(), "\u{2699}"),
                ];
                let dock: Vec<DockItem> = dock_defs
                    .iter()
                    .map(|(id, label_fn, icon)| DockItem {
                        app_id: (*id).into(),
                        label: label_fn(&tr),
                        icon_char: (*icon).into(),
                        is_running: wins.iter().any(|w| w.app_id == *id),
                    })
                    .collect();
                ui.set_dock_items(ModelRc::new(VecModel::from(dock)));

                // Update window list for switcher (with contextual subtitles)
                let win_items: Vec<WindowItem> = wins
                    .iter()
                    .map(|w| WindowItem {
                        title: w.title.clone().into(),
                        app_id: w.app_id.clone().into(),
                        icon_char: w.icon_char.clone().into(),
                        subtitle: w.subtitle.clone().into(),
                    })
                    .collect();
                ui.set_window_list(ModelRc::new(VecModel::from(win_items)));
            }
        }

        // 4d. Update system context for LLM prompt injection — only when state changed
        if accumulator.borrow_mut().context_changed(&snap) {
            bridge.set_system_context(system_context::format_system_context(&snap));
        }

        // 5. Score and display urges
        if !all_urges.is_empty() {
            let scored = scorer.borrow_mut().score(all_urges);
            if !scored.is_empty() {
                tracing::info!(
                    count = scored.len(),
                    top_pressure = scored[0].pressure,
                    top_title = %scored[0].urge.title,
                    "Whisper cards generated"
                );
                cards::push_whisper_cards(&card_mgr, &ui_weak, &scored);
            }
        }
    });

    // Keep timer alive for the duration of the app
    std::mem::forget(timer);

    // ── Chart history timer (1-second) ──
    wire_chart_history(ui, ctx);
}

/// Wire a 1-second timer that maintains 60-point ring buffers for CPU and
/// memory usage, pushing them to the UI as `[float]` models for the chart.
fn wire_chart_history(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let snapshot = ctx.system_snapshot.clone();

    let cpu_buf: RefCell<VecDeque<f32>> = RefCell::new(VecDeque::with_capacity(CHART_HISTORY_LEN));
    let mem_buf: RefCell<VecDeque<f32>> = RefCell::new(VecDeque::with_capacity(CHART_HISTORY_LEN));

    let chart_timer = Timer::default();
    chart_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let snap = snapshot.borrow();
        let cpu_normalized = (snap.cpu_usage_percent / 100.0).clamp(0.0, 1.0);
        let mem_normalized = (snap.memory_usage_percent() / 100.0).clamp(0.0, 1.0);

        {
            let mut cpu = cpu_buf.borrow_mut();
            if cpu.len() >= CHART_HISTORY_LEN {
                cpu.pop_front();
            }
            cpu.push_back(cpu_normalized);
        }
        {
            let mut mem = mem_buf.borrow_mut();
            if mem.len() >= CHART_HISTORY_LEN {
                mem.pop_front();
            }
            mem.push_back(mem_normalized);
        }

        if let Some(ui) = ui_weak.upgrade() {
            if ui.get_current_screen() == 10 {
                let cpu_vec: Vec<f32> = cpu_buf.borrow().iter().copied().collect();
                let mem_vec: Vec<f32> = mem_buf.borrow().iter().copied().collect();
                ui.set_sys_cpu_history(ModelRc::new(VecModel::from(cpu_vec)));
                ui.set_sys_memory_history(ModelRc::new(VecModel::from(mem_vec)));
            }
        }
    });

    std::mem::forget(chart_timer);
}

/// Handle a keybind action.
fn handle_keybind(ui: &App, action: &str) {
    match action {
        "open-lens" => {
            if ui.get_current_screen() == 1 {
                ui.set_lens_open(true);
            }
        }
        "lock-screen" => {
            ui.set_current_screen(3);
            ui.set_lock_error("".into());
            ui.set_lock_date_text(app_context::current_date_text().into());
            ui.set_lock_greeting(ui.get_greeting_text());
            tracing::info!("Screen locked via hotkey");
        }
        "open-terminal" => {
            let _ = std::process::Command::new("foot").spawn();
        }
        "open-files" => {
            ui.set_current_screen(8);
            ui.invoke_navigate(8);
        }
        "open-settings" => {
            ui.set_current_screen(7);
            ui.invoke_navigate(7);
        }
        "screenshot" => {
            super::screenshot::take_screenshot(
                ui.as_weak(),
                yantrik_os::screenshot::CaptureMode::FullScreen,
            );
        }
        "screenshot-region" => {
            super::screenshot::take_screenshot(
                ui.as_weak(),
                yantrik_os::screenshot::CaptureMode::Region,
            );
        }
        "screenshot-clipboard" => {
            super::screenshot::take_screenshot(
                ui.as_weak(),
                yantrik_os::screenshot::CaptureMode::ClipboardFull,
            );
        }
        "screenshot-clipboard-region" => {
            super::screenshot::take_screenshot(
                ui.as_weak(),
                yantrik_os::screenshot::CaptureMode::ClipboardRegion,
            );
        }
        "clipboard-history" => {
            if ui.get_current_screen() == 1 {
                ui.set_clip_panel_open(!ui.get_clip_panel_open());
            }
        }
        "toggle-dnd" => {
            let will_enable = !ui.get_dnd_mode();
            // Invoke the callback so settings persistence fires too
            ui.invoke_toggle_dnd_mode();
            let msg = if will_enable { "Do Not Disturb: ON" } else { "Do Not Disturb: OFF" };
            super::toast::push_toast(&ui.as_weak(), "System", msg, "", 0);
            tracing::info!(dnd = will_enable, "Do Not Disturb toggled via hotkey");
        }
        "power-menu" => {
            if ui.get_current_screen() == 1 {
                ui.set_power_menu_open(!ui.get_power_menu_open());
            }
        }
        "app-grid" => {
            if ui.get_current_screen() == 1 {
                ui.set_app_grid_open(!ui.get_app_grid_open());
            }
        }
        "window-switcher" => {
            if ui.get_current_screen() == 1 {
                // Refresh window list immediately before showing
                let wins = windows::list_windows();
                let items: Vec<WindowItem> = wins
                    .iter()
                    .map(|w| WindowItem {
                        title: w.title.clone().into(),
                        app_id: w.app_id.clone().into(),
                        icon_char: w.icon_char.clone().into(),
                        subtitle: w.subtitle.clone().into(),
                    })
                    .collect();
                ui.set_window_list(ModelRc::new(VecModel::from(items)));
                ui.set_window_switcher_open(!ui.get_window_switcher_open());
            }
        }
        other => {
            tracing::debug!(action = other, "Unknown keybind action");
        }
    }
}

/// Format a byte count as a human-readable string (KB / MB / GB).
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
