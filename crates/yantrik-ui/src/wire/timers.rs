//! Background timers — clock, think cycle, card tick.

use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app_context::{self, AppContext};
use crate::{cards, App};

/// Wire all background timers.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_clock(ui);
    wire_think(ctx);
    wire_card_tick(ui, ctx);
}

/// Clock timer — updates time and greeting every 30 seconds.
fn wire_clock(ui: &App) {
    let ui_weak = ui.as_weak();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(30), move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_clock_text(app_context::current_time_hhmm().into());
            ui.set_greeting_text(app_context::time_of_day_greeting().into());
        }
    });
    // Keep timer alive
    std::mem::forget(timer);
}

/// Background cognition — think cycle every 60 seconds.
fn wire_think(ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
        bridge.think();
    });
    std::mem::forget(timer);
}

/// Whisper card tick — drives auto-dismiss animations at 100ms.
fn wire_card_tick(ui: &App, ctx: &AppContext) {
    let card_mgr = ctx.card_manager.clone();
    let ui_weak = ui.as_weak();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        let mut mgr = card_mgr.borrow_mut();
        if mgr.tick() {
            cards::sync_whisper_ui(&mgr, &ui_weak);
        }
    });
    std::mem::forget(timer);
}
