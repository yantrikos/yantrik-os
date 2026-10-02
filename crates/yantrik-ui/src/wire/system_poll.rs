//! System poll wiring — 3-second timer that drains system events,
//! runs proactive features, updates status bar,
//! and injects system context into the LLM prompt.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Timer, TimerMode};

use slint::{ModelRc, VecModel};

use crate::app_context::{self, AppContext};
use crate::{cards, features, lock, system_context, windows, App, ProcessData, WindowItem};

/// What a row shows when the reading behind it has not been taken yet. A
/// number nobody has measured is worse than a blank, and a blank is what #50
/// was about, so it is an em dash.
const EM_DASH: &str = "\u{2014}";

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

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(3), move || {
        // 0. Sync interruptibility with focus mode state
        if let Some(ui) = ui_weak.upgrade() {
            let target = if ui.get_focus_mode() { 0.1 } else { 1.0 };
            scorer.borrow_mut().set_interruptibility(target);
        }

        // 0b. The brightness keys move the panel behind the shell's back; one file read. (The
        // network reading is not here: `wire::network` publishes it on NetworkManager's signals.)
        if let Some(ui) = ui_weak.upgrade() {
            super::backlight::refresh(&ui);
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

        // 1-. The volume follows what the audio server said, whoever changed it.
        if let Some(ui) = ui_weak.upgrade() {
            super::audio::apply_events(&ui, &events);
        }

        // 1a. Bridge system events into the cognitive event bus
        for event in &events {
            event_bus.emit_system_event(event.clone());
        }

        // 1b. Notifications are not captured here any more.
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

        // 2b. Feed events into activity accumulator + detect issues. Not in Private mode: what
        // the person runs and where they connect is not recorded while it is on.
        if !crate::private_mode::is_on() {
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
            // The battery and the profile come from the snapshot the events built: UPower's
            // signals (or the sysfs fallback) and power-profiles-daemon's, not a poll here.
            crate::power_status::apply_battery(&ui, &snap);
            crate::power_status::apply_profile(&ui, snap.power_profile.as_ref());
            // The network properties are set by `wire::network`, from NetworkManager's own
            // signals — the address among them, when it reports one. This is
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

            // Auto-lock when the person has left the seat (0 = never). Idle is the compositor's
            // count of keyboard and mouse (#412), so an agent working does not hold it off.
            // From any screen but boot, first run and the lock itself: it was the desktop only,
            // so a machine left on Settings or Memory stayed open.
            ui.set_settings_auto_lock_available(yantrik_os::idle_watch_active());
            let lock_timeout = ui.get_settings_auto_lock_secs().max(0) as u64;
            if lock_timeout > 0
                && snap.user_idle
                && snap.idle_seconds >= lock_timeout
                && may_auto_lock(ui.get_current_screen())
            {
                // The one lock path: the shell's screen and the compositor's session lock (#313).
                ui.invoke_lock_screen();
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
                let win_items = window_items(ui.get_current_screen(), &wins, windows::shell_in_front());
                if let Some(model) = crate::models::changed(ui.get_window_list(), win_items) {
        ui.set_window_list(model);
    }
                // The dock groups the same list by app. After it, so both read one snapshot.
                super::dock_bar::publish(&ui, &catalogue.get());
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

/// The taskbar's entries: the screen the shell is on, when it is one a person reads as a window
/// (`control::screen_entry` — Files, Settings, Agents …), among every window.
///
/// The taskbar lights its first entry as the active one, so the screen goes first only when the
/// shell is what the person is looking at (`shell_front`, from the compositor), or when that cannot
/// be known. With an app window in front of it — Weather over Files — the app stays first and the
/// screen comes second, behind it, where it is.
fn window_items(screen: i32, wins: &[windows::WindowEntry], shell_front: Option<bool>) -> Vec<WindowItem> {
    let mut items: Vec<WindowItem> = wins
        .iter()
        .map(|w| WindowItem {
            title: w.title.clone().into(),
            app_id: w.app_id.clone().into(),
            icon_char: w.icon_char.clone().into(),
            subtitle: w.subtitle.clone().into(),
        })
        .collect();
    if let Some((name, title)) = crate::control::screen_entry(screen) {
        let entry = WindowItem {
            title: title.into(),
            app_id: format!("{}{name}", crate::control::SCREEN_ENTRY_PREFIX).into(),
            icon_char: windows::icon_for_app(name).into(),
            subtitle: "".into(),
        };
        let at = if shell_front == Some(false) && !items.is_empty() { 1 } else { 0 };
        items.insert(at, entry);
    }
    items
}

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

/// Screens the auto-lock may lock from: all but boot (0), first run (2) — no PIN to unlock with
/// yet — and the two locked screens. Never from login (32): the PIN screen unlocks to the
/// desktop, so locking there would trade the login password for the PIN (as `lock` refuses to).
fn may_auto_lock(screen: i32) -> bool {
    !matches!(screen, 0 | 2) && !crate::control::locked_screen(screen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather() -> windows::WindowEntry {
        windows::WindowEntry {
            title: "Weather".into(),
            app_id: "weather".into(),
            wayland_app_id: String::new(),
            icon_char: windows::icon_for_app("weather").into(),
            subtitle: String::new(),
        }
    }

    /// Files is the shell's own screen, drawn as a window; it is on the taskbar while it is up,
    /// first, and the desktop adds nothing.
    #[test]
    fn the_screen_the_shell_is_on_is_a_taskbar_entry() {
        let titles = |items: &[WindowItem]| items.iter().map(|w| w.title.to_string()).collect::<Vec<_>>();
        // The shell in front: its screen is the active entry, first.
        let items = window_items(8, &[weather()], Some(true));
        assert_eq!(titles(&items), ["Files", "Weather"]);
        assert_eq!(items[0].app_id.as_str(), "shell:files");
        assert_eq!(items[1].app_id.as_str(), "weather");
        // Weather in front of Files: Weather stays first, and Files is behind it.
        assert_eq!(titles(&window_items(8, &[weather()], Some(false))), ["Weather", "Files"]);
        // Not knowable: first, as before.
        assert_eq!(titles(&window_items(8, &[weather()], None)), ["Files", "Weather"]);
        // Nothing else open: the screen is the only entry, whatever is said to be in front.
        assert_eq!(titles(&window_items(8, &[], Some(false))), ["Files"]);

        let on_desktop = window_items(1, &[weather()], Some(true));
        assert_eq!(on_desktop.len(), 1);
        assert_eq!(on_desktop[0].title.as_str(), "Weather");
    }

    #[test]
    fn the_auto_lock_locks_from_any_screen_the_person_could_leave_open() {
        for screen in [1, 6, 7, 8, 10, 17, 23] {
            assert!(may_auto_lock(screen), "screen {screen} left open would stay unlocked");
        }
        for screen in [0, 2, 3, 32] {
            assert!(!may_auto_lock(screen), "screen {screen}");
        }
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
