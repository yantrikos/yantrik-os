//! Terminal wiring — PTY lifecycle, key input, AI-native features.
//!
//! Supports multiple tabs (up to 8), each with an independent PTY session.
//! Ctrl+T opens a new tab, Ctrl+W closes the current tab,
//! Ctrl+Tab / Ctrl+Shift+Tab switches between tabs.
//! A 33ms timer polls the active tab's PTY for output and updates the UI.
//!
//! AI features: natural language command bar, ghost autocomplete,
//! dangerous command warnings, error diagnosis, command explanation.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::terminal::{self, TerminalHandle};
use crate::{App, TerminalSegment, TerminalTabData};

/// Maximum number of terminal tabs.
const MAX_TABS: usize = 8;

/// Sync the tab model to the UI: builds TerminalTabData array from the terminals vec.
fn sync_tabs_to_ui(ui: &App, terminals: &[TerminalHandle], active: usize) {
    let tab_data: Vec<TerminalTabData> = terminals
        .iter()
        .enumerate()
        .map(|(i, th)| {
            // Use CWD basename or "Shell N" as tab title
            let cwd = th.get_cwd();
            let title = if cwd.is_empty() {
                slint::format!("Shell {}", i + 1)
            } else {
                let basename = cwd.rsplit('/').next().unwrap_or(&cwd);
                slint::SharedString::from(basename).into()
            };
            TerminalTabData {
                title,
                is_active: i == active,
                is_alive: th.is_alive(),
            }
        })
        .collect();
    ui.set_terminal_tab_count(terminals.len() as i32);
    ui.set_terminal_active_tab(active as i32);
    ui.set_terminal_tabs(ModelRc::new(VecModel::from(tab_data)));
}

