//! Shared token streaming — deduplicates the streaming logic used by
//! on_send_message and on_lens_submit.
//!
//! Both callers need to:
//! 1. Add a user message bubble
//! 2. Add an empty assistant bubble (is_streaming: true)
//! 3. Set is_generating / is_thinking / companion_status / lens_chat_mode
//! 4. Poll tokens at 16ms, appending to the last message
//! 5. On __DONE__, finalize the assistant bubble

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::{Model, SharedString, Timer, TimerMode, VecModel};

use crate::bridge::CompanionBridge;
use crate::{App, MessageData};

/// Start streaming tokens from the companion into the message model.
///
/// Adds user + empty assistant bubbles, sets UI state, starts a 16ms poll timer.
/// The caller provides `timer_slot` (an `Rc<RefCell<Option<Timer>>>`) to keep the
/// timer alive. The timer self-cleans by setting the slot to `None` on `__DONE__`.
pub fn start_ai_stream(
    ui_weak: slint::Weak<App>,
    bridge: &Arc<CompanionBridge>,
    text: &str,
    timer_slot: &Rc<RefCell<Option<Timer>>>,
) {
    // 1. Add user message + empty assistant bubble
    if let Some(ui) = ui_weak.upgrade() {
        let messages = ui.get_messages();
        let model = messages
            .as_any()
            .downcast_ref::<VecModel<MessageData>>()
            .unwrap();
        model.push(MessageData {
            role: "user".into(),
            content: SharedString::from(text),
            is_streaming: false,
        });
        model.push(MessageData {
            role: "assistant".into(),
            content: "".into(),
            is_streaming: true,
        });
        ui.set_is_generating(true);
        ui.set_is_thinking(true);
        ui.set_companion_status("thinking".into());
        ui.set_lens_chat_mode(true);
    }

    // 2. Start streaming from companion
    let token_rx = bridge.send_message(text.to_string());
    let timer_handle = timer_slot.clone();
    let ui_weak_stream = ui_weak.clone();

    // 3. Poll tokens at 16ms (60fps)
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let mut done = false;
        while let Ok(token) = token_rx.try_recv() {
            if token == "__DONE__" {
                done = true;
                break;
            }
            if let Some(ui) = ui_weak_stream.upgrade() {
                let messages = ui.get_messages();
                let model = messages
                    .as_any()
                    .downcast_ref::<VecModel<MessageData>>()
                    .unwrap();
                let count = model.row_count();
                if count > 0 {
                    let mut last = model.row_data(count - 1).unwrap();
                    let mut content = last.content.to_string();
                    content.push_str(&token);
                    last.content = SharedString::from(&content);
                    model.set_row_data(count - 1, last);
                }
            }
        }
        if done {
            if let Some(ui) = ui_weak_stream.upgrade() {
                ui.set_is_generating(false);
                ui.set_is_thinking(false);
                ui.set_companion_status("idle".into());
                let messages = ui.get_messages();
                let model = messages
                    .as_any()
                    .downcast_ref::<VecModel<MessageData>>()
                    .unwrap();
                let count = model.row_count();
                if count > 0 {
                    let mut last = model.row_data(count - 1).unwrap();
                    last.is_streaming = false;
                    model.set_row_data(count - 1, last);
                }
            }
            *timer_handle.borrow_mut() = None;
        }
    });
    *timer_slot.borrow_mut() = Some(timer);
}

/// Start a proactive AI stream — only the assistant's response is shown (no user bubble).
/// Used for morning brief and other proactive messages where the AI speaks first.
pub fn start_proactive_stream(
    ui_weak: slint::Weak<App>,
    bridge: &Arc<CompanionBridge>,
    hidden_prompt: &str,
    timer_slot: &Rc<RefCell<Option<Timer>>>,
) {
    // Only add assistant bubble (the prompt is hidden from the user)
    if let Some(ui) = ui_weak.upgrade() {
        let messages = ui.get_messages();
        let model = messages
            .as_any()
            .downcast_ref::<VecModel<MessageData>>()
            .unwrap();
        model.push(MessageData {
            role: "assistant".into(),
            content: "".into(),
            is_streaming: true,
        });
        ui.set_is_generating(true);
        ui.set_is_thinking(true);
        ui.set_companion_status("thinking".into());
    }

    let token_rx = bridge.send_message(hidden_prompt.to_string());
    let timer_handle = timer_slot.clone();
    let ui_weak_stream = ui_weak.clone();

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let mut done = false;
        while let Ok(token) = token_rx.try_recv() {
            if token == "__DONE__" {
                done = true;
                break;
            }
            if let Some(ui) = ui_weak_stream.upgrade() {
                let messages = ui.get_messages();
                let model = messages
                    .as_any()
                    .downcast_ref::<VecModel<MessageData>>()
                    .unwrap();
                let count = model.row_count();
                if count > 0 {
                    let mut last = model.row_data(count - 1).unwrap();
                    let mut content = last.content.to_string();
                    content.push_str(&token);
                    last.content = SharedString::from(&content);
                    model.set_row_data(count - 1, last);
                }
            }
        }
        if done {
            if let Some(ui) = ui_weak_stream.upgrade() {
                ui.set_is_generating(false);
                ui.set_is_thinking(false);
                ui.set_companion_status("idle".into());
                let messages = ui.get_messages();
                let model = messages
                    .as_any()
                    .downcast_ref::<VecModel<MessageData>>()
                    .unwrap();
                let count = model.row_count();
                if count > 0 {
                    let mut last = model.row_data(count - 1).unwrap();
                    last.is_streaming = false;
                    model.set_row_data(count - 1, last);
                }
            }
            *timer_handle.borrow_mut() = None;
        }
    });
    *timer_slot.borrow_mut() = Some(timer);
}
