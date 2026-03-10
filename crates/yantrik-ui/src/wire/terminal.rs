//! Terminal wiring — PTY lifecycle, key input, AI error detection.
//!
//! Supports multiple tabs (up to 8), each with an independent PTY session.
//! Ctrl+T opens a new tab, Ctrl+W closes the current tab,
//! Ctrl+Tab / Ctrl+Shift+Tab switches between tabs.
//! A 33ms timer polls the active tab's PTY for output and updates the UI.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::terminal::{self, TerminalHandle};
use crate::{App, TerminalTabData};

/// Maximum number of terminal tabs.
const MAX_TABS: usize = 8;

/// Sync the tab model to the UI: builds TerminalTabData array from the terminals vec.
fn sync_tabs_to_ui(ui: &App, terminals: &[TerminalHandle], active: usize) {
    let tab_data: Vec<TerminalTabData> = terminals
        .iter()
        .enumerate()
        .map(|(i, th)| TerminalTabData {
            title: slint::format!("Shell {}", i + 1),
            is_active: i == active,
            is_alive: th.is_alive(),
        })
        .collect();
    ui.set_terminal_tab_count(terminals.len() as i32);
    ui.set_terminal_active_tab(active as i32);
    ui.set_terminal_tabs(ModelRc::new(VecModel::from(tab_data)));
}

/// Update the UI with the active tab's terminal output/state.
fn sync_active_tab_output(ui: &App, terminals: &[TerminalHandle], active: usize) {
    if let Some(th) = terminals.get(active) {
        ui.set_terminal_output(th.get_full_text().into());
        ui.set_terminal_is_alive(th.is_alive());
        let (row, col) = th.cursor_position();
        ui.set_terminal_cursor_row(row as i32);
        ui.set_terminal_cursor_col(col as i32);
    }
}

