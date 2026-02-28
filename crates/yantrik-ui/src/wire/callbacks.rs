//! Miscellaneous callbacks — lock, onboarding, focus, file browser,
//! whisper cards, memory search.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{
    bridge, cards, filebrowser, focus, lock, notifications, onboarding, App, FileEntry, MemoryItem,
};

/// Wire all miscellaneous callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_lock(ui);
    wire_onboarding(ui);
    wire_focus(ui);
    wire_file_browser(ui, ctx);
    wire_whisper_cards(ui, ctx);
    wire_memory_search(ui, ctx);
    wire_notifications(ui, ctx);
    wire_quick_settings(ui);
}

// ── Lock screen ──

fn wire_lock(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_try_unlock(move |pin| {
        let pin = pin.to_string();
        if lock::check_pin(&pin) {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_current_screen(1);
                ui.set_lock_error("".into());
                tracing::info!("Screen unlocked");
            }
        } else {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_lock_error("Wrong PIN".into());
            }
            tracing::debug!("Unlock failed — wrong PIN");
        }
    });

    let ui_weak_lock = ui.as_weak();
    ui.on_lock_screen(move || {
        if let Some(ui) = ui_weak_lock.upgrade() {
            ui.set_current_screen(3);
            ui.set_lock_error("".into());
            tracing::info!("Screen locked");
        }
    });
}

// ── Onboarding ──

fn wire_onboarding(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_onboarding_ready(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_lens_open(true);
        }
        tracing::info!("Onboarding: user ready, opening Lens");
    });

    ui.on_onboarding_skip(move || {
        onboarding::write_marker();
        tracing::info!("Onboarding skipped");
    });
}

// ── Focus mode ──

fn wire_focus(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_end_focus_mode(move || {
        if let Some(ui) = ui_weak.upgrade() {
            focus::end(&ui);
        }
        tracing::info!("Focus mode ended by user");
    });
}

// ── File browser ──

fn wire_file_browser(ui: &App, ctx: &AppContext) {
    let browser_path = ctx.browser_path.clone();

    // Navigate into a subdirectory
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    ui.on_file_navigate_dir(move |name| {
        let name = name.to_string();
        let new_path = {
            let current = bp.borrow();
            filebrowser::child_path(&current, &name)
        };
        *bp.borrow_mut() = new_path.clone();
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_file_browser_path(SharedString::from(&new_path));
            set_file_entries(&ui, &new_path);
        }
    });

    // Open a file with xdg-open
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    ui.on_file_open(move |name| {
        let name = name.to_string();
        let full = {
            let current = bp.borrow();
            let expanded = filebrowser::expand_home(&current);
            expanded.join(&name)
        };
        tracing::info!(path = %full.display(), "Opening file");
        let _ = std::process::Command::new("xdg-open").arg(&full).spawn();
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_current_screen(1);
        }
    });

    // Go up one directory
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    ui.on_file_go_up(move || {
        let new_path = {
            let current = bp.borrow();
            filebrowser::parent_path(&current)
        };
        *bp.borrow_mut() = new_path.clone();
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_file_browser_path(SharedString::from(&new_path));
            set_file_entries(&ui, &new_path);
        }
    });
}

/// List a directory and push entries to the UI.
fn set_file_entries(ui: &App, path: &str) {
    let entries = filebrowser::list_dir(path);
    let items: Vec<FileEntry> = entries
        .into_iter()
        .map(|e| FileEntry {
            name: e.name.into(),
            is_dir: e.is_dir,
            size_text: e.size_text.into(),
            modified_text: e.modified_text.into(),
            icon_char: e.icon_char.into(),
        })
        .collect();
    ui.set_file_browser_entries(ModelRc::new(VecModel::from(items)));
}

// ── Whisper cards ──

