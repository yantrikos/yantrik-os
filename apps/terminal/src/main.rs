//! Yantrik Terminal — standalone app binary.
//!
//! Basic terminal emulator using std::process::Command.
//! PTY support is stubbed out (requires platform-specific libraries).

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-terminal");

    let app = TerminalApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

// ── Wire all callbacks ───────────────────────────────────────────────

fn wire(app: &TerminalApp) {
    let output_buffer: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let cwd: Rc<RefCell<String>> = Rc::new(RefCell::new(
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string()),
    ));

    // Set initial state
    app.set_is_alive(true);
    app.set_current_directory(cwd.borrow().clone().into());
    app.set_tab_count(1);
    app.set_active_tab(0);
    let initial_tab = TerminalTabData {
        title: "Terminal".into(),
        is_active: true,
        is_alive: true,
    };
    app.set_tabs(ModelRc::new(VecModel::from(vec![initial_tab])));

    // Show welcome prompt
    {
        let welcome = format!("Yantrik Terminal v0.1.0\n$ ");
        *output_buffer.borrow_mut() = welcome.clone();
        app.set_terminal_output(welcome.into());
    }

    // Key pressed — simplified: we collect input and run on Enter
    {
        let weak = app.as_weak();
        let buf = output_buffer.clone();
        let cwd_ref = cwd.clone();
        let input_line: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

        app.on_terminal_key_pressed(move |event| {
            let Some(ui) = weak.upgrade() else {
                return slint::private_unstable_api::re_exports::EventResult::Reject;
            };

            let text = event.text.to_string();

            // Enter key
            if text == "\n" || text == "\r" {
                let cmd_str = input_line.borrow().clone();
                *input_line.borrow_mut() = String::new();

                if cmd_str.trim().is_empty() {
                    let mut b = buf.borrow_mut();
                    b.push_str("\n$ ");
                    ui.set_terminal_output(b.clone().into());
                    return slint::private_unstable_api::re_exports::EventResult::Accept;
                }

                // Handle 'cd' specially
                let parts: Vec<&str> = cmd_str.trim().split_whitespace().collect();
                if parts.first() == Some(&"cd") {
                    let target = parts.get(1).unwrap_or(&"~");
                    let target = if *target == "~" {
                        std::env::var("HOME")
                            .or_else(|_| std::env::var("USERPROFILE"))
                            .unwrap_or_else(|_| "/".to_string())
                    } else {
                        let current = cwd_ref.borrow().clone();
                        let p = std::path::Path::new(&current).join(target);
                        p.to_string_lossy().to_string()
                    };
                    if std::path::Path::new(&target).is_dir() {
                        *cwd_ref.borrow_mut() = target.clone();
                        ui.set_current_directory(target.into());
                        let mut b = buf.borrow_mut();
                        b.push_str("\n$ ");
                        ui.set_terminal_output(b.clone().into());
                    } else {
                        let mut b = buf.borrow_mut();
                        b.push_str(&format!("\ncd: no such directory: {}\n$ ", target));
                        ui.set_terminal_output(b.clone().into());
                    }
                    return slint::private_unstable_api::re_exports::EventResult::Accept;
                }

                // Handle 'clear'
                if cmd_str.trim() == "clear" {
                    *buf.borrow_mut() = "$ ".to_string();
                    ui.set_terminal_output("$ ".into());
                    return slint::private_unstable_api::re_exports::EventResult::Accept;
                }

                // Handle 'exit'
                if cmd_str.trim() == "exit" {
                    ui.set_is_alive(false);
                    return slint::private_unstable_api::re_exports::EventResult::Accept;
                }

                // Run command via std::process::Command
                let current_dir = cwd_ref.borrow().clone();
                let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
                let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

                let result = std::process::Command::new(shell)
                    .arg(flag)
                    .arg(&cmd_str)
                    .current_dir(&current_dir)
                    .output();

                let output_text = match result {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let mut combined = String::new();
                        if !stdout.is_empty() {
                            combined.push_str(&stdout);
                        }
                        if !stderr.is_empty() {
                            combined.push_str(&stderr);
                        }
                        if combined.is_empty() {
                            String::new()
                        } else {
                            combined
                        }
                    }
                    Err(e) => format!("Error: {}\n", e),
                };

                let mut b = buf.borrow_mut();
                b.push('\n');
                b.push_str(&output_text);
                if !output_text.ends_with('\n') && !output_text.is_empty() {
                    b.push('\n');
                }
                b.push_str("$ ");
                ui.set_terminal_output(b.clone().into());

                return slint::private_unstable_api::re_exports::EventResult::Accept;
            }

            // Backspace
            if text == "\u{8}" || text == "\u{7f}" {
                let mut line = input_line.borrow_mut();
                if !line.is_empty() {
                    line.pop();
                    let mut b = buf.borrow_mut();
                    b.pop();
                    ui.set_terminal_output(b.clone().into());
                }
                return slint::private_unstable_api::re_exports::EventResult::Accept;
            }

            // Regular character
            if !text.is_empty() && text.chars().all(|c| !c.is_control()) {
                input_line.borrow_mut().push_str(&text);
                let mut b = buf.borrow_mut();
                b.push_str(&text);
                ui.set_terminal_output(b.clone().into());
                return slint::private_unstable_api::re_exports::EventResult::Accept;
            }

            slint::private_unstable_api::re_exports::EventResult::Reject
        });
    }

    // Tab management stubs
    app.on_new_tab(|| { tracing::info!("New tab requested (standalone mode — single tab only)"); });
    app.on_close_tab(|_| { tracing::info!("Close tab requested (standalone mode)"); });
    app.on_switch_tab(|_| {});

    // AI stubs
    app.on_request_ai_help(|| { tracing::info!("AI help requested (standalone mode)"); });
    app.on_dismiss_suggestion(|| {});
    app.on_ai_bar_submit(|_| { tracing::info!("AI bar submit (standalone mode)"); });
    app.on_ai_run_command(|| {});
    app.on_accept_ghost(|| {});

    // Search stubs
    app.on_search_query_changed(|_| {});
    app.on_search_next(|| {});
    app.on_search_prev(|| {});

    // Split pane stubs
    app.on_terminal_split_toggle(|| { tracing::info!("Split toggle (standalone mode)"); });
    app.on_terminal_switch_pane(|_| {});
    app.on_terminal_split_input(|_| {});

    // Profile stubs
    app.on_terminal_set_profile(|_| {});

    // Other stubs
    app.on_restart_terminal(|| { tracing::info!("Restart terminal (standalone mode)"); });
    app.on_danger_proceed(|| {});
    app.on_danger_cancel(|| {});
    app.on_explain_line(|_| {});
    app.on_terminal_area_resized(|_w, _h| {});
}