/// Wire terminal callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let terminals = ctx.terminals.clone();
    let active_tab = ctx.terminal_active.clone();
    let bridge = ctx.bridge.clone();

    // Split pane state: the optional second terminal handle
    let split_handle: Rc<RefCell<Option<TerminalHandle>>> = Rc::new(RefCell::new(None));
    // Which pane is active: 0 = primary, 1 = split
    let active_pane: Rc<RefCell<i32>> = Rc::new(RefCell::new(0));
    // Current profile name
    let profile_name: Rc<RefCell<String>> = Rc::new(RefCell::new("Default".into()));

    // ── Key press handler ──
    // Intercepts Ctrl+T (new tab), Ctrl+W (close tab), Ctrl+Tab (next),
    // Ctrl+Shift+Tab (prev) before forwarding to PTY.
    let term_key = terminals.clone();
    let active_key = active_tab.clone();
    let ui_weak_key = ui.as_weak();
    ui.on_terminal_key_pressed(move |event| {
        let key_text = event.text.to_string();
        let shift = event.modifiers.shift;
        let ctrl = event.modifiers.control;

        // Ctrl+T — new tab (0x14 is the control code for Ctrl+T)
        if ctrl && (key_text == "t" || key_text == "T" || key_text == "\x14") {
            let mut tabs = term_key.borrow_mut();
            if tabs.len() < MAX_TABS {
                match TerminalHandle::spawn(24, 80) {
                    Ok(th) => {
                        tabs.push(th);
                        let new_idx = tabs.len() - 1;
                        *active_key.borrow_mut() = new_idx;
                        if let Some(ui) = ui_weak_key.upgrade() {
                            sync_tabs_to_ui(&ui, &tabs, new_idx);
                            sync_active_tab_output(&ui, &tabs, new_idx);
                            ui.set_terminal_has_suggestion(false);
                            ui.set_terminal_ai_suggestion("".into());
                        }
                        tracing::info!(tab = new_idx + 1, "New terminal tab opened");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to spawn terminal tab");
                    }
                }
            }
            return slint::private_unstable_api::re_exports::EventResult::Accept;
        }

        // Ctrl+W — close current tab (0x17 is the control code for Ctrl+W)
        if ctrl && (key_text == "w" || key_text == "W" || key_text == "\x17") {
            let mut tabs = term_key.borrow_mut();
            if !tabs.is_empty() {
                let idx = *active_key.borrow();
                tabs.remove(idx);

                if tabs.is_empty() {
                    // Last tab closed — navigate back to desktop
                    *active_key.borrow_mut() = 0;
                    if let Some(ui) = ui_weak_key.upgrade() {
                        sync_tabs_to_ui(&ui, &tabs, 0);
                        ui.set_terminal_output("".into());
                        // Navigate to desktop
                        ui.set_current_screen(1);
                        ui.invoke_navigate(1);
                    }
                    tracing::info!("Last terminal tab closed, navigating to desktop");
                } else {
                    // Adjust active index
                    let new_idx = if idx >= tabs.len() { tabs.len() - 1 } else { idx };
                    *active_key.borrow_mut() = new_idx;
                    if let Some(ui) = ui_weak_key.upgrade() {
                        sync_tabs_to_ui(&ui, &tabs, new_idx);
                        sync_active_tab_output(&ui, &tabs, new_idx);
                        ui.set_terminal_has_suggestion(false);
                        ui.set_terminal_ai_suggestion("".into());
                    }
                    tracing::info!(tab = new_idx + 1, total = tabs.len(), "Terminal tab closed");
                }
            }
            return slint::private_unstable_api::re_exports::EventResult::Accept;
        }

        // Ctrl+Tab / Ctrl+Shift+Tab — cycle tabs
        if ctrl && key_text == "\t" {
            let tabs = term_key.borrow();
            if tabs.len() > 1 {
                let current = *active_key.borrow();
                let new_idx = if shift {
                    // Ctrl+Shift+Tab — previous tab
                    if current == 0 { tabs.len() - 1 } else { current - 1 }
                } else {
                    // Ctrl+Tab — next tab
                    (current + 1) % tabs.len()
                };
                drop(tabs);
                *active_key.borrow_mut() = new_idx;
                let tabs = term_key.borrow();
                if let Some(ui) = ui_weak_key.upgrade() {
                    sync_tabs_to_ui(&ui, &tabs, new_idx);
                    sync_active_tab_output(&ui, &tabs, new_idx);
                    ui.set_terminal_has_suggestion(false);
                    ui.set_terminal_ai_suggestion("".into());
                }
            }
            return slint::private_unstable_api::re_exports::EventResult::Accept;
        }

        // Forward other keys to the active tab's PTY
        let guard = term_key.borrow();
        let idx = *active_key.borrow();
        if let Some(th) = guard.get(idx) {
            let app_cursor = th.application_cursor_mode();
            if let Some(bytes) = terminal::key_to_pty_bytes(&key_text, shift, ctrl, app_cursor) {
                th.write_bytes(&bytes);
            }
        }
        slint::private_unstable_api::re_exports::EventResult::Accept
    });

    // ── Tab switch callback (from UI click) ──
    let term_switch = terminals.clone();
    let active_switch = active_tab.clone();
    let ui_weak_switch = ui.as_weak();
    ui.on_terminal_switch_tab(move |idx| {
        let idx = idx as usize;
        let tabs = term_switch.borrow();
        if idx < tabs.len() {
            *active_switch.borrow_mut() = idx;
            if let Some(ui) = ui_weak_switch.upgrade() {
                sync_tabs_to_ui(&ui, &tabs, idx);
                sync_active_tab_output(&ui, &tabs, idx);
                ui.set_terminal_has_suggestion(false);
                ui.set_terminal_ai_suggestion("".into());
            }
        }
    });

    // ── New tab callback (from UI + button) ──
    let term_new = terminals.clone();
    let active_new = active_tab.clone();
    let ui_weak_new = ui.as_weak();
    ui.on_terminal_new_tab(move || {
        let mut tabs = term_new.borrow_mut();
        if tabs.len() < MAX_TABS {
            match TerminalHandle::spawn(24, 80) {
                Ok(th) => {
                    tabs.push(th);
                    let new_idx = tabs.len() - 1;
                    *active_new.borrow_mut() = new_idx;
                    if let Some(ui) = ui_weak_new.upgrade() {
                        sync_tabs_to_ui(&ui, &tabs, new_idx);
                        sync_active_tab_output(&ui, &tabs, new_idx);
                        ui.set_terminal_has_suggestion(false);
                        ui.set_terminal_ai_suggestion("".into());
                    }
                    tracing::info!(tab = new_idx + 1, "New terminal tab opened via button");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to spawn terminal tab");
                }
            }
        }
    });

    // ── Close tab callback (from UI x button) ──
    let term_close = terminals.clone();
    let active_close = active_tab.clone();
    let ui_weak_close = ui.as_weak();
    ui.on_terminal_close_tab(move |idx| {
        let idx = idx as usize;
        let mut tabs = term_close.borrow_mut();
        if idx < tabs.len() {
            tabs.remove(idx);

            if tabs.is_empty() {
                *active_close.borrow_mut() = 0;
                if let Some(ui) = ui_weak_close.upgrade() {
                    sync_tabs_to_ui(&ui, &tabs, 0);
                    ui.set_terminal_output("".into());
                    ui.set_current_screen(1);
                    ui.invoke_navigate(1);
                }
                tracing::info!("Last terminal tab closed via button, navigating to desktop");
            } else {
                let current = *active_close.borrow();
                let new_idx = if idx == current {
                    // Closed the active tab: stay at same index if possible, else go back
                    if current >= tabs.len() { tabs.len() - 1 } else { current }
                } else if idx < current {
                    // Closed a tab before the active one: shift index back
                    current - 1
                } else {
                    // Closed a tab after the active one: no change needed
                    current
                };
                *active_close.borrow_mut() = new_idx;
                if let Some(ui) = ui_weak_close.upgrade() {
                    sync_tabs_to_ui(&ui, &tabs, new_idx);
                    sync_active_tab_output(&ui, &tabs, new_idx);
                    ui.set_terminal_has_suggestion(false);
                    ui.set_terminal_ai_suggestion("".into());
                }
                tracing::info!(tab = new_idx + 1, total = tabs.len(), "Terminal tab closed via button");
            }
        }
    });

    // ── AI help request ──
    let bridge_help = bridge.clone();
    let term_help = terminals.clone();
    let active_help = active_tab.clone();
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

        // Get recent terminal context from active tab
        let context = {
            let guard = term_help.borrow();
            let idx = *active_help.borrow();
            if let Some(th) = guard.get(idx) {
                let rows = th.get_rows();
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

        let token_rx = bridge_help.send_message(prompt);

        if let Some(ui) = ui_weak_help.upgrade() {
            ui.set_terminal_ai_suggestion("Thinking...".into());

            let weak = ui_weak_help.clone();
            let collected = Rc::new(RefCell::new(String::new()));
            let collected_inner = collected.clone();
            let poll_timer = Timer::default();
            poll_timer.start(
                TimerMode::Repeated,
                Duration::from_millis(50),
                move || {
                    let mut got_token = false;
                    while let Ok(token) = token_rx.try_recv() {
                        if token == "__DONE__" {
                            if let Some(ui) = weak.upgrade() {
                                let text = collected_inner.borrow().clone();
                                ui.set_terminal_ai_suggestion(text.into());
                            }
                            return;
                        }
                        if token.starts_with("__") && token.ends_with("__") {
                            continue;
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
            std::mem::forget(poll_timer);
        }
    });

    // ── Search query changed ──
    let term_search = terminals.clone();
    let active_search = active_tab.clone();
    let ui_weak_search = ui.as_weak();
    ui.on_terminal_search_query_changed(move |query| {
        let query = query.to_string();
        if let Some(ui) = ui_weak_search.upgrade() {
            if query.is_empty() {
                ui.set_terminal_search_match_count(0);
                ui.set_terminal_search_current_match(0);
                return;
            }
            let guard = term_search.borrow();
            let idx = *active_search.borrow();
            if let Some(th) = guard.get(idx) {
                let text = th.get_full_text();
                let lower_text = text.to_lowercase();
                let lower_query = query.to_lowercase();
                let count = lower_text.matches(&lower_query).count() as i32;
                ui.set_terminal_search_match_count(count);
                ui.set_terminal_search_current_match(if count > 0 { 1 } else { 0 });
            }
        }
    });

    // ── Search next ──
    let ui_weak_snext = ui.as_weak();
    ui.on_terminal_search_next(move || {
        if let Some(ui) = ui_weak_snext.upgrade() {
            let count = ui.get_terminal_search_match_count();
            let current = ui.get_terminal_search_current_match();
            if count > 0 {
                let next = if current >= count { 1 } else { current + 1 };
                ui.set_terminal_search_current_match(next);
            }
        }
    });

    // ── Search prev ──
    let ui_weak_sprev = ui.as_weak();
    ui.on_terminal_search_prev(move || {
        if let Some(ui) = ui_weak_sprev.upgrade() {
            let count = ui.get_terminal_search_match_count();
            let current = ui.get_terminal_search_current_match();
            if count > 0 {
                let prev = if current <= 1 { count } else { current - 1 };
                ui.set_terminal_search_current_match(prev);
            }
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

    // ── Restart terminal (restarts active tab) ──
    let term_restart = terminals.clone();
    let active_restart = active_tab.clone();
    let ui_weak_restart = ui.as_weak();
    ui.on_terminal_restart(move || {
        let mut tabs = term_restart.borrow_mut();
        let idx = *active_restart.borrow();

        match TerminalHandle::spawn(24, 80) {
            Ok(th) => {
                if idx < tabs.len() {
                    tabs[idx] = th;
                } else {
                    // Shouldn't happen, but handle gracefully
                    tabs.push(th);
                    *active_restart.borrow_mut() = tabs.len() - 1;
                }
                if let Some(ui) = ui_weak_restart.upgrade() {
                    let active = *active_restart.borrow();
                    sync_tabs_to_ui(&ui, &tabs, active);
                    sync_active_tab_output(&ui, &tabs, active);
                    ui.set_terminal_has_suggestion(false);
                }
                tracing::info!("Terminal tab restarted");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to restart terminal tab");
            }
        }
    });

    // ── Split pane toggle ──
    let split_h = split_handle.clone();
    let ui_weak_split = ui.as_weak();
    ui.on_terminal_split_toggle(move || {
        let mut sh = split_h.borrow_mut();
        if sh.is_some() {
            // Close split
            *sh = None;
            if let Some(ui) = ui_weak_split.upgrade() {
                ui.set_terminal_split_active(false);
                ui.set_terminal_split_output("".into());
            }
            tracing::info!("Terminal split pane closed");
        } else {
            // Open split — spawn a new PTY
            match TerminalHandle::spawn(24, 80) {
                Ok(th) => {
                    *sh = Some(th);
                    if let Some(ui) = ui_weak_split.upgrade() {
                        ui.set_terminal_split_active(true);
                    }
                    tracing::info!("Terminal split pane opened");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to spawn split terminal");
                }
            }
        }
    });

    // ── Switch active pane ──
    let pane = active_pane.clone();
    ui.on_terminal_switch_pane(move |idx| {
        *pane.borrow_mut() = idx;
    });

    // ── Split pane input (key text forwarded from split FocusScope) ──
    let split_h_input = split_handle.clone();
    ui.on_terminal_split_input(move |text| {
        let guard = split_h_input.borrow();
        if let Some(th) = guard.as_ref() {
            let key_text = text.to_string();
            if let Some(bytes) = terminal::key_to_pty_bytes(&key_text, false, false, th.application_cursor_mode()) {
                th.write_bytes(&bytes);
            }
        }
    });

    // ── Set profile ──
    let prof = profile_name.clone();
    let ui_weak_prof = ui.as_weak();
    ui.on_terminal_set_profile(move |name| {
        let name_str = name.to_string();
        *prof.borrow_mut() = name_str.clone();
        if let Some(ui) = ui_weak_prof.upgrade() {
            ui.set_terminal_profile_name(name.clone());
        }
        tracing::info!(profile = %name_str, "Terminal profile set");
    });

    // Store split handle in context for poll timer access
    *ctx.terminal_split_handle.borrow_mut() = Some(split_handle.clone());
}

/// Start the terminal output polling timer.
/// Called from navigate.rs when entering screen 14.
/// Polls the active tab's PTY for output and updates the UI.
/// Also polls the split pane if active.
pub fn start_poll_timer(
    ui: &App,
    terminals: &Rc<RefCell<Vec<TerminalHandle>>>,
    active_tab: &Rc<RefCell<usize>>,
    bridge: &Arc<crate::bridge::CompanionBridge>,
    timer_slot: &Rc<RefCell<Option<Timer>>>,
) {
    let term = terminals.clone();
    let active = active_tab.clone();
    let ui_weak = ui.as_weak();
    let bridge = bridge.clone();
    let error_cooldown: Rc<RefCell<f64>> = Rc::new(RefCell::new(0.0));

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(33), move || {
        let guard = term.borrow();
        let idx = *active.borrow();
        let th = match guard.get(idx) {
            Some(th) => th,
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

            // Sync tab alive states (lightweight — just update is_alive flags)
            sync_tabs_to_ui(&ui, &guard, idx);

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
                        let short = if error_context.len() > 120 {
                            format!("{}...", &error_context[..120])
                        } else {
                            error_context.clone()
                        };
                        ui.set_terminal_ai_suggestion(short.into());
                        ui.set_terminal_has_suggestion(true);

                        bridge.record_system_event(
                            format!("Terminal error: {}", &error_context[..error_context.len().min(200)]),
                            "terminal/error".to_string(),
                            0.6,
                        );

                        *error_cooldown.borrow_mut() = now + 10.0;
                    }
                }
            }
        }
    });
    *timer_slot.borrow_mut() = Some(timer);
}

/// Start polling the split pane terminal output.
/// Called alongside the main poll timer.
pub fn start_split_poll_timer(
    ui: &App,
    split_handle: &Rc<RefCell<Option<TerminalHandle>>>,
    timer_slot: &Rc<RefCell<Option<Timer>>>,
) {
    let sh = split_handle.clone();
    let ui_weak = ui.as_weak();

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(33), move || {
        let guard = sh.borrow();
        if let Some(th) = guard.as_ref() {
            if let Some(ui) = ui_weak.upgrade() {
                let text = th.get_full_text();
                ui.set_terminal_split_output(text.into());
                let (row, col) = th.cursor_position();
                ui.set_terminal_split_cursor_row(row as i32);
                ui.set_terminal_split_cursor_col(col as i32);
            }
        }
    });
    *timer_slot.borrow_mut() = Some(timer);
}
