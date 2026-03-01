//! Background timers — clock, think cycle, card tick, morning brief.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app_context::{self, AppContext};
use crate::{cards, streaming, App};

/// Wire all background timers.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_clock(ui);
    wire_think(ctx);
    wire_card_tick(ui, ctx);
    wire_morning_brief(ui, ctx);
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

/// Morning brief — fires once 5s after boot, proactively greets the user.
/// The prompt is hidden; only the AI's response appears in chat.
fn wire_morning_brief(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let timer_slot: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let slot = timer_slot.clone();

    let timer = Timer::default();
    timer.start(TimerMode::SingleShot, Duration::from_secs(5), move || {
        // Only fire if companion is online
        if !bridge.is_online() {
            tracing::info!("Morning brief skipped — companion offline");
            return;
        }
        let prompt = concat!(
            "You just started up. Give me a short morning brief (3-5 sentences max). ",
            "First use recall_workspace to check if I have a previous session snapshot. ",
            "Mention: the time of day, any system status worth noting from your context ",
            "(battery, memory, disk, network), and if you found a workspace snapshot, ",
            "briefly mention what I was last working on. End with something warm. ",
            "Be concise — this is the first thing I see when I log in. ",
            "Do NOT use bullet points or headers. Just natural sentences."
        );
        tracing::info!("Generating morning brief");
        streaming::start_proactive_stream(ui_weak.clone(), &bridge, prompt, &slot);
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
