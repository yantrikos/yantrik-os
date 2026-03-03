//! Terminal wiring — PTY lifecycle, key input, AI error detection.
//!
//! Spawns the terminal lazily on first navigation to screen 14.
//! A 16ms timer polls the PTY for new output and updates the UI.
//! Error detection runs on each poll tick and surfaces suggestions.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app_context::AppContext;
use crate::terminal::{self, TerminalHandle};
use crate::App;

/// Wire terminal callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let terminal = ctx.terminal.clone();
    let bridge = ctx.bridge.clone();

    // ── Key press handler ──
    let term_key = terminal.clone();
    ui.on_terminal_key_pressed(move |event| {
        let guard = term_key.borrow();
        if let Some(ref th) = *guard {
            let key_text = event.text.to_string();
            let shift = event.modifiers.shift;
            let ctrl = event.modifiers.control;
            let app_cursor = th.application_cursor_mode();

            if let Some(bytes) = terminal::key_to_pty_bytes(&key_text, shift, ctrl, app_cursor) {
                th.write_bytes(&bytes);
            }
        }
        slint::private_unstable_api::re_exports::EventResult::Accept
    });

    // ── AI help request ──
    let bridge_help = bridge.clone();
    let term_help = terminal.clone();
    let ui_weak_help = ui.as_weak();
    ui.on_terminal_request_ai_help(move || {
        let suggestion = {
            if let Some(ui) = ui_weak_help.upgrade() {
                ui.get_terminal_ai_suggestion().to_string()
            } else {
                return;
            }
        };

        if suggestion.is_empty() {
            return;
        }

        // Get recent terminal context
        let context = {
            let guard = term_help.borrow();
            if let Some(ref th) = *guard {
                let rows = th.get_rows();
                // Last 20 lines of terminal output
                let start = rows.len().saturating_sub(20);
                rows[start..].join("\n")
            } else {
                String::new()
            }
        };

        let prompt = format!(
            "The user's terminal shows an error. Help diagnose and fix it.\n\
             \n\
             Terminal output (last 20 lines):\n\
             ```\n{}\n```\n\
             \n\
             Error summary: {}\n\
             \n\
             Provide a clear, concise fix. If it's a command, show the exact command to run.",
            context, suggestion
        );

        // Send to companion and stream response inline
        let token_rx = bridge_help.send_message(prompt);

        if let Some(ui) = ui_weak_help.upgrade() {
            ui.set_terminal_ai_suggestion("Thinking...".into());

            // Poll for streaming tokens and display inline
            let weak = ui_weak_help.clone();
            let collected = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
            let collected_inner = collected.clone();
            let poll_timer = slint::Timer::default();
            poll_timer.start(
                slint::TimerMode::Repeated,
                std::time::Duration::from_millis(50),
                move || {
                    let mut got_token = false;
                    // Drain all available tokens
                    while let Ok(token) = token_rx.try_recv() {
                        // Filter internal markers
                        if token == "__DONE__" {
                            // Generation complete — show final (keep newlines for column display)
                            if let Some(ui) = weak.upgrade() {
                                let text = collected_inner.borrow().clone();
                                ui.set_terminal_ai_suggestion(text.into());
                            }
                            return;
                        }
                        if token.starts_with("__") && token.ends_with("__") {
                            continue; // Skip internal markers like __REPLACE__
                        }
                        collected_inner.borrow_mut().push_str(&token);
                        got_token = true;
                    }
                    if got_token {
                        if let Some(ui) = weak.upgrade() {
                            let text = collected_inner.borrow().clone();
                            ui.set_terminal_ai_suggestion(text.into());
                        }
                    }
                },
            );
            // Keep timer alive until done
            std::mem::forget(poll_timer);
        }
    });

    // ── Dismiss suggestion ──
    let ui_weak_dismiss = ui.as_weak();
    ui.on_terminal_dismiss_suggestion(move || {
        if let Some(ui) = ui_weak_dismiss.upgrade() {
            ui.set_terminal_has_suggestion(false);
            ui.set_terminal_ai_suggestion("".into());
        }
    });

    // ── Restart terminal ──
    let term_restart = terminal.clone();
    let ui_weak_restart = ui.as_weak();
    ui.on_terminal_restart(move || {
        // Drop old terminal
        *term_restart.borrow_mut() = None;

        // Spawn new one
        match TerminalHandle::spawn(24, 80) {
            Ok(th) => {
                *term_restart.borrow_mut() = Some(th);
                if let Some(ui) = ui_weak_restart.upgrade() {
                    ui.set_terminal_is_alive(true);
                    ui.set_terminal_output("".into());
                    ui.set_terminal_has_suggestion(false);
                }
                tracing::info!("Terminal restarted");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to restart terminal");
            }
        }
    });
}

/// Start the terminal output polling timer.
/// Called from navigate.rs when entering screen 14.
pub fn start_poll_timer(
    ui: &App,
    terminal: &Rc<RefCell<Option<TerminalHandle>>>,
    bridge: &Arc<crate::bridge::CompanionBridge>,
    timer_slot: &Rc<RefCell<Option<Timer>>>,
) {
    let term = terminal.clone();
    let ui_weak = ui.as_weak();
    let bridge = bridge.clone();
    let error_cooldown: Rc<RefCell<f64>> = Rc::new(RefCell::new(0.0));

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(33), move || {
        let guard = term.borrow();
        let th = match *guard {
            Some(ref th) => th,
            None => return,
        };

        if let Some(ui) = ui_weak.upgrade() {
            // Update terminal output
            let text = th.get_full_text();
            ui.set_terminal_output(text.into());

            // Update alive status
            let alive = th.is_alive();
            ui.set_terminal_is_alive(alive);

            // Update cursor position
            let (row, col) = th.cursor_position();
            ui.set_terminal_cursor_row(row as i32);
            ui.set_terminal_cursor_col(col as i32);

            // AI error detection (with cooldown)
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();

            let cooldown = *error_cooldown.borrow();
            if now > cooldown {
                let last_output = th.take_last_output();
                if !last_output.is_empty() {
                    if let Some(error_context) = terminal::detect_errors(&last_output) {
                        // Show suggestion
                        let short = if error_context.len() > 120 {
                            format!("{}...", &error_context[..120])
                        } else {
                            error_context.clone()
                        };
                        ui.set_terminal_ai_suggestion(short.into());
                        ui.set_terminal_has_suggestion(true);

                        // Record in memory
                        bridge.record_system_event(
                            format!("Terminal error: {}", &error_context[..error_context.len().min(200)]),
                            "terminal/error".to_string(),
                            0.6,
                        );

                        // 10-second cooldown between error detections
                        *error_cooldown.borrow_mut() = now + 10.0;
                    }
                }
            }
        }
    });
    *timer_slot.borrow_mut() = Some(timer);
}
