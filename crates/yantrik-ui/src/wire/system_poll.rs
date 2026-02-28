//! System poll wiring — 3-second timer that drains system events,
//! runs proactive features, handles keybinds, updates status bar,
//! and injects system context into the LLM prompt.

use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app_context::AppContext;
use crate::{cards, features, lock, system_context, App};

/// Wire the system poll timer.
pub fn wire(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let observer = ctx.observer.clone();
    let registry = ctx.feature_registry.clone();
    let scorer = ctx.scorer.clone();
    let snapshot = ctx.system_snapshot.clone();
    let bridge = ctx.bridge.clone();
    let card_mgr = ctx.card_manager.clone();

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

        // 2. Process each event through features
        let mut all_urges = Vec::new();
        for event in &events {
            snapshot.borrow_mut().apply(event);
            let snap = snapshot.borrow();
            let ctx = features::FeatureContext {
                system: &snap,
                clock: std::time::SystemTime::now(),
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
            };
            all_urges.extend(registry.borrow_mut().tick(&ctx));
        }

        // 3. Forward significant events to companion memory
        for event in &events {
            if let Some((text, domain, importance)) = system_context::event_to_memory(event) {
                bridge.record_system_event(text, domain, importance);
            }
        }

        // 4. Update status bar from snapshot
        let snap = snapshot.borrow();
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_battery_level(snap.battery_level as i32);
            ui.set_battery_charging(snap.battery_charging);
            ui.set_wifi_connected(snap.network_connected);

            // Auto-lock on idle (only from desktop screen)
            if snap.user_idle
                && snap.idle_seconds >= lock::DEFAULT_IDLE_LOCK_SECS
                && ui.get_current_screen() == 1
            {
                ui.set_current_screen(3);
                ui.set_lock_error("".into());
                tracing::info!(idle_secs = snap.idle_seconds, "Auto-locked due to idle");
            }
        }

        // 4b. Update system context for LLM prompt injection
        bridge.set_system_context(system_context::format_system_context(&snap));

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
            let _ = std::process::Command::new("grim")
                .arg(format!(
                    "{}/screenshot-{}.png",
                    std::env::var("HOME").unwrap_or_default(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                ))
                .spawn();
        }
        other => {
            tracing::debug!(action = other, "Unknown keybind action");
        }
    }
}
