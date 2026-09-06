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

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("terminal") else { return };

    let app = TerminalApp::new().unwrap();

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    wire(&app);
    app.run().unwrap();
}

// ── Wire all callbacks ───────────────────────────────────────────────

/// The last thing this terminal ran, and whether it worked.
#[derive(Clone)]
struct LastCommand {
    command: String,
    exit_code: i32,
}

// ── The control surface ──────────────────────────────────────────────
//
// Read-only, and deliberately so. The companion already has `run_command`, which runs on a worker
// thread; this terminal runs commands with a blocking `Command::output()` on the UI thread, so an
// agent-issued command would freeze the window for as long as it took. Publishing a `run` action
// here would be a second, worse path to something we already do properly.
//
// What it *can* do that nothing else can is say what the person at the keyboard is doing — which
// directory they are in, what they last ran, and whether it failed. That is the whole point of
// the ErrorCompanion feature, and until now it had no way to find out.

fn publish_control(
    app: &TerminalApp,
    output: Rc<RefCell<String>>,
    last_command: Rc<RefCell<Option<LastCommand>>>,
) {
    use yantrik_app_runtime::control::{Action, App, View};

    let describe = {
        let weak = app.as_weak();
        let out = output.clone();
        let last = last_command;
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Terminal — closing");
            };
            let cwd = ui.get_current_directory().to_string();
            let last = last.borrow().clone();

            let summary = match &last {
                Some(c) if c.exit_code != 0 => {
                    format!("Terminal — in {cwd}, `{}` failed with {}", c.command, c.exit_code)
                }
                Some(c) => format!("Terminal — in {cwd}, last ran `{}`", c.command),
                None => format!("Terminal — in {cwd}, nothing run yet"),
            };

            // The tail, not the transcript. A long session's scrollback is unbounded, and what
            // anyone wants is what just happened.
            let buffer = out.borrow();
            let tail: String = buffer
                .lines()
                .rev()
                .take(60)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");

            View::new(summary)
                .with("directory", cwd)
                .with("alive", ui.get_is_alive())
                .with(
                    "last_command",
                    match &last {
                        Some(c) => serde_json::json!({
                            "command": c.command,
                            "exit_code": c.exit_code,
                            "failed": c.exit_code != 0,
                        }),
                        None => serde_json::Value::Null,
                    },
                )
                .with("recent_output", tail)
        }
    };

    let weak = app.as_weak();
    let cleared = output;

    App::new("terminal")
        .describe(describe)
        .action(Action::new("clear", "Empty the terminal's scrollback"), move |_| {
            let ui = weak.upgrade().ok_or_else(|| "Terminal window is gone".to_string())?;
            *cleared.borrow_mut() = "$ ".to_string();
            ui.set_terminal_output("$ ".into());
            Ok(serde_json::json!({ "cleared": true }))
        })
        .serve();
}

fn wire(app: &TerminalApp) {
    let output_buffer: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let last_command: Rc<RefCell<Option<LastCommand>>> = Rc::new(RefCell::new(None));
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
        let last_run = last_command.clone();

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
                    Ok(ref output) => {
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
                    Err(ref e) => format!("Error: {}\n", e),
                };

                *last_run.borrow_mut() = Some(LastCommand {
                    command: cmd_str.clone(),
                    // 127 is the shell's own "could not run it", which is the closest honest
                    // answer when the process never started.
                    exit_code: result.as_ref().map(|o| o.status.code().unwrap_or(-1)).unwrap_or(127),
                });

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

    publish_control(app, output_buffer.clone(), last_command.clone());

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
