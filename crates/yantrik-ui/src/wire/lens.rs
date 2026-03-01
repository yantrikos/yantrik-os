//! Intent Lens wiring — query changed, result selected, open/close.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app_context::AppContext;
use crate::{focus, lens, onboarding, App, LensResult};

/// Wire all Lens callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_query(ui, ctx);
    wire_result_selected(ui, ctx);
    wire_open_close(ui);
}

/// Live search: build results as the user types.
fn wire_query(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let apps = ctx.installed_apps.clone();
    let clip = ctx.clip_history.clone();

    ui.on_lens_query(move |text| {
        let query = text.to_string();
        if let Some(ui) = ui_weak.upgrade() {
            if query.is_empty() {
                let onboarding = ui.get_onboarding_step();
                if onboarding > 0 {
                    let guide = onboarding::guide_result(onboarding);
                    ui.set_lens_results(ModelRc::new(VecModel::from(vec![guide])));
                } else {
                    ui.set_lens_results(ModelRc::new(VecModel::<LensResult>::default()));
                }
                ui.set_lens_chat_mode(false);
                return;
            }

            let clip_entries: Vec<(usize, crate::clipboard::ClipEntry)> = clip
                .lock()
                .map(|h| {
                    h.recent(6)
                        .into_iter()
                        .enumerate()
                        .map(|(i, e)| (i, e.clone()))
                        .collect()
                })
                .unwrap_or_default();

            let onboarding = ui.get_onboarding_step();
            let companion_online = ui.get_companion_online();
            let results = lens::build_results(&query, onboarding, &apps, &clip_entries, companion_online);
            ui.set_lens_results(ModelRc::new(VecModel::from(results)));
        }
    });
}

/// Handle a result selection: resolve action, execute, advance onboarding.
fn wire_result_selected(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let apps = ctx.installed_apps.clone();
    let clip = ctx.clip_history.clone();

    ui.on_lens_result_selected(move |action_id| {
        let action = action_id.to_string();
        tracing::info!(action = %action, "Lens result selected");

        match lens::resolve_action(&action, &apps) {
            lens::LensAction::Launch(cmd) => {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                let (bin, args) = match parts.split_first() {
                    Some((b, a)) => (*b, a),
                    None => {
                        tracing::error!("Empty launch command");
                        return;
                    }
                };
                match std::process::Command::new(bin).args(args).spawn() {
                    Ok(_) => tracing::info!(cmd, "App started"),
                    Err(e) => tracing::error!(cmd, error = %e, "Failed to launch"),
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                }
            }
            lens::LensAction::OpenUrl(url) => {
                tracing::info!(%url, "Opening URL from Lens");
                match std::process::Command::new("xdg-open").arg(&url).spawn() {
                    Ok(_) => tracing::info!(%url, "URL opened"),
                    Err(e) => tracing::error!(%url, error = %e, "Failed to open URL"),
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                }
            }
            lens::LensAction::SubmitToAI(query) => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.invoke_lens_submit(SharedString::from(&query));
                }
            }
            lens::LensAction::StartFocus(secs) => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                    focus::start(&ui, secs);
                }
            }
            lens::LensAction::ClipboardPaste(index) => {
                if let Ok(h) = clip.lock() {
                    if let Some(entry) = h.get(index) {
                        let mut child = match std::process::Command::new("wl-copy")
                            .stdin(std::process::Stdio::piped())
                            .spawn()
                        {
                            Ok(c) => c,
                            Err(e) => {
                                tracing::error!(error = %e, "Failed to run wl-copy");
                                return;
                            }
                        };
                        if let Some(stdin) = child.stdin.as_mut() {
                            use std::io::Write;
                            let _ = stdin.write_all(entry.content.as_bytes());
                        }
                        let _ = child.wait();
                        tracing::info!(index, "Clipboard history entry restored");
                    }
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                }
            }
            lens::LensAction::LockScreen => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                    ui.invoke_lock_screen();
                }
            }
            lens::LensAction::OpenSettings => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                    ui.set_current_screen(7);
                    ui.invoke_navigate(7);
                }
            }
            lens::LensAction::OpenFileBrowser => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                    ui.set_current_screen(8);
                    ui.invoke_navigate(8);
                }
            }
            lens::LensAction::CloseLens => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
                }
            }
            lens::LensAction::Noop => {}
        }

        // Advance onboarding steps
        if let Some(ui) = ui_weak.upgrade() {
            let step = ui.get_onboarding_step();
            if step == 1 && action.starts_with("launch:") {
                ui.set_onboarding_step(2);
                tracing::info!(
                    "Onboarding: step 1 complete (launched app), advancing to step 2"
                );
            } else if step == 2 && action.starts_with("setting:focus") {
                ui.set_onboarding_step(0);
                onboarding::write_marker();
                tracing::info!("Onboarding complete — marker written");
            }
        }
    });
}

/// Open/close Lens: reset state on close.
fn wire_open_close(ui: &App) {
    let ui_weak_open = ui.as_weak();
    ui.on_open_lens(move || {
        tracing::debug!("Lens opened");
        let _ = ui_weak_open.upgrade();
    });

    let ui_weak_close = ui.as_weak();
    ui.on_close_lens(move || {
        tracing::debug!("Lens closed");
        if let Some(ui) = ui_weak_close.upgrade() {
            ui.set_lens_results(ModelRc::new(VecModel::<LensResult>::default()));
            ui.set_lens_chat_mode(false);
        }
    });
}