fn wire_whisper_cards(ui: &App, ctx: &AppContext) {
    let card_mgr = ctx.card_manager.clone();
    let bridge = ctx.bridge.clone();

    // Dismiss a whisper card
    let mgr = card_mgr.clone();
    let br = bridge.clone();
    let ui_weak = ui.as_weak();
    ui.on_whisper_card_dismissed(move |id| {
        let id = id.to_string();
        let mut mgr = mgr.borrow_mut();
        if let Some(source) = mgr.dismiss(&id) {
            cards::sync_whisper_ui(&mgr, &ui_weak);
            br.record_system_event(
                format!("Whisper card dismissed: {}", id),
                "whisper-cards".to_string(),
                0.2,
            );
            tracing::debug!(id, source, "Whisper card dismissed");
        }
    });

    // Action on a whisper card (dismiss + open Lens)
    let mgr = card_mgr.clone();
    let br = bridge.clone();
    let ui_weak = ui.as_weak();
    ui.on_whisper_card_action(move |id| {
        let id = id.to_string();
        let mut mgr = mgr.borrow_mut();
        if let Some(source) = mgr.dismiss(&id) {
            cards::sync_whisper_ui(&mgr, &ui_weak);
            br.record_system_event(
                format!("Whisper card acted on: {}", id),
                "whisper-cards".to_string(),
                0.3,
            );
            tracing::debug!(id, source, "Whisper card action");
        }
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_lens_open(true);
        }
    });

    // Whisper hint badge clicked — open Lens
    let ui_weak = ui.as_weak();
    ui.on_whisper_hint_clicked(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_lens_open(true);
        }
    });
}

// ── Notifications ──

fn wire_notifications(ui: &App, ctx: &AppContext) {
    // Clear all notifications
    let store = ctx.notification_store.clone();
    let ui_weak = ui.as_weak();
    ui.on_notification_clear_all(move || {
        store.borrow_mut().clear();
        notifications::sync_to_ui(&store.borrow(), &ui_weak);
        tracing::debug!("Notifications cleared");
    });

    // Mark all as read
    let store = ctx.notification_store.clone();
    let ui_weak = ui.as_weak();
    ui.on_notification_mark_all_read(move || {
        store.borrow_mut().mark_all_read();
        notifications::sync_to_ui(&store.borrow(), &ui_weak);
        tracing::debug!("All notifications marked as read");
    });

    // Tap a notification (mark as read)
    let store = ctx.notification_store.clone();
    let ui_weak = ui.as_weak();
    ui.on_notification_tapped(move |id| {
        if let Ok(id_num) = id.to_string().parse::<u64>() {
            store.borrow_mut().mark_read(id_num);
            notifications::sync_to_ui(&store.borrow(), &ui_weak);
        }
    });
}

// ── Quick Settings ──

fn wire_quick_settings(ui: &App) {
    // Toggle WiFi via nmcli
    ui.on_toggle_wifi(move || {
        // Read current state and toggle
        let output = std::process::Command::new("nmcli")
            .args(["radio", "wifi"])
            .output();
        let currently_on = output
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
            .unwrap_or(false);
        let new_state = if currently_on { "off" } else { "on" };
        let _ = std::process::Command::new("nmcli")
            .args(["radio", "wifi", new_state])
            .spawn();
        tracing::info!(new_state, "WiFi toggled");
    });

    // Brightness via brightnessctl
    ui.on_brightness_changed(move |level| {
        let pct = format!("{}%", level);
        let _ = std::process::Command::new("brightnessctl")
            .args(["s", &pct])
            .spawn();
        tracing::debug!(level, "Brightness changed");
    });

    // Volume via amixer
    ui.on_volume_changed(move |level| {
        let pct = format!("{}%", level);
        let _ = std::process::Command::new("amixer")
            .args(["-M", "set", "Master", &pct])
            .spawn();
        tracing::debug!(level, "Volume changed");
    });
}

// ── Memory search ──

fn wire_memory_search(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let search_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let timer_inner = search_timer.clone();

    ui.on_search_memories(move |query| {
        let query = query.to_string();
        if query.is_empty() {
            return;
        }

        if let Some(ui) = ui_weak.upgrade() {
            ui.set_is_searching_memories(true);
        }

        let reply_rx = bridge.recall_memories(query);
        let weak = ui_weak.clone();
        let handle = timer_inner.clone();
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            if let Ok(results) = reply_rx.try_recv() {
                if let Some(ui) = weak.upgrade() {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs_f64();

                    let items: Vec<MemoryItem> = results
                        .iter()
                        .map(|r| MemoryItem {
                            rid: r.rid.clone().into(),
                            text: r.text.clone().into(),
                            memory_type: r.memory_type.clone().into(),
                            importance: r.importance as f32,
                            valence: r.valence as f32,
                            score: r.score as f32,
                            time_ago: bridge::format_time_ago(now - r.created_at).into(),
                        })
                        .collect();
                    ui.set_memory_results(ModelRc::new(VecModel::from(items)));
                    ui.set_is_searching_memories(false);
                }
                *handle.borrow_mut() = None;
            }
        });
        *timer_inner.borrow_mut() = Some(timer);
    });
}
