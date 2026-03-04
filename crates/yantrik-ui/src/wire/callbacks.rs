//! Miscellaneous callbacks — lock, onboarding, focus, file browser,
//! whisper cards, memory search.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::{self, AppContext};
use crate::mime_dispatch::{self, FileAction};
use crate::app_context::FileClipOp;
use crate::{
    bridge, cards, filebrowser, focus, lock, notifications, onboarding, App, BreadcrumbSegment,
    FileDetailData, FileEntry, MemoryItem,
};

/// Wire all miscellaneous callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_lock(ui);
    wire_onboarding(ui);
    wire_focus(ui);
    wire_file_browser(ui, ctx);
    wire_whisper_cards(ui, ctx);
    wire_memory_search(ui, ctx);
    wire_notifications(ui, ctx);
    wire_quick_settings(ui);
}

// ── Lock screen ──

fn wire_lock(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_try_unlock(move |pin| {
        let pin = pin.to_string();
        if lock::check_pin(&pin) {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_current_screen(1);
                ui.set_lock_error("".into());
                tracing::info!("Screen unlocked");
            }
        } else {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_lock_error("Wrong PIN".into());
            }
            tracing::debug!("Unlock failed — wrong PIN");
        }
    });

    let ui_weak_lock = ui.as_weak();
    ui.on_lock_screen(move || {
        if let Some(ui) = ui_weak_lock.upgrade() {
            ui.set_current_screen(3);
            ui.set_lock_error("".into());
            ui.set_lock_date_text(app_context::current_date_text().into());
            ui.set_lock_greeting(ui.get_greeting_text());
            tracing::info!("Screen locked");
        }
    });
}

// ── Onboarding ──

fn wire_onboarding(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_onboarding_ready(move || {
        onboarding::write_marker();
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_lens_open(true);
        }
        tracing::info!("Onboarding complete — marker written, opening Lens");
    });

    ui.on_onboarding_skip(move || {
        onboarding::write_marker();
        tracing::info!("Onboarding skipped");
    });
}

// ── Focus mode ──

fn wire_focus(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_end_focus_mode(move || {
        if let Some(ui) = ui_weak.upgrade() {
            focus::end(&ui);
        }
        tracing::info!("Focus mode ended by user");
    });
}

// ── File browser ──

