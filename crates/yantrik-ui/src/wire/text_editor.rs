//! Text Editor wiring — multi-tab editing, line numbers, file type detection,
//! autosave, go-to-line, AI assist, and find/replace.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::App;

/// Per-tab state held in Rust.
pub struct TabState {
    title: String,
    path: String,
    content: String,
    is_modified: bool,
    is_readonly: bool,
    /// Byte offset positions for find matches (recomputed on query/content change).
    find_matches: Vec<usize>,
    find_index: usize,
    /// File size in bytes (0 for untitled/new files).
    file_size: u64,
}

impl TabState {
    fn new_untitled() -> Self {
        Self {
            title: "untitled".into(),
            path: String::new(),
            content: String::new(),
            is_modified: false,
            is_readonly: false,
            find_matches: Vec::new(),
            find_index: 0,
            file_size: 0,
        }
    }

    fn from_file(path: &PathBuf) -> Self {
        let title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let path_str = path.display().to_string();
        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let (content, is_readonly) = match std::fs::read_to_string(path) {
            Ok(c) => (c, false),
            Err(e) => (format!("Error reading file: {}", e), true),
        };
        Self {
            title,
            path: path_str,
            content,
            is_modified: false,
            is_readonly,
            find_matches: Vec::new(),
            find_index: 0,
            file_size,
        }
    }
}

/// Detect file type from extension.
fn detect_file_type(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "rs" => "Rust",
        "py" => "Python",
        "js" => "JavaScript",
        "ts" => "TypeScript",
        "tsx" => "TypeScript React",
        "jsx" => "JavaScript React",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "scss" | "sass" => "SCSS",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "xml" => "XML",
        "md" | "markdown" => "Markdown",
        "sh" | "bash" | "zsh" => "Shell",
        "c" => "C",
        "cpp" | "cc" | "cxx" => "C++",
        "h" | "hpp" => "C/C++ Header",
        "java" => "Java",
        "go" => "Go",
        "rb" => "Ruby",
        "php" => "PHP",
        "swift" => "Swift",
        "kt" | "kts" => "Kotlin",
        "sql" => "SQL",
        "lua" => "Lua",
        "r" => "R",
        "dart" => "Dart",
        "slint" => "Slint",
        "csv" => "CSV",
        "txt" => "Plain Text",
        "log" => "Log",
        "conf" | "cfg" | "ini" => "Config",
        "dockerfile" => "Dockerfile",
        "makefile" => "Makefile",
        _ => "Plain Text",
    }
}

/// Format file size into a human-readable string.
fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1_048_576 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1_073_741_824 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    }
}

/// Detect line ending style from content.
fn detect_line_ending(content: &str) -> &'static str {
    if content.contains("\r\n") {
        "CRLF"
    } else {
        "LF"
    }
}

/// Generate line numbers text for the gutter.
fn make_line_numbers(total: usize) -> String {
    let width = total.to_string().len().max(3);
    let mut s = String::with_capacity(total * (width + 1));
    for i in 1..=total {
        if i > 1 {
            s.push('\n');
        }
        let num = format!("{:>width$}", i, width = width);
        s.push_str(&num);
    }
    s
}

