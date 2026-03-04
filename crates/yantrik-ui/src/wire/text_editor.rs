//! Text Editor wiring — file loading, saving, AI assist, and find/replace.
//!
//! Save: if file has a path, saves directly. If untitled/new,
//! opens an inline Save As dialog for the user to pick location + name.
//! AI assist: summarize, improve, or custom prompt with document context.
//! Find/Replace: case-insensitive search with match navigation and replacement.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, SharedString, Timer, TimerMode};

use crate::app_context::AppContext;
use crate::App;

/// Wire text editor callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let editor_path = ctx.editor_file_path.clone();

    // Save file — if path exists, save directly; otherwise show Save As dialog
    let ui_weak = ui.as_weak();
    let ep = editor_path.clone();
    ui.on_editor_save(move || {
        let path_str = ep.borrow().clone();
        if path_str.is_empty() {
            // No path — show Save As dialog
            if let Some(ui) = ui_weak.upgrade() {
                // Pre-fill with home directory and a default filename
                let home = std::env::var("HOME").unwrap_or_else(|_| "/home/yantrik".into());
                ui.set_editor_save_dir(format!("{}/", home).into());
                ui.set_editor_save_filename("untitled.txt".into());
                ui.set_editor_save_error("".into());
                ui.set_editor_show_save_dialog(true);
            }
            return;
        }
        if let Some(ui) = ui_weak.upgrade() {
            save_to_path(&ui, &path_str);
        }
    });

    // Save As — save to user-specified directory + filename
    let ui_weak = ui.as_weak();
    let ep = editor_path.clone();
    ui.on_editor_save_as(move |dir, filename| {
        let Some(ui) = ui_weak.upgrade() else { return };

        let dir_str = dir.to_string().trim().to_string();
        let name_str = filename.to_string().trim().to_string();

        if name_str.is_empty() {
            ui.set_editor_save_error("Filename cannot be empty".into());
            return;
        }

        // Expand ~ to home
        let expanded_dir = if dir_str.starts_with("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/yantrik".into());
            dir_str.replacen("~", &home, 1)
        } else if dir_str == "~" {
            std::env::var("HOME").unwrap_or_else(|_| "/home/yantrik".into())
        } else {
            dir_str.clone()
        };

        let path = PathBuf::from(&expanded_dir).join(&name_str);

        // Check directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                ui.set_editor_save_error(format!("Directory does not exist: {}", parent.display()).into());
                return;
            }
        }

        // Check not overwriting without warning
        if path.exists() {
            // Just warn — user can dismiss and rename
            ui.set_editor_save_error(format!("File exists — will overwrite: {}", path.display()).into());
            // Still save on second click (error was already shown)
            // Actually, let's just save — the user clicked Save explicitly
        }

        let path_str = path.display().to_string();
        save_to_path(&ui, &path_str);

        // Update editor state with new path
        *ep.borrow_mut() = path_str;
        ui.set_editor_file_name(name_str.into());
        ui.set_editor_show_save_dialog(false);
        ui.set_editor_save_error("".into());
    });

    // Content changed — wired below (after find/replace setup) to include match recomputation

    // AI assist — send prompt with document context, stream response
    let ui_weak = ui.as_weak();
    let bridge = ctx.bridge.clone();
    let ai_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let ai_timer_req = ai_timer.clone();
    ui.on_editor_ai_request(move |prompt| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let prompt_str = prompt.to_string();
        if prompt_str.is_empty() {
            return;
        }

        // Cancel any in-progress AI request
        *ai_timer_req.borrow_mut() = None;

        // Get document content for context
        let content = ui.get_editor_file_content().to_string();
        let file_name = ui.get_editor_file_name().to_string();

        // Truncate content for context (first 2000 chars)
        let context_preview = if content.len() > 2000 {
            format!("{}...", &content[..2000])
        } else {
            content.clone()
        };

        // Build prompt based on action — instruct AI to respond directly without tools
        let preamble = "Respond directly with text only. Do NOT use any tools or file operations. This is an inline editor assist request.";
        let full_prompt = match prompt_str.as_str() {
            "summarize" => format!(
                "{}\n\nSummarize this document concisely (3-5 sentences):\n\nFile: {}\n\n{}",
                preamble, file_name, context_preview
            ),
            "improve" => format!(
                "{}\n\nSuggest improvements for this text. Be concise and actionable:\n\nFile: {}\n\n{}",
                preamble, file_name, context_preview
            ),
            _ => format!(
                "{}\n\nRegarding this document ({}):\n\n{}\n\nUser request: {}",
                preamble, file_name, context_preview, prompt_str
            ),
        };

        ui.set_editor_ai_is_working(true);
        ui.set_editor_ai_response("".into());

        let token_rx = bridge.send_message(full_prompt);
        let weak = ui_weak.clone();
        let timer_handle = ai_timer_req.clone();
        let start_time = std::time::Instant::now();

        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let mut done = false;
            while let Ok(token) = token_rx.try_recv() {
                if token == "__DONE__" {
                    done = true;
                    break;
                }
                // Filter internal markers and tool usage lines
                if token.starts_with("__") && token.ends_with("__") {
                    continue;
                }
                if let Some(ui) = weak.upgrade() {
                    let current = ui.get_editor_ai_response().to_string();
                    let mut updated = format!("{}{}", current, token);
                    // Strip [Using ...] tool markers from accumulated text
                    while let Some(start) = updated.find("[Using ") {
                        if let Some(end) = updated[start..].find("...]") {
                            updated = format!("{}{}", &updated[..start], &updated[start + end + 4..]);
                        } else {
                            break;
                        }
                    }
                    // Strip leading whitespace/newlines from cleaned text
                    let trimmed = updated.trim_start_matches('\n').to_string();
                    ui.set_editor_ai_response(SharedString::from(&trimmed));
                }
            }
            if !done && start_time.elapsed() > Duration::from_secs(30) {
                if let Some(ui) = weak.upgrade() {
                    if ui.get_editor_ai_response().is_empty() {
                        ui.set_editor_ai_response("AI is busy — try again later.".into());
                    }
                    ui.set_editor_ai_is_working(false);
                }
                *timer_handle.borrow_mut() = None;
                return;
            }
            if done {
                if let Some(ui) = weak.upgrade() {
                    ui.set_editor_ai_is_working(false);
                }
                *timer_handle.borrow_mut() = None;
            }
        });
        *ai_timer_req.borrow_mut() = Some(timer);

        tracing::info!(action = %prompt_str, file = %file_name, "Editor AI request");
    });

    // AI insert — append AI response to document content
    let ui_weak = ui.as_weak();
    ui.on_editor_ai_insert(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let response = ui.get_editor_ai_response().to_string();
        if response.is_empty() {
            return;
        }
        let current = ui.get_editor_file_content().to_string();
        let updated = if current.is_empty() {
            response
        } else {
            format!("{}\n\n{}", current, response)
        };
        ui.set_editor_file_content(SharedString::from(&updated));
        ui.set_editor_is_modified(true);
        // Clear AI state after insert
        ui.set_editor_ai_response("".into());
        ui.set_editor_ai_prompt("".into());
        tracing::info!("AI response inserted into document");
    });

    // AI dismiss — close panel and clear state
    let ui_weak = ui.as_weak();
    let ai_timer_dismiss = ai_timer.clone();
    ui.on_editor_ai_dismiss(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        // Cancel any in-progress request
        *ai_timer_dismiss.borrow_mut() = None;
        ui.set_editor_show_ai_panel(false);
        ui.set_editor_ai_response("".into());
        ui.set_editor_ai_prompt("".into());
        ui.set_editor_ai_is_working(false);
    });

    // ── Find & Replace ──

    // Shared state: list of byte-offset positions for each match
    let find_matches: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    // Current match index (0-based internally, displayed as 1-based)
    let find_index: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));

    // Helper: recompute matches for current query in file content (wrapped in Rc for sharing)
    let recompute_matches: Rc<dyn Fn(&App)> = {
        let find_matches = find_matches.clone();
        let find_index = find_index.clone();
        Rc::new(move |ui: &App| {
            let query = ui.get_editor_find_query().to_string();
            let content = ui.get_editor_file_content().to_string();
            let mut matches = Vec::new();

            if !query.is_empty() {
                let query_lower = query.to_lowercase();
                let content_lower = content.to_lowercase();
                let mut start = 0;
                while let Some(pos) = content_lower[start..].find(&query_lower) {
                    matches.push(start + pos);
                    start += pos + query_lower.len();
                }
            }

            let count = matches.len() as i32;
            *find_matches.borrow_mut() = matches;

            // Clamp index
            let idx = if count == 0 {
                0
            } else {
                let cur = *find_index.borrow();
                if cur >= count as usize { 0 } else { cur }
            };
            *find_index.borrow_mut() = idx;

            ui.set_editor_find_match_count(count);
            ui.set_editor_find_current_match(if count > 0 { idx as i32 + 1 } else { 0 });
        })
    };

    // Find query changed — recompute matches
    let ui_weak = ui.as_weak();
    let recompute = recompute_matches.clone();
    ui.on_editor_find_query_changed(move |_query| {
        let Some(ui) = ui_weak.upgrade() else { return };
        recompute(&ui);
    });

    // Find next — advance to next match
    let ui_weak = ui.as_weak();
    let fm = find_matches.clone();
    let fi = find_index.clone();
    ui.on_editor_find_next(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let matches = fm.borrow();
        if matches.is_empty() {
            return;
        }
        let mut idx = *fi.borrow();
        idx = if idx + 1 >= matches.len() { 0 } else { idx + 1 };
        *fi.borrow_mut() = idx;
        ui.set_editor_find_current_match(idx as i32 + 1);
    });

    // Find prev — go to previous match
    let ui_weak = ui.as_weak();
    let fm = find_matches.clone();
    let fi = find_index.clone();
    ui.on_editor_find_prev(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let matches = fm.borrow();
        if matches.is_empty() {
            return;
        }
        let mut idx = *fi.borrow();
        idx = if idx == 0 { matches.len() - 1 } else { idx - 1 };
        *fi.borrow_mut() = idx;
        ui.set_editor_find_current_match(idx as i32 + 1);
    });

    // Replace current — replace the match at current index
    let ui_weak = ui.as_weak();
    let fm = find_matches.clone();
    let fi = find_index.clone();
    let recompute = recompute_matches.clone();
    ui.on_editor_replace_current(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let matches = fm.borrow().clone();
        if matches.is_empty() {
            return;
        }
        let idx = *fi.borrow();
        if idx >= matches.len() {
            return;
        }
        let query = ui.get_editor_find_query().to_string();
        let replacement = ui.get_editor_replace_text().to_string();
        let content = ui.get_editor_file_content().to_string();
        let match_pos = matches[idx];

        // Replace the exact occurrence (preserving original case in the replacement)
        let before = &content[..match_pos];
        let after = &content[match_pos + query.len()..];
        let updated = format!("{}{}{}", before, replacement, after);

        ui.set_editor_file_content(SharedString::from(&updated));
        ui.set_editor_is_modified(true);

        // Recompute matches after replacement
        recompute(&ui);
        tracing::debug!("Find & Replace: replaced occurrence at offset {}", match_pos);
    });

    // Replace all — replace every occurrence
    let ui_weak = ui.as_weak();
    let recompute = recompute_matches.clone();
    ui.on_editor_replace_all(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let query = ui.get_editor_find_query().to_string();
        let replacement = ui.get_editor_replace_text().to_string();
        if query.is_empty() {
            return;
        }
        let content = ui.get_editor_file_content().to_string();

        // Case-insensitive replace all: rebuild string from match positions
        let query_lower = query.to_lowercase();
        let content_lower = content.to_lowercase();
        let mut result = String::with_capacity(content.len());
        let mut last_end = 0;
        let mut count = 0;

        let mut start = 0;
        while let Some(pos) = content_lower[start..].find(&query_lower) {
            let abs_pos = start + pos;
            result.push_str(&content[last_end..abs_pos]);
            result.push_str(&replacement);
            last_end = abs_pos + query.len();
            start = abs_pos + query_lower.len();
            count += 1;
        }
        result.push_str(&content[last_end..]);

        if count > 0 {
            ui.set_editor_file_content(SharedString::from(&result));
            ui.set_editor_is_modified(true);
            tracing::info!(count, "Find & Replace: replaced all occurrences");
        }

        // Recompute (should be 0 matches now if replacement differs from query)
        recompute(&ui);
    });

    // Content changed — mark as modified + recompute find matches if bar is open
    let ui_weak = ui.as_weak();
    let recompute_on_edit = recompute_matches.clone();
    ui.on_editor_content_changed(move |_text: SharedString| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_editor_is_modified(true);
            if ui.get_editor_show_find_bar() {
                recompute_on_edit(&ui);
            }
        }
    });
}

/// Save the editor content to the given path.
fn save_to_path(ui: &App, path_str: &str) {
    let content = ui.get_editor_file_content().to_string();
    let path = PathBuf::from(path_str);
    match std::fs::write(&path, &content) {
        Ok(()) => {
            tracing::info!(path = %path.display(), "File saved");
            ui.set_editor_is_modified(false);
        }
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "Failed to save file");
            ui.set_editor_save_error(format!("Save failed: {}", e).into());
        }
    }
}

/// Load a file into the editor. Call this when navigating to screen 12.
pub fn load_file(ui: &App, path: &PathBuf, editor_path: &Rc<RefCell<String>>) {
    let path_str = path.display().to_string();
    *editor_path.borrow_mut() = path_str;

    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    ui.set_editor_file_name(name.into());
    ui.set_editor_is_modified(false);
    ui.set_editor_show_save_dialog(false);

    match std::fs::read_to_string(path) {
        Ok(content) => {
            ui.set_editor_file_content(content.into());
            ui.set_editor_is_readonly(false);
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Failed to read file for editor");
            ui.set_editor_file_content(format!("Error reading file: {}", e).into());
            ui.set_editor_is_readonly(true);
        }
    }
}