fn wire_file_browser(ui: &App, ctx: &AppContext) {
    let browser_path = ctx.browser_path.clone();
    let show_hidden = ctx.browser_show_hidden.clone();
    let file_clip = ctx.file_clipboard.clone();
    let history_back = ctx.browser_history_back.clone();
    let history_forward = ctx.browser_history_forward.clone();
    let sort_field = ctx.browser_sort_field.clone();
    let sort_ascending = ctx.browser_sort_ascending.clone();
    let filter_text = ctx.browser_filter.clone();

    // Helper: navigate to a path, push old path to back stack, clear forward stack, update free space
    fn navigate_to(
        ui: &App,
        bp: &Rc<RefCell<String>>,
        sh: &Rc<RefCell<bool>>,
        hb: &Rc<RefCell<Vec<String>>>,
        hf: &Rc<RefCell<Vec<String>>>,
        sf: &Rc<RefCell<String>>,
        sa: &Rc<RefCell<bool>>,
        ft: &Rc<RefCell<String>>,
        path: String,
    ) {
        let old = bp.borrow().clone();
        if old != path {
            hb.borrow_mut().push(old);
            hf.borrow_mut().clear();
        }
        *bp.borrow_mut() = path.clone();
        ui.set_file_browser_path(SharedString::from(&path));
        refresh_entries(ui, &path, *sh.borrow(), &sf.borrow(), *sa.borrow(), &ft.borrow());
        update_nav_state(ui, hb, hf);
        update_free_space(ui, &path);
    }

    // Navigate into a subdirectory
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let hb = history_back.clone();
    let hf = history_forward.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_navigate_dir(move |name| {
        let new_path = {
            let current = bp.borrow();
            filebrowser::child_path(&current, &name.to_string())
        };
        if let Some(ui) = ui_weak.upgrade() {
            navigate_to(&ui, &bp, &sh, &hb, &hf, &sf, &sa, &ft, new_path);
        }
    });

    // Navigate to an absolute/display path (breadcrumb click or sidebar)
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let hb = history_back.clone();
    let hf = history_forward.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_navigate_to_path(move |path| {
        if let Some(ui) = ui_weak.upgrade() {
            navigate_to(&ui, &bp, &sh, &hb, &hf, &sf, &sa, &ft, path.to_string());
        }
    });

    // Go back in navigation history
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let hb = history_back.clone();
    let hf = history_forward.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_go_back(move || {
        let prev = hb.borrow_mut().pop();
        if let Some(prev_path) = prev {
            let current = bp.borrow().clone();
            hf.borrow_mut().push(current);
            *bp.borrow_mut() = prev_path.clone();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_file_browser_path(SharedString::from(&prev_path));
                refresh_entries(&ui, &prev_path, *sh.borrow(), &sf.borrow(), *sa.borrow(), &ft.borrow());
                update_nav_state(&ui, &hb, &hf);
                update_free_space(&ui, &prev_path);
            }
        }
    });

    // Go forward in navigation history
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let hb = history_back.clone();
    let hf = history_forward.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_go_forward(move || {
        let next = hf.borrow_mut().pop();
        if let Some(next_path) = next {
            let current = bp.borrow().clone();
            hb.borrow_mut().push(current);
            *bp.borrow_mut() = next_path.clone();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_file_browser_path(SharedString::from(&next_path));
                refresh_entries(&ui, &next_path, *sh.borrow(), &sf.borrow(), *sa.borrow(), &ft.borrow());
                update_nav_state(&ui, &hb, &hf);
                update_free_space(&ui, &next_path);
            }
        }
    });

    // Open a file — route through mime_dispatch
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let iv_state = ctx.image_viewer_state.clone();
    let ed_path = ctx.editor_file_path.clone();
    let mp_handle = ctx.media_player.clone();
    ui.on_file_open(move |name| {
        let name_str = name.to_string();
        let full = {
            let current = bp.borrow();
            let expanded = filebrowser::expand_home(&current);
            expanded.join(&name_str)
        };
        tracing::info!(path = %full.display(), "Opening file");

        match mime_dispatch::classify(&name_str) {
            FileAction::ImageViewer => {
                iv_state.borrow_mut().open(&full);
                if let Some(ui) = ui_weak.upgrade() {
                    super::image_viewer::load_current_image(&ui, &iv_state.borrow());
                    ui.set_current_screen(11);
                    ui.invoke_navigate(11);
                }
            }
            FileAction::TextEditor => {
                if let Some(ui) = ui_weak.upgrade() {
                    super::text_editor::load_file(&ui, &full, &ed_path);
                    ui.set_current_screen(12);
                    ui.invoke_navigate(12);
                }
            }
            FileAction::AudioPlayer => {
                if let Some(ui) = ui_weak.upgrade() {
                    super::media_player::start_playback(&ui, &full, &mp_handle);
                    ui.set_current_screen(13);
                    ui.invoke_navigate(13);
                }
            }
            FileAction::External(cmd) => {
                let _ = std::process::Command::new(&cmd).arg(&full).spawn();
            }
        }
    });

    // Go up one directory
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let hb = history_back.clone();
    let hf = history_forward.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_go_up(move || {
        let new_path = {
            let current = bp.borrow();
            filebrowser::parent_path(&current)
        };
        if let Some(ui) = ui_weak.upgrade() {
            navigate_to(&ui, &bp, &sh, &hb, &hf, &sf, &sa, &ft, new_path);
        }
    });

    // Toggle hidden files
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_toggle_hidden(move || {
        let new_val = !*sh.borrow();
        *sh.borrow_mut() = new_val;
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_file_show_hidden(new_val);
            let path = bp.borrow().clone();
            refresh_entries(&ui, &path, new_val, &sf.borrow(), *sa.borrow(), &ft.borrow());
        }
    });

    // Delete
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_delete(move |name| {
        let dir = bp.borrow().clone();
        match filebrowser::delete_entry(&dir, &name.to_string()) {
            Ok(()) => tracing::info!(name = %name, "File deleted"),
            Err(e) => tracing::error!(name = %name, error = %e, "Delete failed"),
        }
        if let Some(ui) = ui_weak.upgrade() {
            refresh_entries(&ui, &dir, *sh.borrow(), &sf.borrow(), *sa.borrow(), &ft.borrow());
        }
    });

    // Rename
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_rename(move |old_name, new_name| {
        let dir = bp.borrow().clone();
        match filebrowser::rename_entry(&dir, &old_name.to_string(), &new_name.to_string()) {
            Ok(()) => tracing::info!(old = %old_name, new = %new_name, "File renamed"),
            Err(e) => tracing::error!(old = %old_name, error = %e, "Rename failed"),
        }
        if let Some(ui) = ui_weak.upgrade() {
            refresh_entries(&ui, &dir, *sh.borrow(), &sf.borrow(), *sa.borrow(), &ft.borrow());
        }
    });

    // Copy (stage to file clipboard)
    let bp = browser_path.clone();
    let fc = file_clip.clone();
    let ui_weak = ui.as_weak();
    ui.on_file_copy(move |name| {
        let dir = bp.borrow().clone();
        *fc.borrow_mut() = Some(FileClipOp::Copy {
            src_dir: dir,
            name: name.to_string(),
        });
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_file_has_clipboard(true);
        }
        tracing::info!(name = %name, "File copied to clipboard");
    });

    // Cut (stage to file clipboard)
    let bp = browser_path.clone();
    let fc = file_clip.clone();
    let ui_weak = ui.as_weak();
    ui.on_file_cut(move |name| {
        let dir = bp.borrow().clone();
        *fc.borrow_mut() = Some(FileClipOp::Cut {
            src_dir: dir,
            name: name.to_string(),
        });
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_file_has_clipboard(true);
        }
        tracing::info!(name = %name, "File cut to clipboard");
    });

    // Paste (execute copy/move from file clipboard)
    let bp = browser_path.clone();
    let fc = file_clip.clone();
    let sh = show_hidden.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    let ui_weak = ui.as_weak();
    ui.on_file_paste(move || {
        let dst_dir = bp.borrow().clone();
        let op = fc.borrow().clone();
        match op {
            Some(FileClipOp::Copy { src_dir, name }) => {
                match filebrowser::copy_entry(&src_dir, &name, &dst_dir) {
                    Ok(()) => tracing::info!(name = %name, "File pasted (copy)"),
                    Err(e) => tracing::error!(name = %name, error = %e, "Paste (copy) failed"),
                }
            }
            Some(FileClipOp::Cut { src_dir, name }) => {
                match filebrowser::move_entry(&src_dir, &name, &dst_dir) {
                    Ok(()) => tracing::info!(name = %name, "File pasted (move)"),
                    Err(e) => tracing::error!(name = %name, error = %e, "Paste (move) failed"),
                }
                *fc.borrow_mut() = None;
            }
            None => {}
        }
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_file_has_clipboard(fc.borrow().is_some());
            refresh_entries(&ui, &dst_dir, *sh.borrow(), &sf.borrow(), *sa.borrow(), &ft.borrow());
        }
    });

    // Create folder
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    let ui_weak = ui.as_weak();
    ui.on_file_create_folder(move |name| {
        let dir = bp.borrow().clone();
        match filebrowser::create_folder(&dir, &name.to_string()) {
            Ok(()) => tracing::info!(name = %name, "Folder created"),
            Err(e) => tracing::error!(name = %name, error = %e, "Create folder failed"),
        }
        if let Some(ui) = ui_weak.upgrade() {
            refresh_entries(&ui, &dir, *sh.borrow(), &sf.borrow(), *sa.borrow(), &ft.borrow());
        }
    });

    // File selection changed — load file details + clear AI summary
    let bp = browser_path.clone();
    let ui_weak = ui.as_weak();
    let summary_timer_sel = ctx.summary_timer.clone();
    ui.on_file_selection_changed(move |name| {
        let name = name.to_string();
        if name.is_empty() {
            return;
        }
        let dir = bp.borrow().clone();
        let detail = filebrowser::get_file_details(&dir, &name);
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_file_detail_data(FileDetailData {
                name: detail.name.into(),
                file_type: detail.file_type.into(),
                size_text: detail.size_text.into(),
                modified_text: detail.modified_text.into(),
                path_text: detail.path_text.into(),
                permissions: detail.permissions.into(),
                preview_text: detail.preview_text.into(),
                is_text_file: detail.is_text_file,
                icon_char: detail.icon_char.into(),
            });
            // Clear previous AI summary and cancel any in-progress streaming
            ui.set_file_ai_summary("".into());
            ui.set_file_is_summarizing(false);
            *summary_timer_sel.borrow_mut() = None;
        }
    });

    // Sort changed
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_sort_changed(move |field, ascending| {
        *sf.borrow_mut() = field.to_string();
        *sa.borrow_mut() = ascending;
        if let Some(ui) = ui_weak.upgrade() {
            let path = bp.borrow().clone();
            refresh_entries(&ui, &path, *sh.borrow(), &sf.borrow(), *sa.borrow(), &ft.borrow());
        }
    });

    // Filter changed
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let sh = show_hidden.clone();
    let sf = sort_field.clone();
    let sa = sort_ascending.clone();
    let ft = filter_text.clone();
    ui.on_file_filter_changed(move |text| {
        *ft.borrow_mut() = text.to_string();
        if let Some(ui) = ui_weak.upgrade() {
            let path = bp.borrow().clone();
            refresh_entries(&ui, &path, *sh.borrow(), &sf.borrow(), *sa.borrow(), &ft.borrow());
        }
    });

    // AI Summarize — on-demand summary of selected text file
    let ui_weak = ui.as_weak();
    let bridge = ctx.bridge.clone();
    let summary_timer = ctx.summary_timer.clone();
    ui.on_file_request_summarize(move || {
        let ui = match ui_weak.upgrade() {
            Some(ui) => ui,
            None => return,
        };

        let detail = ui.get_file_detail_data();
        let preview = detail.preview_text.to_string();
        if preview.is_empty() {
            return;
        }

        // Don't start if already summarizing
        if ui.get_file_is_summarizing() {
            return;
        }

        // Check bridge online
        if !bridge.is_online() {
            ui.set_file_ai_summary("AI is offline".into());
            return;
        }

        let file_name = detail.name.to_string();
        let prompt = format!(
            "Here is the content of \"{}\". Summarize what this code/text does in 2-3 sentences. Do NOT say you can't access the file — the content is below:\n\n{}",
            file_name, preview
        );

        ui.set_file_is_summarizing(true);
        ui.set_file_ai_summary("".into());

        let token_rx = bridge.send_message(prompt);
        let weak = ui_weak.clone();
        let timer_handle = summary_timer.clone();
        let start_time = std::time::Instant::now();

        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let mut done = false;
            while let Ok(token) = token_rx.try_recv() {
                if token == "__DONE__" {
                    done = true;
                    break;
                }
                if let Some(ui) = weak.upgrade() {
                    let current = ui.get_file_ai_summary().to_string();
                    let updated = format!("{}{}", current, token);
                    ui.set_file_ai_summary(SharedString::from(&updated));
                }
            }
            // Timeout after 30 seconds if no response
            if !done && start_time.elapsed() > Duration::from_secs(30) {
                if let Some(ui) = weak.upgrade() {
                    if ui.get_file_ai_summary().is_empty() {
                        ui.set_file_ai_summary("AI is busy — try again later.".into());
                    }
                    ui.set_file_is_summarizing(false);
                }
                *timer_handle.borrow_mut() = None;
                return;
            }
            if done {
                if let Some(ui) = weak.upgrade() {
                    ui.set_file_is_summarizing(false);
                }
                *timer_handle.borrow_mut() = None;
            }
        });
        *summary_timer.borrow_mut() = Some(timer);

        tracing::info!(file = %detail.name, "AI file summary requested");
    });

    // Ask AI — send file context to AI and stream response inline
    let ui_weak = ui.as_weak();
    let bp = browser_path.clone();
    let bridge_ask = ctx.bridge.clone();
    let ask_timer = ctx.summary_timer.clone();
    ui.on_file_request_ask_ai(move || {
        let ui = match ui_weak.upgrade() {
            Some(ui) => ui,
            None => return,
        };

        let detail = ui.get_file_detail_data();
        let name = detail.name.to_string();
        let preview = detail.preview_text.to_string();
        let path = detail.path_text.to_string();

        if ui.get_file_is_summarizing() {
            return;
        }

        if !bridge_ask.is_online() {
            ui.set_file_ai_summary("AI is offline".into());
            return;
        }

        let prompt = if name.is_empty() {
            let dir = bp.borrow().clone();
            format!("What kind of project is in {}? List the key files and what they do.", dir)
        } else if preview.is_empty() {
            format!("Tell me about the file '{}' at path {}. What is its likely purpose based on the name and extension?", name, path)
        } else {
            format!(
                "Analyze this file and tell me what it does, any issues you notice, and suggestions:\n\nFile: {}\nPath: {}\n\nContent:\n{}",
                name, path, preview
            )
        };

        ui.set_file_is_summarizing(true);
        ui.set_file_ai_summary("".into());

        let token_rx = bridge_ask.send_message(prompt.clone());
        let weak = ui_weak.clone();
        let timer_handle = ask_timer.clone();
        let start_time = std::time::Instant::now();

        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let mut done = false;
            while let Ok(token) = token_rx.try_recv() {
                if token == "__DONE__" {
                    done = true;
                    break;
                }
                if token.starts_with("__") && token.ends_with("__") {
                    continue;
                }
                if let Some(ui) = weak.upgrade() {
                    let current = ui.get_file_ai_summary().to_string();
                    let updated = format!("{}{}", current, token);
                    ui.set_file_ai_summary(SharedString::from(&updated));
                }
            }
            if !done && start_time.elapsed() > Duration::from_secs(30) {
                if let Some(ui) = weak.upgrade() {
                    if ui.get_file_ai_summary().is_empty() {
                        ui.set_file_ai_summary("AI is busy — try again later.".into());
                    }
                    ui.set_file_is_summarizing(false);
                }
                *timer_handle.borrow_mut() = None;
                return;
            }
            if done {
                if let Some(ui) = weak.upgrade() {
                    ui.set_file_is_summarizing(false);
                }
                *timer_handle.borrow_mut() = None;
            }
        });
        *ask_timer.borrow_mut() = Some(timer);

        tracing::info!(file = %name, "Ask AI from file browser");
    });
}

