//! Intent Lens wiring — query changed, result selected, open/close.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::bridge::MemoryResult;
use crate::{focus, lens, onboarding, App, LensResult};

/// Look up the selected result from the current UI results by action_id.
fn find_selected_result(ui: &App, action_id: &str) -> Option<(String, String, String)> {
    let results = ui.get_lens_results();
    for i in 0..results.row_count() {
        if let Some(r) = results.row_data(i) {
            if r.action_id.as_str() == action_id {
                return Some((
                    r.title.to_string(),
                    r.result_type.to_string(),
                    r.icon_char.to_string(),
                ));
            }
        }
    }
    None
}

/// Wire all Lens callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_query(ui, ctx);
    wire_result_selected(ui, ctx);
    wire_open_close(ui, ctx);
}

/// Live search: build results as the user types.
/// Integrates instant answers (Phase 4) at position 0 when applicable.
fn wire_query(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let apps = ctx.installed_apps.clone();
    let clip = ctx.clip_history.clone();
    let snapshot = ctx.system_snapshot.clone();
    let frecency = ctx.frecency.clone();
    let bridge = ctx.bridge.clone();

    // Shared state for async memory search
    let memory_rx: Rc<RefCell<Option<Receiver<Vec<MemoryResult>>>>> = Rc::new(RefCell::new(None));
    let memory_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));

    // Clone for the poll timer closure
    let ui_weak_poll = ui.as_weak();
    let memory_rx_poll = memory_rx.clone();
    let memory_timer_poll = memory_timer.clone();

    ui.on_lens_query(move |text| {
        let query = text.to_string();

        // Cancel any pending async memory search
        *memory_timer.borrow_mut() = None;

        if let Some(ui) = ui_weak.upgrade() {
            if query.is_empty() {
                *memory_rx.borrow_mut() = None;
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

            let mut results = Vec::new();

            // Try instant answer first (math, time, battery, date)
            let snap = snapshot.borrow();
            if let Some(answer) = lens::instant_answer(&query, &snap) {
                results.push(answer);
            }

            // Standard results
            results.extend(lens::build_results(&query, onboarding, &apps, &clip_entries, companion_online));

            // Smart ranking: apply frecency + context scoring
            let frecency_ref = frecency.borrow();
            lens::apply_smart_ranking(&mut results, &query, &frecency_ref, &snap.running_processes);
            drop(frecency_ref);
            drop(snap);

            ui.set_lens_results(ModelRc::new(VecModel::from(results)));

            // Fire async memory search for queries > 3 chars when companion is online
            if query.len() > 3 && companion_online {
                let rx = bridge.recall_memories(query);
                *memory_rx.borrow_mut() = Some(rx);

                // Start poll timer to check for memory results
                let timer = Timer::default();
                let ui_poll = ui_weak_poll.clone();
                let rx_poll = memory_rx_poll.clone();
                let t_poll = memory_timer_poll.clone();

                timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
                    let rx_ref = rx_poll.borrow();
                    if let Some(ref rx) = *rx_ref {
                        if let Ok(hits) = rx.try_recv() {
                            drop(rx_ref);
                            // Clear the receiver
                            *rx_poll.borrow_mut() = None;
                            // Stop the timer
                            *t_poll.borrow_mut() = None;

                            if hits.is_empty() {
                                return;
                            }

                            // Merge memory results into current results
                            if let Some(ui) = ui_poll.upgrade() {

                                let memory_results = memory_hits_to_lens(&hits);
                                // Append memory results to existing results
                                let existing = ui.get_lens_results();
                                let mut all: Vec<LensResult> = (0..existing.row_count())
                                    .filter_map(|i| existing.row_data(i))
                                    .collect();

                                if !memory_results.is_empty() {
                                    all.push(lens::lr_divider("── MEMORY ──"));
                                    all.extend(memory_results);
                                }
                                ui.set_lens_results(ModelRc::new(VecModel::from(all)));
                            }
                        }
                    }
                });

                *memory_timer.borrow_mut() = Some(timer);
            }
        }
    });
}