/// Convert backend ColorSegments to Slint TerminalSegment model.
fn segments_to_model(segments: &[terminal::ColorSegment]) -> ModelRc<TerminalSegment> {
    let items: Vec<TerminalSegment> = segments
        .iter()
        .map(|seg| TerminalSegment {
            text: slint::SharedString::from(&seg.text),
            fg_color: slint::Color::from_rgb_u8(seg.fg_r, seg.fg_g, seg.fg_b),
            bold: seg.bold,
            row: seg.row as i32,
            col_start: seg.col_start as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Update the UI with the active tab's terminal output/state.
fn sync_active_tab_output(ui: &App, terminals: &[TerminalHandle], active: usize) {
    if let Some(th) = terminals.get(active) {
        // Plain text fallback
        ui.set_terminal_output(th.get_full_text().into());
        // Color segments
        let segs = th.get_segments();
        ui.set_terminal_segments(segments_to_model(&segs));
        // Alive status
        ui.set_terminal_is_alive(th.is_alive());
        // Cursor
        let (row, col) = th.cursor_position();
        ui.set_terminal_cursor_row(row as i32);
        ui.set_terminal_cursor_col(col as i32);
        // CWD
        let cwd = th.get_cwd();
        if !cwd.is_empty() {
            ui.set_terminal_current_directory(cwd.into());
        }
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
    // Accumulated input line for dangerous command detection
    let input_line: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    // Track last known terminal area size for resize detection
    let last_area_size: Rc<RefCell<(i32, i32)>> = Rc::new(RefCell::new((0, 0)));

    // ── Key press handler ──
    // Intercepts Ctrl+T (new tab), Ctrl+W (close tab), Ctrl+Tab (next),
    // Ctrl+Shift+Tab (prev), dangerous command check on Enter.
    let term_key = terminals.clone();
    let active_key = active_tab.clone();
    let ui_weak_key = ui.as_weak();
    let input_line_key = input_line.clone();
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
                    *active_key.borrow_mut() = 0;
                    if let Some(ui) = ui_weak_key.upgrade() {
                        sync_tabs_to_ui(&ui, &tabs, 0);
                        ui.set_terminal_output("".into());
                        ui.set_current_screen(1);
                        ui.invoke_navigate(1);
                    }
                    tracing::info!("Last terminal tab closed, navigating to desktop");
                } else {
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
                    if current == 0 { tabs.len() - 1 } else { current - 1 }
                } else {
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

        // Ctrl+Shift+C — copy terminal text to clipboard
        if ctrl && shift && (key_text == "c" || key_text == "C" || key_text == "\x03") {
            let guard = term_key.borrow();
            let idx = *active_key.borrow();
            if let Some(th) = guard.get(idx) {
                let text = th.get_full_text();
                // Use wl-copy (Wayland) for clipboard
                if let Ok(mut child) = std::process::Command::new("wl-copy")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                {
                    if let Some(ref mut stdin) = child.stdin {
                        let _ = std::io::Write::write_all(stdin, text.as_bytes());
                    }
                    let _ = child.wait();
                    tracing::debug!("Terminal text copied to clipboard");
                }
            }
            return slint::private_unstable_api::re_exports::EventResult::Accept;
        }

        // Ctrl+Shift+V — paste from clipboard
        if ctrl && shift && (key_text == "v" || key_text == "V" || key_text == "\x16") {
            if let Ok(output) = std::process::Command::new("wl-paste")
                .arg("--no-newline")
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    let guard = term_key.borrow();
                    let idx = *active_key.borrow();
                    if let Some(th) = guard.get(idx) {
                        th.write_bytes(text.as_bytes());
                    }
                }
            }
            return slint::private_unstable_api::re_exports::EventResult::Accept;
        }

        // Track input line for dangerous command detection
        if key_text == "\r" || key_text == "\n" {
            let cmd = input_line_key.borrow().clone();
            if !cmd.trim().is_empty() {
                if let Some(warning) = terminal::is_dangerous_command(&cmd) {
                    // Show danger warning instead of sending to PTY
                    if let Some(ui) = ui_weak_key.upgrade() {
                        ui.set_terminal_show_danger_warning(true);
                        ui.set_terminal_danger_message(warning.into());
                    }
                    // Don't clear input_line yet — danger-proceed will send it
                    return slint::private_unstable_api::re_exports::EventResult::Accept;
                }
            }
            input_line_key.borrow_mut().clear();
        } else if key_text == "\u{0008}" || key_text == "\u{007f}" {
            // Backspace
            input_line_key.borrow_mut().pop();
        } else if !key_text.is_empty() && !ctrl && key_text.len() == 1 {
            let ch = key_text.chars().next().unwrap();
            if !ch.is_control() {
                input_line_key.borrow_mut().push(ch);
            }
        }

        // Dismiss ghost suggestion on any key except Tab
        if key_text != "\t" {
            if let Some(ui) = ui_weak_key.upgrade() {
                let ghost = ui.get_terminal_ghost_suggestion().to_string();
                if !ghost.is_empty() {
                    ui.set_terminal_ghost_suggestion("".into());
                }
            }
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
                    if current >= tabs.len() { tabs.len() - 1 } else { current }
                } else if idx < current {
                    current - 1
                } else {
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

    // ── AI help request (error diagnosis) ──
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
             Provide a clear, concise fix. If it's a command, show the exact command to run.\n\
             Format: first line is the fix command (if any), rest is explanation.",
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

    // ── AI command bar submit (natural language → shell command) ──
    let bridge_ai = bridge.clone();
    let ui_weak_ai = ui.as_weak();
    ui.on_terminal_ai_bar_submit(move |text| {
        let query = text.to_string();
        if query.trim().is_empty() {
            return;
        }

        if let Some(ui) = ui_weak_ai.upgrade() {
            ui.set_terminal_ai_generating(true);
            ui.set_terminal_ai_generated_command("".into());
        }

        let prompt = format!(
            "Generate a shell command for Alpine Linux that does: {}\n\
             Reply with ONLY the command, no explanation, no markdown, no backticks.",
            query
        );

        let token_rx = bridge_ai.send_message(prompt);
        let weak = ui_weak_ai.clone();
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
                            let cmd = collected_inner.borrow().trim().to_string();
                            ui.set_terminal_ai_generated_command(cmd.into());
                            ui.set_terminal_ai_generating(false);
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
                        let cmd = collected_inner.borrow().trim().to_string();
                        ui.set_terminal_ai_generated_command(cmd.into());
                    }
                }
            },
        );
        std::mem::forget(poll_timer);
    });

    // ── AI run command (execute the generated command) ──
    let term_run = terminals.clone();
    let active_run = active_tab.clone();
    let ui_weak_run = ui.as_weak();
    ui.on_terminal_ai_run_command(move || {
        if let Some(ui) = ui_weak_run.upgrade() {
            let cmd = ui.get_terminal_ai_generated_command().to_string();
            if cmd.is_empty() {
                return;
            }

            // Check for dangerous command first
            if let Some(warning) = terminal::is_dangerous_command(&cmd) {
                ui.set_terminal_show_danger_warning(true);
                ui.set_terminal_danger_message(warning.into());
                return;
            }

            // Write command + Enter to PTY
            let guard = term_run.borrow();
            let idx = *active_run.borrow();
            if let Some(th) = guard.get(idx) {
                let full = format!("{}\r", cmd);
                th.write_bytes(full.as_bytes());
            }

            // Clear AI bar state
            ui.set_terminal_ai_generated_command("".into());
        }
    });

    // ── Accept ghost autocomplete ──
    let term_ghost = terminals.clone();
    let active_ghost = active_tab.clone();
    let ui_weak_ghost = ui.as_weak();
    ui.on_terminal_accept_ghost(move || {
        if let Some(ui) = ui_weak_ghost.upgrade() {
            let suggestion = ui.get_terminal_ghost_suggestion().to_string();
            if suggestion.is_empty() {
                return;
            }

            let guard = term_ghost.borrow();
            let idx = *active_ghost.borrow();
            if let Some(th) = guard.get(idx) {
                th.write_bytes(suggestion.as_bytes());
            }
            ui.set_terminal_ghost_suggestion("".into());
        }
    });

    // ── Dangerous command: proceed ──
    let term_danger = terminals.clone();
    let active_danger = active_tab.clone();
    let ui_weak_danger = ui.as_weak();
    let input_line_danger = input_line.clone();
    ui.on_terminal_danger_proceed(move || {
        if let Some(ui) = ui_weak_danger.upgrade() {
            ui.set_terminal_show_danger_warning(false);
            ui.set_terminal_danger_message("".into());

            // Send the original command to PTY
            let cmd = input_line_danger.borrow().clone();
            if !cmd.is_empty() {
                let guard = term_danger.borrow();
                let idx = *active_danger.borrow();
                if let Some(th) = guard.get(idx) {
                    let full = format!("{}\r", cmd);
                    th.write_bytes(full.as_bytes());
                }
            }
            input_line_danger.borrow_mut().clear();
        }
    });

    // ── Dangerous command: cancel ──
    let ui_weak_dcancel = ui.as_weak();
    let input_line_cancel = input_line.clone();
    ui.on_terminal_danger_cancel(move || {
        if let Some(ui) = ui_weak_dcancel.upgrade() {
            ui.set_terminal_show_danger_warning(false);
            ui.set_terminal_danger_message("".into());
        }
        input_line_cancel.borrow_mut().clear();
    });

    // ── Explain line (Alt+Click on terminal row) ──
    let bridge_explain = bridge.clone();
    let term_explain = terminals.clone();
    let active_explain = active_tab.clone();
    let ui_weak_explain = ui.as_weak();
    ui.on_terminal_explain_line(move |row| {
        let line = {
            let guard = term_explain.borrow();
            let idx = *active_explain.borrow();
            if let Some(th) = guard.get(idx) {
                let rows = th.get_rows();
                rows.get(row as usize).cloned().unwrap_or_default()
            } else {
                return;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }

        let prompt = format!(
            "Explain this shell command concisely (2-3 sentences max): {}\n\
             If it's not a command, describe what this output means.",
            trimmed
        );

        let token_rx = bridge_explain.send_message(prompt);

        if let Some(ui) = ui_weak_explain.upgrade() {
            ui.set_terminal_ai_suggestion("Explaining...".into());
            ui.set_terminal_has_suggestion(true);

            let weak = ui_weak_explain.clone();
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

    // ── Terminal area resized (from Slint layout change) ──
    let term_resize = terminals.clone();
    let active_resize = active_tab.clone();
    let last_size = last_area_size.clone();
    ui.on_terminal_area_resized(move |width_px, height_px| {
        let char_w = 7.8f32;
        let line_h = 17.0f32;
        let padding = 8.0f32; // Theme.sp-2

        let cols = ((width_px as f32 - padding * 2.0) / char_w).max(20.0) as u16;
        let rows = ((height_px as f32 - padding * 2.0) / line_h).max(5.0) as u16;

        let mut prev = last_size.borrow_mut();
        if prev.0 == cols as i32 && prev.1 == rows as i32 {
            return;
        }
        *prev = (cols as i32, rows as i32);

        let guard = term_resize.borrow();
        let idx = *active_resize.borrow();
        if let Some(th) = guard.get(idx) {
            th.resize(rows, cols);
            tracing::debug!(rows, cols, "Terminal resized");
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
            *sh = None;
            if let Some(ui) = ui_weak_split.upgrade() {
                ui.set_terminal_split_active(false);
                ui.set_terminal_split_output("".into());
            }
            tracing::info!("Terminal split pane closed");
        } else {
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

    // ── Split pane input ──
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
/// Uses dirty tracking to skip unnecessary UI updates.
/// Converts ColorSegments to Slint model for ANSI color rendering.
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
    let ghost_cooldown: Rc<RefCell<f64>> = Rc::new(RefCell::new(0.0));
    let last_idle_time: Rc<RefCell<f64>> = Rc::new(RefCell::new(0.0));

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(33), move || {
        let guard = term.borrow();
        let idx = *active.borrow();
        let th = match guard.get(idx) {
            Some(th) => th,
            None => return,
        };

        // Skip UI update if nothing changed (dirty tracking)
        let is_dirty = th.is_dirty();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        if let Some(ui) = ui_weak.upgrade() {
            if is_dirty {
                th.clear_dirty();
                *last_idle_time.borrow_mut() = now;

                // Update color segments
                let segs = th.get_segments();
                ui.set_terminal_segments(segments_to_model(&segs));

                // Update plain text fallback
                let text = th.get_full_text();
                ui.set_terminal_output(text.into());

                // Update cursor position
                let (row, col) = th.cursor_position();
                ui.set_terminal_cursor_row(row as i32);
                ui.set_terminal_cursor_col(col as i32);

                // Update alive status
                ui.set_terminal_is_alive(th.is_alive());

                // Update CWD
                let cwd = th.get_cwd();
                if !cwd.is_empty() {
                    ui.set_terminal_current_directory(cwd.into());
                }

                // Sync tab states (alive flags, CWD-based titles)
                sync_tabs_to_ui(&ui, &guard, idx);
            }

            // AI error detection (with cooldown, runs even when not dirty)
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

            // Ghost autocomplete: after 2s idle at prompt, ask LLM for suggestion
            let ghost_cd = *ghost_cooldown.borrow();
            let idle = now - *last_idle_time.borrow();
            if idle > 2.0 && now > ghost_cd && th.is_alive() {
                let current_ghost = ui.get_terminal_ghost_suggestion().to_string();
                if current_ghost.is_empty() {
                    // Check if cursor is at a prompt (heuristic)
                    let rows = th.get_rows();
                    if let Some(last_line) = rows.last() {
                        let trimmed = last_line.trim_end();
                        let at_prompt = trimmed.ends_with('$')
                            || trimmed.ends_with('#')
                            || trimmed.ends_with('>')
                            || trimmed.ends_with("$ ")
                            || trimmed.ends_with("# ");

                        if at_prompt {
                            let context = {
                                let start = rows.len().saturating_sub(10);
                                rows[start..].join("\n")
                            };

                            let prompt = format!(
                                "Based on this terminal session, suggest the most likely next command.\n\
                                 Reply with ONLY the command, nothing else.\n\n\
                                 Terminal context:\n```\n{}\n```",
                                context
                            );

                            let token_rx = bridge.send_message(prompt);
                            let weak = ui_weak.clone();
                            let collected = Rc::new(RefCell::new(String::new()));
                            let collected_inner = collected.clone();
                            let (crow, ccol) = th.cursor_position();

                            let ghost_timer = Timer::default();
                            ghost_timer.start(
                                TimerMode::Repeated,
                                Duration::from_millis(50),
                                move || {
                                    while let Ok(token) = token_rx.try_recv() {
                                        if token == "__DONE__" {
                                            if let Some(ui) = weak.upgrade() {
                                                let cmd = collected_inner.borrow().trim().to_string();
                                                if !cmd.is_empty() && cmd.len() < 200 {
                                                    ui.set_terminal_ghost_suggestion(cmd.into());
                                                    ui.set_terminal_ghost_row(crow as i32);
                                                    ui.set_terminal_ghost_col(ccol as i32);
                                                }
                                            }
                                            return;
                                        }
                                        if token.starts_with("__") && token.ends_with("__") {
                                            continue;
                                        }
                                        collected_inner.borrow_mut().push_str(&token);
                                    }
                                },
                            );
                            std::mem::forget(ghost_timer);

                            *ghost_cooldown.borrow_mut() = now + 8.0; // Don't spam LLM
                        }
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
            if th.is_dirty() {
                th.clear_dirty();
                if let Some(ui) = ui_weak.upgrade() {
                    let text = th.get_full_text();
                    ui.set_terminal_split_output(text.into());
                    let (row, col) = th.cursor_position();
                    ui.set_terminal_split_cursor_row(row as i32);
                    ui.set_terminal_split_cursor_col(col as i32);
                }
            }
        }
    });
    *timer_slot.borrow_mut() = Some(timer);
}