/// List a directory and push entries + breadcrumbs to the UI.
fn refresh_entries(ui: &App, path: &str, show_hidden: bool, sort_field: &str, sort_ascending: bool, name_filter: &str) {
    let entries = filebrowser::list_dir_full(path, show_hidden, name_filter, sort_field, sort_ascending);
    let items: Vec<FileEntry> = entries
        .into_iter()
        .map(|e| FileEntry {
            name: e.name.into(),
            is_dir: e.is_dir,
            size_text: e.size_text.into(),
            modified_text: e.modified_text.into(),
            icon_char: e.icon_char.into(),
        })
        .collect();
    ui.set_file_browser_entries(ModelRc::new(VecModel::from(items)));

    // Update breadcrumbs
    let segments = filebrowser::breadcrumb_segments(path);
    let crumbs: Vec<BreadcrumbSegment> = segments
        .into_iter()
        .map(|(label, full_path)| BreadcrumbSegment {
            label: label.into(),
            full_path: full_path.into(),
        })
        .collect();
    ui.set_file_breadcrumbs(ModelRc::new(VecModel::from(crumbs)));

    // Detect project type for directory badge
    let badge = filebrowser::detect_project_type(path);
    ui.set_file_dir_type_badge(SharedString::from(badge));
}

/// Update can-go-back / can-go-forward UI properties from history stacks.
fn update_nav_state(
    ui: &App,
    hb: &Rc<RefCell<Vec<String>>>,
    hf: &Rc<RefCell<Vec<String>>>,
) {
    ui.set_file_can_go_back(!hb.borrow().is_empty());
    ui.set_file_can_go_forward(!hf.borrow().is_empty());
}

