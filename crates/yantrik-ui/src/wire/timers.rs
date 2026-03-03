//! Background timers — clock, think cycle, card tick, morning brief.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app_context::{self, AppContext};
use crate::{cards, streaming, App};

/// Wire all background timers.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_clock(ui, &ctx.user_name);
    wire_think(ctx);
    wire_card_tick(ui, ctx);
    wire_morning_brief(ui, ctx);
    wire_frecency_persist(ctx);
    wire_hourly_snapshot(ctx);
}

/// Clock timer — updates time and personalized greeting every 30 seconds.
fn wire_clock(ui: &App, user_name: &str) {
    let ui_weak = ui.as_weak();
    let name = user_name.to_string();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(30), move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_clock_text(app_context::current_time_hhmm().into());
            ui.set_greeting_text(
                format!("{}, {}", app_context::time_of_day_greeting(), name).into(),
            );
        }
    });
    // Keep timer alive
    std::mem::forget(timer);
}

/// Background cognition — think cycle every 60 seconds.
/// Passes current interruptibility from FocusFlow so the worker can gate
/// proactive messages during deep work.
fn wire_think(ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let scorer = ctx.scorer.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
        let interruptibility = scorer.borrow().interruptibility();
        bridge.think(interruptibility);
    });
    std::mem::forget(timer);
}

/// Morning brief — fires once 5s after boot, proactively greets the user.
/// The prompt is hidden; only the AI's response appears in chat.
/// Personalized with user name and bond-level tone.
fn wire_morning_brief(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let user_name = ctx.user_name.clone();
    let timer_slot: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let slot = timer_slot.clone();

    let timer = Timer::default();
    timer.start(TimerMode::SingleShot, Duration::from_secs(5), move || {
        // Only fire if companion is online
        if !bridge.is_online() {
            tracing::info!("Morning brief skipped — companion offline");
            return;
        }
        let bond = bridge.bond_level_cached();
        let tone = match bond {
            0..=2 => "Keep it polite and professional.",
            3..=4 => "Be friendly and casual.",
            _ => "Be warm and personal — we know each other well.",
        };
        let prompt = format!(
            "You just started up. Greet {} by name. \
             Give a short morning brief (3-5 sentences max). \
             First use recall_workspace to check for a previous session snapshot. \
             Mention: the time of day, any system status worth noting from your context \
             (battery, memory, disk, network), and if you found a workspace snapshot, \
             briefly mention what they were last working on and suggest continuing. \
             End with something warm. {} \
             Be concise — this is the first thing they see when they log in. \
             Do NOT use bullet points or headers. Just natural sentences.",
            user_name, tone
        );
        tracing::info!(bond, user = %user_name, "Generating personalized morning brief");
        streaming::start_proactive_stream(ui_weak.clone(), &bridge, &prompt, &slot);
    });
    std::mem::forget(timer);
}

/// Frecency store persistence — flush to disk every 30 seconds.
fn wire_frecency_persist(ctx: &AppContext) {
    let frecency = ctx.frecency.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(30), move || {
        frecency.borrow_mut().persist();
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

/// Hourly system snapshot — flushes the ActivityAccumulator and stores the digest.
fn wire_hourly_snapshot(ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let accumulator = ctx.accumulator.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(3600), move || {
        let digest = accumulator.borrow_mut().flush();
        if !digest.is_empty() {
            tracing::info!(len = digest.len(), "Storing hourly system snapshot");
            bridge.record_snapshot(digest);
        }
    });
    std::mem::forget(timer);
}
