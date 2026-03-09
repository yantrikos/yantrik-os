//! Morning Brief Card — wires the structured day brief to the desktop card.
//!
//! Flow:
//! 1. Timer fires 8 seconds after boot (after the existing morning brief LLM prompt at 5s)
//! 2. Requests structured brief from companion (active context sections)
//! 3. Populates the MorningBriefCard Slint properties
//! 4. Card auto-hides after 5 minutes or on user dismiss
//!
//! The card coexists with the existing streaming morning brief in chat —
//! the card is a visual summary, the chat is the conversational greeting.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::bridge::MorningBriefSnapshot;
use crate::{App, BriefSection};

/// Wire the morning brief card on the desktop.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_brief_card(ui, ctx);
    wire_brief_dismiss(ui);
    wire_brief_section_action(ui, ctx);
}

/// Timer: request structured brief 8 seconds after boot, populate the card.
fn wire_brief_card(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();

    let timer = Timer::default();
    timer.start(TimerMode::SingleShot, Duration::from_secs(8), move || {
        // Only show if companion is online
        if !bridge.is_online() {
            tracing::info!("Morning brief card skipped — companion offline");
            return;
        }

        let reply_rx = bridge.request_morning_brief();
        let weak = ui_weak.clone();

        // Poll for the reply (brief should arrive almost instantly — no LLM call)
        let poll_timer = Timer::default();
        let poll_timer_slot: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
        let slot = poll_timer_slot.clone();
        let poll_count: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

        poll_timer.start(TimerMode::Repeated, Duration::from_millis(200), move || {
            *poll_count.borrow_mut() += 1;

            // Give up after 5 seconds (25 polls)
            if *poll_count.borrow() > 25 {
                tracing::warn!("Morning brief reply timed out");
                if let Some(t) = slot.borrow_mut().take() {
                    t.stop();
                }
                return;
            }

            if let Ok(snapshot) = reply_rx.try_recv() {
                populate_brief_card(&weak, &snapshot);
                // Auto-dismiss after 5 minutes
                schedule_auto_dismiss(weak.clone());
                // Stop polling
                if let Some(t) = slot.borrow_mut().take() {
                    t.stop();
                }
            }
        });
        *poll_timer_slot.borrow_mut() = Some(poll_timer);
    });
    std::mem::forget(timer);
}

/// Populate the Slint card with brief data.
fn populate_brief_card(ui_weak: &slint::Weak<App>, snapshot: &MorningBriefSnapshot) {
    let greeting = snapshot.greeting.clone();
    let sections: Vec<BriefSection> = snapshot
        .sections
        .iter()
        .map(|s| BriefSection {
            icon: SharedString::from(&s.icon),
            label: SharedString::from(&s.label),
            content: SharedString::from(&s.content),
            expanded: s.expanded,
            action_id: SharedString::from(&s.action_id),
        })
        .collect();

    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_morning_brief_greeting(SharedString::from(&greeting));
            ui.set_morning_brief_sections(ModelRc::new(VecModel::from(sections)));
            ui.set_morning_brief_visible(true);
            tracing::info!("Morning brief card displayed");
        }
    });
}

/// Schedule auto-dismiss of the brief card after 5 minutes.
fn schedule_auto_dismiss(ui_weak: slint::Weak<App>) {
    let timer = Timer::default();
    timer.start(TimerMode::SingleShot, Duration::from_secs(300), move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_morning_brief_visible(false);
            tracing::debug!("Morning brief card auto-dismissed");
        }
    });
    std::mem::forget(timer);
}

/// Wire the dismiss callback — hides the card.
fn wire_brief_dismiss(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_morning_brief_dismissed(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_morning_brief_visible(false);
            tracing::info!("Morning brief card dismissed by user");
        }
    });
}

/// Wire section action callbacks — navigate to relevant app screens.
fn wire_brief_section_action(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let _bridge = ctx.bridge.clone();
    ui.on_morning_brief_section_action(move |action_id| {
        let action = action_id.to_string();
        if action.starts_with("navigate:") {
            let screen_name = &action["navigate:".len()..];
            let screen_id = match screen_name {
                "weather" => 19,
                "calendar" => 18,
                "email" => 17,
                "notifications" => 9,
                _ => return,
            };
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_current_screen(screen_id);
                tracing::info!(screen = screen_name, "Morning brief section navigated");
            }
        }
    });
}
