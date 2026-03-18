//! Background timers — clock, think cycle, card tick, morning brief.

use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app_context::{self, AppContext};
use crate::{cards, App};

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
/// proactive messages during deep work. Also sends focus data (foreground
/// window, idle seconds) for the Context Cortex.
fn wire_think(ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let scorer = ctx.scorer.clone();
    let snapshot = ctx.system_snapshot.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
        let interruptibility = scorer.borrow().interruptibility();

        // Gather focus data from system snapshot + foreground window
        let snap = snapshot.borrow();
        let idle_secs = snap.idle_seconds;
        drop(snap);

        // Get foreground window title from window list (first entry = most recent)
        let windows = crate::windows::list_windows();
        let (win_title, proc_name) = if let Some(w) = windows.first() {
            (w.title.clone(), w.app_id.clone())
        } else {
            (String::new(), String::new())
        };

        bridge.think(interruptibility, win_title, proc_name, idle_secs);
    });
    std::mem::forget(timer);
}

/// Morning brief — fires once 5s after boot.
/// Sends a natural prompt to the companion, which uses its own tools and memory
/// to compose a personalized morning brief. The companion decides what to include
/// based on available tools, user preferences (remembered via memory), and context.
fn wire_morning_brief(_ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let user_name = ctx.user_name.clone();
    let timer = Timer::default();
    timer.start(TimerMode::SingleShot, Duration::from_secs(5), move || {
        if !bridge.is_online() {
            tracing::info!("Morning brief skipped — companion offline");
            return;
        }
        tracing::info!("Composing morning brief");
        let prompt = format!(
            "You just started up. Compose a morning brief for {user_name}. \
             Use your tools to check email, calendar, weather, system status, \
             and recall recent topics of interest. Skip any sources that fail \
             or that {user_name} has asked you not to include. \
             Keep it natural and concise — a few flowing sentences, no bullet points."
        );
        // Fire-and-forget: the response flows through the normal notification path
        let _rx = bridge.send_message(prompt);
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
