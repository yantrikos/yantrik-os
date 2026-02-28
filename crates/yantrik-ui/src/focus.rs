//! Focus mode — countdown timer + UI state management.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, SharedString, Timer, TimerMode};

use super::App;

/// Format seconds as MM:SS string.
pub fn format_mmss(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Start focus mode: set UI state + start 1-second countdown timer.
pub fn start(ui: &App, duration_secs: u32) {
    ui.set_focus_mode(true);
    ui.set_focus_remaining(format_mmss(duration_secs).into());

    let remaining = Rc::new(RefCell::new(duration_secs));
    let ui_weak = ui.as_weak();
    let remaining_tick = remaining.clone();

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let mut r = remaining_tick.borrow_mut();
        if *r == 0 {
            return;
        }

        let still_active = ui_weak
            .upgrade()
            .map_or(false, |ui: super::App| ui.get_focus_mode());
        if !still_active {
            *r = 0;
            return;
        }

        *r -= 1;
        if let Some(ui) = ui_weak.upgrade() {
            if *r == 0 {
                end(&ui);
                ui.set_notification_text("Focus session complete. Nice work.".into());
                ui.set_show_notification(true);
                tracing::info!("Focus mode completed (timer expired)");
            } else {
                ui.set_focus_remaining(SharedString::from(format_mmss(*r)));
            }
        }
    });

    // Keep the timer alive — it self-terminates when remaining reaches 0.
    std::mem::forget(timer);

    tracing::info!(duration_secs, "Focus mode started");
}

/// End focus mode: reset UI state.
pub fn end(ui: &App) {
    ui.set_focus_mode(false);
    ui.set_focus_remaining("".into());
}