/// Update the free-space-text property for the current path's filesystem.
fn update_free_space(ui: &App, path: &str) {
    let expanded = filebrowser::expand_home(path);
    let dir = expanded.to_string_lossy().to_string();
    match std::process::Command::new("df")
        .args(["-h", "--output=avail", &dir])
        .output()
    {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            if let Some(avail) = text.lines().nth(1) {
                ui.set_file_free_space_text(
                    SharedString::from(format!("{} free", avail.trim())),
                );
            }
        }
        _ => {}
    }
}

// ── Whisper cards ──

fn wire_whisper_cards(ui: &App, ctx: &AppContext) {
    let card_mgr = ctx.card_manager.clone();
    let bridge = ctx.bridge.clone();

    // Dismiss a whisper card
    let mgr = card_mgr.clone();
    let br = bridge.clone();
    let ui_weak = ui.as_weak();
    ui.on_whisper_card_dismissed(move |id| {
        let id = id.to_string();
        let mut mgr = mgr.borrow_mut();
        if let Some(source) = mgr.dismiss(&id) {
            cards::sync_whisper_ui(&mgr, &ui_weak);
            br.record_system_event(
                format!("Whisper card dismissed: {}", id),
                "whisper-cards".to_string(),
                0.2,
            );
            tracing::debug!(id, source, "Whisper card dismissed");
        }
    });

    // Action on a whisper card (dismiss + open Lens)
    let mgr = card_mgr.clone();
    let br = bridge.clone();
    let ui_weak = ui.as_weak();
    ui.on_whisper_card_action(move |id| {
        let id = id.to_string();
        let mut mgr = mgr.borrow_mut();
        if let Some(source) = mgr.dismiss(&id) {
            cards::sync_whisper_ui(&mgr, &ui_weak);
            br.record_system_event(
                format!("Whisper card acted on: {}", id),
                "whisper-cards".to_string(),
                0.3,
            );
            tracing::debug!(id, source, "Whisper card action");
        }
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_lens_open(true);
        }
    });

    // Whisper hint badge clicked — open Lens
    let ui_weak = ui.as_weak();
    ui.on_whisper_hint_clicked(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_lens_open(true);
        }
    });
}

