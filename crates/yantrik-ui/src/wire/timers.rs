//! Background timers — clock, think cycle, card tick, frecency flush, hourly snapshot.
//!
//! The morning brief is not here any more. It was: five seconds after every shell start this
//! module asked the companion to compose one, with tools, and the companion runs one thing at
//! a time — so for the minutes that took, every `companion.tool` and `companion.ask` sat in
//! the queue behind it. The shell restarts on every deploy and every crash, which made the
//! machine's mind unreachable after each one. `wire::morning_brief` owns the brief now, both
//! the card and the conversational half, behind one once-a-day claim that survives restarts.

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
    wire_frecency_persist(ctx);
    wire_hourly_snapshot(ctx);
}

/// How long until the next minute begins, given the time since the epoch.
///
/// The clock reads "Fri 2 Oct · 14:32" and has no seconds, so the only moment it can be wrong is
/// the instant the minute changes. A repeating 30-second timer was wrong for up to 29 of every
/// 30 seconds around that instant and woke the shell twice a minute to say nothing; this asks to be
/// woken once, on the minute. A few milliseconds past the boundary, so the time is read inside the
/// new minute and never the old one.
pub(crate) fn until_next_minute(since_epoch: Duration) -> Duration {
    let into_minute = since_epoch.as_millis() % 60_000;
    Duration::from_millis((60_000 - into_minute) as u64 + 20)
}

/// The clock and the personalised greeting: set now, then again at the top of every minute.
///
/// One single-shot timer that arms the next one, not a repeating one: between minutes nothing here
/// is scheduled to run (the idle rule: no repeating Slint timers for state).
fn wire_clock(ui: &App, user_name: &str) {
    tick_clock(ui.as_weak(), user_name.to_string());
}

fn tick_clock(ui_weak: slint::Weak<App>, name: String) {
    let Some(ui) = ui_weak.upgrade() else { return };
    ui.set_clock_text(app_context::current_time_hhmm().into());
    ui.set_date_text(app_context::current_date_short().into());
    ui.set_greeting_text(format!("{}, {}", app_context::time_of_day_greeting(), name).into());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    Timer::single_shot(until_next_minute(now), move || tick_clock(ui_weak, name));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The wake-up lands just after the minute turns, wherever in the minute it is asked.
    #[test]
    fn the_clock_wakes_once_just_after_the_minute_turns() {
        let after = |secs: f64| until_next_minute(Duration::from_secs_f64(secs));
        // Half a second into a minute: 59.5 s to go, and the 20 ms of slack.
        assert_eq!(after(60.0 * 7.0 + 0.5), Duration::from_millis(59_520));
        // Exactly on the minute: a whole minute, never zero (which would spin).
        assert_eq!(after(60.0 * 9.0), Duration::from_millis(60_020));
        // One millisecond before the turn: just the slack and the millisecond.
        assert_eq!(after(60.0 * 3.0 - 0.001), Duration::from_millis(21));
    }

    /// The idle rule, read from the source: no repeating timer drives the clock.
    #[test]
    fn the_clock_has_no_repeating_timer() {
        let src = include_str!("timers.rs");
        let clock = &src[src.find("fn tick_clock(").unwrap()..src.find("/// Background cognition").unwrap()];
        assert!(clock.contains("Timer::single_shot("), "the clock re-arms itself once per minute");
        assert!(!clock.contains("TimerMode::Repeated"), "and never repeats on an interval");
    }
}
