//! System poll wiring — 3-second timer that drains system events,
//! runs proactive features, handles keybinds, updates status bar,
//! and injects system context into the LLM prompt.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Timer, TimerMode};

use slint::{ModelRc, VecModel};

// The network service's own shape, rather than hand-read JSON keys: the
// contract is what keeps the two ends of this wire from drifting.
use yantrik_ipc_contracts::network::{method as network_method, NetworkStatus};

use crate::app_context::{self, AppContext};
use crate::{cards, features, lock, system_context, windows, App, ProcessData, WindowItem};

/// What a row shows when the reading behind it has not been taken yet. A
/// number nobody has measured is worse than a blank, and a blank is what #50
/// was about, so it is an em dash.
const EM_DASH: &str = "\u{2014}";

/// Maximum number of data points in the chart history ring buffer.
const CHART_HISTORY_LEN: usize = 60;

/// How often the network reading is re-asked of the network service. The
/// service answers from live interface state; 15s matches the observer's own
/// network poll cadence and keeps an RPC out of most 3s ticks.
const NETWORK_REFRESH: Duration = Duration::from_secs(15);

/// How long to leave a service that did not answer alone. The call blocks the
/// thread that draws the screen for up to the client's timeout, so a wedged
/// service is re-asked once a minute rather than four times; the observer's
/// flag carries the reading in the meantime.
const NETWORK_RETRY: Duration = Duration::from_secs(60);

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
    let event_bus = ctx.event_bus.clone();
    let catalogue = ctx.installed_apps.clone();

    // Dedup cache: prevents recording the same system event to memory more than
    // once per 5 minutes. Key = event text, Value = last recorded time.
    let event_dedup: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
    const DEDUP_WINDOW: Duration = Duration::from_secs(300); // 5 minutes

    // Network memory gate: the connection state behind the last network memory
    // recorded. A time window is not enough for the network — the connection is
    // re-announced every few minutes forever, so every window that expires lets
    // another identical row into the store (#31). Only a change of state is a
    // new fact.
    let last_network: RefCell<Option<system_context::NetworkState>> = RefCell::new(None);

    // Network cache: when the network service was last asked, and what it said.
    // `None` inside the option is the service failing to answer, which is a
    // different thing from not having asked yet.
    let net_cache: RefCell<Option<(Instant, Option<NetworkStatus>)>> = RefCell::new(None);

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(3), move || {
        // 0. Sync interruptibility with focus mode state
        if let Some(ui) = ui_weak.upgrade() {
            let target = if ui.get_focus_mode() { 0.1 } else { 1.0 };
            scorer.borrow_mut().set_interruptibility(target);
        }

        // 0b. Publish the network reading.
        //
        // Before the early return below, and not inside the status-bar block
        // further down, because the status bar is drawn above every screen
        // while that block runs only on ticks where the observer happened to
        // have something to say. The service is asked at its own cadence; the
        // observer's flag is the fallback for when it cannot be reached.
        if let Some(ui) = ui_weak.upgrade() {
            // Read the cache out before the refresh arm below can write to it:
            // a borrow guard held across that would make borrow_mut panic.
            let cached = net_cache.borrow().as_ref().and_then(|(asked, answer)| {
                let window = if answer.is_some() { NETWORK_REFRESH } else { NETWORK_RETRY };
                (asked.elapsed() < window).then(|| answer.clone())
            });
            let answer = match cached {
                Some(answer) => answer,
                None => {
                    let fresh = ask_network_service();
                    *net_cache.borrow_mut() = Some((Instant::now(), fresh.clone()));
                    fresh
                }
            };
            let readout = {
                let snap = snapshot.borrow();
                network_readout(
                    answer.as_ref(),
                    snap.network_connected,
                    snap.network_ssid.as_deref(),
                )
            };
            publish_network(&ui, &readout);
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

        // 1a. Bridge system events into the cognitive event bus
        for event in &events {
            event_bus.emit_system_event(event.clone());
        }

        // 1b. Handle keybind events (UI actions, not features)
        for event in &events {
            if let yantrik_os::SystemEvent::KeybindTriggered { action } = event {
                if let Some(ui) = ui_weak.upgrade() {
                    handle_keybind(&ui, action);
                }
            }
        }

        // 1c. Notifications are not captured here any more.
        //
        // This block used to write every `NotificationReceived` into the shell's own private
        // store and raise its own toast — a second store and a second toast path beside the
        // notifications service, which is why a `notify-send` was in the notification centre
        // and a screenshot was not, or the other way round depending on which daemon had won
        // the bus name that boot. `wire::notifications` polls the one store, raises the toast,
        // and puts the event back on this channel, so everything below still sees it.

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
                // A `NetworkChanged` event announces the current connection and fires on
                // every link flap and DHCP renewal, not only when something happens (#31).
                // Record a memory when the connection state changes and drop repeats of
                // the state behind the last network memory, or the store fills with
                // identical rows for one connection and the Memory screen counts them all.
                let is_network_repeat = system_context::gate_network_memory(
                    &mut last_network.borrow_mut(),
                    event,
                ) == Some(false);
                if is_network_repeat {
                    continue;
                }

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
            ui.set_battery_available(snap.battery_available);
            ui.set_battery_level(snap.battery_level as i32);
            ui.set_battery_charging(snap.battery_charging);
            // The network properties are set in step 0b, above every early
            // return, because the status bar carries them on every screen —
            // the address among them, when the service reports one. This is
            // the fallback for when it does not: whatever `ip` lists first.
            if snap.network_connected {
                if ui.get_settings_ip_address().is_empty() {
                    let ip = std::process::Command::new("ip")
                        .args(["-4", "addr", "show", "scope", "global"])
                        .output()
                        .ok()
                        .and_then(|o| if o.status.success() {
                            String::from_utf8(o.stdout).ok()
                        } else { None })
                        .and_then(|out| {
                            out.lines()
                                .find(|l| l.contains("inet "))
                                .and_then(|l| l.split_whitespace()
                                    .nth(1)
                                    .and_then(|cidr| cidr.split('/').next())
                                    .map(|s| s.to_string()))
                        })
                        .unwrap_or_default();
                    ui.set_settings_ip_address(slint::SharedString::from(ip.as_str()));
                }
            }

            // Ambient Intelligence: push sentiment, cognitive load, time-of-day
            let (sentiment, cognitive_load) = bridge.ambient_state();
            let time_of_day = crate::ambient::AmbientState::time_of_day();
            ui.set_particle_sentiment(sentiment);
            ui.set_particle_cognitive_load(cognitive_load);
            ui.set_particle_time_of_day(time_of_day);

            // Panel load readout. The dashboard (screen 10) sets its own detailed figures
            // only while visible; the status bar is on every screen, so this is unconditional.
            ui.set_bar_cpu_percent(snap.cpu_usage_percent.round() as i32);
            ui.set_bar_mem_text(
                format!(
                    "{} / {}",
                    format_bytes(snap.memory_used_bytes),
                    format_bytes(snap.memory_total_bytes)
                )
                .into(),
            );

            ui.set_bar_mem_percent(if snap.memory_total_bytes > 0 {
                (snap.memory_used_bytes * 100 / snap.memory_total_bytes) as i32
            } else { 0 });
            if snap.swap_total_bytes > 0 {
                ui.set_bar_swap_percent((snap.swap_used_bytes * 100 / snap.swap_total_bytes) as i32);
                ui.set_bar_swap_text(format!("{} / {}", format_bytes(snap.swap_used_bytes), format_bytes(snap.swap_total_bytes)).into());
            } else {
                ui.set_bar_swap_percent(0);
                ui.set_bar_swap_text("none".into());
            }
            if snap.disk_total_bytes > 0 {
                let used = snap.disk_total_bytes.saturating_sub(snap.disk_available_bytes);
                ui.set_bar_disk_percent((used * 100 / snap.disk_total_bytes) as i32);
                ui.set_bar_disk_text(format!("{} / {}", format_bytes(used), format_bytes(snap.disk_total_bytes)).into());
            }

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
                update_memory_readouts(&ui, &snap);
                // Uptime moves while the screen is open. Read on entry only,
                // it was as stale as the About screen's was before #50 — a
                // minute out after a minute of looking at it.
                ui.set_sys_uptime_text(super::about::read_uptime().into());

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
        //
        // On EVERY screen, not just the desktop.
        //
        // This used to be guarded by `current_screen == 1`, from when the taskbar lived inside
        // DesktopScreen and there was nothing to update anywhere else. The taskbar was moved up
        // to the shell so it is the same bar above every screen — and its data source stayed
        // behind. So the list froze the moment you left the desktop: a window closed while you
        // were in Files stayed in the taskbar indefinitely. Photographed with the taskbar
        // offering "New Tab - Chromium" while `describe` said 0 windows open and the compositor
        // listed none.
        if let Some(ui) = ui_weak.upgrade() {
            {
                let wins = windows::list_windows_throttled();

                // The pinned apps, with their running marks.
                //
                // This was a hardcoded list of sixteen built here every three seconds, and it was
                // the whole of START — including `launchpad`, a tile for the Apps button sitting
                // in the corner of the same screen. Which apps appear is now the person's pinned
                // list; this only refreshes whether each is running.
                super::pins::publish(&ui, &catalogue.get());

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
                if let Some(model) = crate::models::changed(ui.get_window_list(), win_items) {
        ui.set_window_list(model);
    }
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
    // A keybind is not a keystroke: it arrives over the session D-Bus, which any process
    // running as this user can send to, so this is a door onto the desktop like the socket and
    // is held to the same rule (#203). While the login or lock screen is up, nothing here
    // launches, navigates, screenshots or toggles — the arms below that check `screen == 1`
    // only ever guarded the lens and the overlays, and the rest ran on the lock screen.
    // Unlocking goes through the lock screen's own callbacks, never through a keybind.
    if crate::control::locked_screen(ui.get_current_screen()) {
        tracing::debug!(action, "Keybind dropped — the desktop is waiting for the person to sign in");
        return;
    }
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
            // The one toast that is deliberately not stored: it acknowledges a key the person
            // just pressed, and it has to appear while notifications are being silenced. See
            // `toast::local`.
            super::toast::local(&ui.as_weak(), "System", msg, "", 0);
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

/// Push the snapshot's memory and swap figures into the System Dashboard's
/// readouts. Shared by the live update above and the on-entry population in
/// navigate.rs: the dashboard's "Used / Cached / Free" rows once rendered as
/// bare labels because the entry path set only the headline figure, and one
/// function feeding both paths is what keeps them from drifting apart again.
pub(crate) fn update_memory_readouts(ui: &App, snap: &yantrik_os::SystemSnapshot) {
    let r = memory_readouts(snap);
    ui.set_sys_memory_usage(r.usage_percent);
    ui.set_sys_memory_text(r.headline.as_str().into());
    ui.set_sys_memory_used_percent(r.used_percent);
    ui.set_sys_memory_cached_percent(r.cached_percent);
    ui.set_sys_memory_used_text(r.used.as_str().into());
    ui.set_sys_memory_cached_text(r.cached.as_str().into());
    ui.set_sys_memory_free_text(r.free.as_str().into());
    ui.set_sys_swap_usage(r.swap_percent);
    ui.set_sys_swap_text(r.swap.as_str().into());
}

/// The numbers the System Dashboard's memory rows show.
struct MemoryReadouts {
    usage_percent: f32,
    headline: String,
    used: String,
    cached: String,
    free: String,
    used_percent: f32,
    cached_percent: f32,
    swap: String,
    swap_percent: f32,
}

/// The snapshot's memory figures, formatted.
///
/// Every row goes to an em dash when the total is zero, which is the snapshot
/// before the first `MemoryPressure` event has arrived rather than a machine
/// with no memory in it. Printing the zeros instead would replace the bare
/// "Used:" labels of #50 with "Used: 0 B" — a measurement nobody has taken,
/// rendered as a fact.
fn memory_readouts(snap: &yantrik_os::SystemSnapshot) -> MemoryReadouts {
    let total = snap.memory_total_bytes;
    if total == 0 {
        return MemoryReadouts {
            usage_percent: 0.0,
            headline: EM_DASH.to_string(),
            used: EM_DASH.to_string(),
            cached: EM_DASH.to_string(),
            free: EM_DASH.to_string(),
            used_percent: 0.0,
            cached_percent: 0.0,
            swap: EM_DASH.to_string(),
            swap_percent: 0.0,
        };
    }

    let used = snap.memory_used_bytes;
    let cached = snap.memory_cached_bytes;
    let free = snap.memory_free_bytes;
    let swap_total = snap.swap_total_bytes;
    let swap_used = snap.swap_used_bytes;

    MemoryReadouts {
        usage_percent: snap.memory_usage_percent(),
        headline: format!("{} / {}", format_bytes(used), format_bytes(total)),
        used: format_bytes(used),
        cached: format_bytes(cached),
        // The row is labelled "Free", so it is free memory and not the total.
        free: format_bytes(free),
        used_percent: (used as f64 / total as f64 * 100.0) as f32,
        cached_percent: (cached as f64 / total as f64 * 100.0) as f32,
        swap: if swap_total > 0 {
            format!("{} / {}", format_bytes(swap_used), format_bytes(swap_total))
        } else {
            // This machine has no swap, which is a fact and not a failure.
            "none".to_string()
        },
        swap_percent: if swap_total > 0 {
            (swap_used as f64 / swap_total as f64 * 100.0) as f32
        } else {
            0.0
        },
    }
}

/// What the shell knows about being online, and over what.
///
/// One reading behind the status bar's indicator, the System screen's network
/// row, Settings > Network and `describe`. Each used to derive its own, which
/// is how the shell came to draw a Wi-Fi mark over NetworkManager's "Wired
/// connection 1" on a machine with no wireless device in it.
#[derive(Debug, Clone, PartialEq)]
struct NetworkReadout {
    /// Something is up and carrying traffic. True on a wired machine — this is
    /// what the status bar's indicator means, and what a mind reads to decide
    /// whether it can fetch anything.
    online: bool,
    /// The medium in the service's own word: `wifi`, `ethernet`, `vpn`,
    /// `bridge`, `other` — or empty, which is the service having named none.
    /// Only `wifi` is wireless.
    medium: String,
    /// The label the network row stands under.
    label: String,
    /// What stands beside it: the SSID, the connection's name, the address.
    detail: String,
    /// The SSID, and only ever an SSID — empty on anything not wireless.
    ssid: String,
    /// The address the service reports for the connection that is up.
    ip: String,
}

/// Derive the reading from the network service's answer, falling back to the
/// observer when the service cannot be reached.
///
/// `observer_connected` and `observer_connection` are the observer's
/// `NetworkChanged`: whether *some* interface is up, and the name
/// NetworkManager gives the primary connection. That name is the connection's,
/// not an SSID — on the wired test machine it is "Wired connection 1" — so it
/// is used as a name and never as evidence of wireless.
fn network_readout(
    service: Option<&NetworkStatus>,
    observer_connected: bool,
    observer_connection: Option<&str>,
) -> NetworkReadout {
    let Some(status) = service else {
        // The service is the only component that knows the medium, so with it
        // unreachable nothing here names one. The observer still knows whether
        // an interface is up, which is all "online" claims.
        return NetworkReadout {
            online: observer_connected,
            medium: String::new(),
            label: "Network".to_string(),
            detail: if observer_connected {
                observer_connection.unwrap_or("Connected").to_string()
            } else {
                "Offline".to_string()
            },
            ssid: String::new(),
            ip: String::new(),
        };
    };

    let medium = status.conn_type.trim().to_ascii_lowercase();
    let label = match medium.as_str() {
        "wifi" => "Wi-Fi",
        "ethernet" => "Ethernet",
        "vpn" => "VPN",
        "bridge" => "Bridge",
        _ => "Network",
    };
    let ssid = if medium == "wifi" {
        status.ssid.clone().unwrap_or_default()
    } else {
        String::new()
    };
    let ip = status.ip_address.clone().unwrap_or_default();
    let detail = if !status.connected {
        "Offline".to_string()
    } else {
        [ssid.as_str(), observer_connection.unwrap_or(""), ip.as_str()]
            .into_iter()
            .find(|candidate| !candidate.is_empty())
            .unwrap_or("Connected")
            .to_string()
    };

    NetworkReadout {
        online: status.connected,
        medium,
        label: label.to_string(),
        detail,
        ssid,
        ip,
    }
}

/// Ask the network service what is up. `None` is "could not be reached",
/// which is not the same answer as "nothing is connected".
fn ask_network_service() -> Option<NetworkStatus> {
    yantrik_ipc_transport::SyncRpcClient::for_service("network")
        .call_typed(network_method::STATUS, &serde_json::json!({}))
        .ok()
}

/// Push the reading into every property that states it.
///
/// Every write is guarded by a comparison. This runs on every 3s tick, a
/// Slint property set marks its dependents dirty whether or not the value
/// moved, and the status bar's mark depends on these — so writing
/// unconditionally would repaint an idle desktop every three seconds.
fn publish_network(ui: &App, r: &NetworkReadout) {
    if ui.get_network_online() != r.online {
        ui.set_network_online(r.online);
    }
    if ui.get_network_medium().as_str() != r.medium {
        ui.set_network_medium(r.medium.as_str().into());
    }
    if ui.get_network_label().as_str() != r.label {
        ui.set_network_label(r.label.as_str().into());
    }
    if ui.get_network_detail().as_str() != r.detail {
        ui.set_network_detail(r.detail.as_str().into());
    }
    // Wireless specifically: the Quick Settings tile, which is a radio.
    let wireless = r.online && r.medium == "wifi";
    if ui.get_wifi_connected() != wireless {
        ui.set_wifi_connected(wireless);
    }
    if ui.get_sys_wifi_ssid().as_str() != r.ssid {
        ui.set_sys_wifi_ssid(r.ssid.as_str().into());
    }
    // The address Settings > Network shows. The service reports it for the
    // connection that is actually up; the `ip` command the poll falls back on
    // takes the first global address it finds, which on a machine with a VPN
    // or a container bridge is not necessarily this one. One owner, so that
    // nothing goes on showing an address after the connection has gone.
    if !r.online {
        ui.set_settings_ip_address(Default::default());
    } else if !r.ip.is_empty() {
        ui.set_settings_ip_address(r.ip.as_str().into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The live machine's answer, captured from `yos describe network`:
    /// online via ethernet at 192.168.4.44, ssid null, no wireless adapter.
    fn wired_machine() -> NetworkStatus {
        NetworkStatus {
            connected: true,
            conn_type: "ethernet".to_string(),
            ssid: None,
            ip_address: Some("192.168.4.44".to_string()),
        }
    }

    #[test]
    fn a_wired_machine_is_online_and_is_not_wifi() {
        let r = network_readout(Some(&wired_machine()), true, Some("Wired connection 1"));
        // The whole of #50's second screen: "WiFi" stood over NetworkManager's
        // connection name on a machine with no wireless device.
        assert_eq!(r.label, "Ethernet");
        assert_eq!(r.detail, "Wired connection 1");
        assert_eq!(r.ssid, "");
        assert_eq!(r.medium, "ethernet");
        // And it is online, which is what the status bar's mark means. Deriving
        // that from the wifi flag drew a wired machine as offline.
        assert!(r.online);
        assert_eq!(r.ip, "192.168.4.44");
    }

    #[test]
    fn a_wireless_machine_shows_its_ssid() {
        let status = NetworkStatus {
            connected: true,
            conn_type: "wifi".to_string(),
            ssid: Some("Wombat".to_string()),
            ip_address: Some("10.0.0.8".to_string()),
        };
        let r = network_readout(Some(&status), true, Some("Wombat"));
        assert_eq!(r.label, "Wi-Fi");
        assert_eq!(r.detail, "Wombat");
        assert_eq!(r.ssid, "Wombat");
        assert!(r.online);
    }

    #[test]
    fn a_connection_with_no_name_falls_back_to_its_address() {
        let mut status = wired_machine();
        status.ssid = None;
        let r = network_readout(Some(&status), true, None);
        assert_eq!(r.detail, "192.168.4.44");
    }

    #[test]
    fn nothing_up_says_so() {
        // The service's own answer when no interface is carrying anything:
        // connected false, type "none".
        let status = NetworkStatus {
            connected: false,
            conn_type: "none".to_string(),
            ssid: None,
            ip_address: None,
        };
        let r = network_readout(Some(&status), false, None);
        assert!(!r.online);
        assert_eq!(r.label, "Network");
        assert_eq!(r.detail, "Offline");
        // A name left over from the last connection is not evidence of one.
        let r = network_readout(Some(&status), true, Some("Wired connection 1"));
        assert!(!r.online);
        assert_eq!(r.detail, "Offline");
    }

    #[test]
    fn an_unreachable_service_claims_no_medium() {
        // Whether an interface is up is the observer's to answer. What it is
        // carrying is not, so an unverifiable "Wi-Fi" is not printed.
        let r = network_readout(None, true, Some("Wired connection 1"));
        assert!(r.online);
        assert_eq!(r.medium, "");
        assert_eq!(r.label, "Network");
        assert_eq!(r.detail, "Wired connection 1");
        assert_eq!(r.ssid, "");

        let r = network_readout(None, false, None);
        assert!(!r.online);
        assert_eq!(r.detail, "Offline");
    }

    #[test]
    fn formats_byte_counts() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn memory_rows_with_no_reading_yet_are_dashes() {
        // The snapshot before the first MemoryPressure event. The rows read
        // "Used:" with nothing beside them until #50; they must not now read
        // "Used: 0 B" on a machine with 7.8 GB in it.
        let r = memory_readouts(&yantrik_os::SystemSnapshot::default());
        assert_eq!(r.used, EM_DASH);
        assert_eq!(r.cached, EM_DASH);
        assert_eq!(r.free, EM_DASH);
        assert_eq!(r.headline, EM_DASH);
        assert_eq!(r.used_percent, 0.0);
    }

    #[test]
    fn memory_rows_state_the_breakdown() {
        // The live machine: 7.8 GB total, 1.7 GB used.
        const GB: u64 = 1024 * 1024 * 1024;
        let snap = yantrik_os::SystemSnapshot {
            memory_total_bytes: 8 * GB,
            memory_used_bytes: 2 * GB,
            memory_cached_bytes: 1 * GB,
            memory_free_bytes: 5 * GB,
            swap_total_bytes: 0,
            ..Default::default()
        };
        let r = memory_readouts(&snap);
        assert_eq!(r.headline, "2.0 GB / 8.0 GB");
        assert_eq!(r.used, "2.0 GB");
        assert_eq!(r.cached, "1.0 GB");
        assert_eq!(r.free, "5.0 GB");
        assert_eq!(r.used_percent, 25.0);
        assert_eq!(r.cached_percent, 12.5);
        // No swap on this machine, which is not a reading that failed.
        assert_eq!(r.swap, "none");
        assert_eq!(r.swap_percent, 0.0);
    }
}