// ── Notifications ──

fn wire_notifications(ui: &App, ctx: &AppContext) {
    // Clear all notifications
    let store = ctx.notification_store.clone();
    let ui_weak = ui.as_weak();
    ui.on_notification_clear_all(move || {
        store.borrow_mut().clear();
        notifications::sync_to_ui(&store.borrow(), &ui_weak);
        tracing::debug!("Notifications cleared");
    });

    // Mark all as read
    let store = ctx.notification_store.clone();
    let ui_weak = ui.as_weak();
    ui.on_notification_mark_all_read(move || {
        store.borrow_mut().mark_all_read();
        notifications::sync_to_ui(&store.borrow(), &ui_weak);
        tracing::debug!("All notifications marked as read");
    });

    // Tap a notification (mark as read)
    let store = ctx.notification_store.clone();
    let ui_weak = ui.as_weak();
    ui.on_notification_tapped(move |id| {
        if let Ok(id_num) = id.to_string().parse::<u64>() {
            store.borrow_mut().mark_read(id_num);
            notifications::sync_to_ui(&store.borrow(), &ui_weak);
        }
    });

    // Clear all notifications for a specific app group
    let store = ctx.notification_store.clone();
    let ui_weak = ui.as_weak();
    ui.on_notification_clear_group(move |app_name| {
        store.borrow_mut().clear_group(&app_name.to_string());
        notifications::sync_to_ui(&store.borrow(), &ui_weak);
        tracing::debug!(app = %app_name, "Notification group cleared");
    });
}