/// Wire text editor callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let tabs: Rc<RefCell<Vec<TabState>>> = Rc::new(RefCell::new(vec![TabState::new_untitled()]));
    let active: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));

    // Store tabs handle in ctx for load_file access
    *ctx.editor_tabs.borrow_mut() = Some(tabs.clone());
    *ctx.editor_active_tab.borrow_mut() = Some(active.clone());

    // Initial sync
    sync_tabs_to_ui(ui, &tabs.borrow(), *active.borrow());
    sync_active_tab(ui, &tabs.borrow(), *active.borrow());

    // ── Tab management ──

    let ui_weak = ui.as_weak();
    let tabs_sw = tabs.clone();
    let active_sw = active.clone();
    ui.on_editor_switch_tab(move |idx| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let idx = idx as usize;
        let mut tabs = tabs_sw.borrow_mut();
        let mut act = active_sw.borrow_mut();

        // Save current tab's content before switching
        let current = *act;
        if current < tabs.len() {
            tabs[current].content = ui.get_editor_file_content().to_string();
            tabs[current].is_modified = ui.get_editor_is_modified();
        }

        if idx < tabs.len() {
            *act = idx;
            sync_tabs_to_ui(&ui, &tabs, idx);
            sync_active_tab(&ui, &tabs, idx);
        }
    });

    let ui_weak = ui.as_weak();
    let tabs_nt = tabs.clone();
    let active_nt = active.clone();
    ui.on_editor_new_tab(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let mut tabs = tabs_nt.borrow_mut();
        let mut act = active_nt.borrow_mut();

        // Save current tab before switching
        let current = *act;
        if current < tabs.len() {
            tabs[current].content = ui.get_editor_file_content().to_string();
            tabs[current].is_modified = ui.get_editor_is_modified();
        }

        if tabs.len() >= 12 {
            return;
        }
        tabs.push(TabState::new_untitled());
        let new_idx = tabs.len() - 1;
        *act = new_idx;
        sync_tabs_to_ui(&ui, &tabs, new_idx);
        sync_active_tab(&ui, &tabs, new_idx);
    });

    let ui_weak = ui.as_weak();
    let tabs_ct = tabs.clone();
    let active_ct = active.clone();
    ui.on_editor_close_tab(move |idx| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let idx = idx as usize;
        let mut tabs = tabs_ct.borrow_mut();
        let mut act = active_ct.borrow_mut();

        if tabs.len() <= 1 {
            // Don't close the last tab — reset it to untitled
            tabs[0] = TabState::new_untitled();
            *act = 0;
            sync_tabs_to_ui(&ui, &tabs, 0);
            sync_active_tab(&ui, &tabs, 0);
            return;
        }

        if idx >= tabs.len() {
            return;
        }
        tabs.remove(idx);
        // Adjust active index
        if *act >= tabs.len() {
            *act = tabs.len() - 1;
        } else if *act > idx {
            *act -= 1;
        } else if *act == idx && *act >= tabs.len() {
            *act = tabs.len() - 1;
        }
        let new_active = *act;
        sync_tabs_to_ui(&ui, &tabs, new_active);
        sync_active_tab(&ui, &tabs, new_active);
    });

    // ── Save ──
    let ui_weak = ui.as_weak();
    let tabs_save = tabs.clone();
    let active_save = active.clone();
    ui.on_editor_save(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let mut tabs = tabs_save.borrow_mut();
        let act = *active_save.borrow();
        if act >= tabs.len() { return; }

        // Sync content from UI
        tabs[act].content = ui.get_editor_file_content().to_string();

        if tabs[act].path.is_empty() {
            // No path — show Save As dialog
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/yantrik".into());
            ui.set_editor_save_dir(format!("{}/", home).into());
            ui.set_editor_save_filename("untitled.txt".into());
            ui.set_editor_save_error("".into());
            ui.set_editor_show_save_dialog(true);
            return;
        }

        save_to_path(&ui, &tabs[act].path);
        tabs[act].is_modified = false;
        let act_idx = act;
        sync_tabs_to_ui(&ui, &tabs, act_idx);
    });

    // ── Save As ──
    let ui_weak = ui.as_weak();
    let tabs_sa = tabs.clone();
    let active_sa = active.clone();
    ui.on_editor_save_as(move |dir, filename| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let dir_str = dir.to_string().trim().to_string();
        let name_str = filename.to_string().trim().to_string();

        if name_str.is_empty() {
            ui.set_editor_save_error("Filename cannot be empty".into());
            return;
        }

        let expanded_dir = if dir_str.starts_with("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/yantrik".into());
            dir_str.replacen("~", &home, 1)
        } else if dir_str == "~" {
            std::env::var("HOME").unwrap_or_else(|_| "/home/yantrik".into())
        } else {
            dir_str
        };

        let path = PathBuf::from(&expanded_dir).join(&name_str);
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                ui.set_editor_save_error(format!("Directory does not exist: {}", parent.display()).into());
                return;
            }
        }

        let path_str = path.display().to_string();
        save_to_path(&ui, &path_str);

        let mut tabs = tabs_sa.borrow_mut();
        let act = *active_sa.borrow();
        if act < tabs.len() {
            tabs[act].path = path_str;
            tabs[act].title = name_str.clone();
            tabs[act].is_modified = false;
        }

        ui.set_editor_file_name(name_str.into());
        ui.set_editor_show_save_dialog(false);
        ui.set_editor_save_error("".into());

        let act_idx = act;
        sync_tabs_to_ui(&ui, &tabs, act_idx);
        // Update file type
        if act < tabs.len() {
            let ft = detect_file_type(&tabs[act].title);
            ui.set_editor_file_type(ft.into());
        }
    });

    // ── Content changed — mark modified + update line info ──
    let ui_weak = ui.as_weak();
    let tabs_cc = tabs.clone();
    let active_cc = active.clone();
    let find_recompute_flag: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let fr_flag = find_recompute_flag.clone();
    ui.on_editor_content_changed(move |text| {
        let Some(ui) = ui_weak.upgrade() else { return };
        ui.set_editor_is_modified(true);

        let content = text.to_string();
        let total_lines = content.lines().count().max(1);
        ui.set_editor_total_lines(total_lines as i32);
        ui.set_editor_line_numbers_text(make_line_numbers(total_lines).into());
        ui.set_editor_line_ending(detect_line_ending(&content).into());

        // Mark tab modified
        let mut tabs = tabs_cc.borrow_mut();
        let act = *active_cc.borrow();
        if act < tabs.len() {
            tabs[act].is_modified = true;
            tabs[act].content = content;
        }

        // Flag for find recompute
        *fr_flag.borrow_mut() = true;
    });

    // ── Go-to-line ──
    let ui_weak = ui.as_weak();
    ui.on_editor_goto_line(move |line| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let content = ui.get_editor_file_content().to_string();
        let total = content.lines().count().max(1);
        let target = (line as usize).clamp(1, total);

        // Update cursor position display
        ui.set_editor_cursor_line(target as i32);
        ui.set_editor_cursor_col(1);
        tracing::info!(line = target, "Go to line");
    });

    // ── AI assist — send prompt with document context, stream response ──
    let ui_weak = ui.as_weak();
    let bridge = ctx.bridge.clone();
    let ai_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    let ai_timer_req = ai_timer.clone();
    ui.on_editor_ai_request(move |prompt| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let prompt_str = prompt.to_string();
        if prompt_str.is_empty() { return; }

        *ai_timer_req.borrow_mut() = None;

        let content = ui.get_editor_file_content().to_string();
        let file_name = ui.get_editor_file_name().to_string();

        let context_preview = if content.len() > 2000 {
            format!("{}...", &content[..2000])
        } else {
            content
        };

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
                if token.starts_with("__") && token.ends_with("__") {
                    continue;
                }
                if let Some(ui) = weak.upgrade() {
                    let current = ui.get_editor_ai_response().to_string();
                    let mut updated = format!("{}{}", current, token);
                    while let Some(start) = updated.find("[Using ") {
                        if let Some(end) = updated[start..].find("...]") {
                            updated = format!("{}{}", &updated[..start], &updated[start + end + 4..]);
                        } else {
                            break;
                        }
                    }
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

    // AI insert
    let ui_weak = ui.as_weak();
    ui.on_editor_ai_insert(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let response = ui.get_editor_ai_response().to_string();
        if response.is_empty() { return; }
        let current = ui.get_editor_file_content().to_string();
        let updated = if current.is_empty() { response } else { format!("{}\n\n{}", current, response) };
        ui.set_editor_file_content(SharedString::from(&updated));
        ui.set_editor_is_modified(true);
        ui.set_editor_ai_response("".into());
        ui.set_editor_ai_prompt("".into());
    });

    // AI dismiss
    let ui_weak = ui.as_weak();
    let ai_timer_dismiss = ai_timer.clone();
    ui.on_editor_ai_dismiss(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        *ai_timer_dismiss.borrow_mut() = None;
        ui.set_editor_show_ai_panel(false);
        ui.set_editor_ai_response("".into());
        ui.set_editor_ai_prompt("".into());
        ui.set_editor_ai_is_working(false);
    });

    // ── Find & Replace ──

    let find_matches: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let find_index: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));

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

    let ui_weak = ui.as_weak();
    let recompute = recompute_matches.clone();
    ui.on_editor_find_query_changed(move |_| {
        if let Some(ui) = ui_weak.upgrade() {
            recompute(&ui);
        }
    });

    let ui_weak = ui.as_weak();
    let fm = find_matches.clone();
    let fi = find_index.clone();
    ui.on_editor_find_next(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let matches = fm.borrow();
        if matches.is_empty() { return; }
        let mut idx = *fi.borrow();
        idx = if idx + 1 >= matches.len() { 0 } else { idx + 1 };
        *fi.borrow_mut() = idx;
        ui.set_editor_find_current_match(idx as i32 + 1);
    });

    let ui_weak = ui.as_weak();
    let fm = find_matches.clone();
    let fi = find_index.clone();
    ui.on_editor_find_prev(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let matches = fm.borrow();
        if matches.is_empty() { return; }
        let mut idx = *fi.borrow();
        idx = if idx == 0 { matches.len() - 1 } else { idx - 1 };
        *fi.borrow_mut() = idx;
        ui.set_editor_find_current_match(idx as i32 + 1);
    });

    let ui_weak = ui.as_weak();
    let fm = find_matches.clone();
    let fi = find_index.clone();
    let recompute = recompute_matches.clone();
    ui.on_editor_replace_current(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let matches = fm.borrow().clone();
        if matches.is_empty() { return; }
        let idx = *fi.borrow();
        if idx >= matches.len() { return; }
        let query = ui.get_editor_find_query().to_string();
        let replacement = ui.get_editor_replace_text().to_string();
        let content = ui.get_editor_file_content().to_string();
        let match_pos = matches[idx];

        let before = &content[..match_pos];
        let after = &content[match_pos + query.len()..];
        let updated = format!("{}{}{}", before, replacement, after);

        ui.set_editor_file_content(SharedString::from(&updated));
        ui.set_editor_is_modified(true);
        recompute(&ui);
    });

    let ui_weak = ui.as_weak();
    let recompute = recompute_matches.clone();
    ui.on_editor_replace_all(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let query = ui.get_editor_find_query().to_string();
        let replacement = ui.get_editor_replace_text().to_string();
        if query.is_empty() { return; }
        let content = ui.get_editor_file_content().to_string();

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
            tracing::info!(count, "Replaced all occurrences");
        }
        recompute(&ui);
    });

    // ── Encoding picker ──
    let ui_weak = ui.as_weak();
    ui.on_editor_set_encoding(move |enc| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let enc_str = enc.to_string();
        ui.set_editor_encoding(enc.clone());
        tracing::info!(encoding = %enc_str, "Editor encoding set");
    });

    // ── Line ending picker ──
    let ui_weak = ui.as_weak();
    ui.on_editor_set_line_ending(move |le| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let le_str = le.to_string();
        let content = ui.get_editor_file_content().to_string();

        // Convert line endings in the content
        let converted = match le_str.as_str() {
            "LF" => content.replace("\r\n", "\n").replace('\r', "\n"),
            "CRLF" => {
                // First normalize to LF, then convert to CRLF
                let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
                normalized.replace('\n', "\r\n")
            }
            "CR" => content.replace("\r\n", "\r").replace('\n', "\r"),
            _ => content,
        };

        ui.set_editor_file_content(SharedString::from(&converted));
        ui.set_editor_line_ending(le.clone());
        ui.set_editor_is_modified(true);
        tracing::info!(line_ending = %le_str, "Editor line ending set");
    });

    // ── Minimap toggle ──
    let ui_weak = ui.as_weak();
    ui.on_editor_toggle_minimap(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let visible = ui.get_editor_minimap_visible();
        ui.set_editor_minimap_visible(!visible);
        tracing::info!(visible = !visible, "Editor minimap toggled");
    });

    // ── Autosave timer (every 30s, save if modified and has path) ──
    let ui_weak = ui.as_weak();
    let tabs_auto = tabs.clone();
    let active_auto = active.clone();
    let autosave_timer = Timer::default();
    autosave_timer.start(TimerMode::Repeated, Duration::from_secs(30), move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let mut tabs = tabs_auto.borrow_mut();
        let act = *active_auto.borrow();
        if act >= tabs.len() { return; }

        // Sync current content
        tabs[act].content = ui.get_editor_file_content().to_string();
        tabs[act].is_modified = ui.get_editor_is_modified();

        if tabs[act].is_modified && !tabs[act].path.is_empty() && !tabs[act].is_readonly {
            save_to_path(&ui, &tabs[act].path);
            tabs[act].is_modified = false;

            // Update saved time display
            ui.set_editor_last_saved_time(crate::app_context::current_time_hhmm().into());

            sync_tabs_to_ui(&ui, &tabs, act);
            tracing::debug!("Autosaved tab {}", act);
        }
    });
    // Keep timer alive
    std::mem::forget(autosave_timer);
}