/// Convert bridge MemoryResult hits into LensResult items.
/// If a memory hit contains a recognizable file path, make it directly openable.
fn memory_hits_to_lens(hits: &[MemoryResult]) -> Vec<LensResult> {
    hits.iter()
        .take(5)
        .map(|hit| {
            let preview: String = hit.text.chars().take(80).collect();
            // Check if this memory references a file path — make it actionable
            let (icon, action, rtype) = if let Some(path) = extract_file_path(&hit.text) {
                ("F", format!("exec:xdg-open {}", path), "find")
            } else {
                ("🧠", format!("memory:{}", hit.text.chars().take(200).collect::<String>()), "memory")
            };
            LensResult {
                result_type: rtype.into(),
                title: SharedString::from(preview),
                subtitle: SharedString::from(format!("{} • relevance {:.0}%", hit.memory_type, hit.score * 100.0)),
                icon_char: icon.into(),
                action_id: SharedString::from(action),
                score: hit.score as f32,
                is_loading: false,
                inline_value: SharedString::default(),
            }
        })
        .collect()
}

/// Extract a file path from memory text if present.
/// Looks for patterns like /home/.../file.ext or ~/Documents/file.ext.
fn extract_file_path(text: &str) -> Option<String> {
    for word in text.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| c == '\'' || c == '"' || c == ',' || c == ':');
        if (trimmed.starts_with('/') || trimmed.starts_with("~/"))
            && trimmed.len() > 3
            && trimmed.contains('.')
            && !trimmed.contains("://")
        {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Handle a result selection: resolve action, execute, advance onboarding.
fn wire_result_selected(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let apps = ctx.installed_apps.clone();
    let clip = ctx.clip_history.clone();
    let frecency = ctx.frecency.clone();

    ui.on_lens_result_selected(move |action_id| {
        let action = action_id.to_string();
        tracing::info!(action = %action, "Lens result selected");

        // Record frecency for this action
        if let Some(ui) = ui_weak.upgrade() {
            if let Some((title, result_type, icon_char)) = find_selected_result(&ui, &action) {
                frecency.borrow_mut().record(&action, &title, &result_type, &icon_char);
            }
        }

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
            lens::LensAction::CopyToClipboard(text) => {
                // Copy calculator result to clipboard via wl-copy
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
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                tracing::info!("Calculator result copied to clipboard");
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_lens_open(false);
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

/// Open/close Lens: populate context suggestions on open, reset on close.
fn wire_open_close(ui: &App, ctx: &AppContext) {
    let ui_weak_open = ui.as_weak();
    let snapshot = ctx.system_snapshot.clone();
    let notification_store = ctx.notification_store.clone();
    let frecency = ctx.frecency.clone();

    ui.on_open_lens(move || {
        tracing::debug!("Lens opened");
        if let Some(ui) = ui_weak_open.upgrade() {
            // Restore chat mode if there are existing messages
            let messages = ui.get_messages();
            let msg_model = messages
                .as_any()
                .downcast_ref::<slint::VecModel<crate::MessageData>>();
            if let Some(model) = msg_model {
                if model.row_count() > 0 {
                    ui.set_lens_chat_mode(true);
                }
            }

            // Build contextual suggestions from current system state
            let snap = snapshot.borrow();
            let unread = notification_store.borrow().unread_count();
            let companion_online = ui.get_companion_online();
            let suggestions = lens::build_context_suggestions(
                &snap,
                unread,
                companion_online,
                &snap.running_processes,
            );
            ui.set_lens_suggestions(ModelRc::new(VecModel::from(suggestions)));

            // Build recent actions from frecency store
            let frecency_ref = frecency.borrow();
            let frecency_entries = frecency_ref.top_n(5);
            let recents = lens::frecency_to_recents(&frecency_entries);
            drop(frecency_ref);
            ui.set_lens_recents(ModelRc::new(VecModel::from(recents)));
        }
    });

    let ui_weak_close = ui.as_weak();
    ui.on_close_lens(move || {
        tracing::debug!("Lens closed");
        if let Some(ui) = ui_weak_close.upgrade() {
            ui.set_lens_results(ModelRc::new(VecModel::<LensResult>::default()));
            ui.set_lens_suggestions(ModelRc::new(VecModel::<LensResult>::default()));
            ui.set_lens_recents(ModelRc::new(VecModel::<LensResult>::default()));
            ui.set_lens_chat_mode(false);
        }
    });
}