// ── Quick Settings ──

fn wire_quick_settings(ui: &App) {
    // Toggle WiFi via nmcli
    ui.on_toggle_wifi(move || {
        // Read current state and toggle
        let output = std::process::Command::new("nmcli")
            .args(["radio", "wifi"])
            .output();
        let currently_on = output
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
            .unwrap_or(false);
        let new_state = if currently_on { "off" } else { "on" };
        let _ = std::process::Command::new("nmcli")
            .args(["radio", "wifi", new_state])
            .spawn();
        tracing::info!(new_state, "WiFi toggled");
    });

    // Brightness via brightnessctl
    ui.on_brightness_changed(move |level| {
        let pct = format!("{}%", level);
        let _ = std::process::Command::new("brightnessctl")
            .args(["s", &pct])
            .spawn();
        tracing::debug!(level, "Brightness changed");
    });

    // Volume via amixer
    ui.on_volume_changed(move |level| {
        let pct = format!("{}%", level);
        let _ = std::process::Command::new("amixer")
            .args(["-M", "set", "Master", &pct])
            .spawn();
        tracing::debug!(level, "Volume changed");
    });
}

// ── Memory search ──

fn wire_memory_search(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let ui_weak = ui.as_weak();
    let search_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let timer_inner = search_timer.clone();

    ui.on_search_memories(move |query| {
        let query = query.to_string();
        if query.is_empty() {
            return;
        }

        if let Some(ui) = ui_weak.upgrade() {
            ui.set_is_searching_memories(true);
        }

        let reply_rx = bridge.recall_memories(query);
        let weak = ui_weak.clone();
        let handle = timer_inner.clone();
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            if let Ok(results) = reply_rx.try_recv() {
                if let Some(ui) = weak.upgrade() {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs_f64();

                    let items: Vec<MemoryItem> = results
                        .iter()
                        .map(|r| MemoryItem {
                            rid: r.rid.clone().into(),
                            text: r.text.clone().into(),
                            memory_type: r.memory_type.clone().into(),
                            importance: r.importance as f32,
                            valence: r.valence as f32,
                            score: r.score as f32,
                            time_ago: bridge::format_time_ago(now - r.created_at).into(),
                        })
                        .collect();
                    ui.set_memory_results(ModelRc::new(VecModel::from(items)));
                    ui.set_is_searching_memories(false);
                }
                *handle.borrow_mut() = None;
            }
        });
        *timer_inner.borrow_mut() = Some(timer);
    });
}