/// Sync tab list model to UI.
fn sync_tabs_to_ui(ui: &App, tabs: &[TabState], active: usize) {
    let tab_data: Vec<crate::EditorTabData> = tabs
        .iter()
        .enumerate()
        .map(|(i, t)| crate::EditorTabData {
            title: t.title.clone().into(),
            is_active: i == active,
            is_modified: t.is_modified,
            path: t.path.clone().into(),
        })
        .collect();

    ui.set_editor_tabs(ModelRc::new(VecModel::from(tab_data)));
    ui.set_editor_tab_count(tabs.len() as i32);
    ui.set_editor_active_tab(active as i32);
}

/// Sync the active tab's content/metadata to the UI.
fn sync_active_tab(ui: &App, tabs: &[TabState], active: usize) {
    if active >= tabs.len() { return; }
    let tab = &tabs[active];

    ui.set_editor_file_name(tab.title.clone().into());
    ui.set_editor_file_content(tab.content.clone().into());
    ui.set_editor_is_modified(tab.is_modified);
    ui.set_editor_is_readonly(tab.is_readonly);

    // Line info
    let total_lines = tab.content.lines().count().max(1);
    ui.set_editor_total_lines(total_lines as i32);
    ui.set_editor_line_numbers_text(make_line_numbers(total_lines).into());
    ui.set_editor_cursor_line(1);
    ui.set_editor_cursor_col(1);

    // File type + encoding
    ui.set_editor_file_type(detect_file_type(&tab.title).into());
    ui.set_editor_encoding("UTF-8".into());
    ui.set_editor_line_ending(detect_line_ending(&tab.content).into());

    // File size display + large file warning
    if tab.file_size > 0 {
        ui.set_editor_file_size_text(format_file_size(tab.file_size).into());
        ui.set_editor_large_file_warning(tab.file_size > 1_048_576); // > 1MB
    } else {
        ui.set_editor_file_size_text("".into());
        ui.set_editor_large_file_warning(false);
    }
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

/// Load a file into the editor — opens in a new tab or switches to existing.
pub fn load_file(
    ui: &App,
    path: &PathBuf,
    tabs_handle: &Rc<RefCell<Option<Rc<RefCell<Vec<TabState>>>>>>,
    active_handle: &Rc<RefCell<Option<Rc<RefCell<usize>>>>>,
) {
    let tabs_opt = tabs_handle.borrow();
    let active_opt = active_handle.borrow();

    if let (Some(tabs_rc), Some(active_rc)) = (tabs_opt.as_ref(), active_opt.as_ref()) {
        let mut tabs = tabs_rc.borrow_mut();
        let mut act = active_rc.borrow_mut();
        let path_str = path.display().to_string();

        // Check if file is already open in a tab
        for (i, tab) in tabs.iter().enumerate() {
            if tab.path == path_str {
                // Save current tab content before switching
                if *act < tabs.len() {
                    tabs[*act].content = ui.get_editor_file_content().to_string();
                    tabs[*act].is_modified = ui.get_editor_is_modified();
                }
                *act = i;
                sync_tabs_to_ui(ui, &tabs, i);
                sync_active_tab(ui, &tabs, i);
                return;
            }
        }

        // Save current tab content before switching
        let current = *act;
        if current < tabs.len() {
            tabs[current].content = ui.get_editor_file_content().to_string();
            tabs[current].is_modified = ui.get_editor_is_modified();
        }

        // If the current tab is untitled and empty, replace it
        if tabs.len() == 1 && tabs[0].path.is_empty() && tabs[0].content.is_empty() && !tabs[0].is_modified {
            tabs[0] = TabState::from_file(path);
            *act = 0;
            sync_tabs_to_ui(ui, &tabs, 0);
            sync_active_tab(ui, &tabs, 0);
        } else if tabs.len() < 12 {
            // Open in new tab
            tabs.push(TabState::from_file(path));
            let new_idx = tabs.len() - 1;
            *act = new_idx;
            sync_tabs_to_ui(ui, &tabs, new_idx);
            sync_active_tab(ui, &tabs, new_idx);
        } else {
            // Max tabs — replace current
            tabs[current] = TabState::from_file(path);
            sync_tabs_to_ui(ui, &tabs, current);
            sync_active_tab(ui, &tabs, current);
        }
    } else {
        // Fallback: no tab state yet — direct load (shouldn't happen after wire())
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        ui.set_editor_file_name(name.into());
        ui.set_editor_is_modified(false);
        ui.set_editor_show_save_dialog(false);
        match std::fs::read_to_string(path) {
            Ok(content) => {
                ui.set_editor_file_content(content.into());
                ui.set_editor_is_readonly(false);
            }
            Err(e) => {
                ui.set_editor_file_content(format!("Error reading file: {}", e).into());
                ui.set_editor_is_readonly(true);
            }
        }
    }
}
