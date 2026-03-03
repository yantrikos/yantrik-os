//! Notes Editor wiring — load, save, search, create, delete notes.
//!
//! Notes stored as `.md` files in `~/.local/share/yantrik/notes/`.
//! On save, also indexed in YantrikDB via bridge for semantic recall.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app_context::AppContext;
use crate::App;

/// Wire notes editor callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let notes_dir = notes_directory();
    let current_file = Rc::new(RefCell::new(String::new()));
    let bridge = ctx.bridge.clone();

    // ── New note ──
    let nd = notes_dir.clone();
    let cf = current_file.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_new(move || {
        let dir = nd.clone();
        let _ = std::fs::create_dir_all(&dir);

        // Generate filename: timestamp-untitled.md
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let filename = format!("{}-untitled.md", ts);
        let path = dir.join(&filename);

        // Create empty file with a heading
        let content = "# New Note\n\n";
        if let Err(e) = std::fs::write(&path, content) {
            tracing::error!(error = %e, "Failed to create note");
            return;
        }

        *cf.borrow_mut() = path.display().to_string();

        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_current_content(content.into());
            ui.set_notes_current_title("New Note".into());
            ui.set_notes_is_modified(false);

            // Refresh list and select the new note (it'll be first since newest)
            let entries = scan_notes(&dir);
            let count = entries.len() as i32;
            ui.set_notes_list(ModelRc::new(VecModel::from(entries)));
            ui.set_notes_note_count(count);
            ui.set_notes_selected_index(0);
        }

        tracing::info!(path = %path.display(), "New note created");
    });

    // ── Save note ──
    let cf = current_file.clone();
    let nd = notes_dir.clone();
    let bridge_save = bridge.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_save(move || {
        let path_str = cf.borrow().clone();
        if path_str.is_empty() {
            return;
        }
        if let Some(ui) = ui_weak.upgrade() {
            let content = ui.get_notes_current_content().to_string();
            let path = PathBuf::from(&path_str);

            // Extract title from first heading or first line
            let title = extract_title(&content);

            // Rename file to match title if it's still "untitled"
            let final_path = if path.file_name().map_or(false, |f| f.to_string_lossy().contains("-untitled.md")) {
                let slug = slugify(&title);
                let stem = path.file_stem().unwrap().to_string_lossy();
                let ts_part = stem.split('-').next().unwrap_or("0");
                let new_name = format!("{}-{}.md", ts_part, slug);
                let new_path = path.with_file_name(new_name);
                if new_path != path {
                    let _ = std::fs::rename(&path, &new_path);
                    *cf.borrow_mut() = new_path.display().to_string();
                    new_path
                } else {
                    path
                }
            } else {
                path
            };

            match std::fs::write(&final_path, &content) {
                Ok(()) => {
                    tracing::info!(path = %final_path.display(), "Note saved");
                    ui.set_notes_is_modified(false);
                    ui.set_notes_current_title(title.clone().into());

                    // Index in YantrikDB for semantic recall
                    let short = if content.len() > 500 {
                        format!("{}...", &content[..500])
                    } else {
                        content.clone()
                    };
                    bridge_save.record_system_event(
                        format!("Note '{}': {}", title, short),
                        "user/notes".to_string(),
                        0.8,
                    );

                    // Refresh list
                    let entries = scan_notes(&nd);
                    let count = entries.len() as i32;
                    ui.set_notes_list(ModelRc::new(VecModel::from(entries)));
                    ui.set_notes_note_count(count);
                }
                Err(e) => {
                    tracing::error!(path = %final_path.display(), error = %e, "Failed to save note");
                }
            }
        }
    });

    // ── Delete note ──
    let cf = current_file.clone();
    let nd = notes_dir.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_delete(move || {
        let path_str = cf.borrow().clone();
        if path_str.is_empty() {
            return;
        }
        if let Err(e) = std::fs::remove_file(&path_str) {
            tracing::error!(path = %path_str, error = %e, "Failed to delete note");
            return;
        }
        tracing::info!(path = %path_str, "Note deleted");

        *cf.borrow_mut() = String::new();

        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_current_content("".into());
            ui.set_notes_current_title("".into());
            ui.set_notes_is_modified(false);
            ui.set_notes_selected_index(-1);

            let entries = scan_notes(&nd);
            let count = entries.len() as i32;
            ui.set_notes_list(ModelRc::new(VecModel::from(entries)));
            ui.set_notes_note_count(count);
        }
    });

    // ── Select note ──
    let cf = current_file.clone();
    let nd = notes_dir.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_select(move |index| {
        if index < 0 {
            return;
        }
        if let Some(ui) = ui_weak.upgrade() {
            let entries = scan_notes(&nd);
            let idx = index as usize;
            if idx >= entries.len() {
                return;
            }
            let entry = &entries[idx];
            let path = nd.join(&entry.filename.to_string());
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    *cf.borrow_mut() = path.display().to_string();
                    ui.set_notes_current_content(content.into());
                    ui.set_notes_current_title(entry.title.to_string().into());
                    ui.set_notes_is_modified(false);
                    ui.set_notes_selected_index(index);
                }
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "Failed to read note");
                }
            }
        }
    });

    // ── Search notes ──
    let nd = notes_dir.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_search(move |query| {
        let query = query.to_string();
        if let Some(ui) = ui_weak.upgrade() {
            let all = scan_notes(&nd);
            if query.is_empty() {
                let count = all.len() as i32;
                ui.set_notes_list(ModelRc::new(VecModel::from(all)));
                ui.set_notes_note_count(count);
            } else {
                let lower = query.to_lowercase();
                let filtered: Vec<_> = all
                    .into_iter()
                    .filter(|e| {
                        e.title.to_string().to_lowercase().contains(&lower)
                            || e.preview.to_string().to_lowercase().contains(&lower)
                    })
                    .collect();
                let count = filtered.len() as i32;
                ui.set_notes_list(ModelRc::new(VecModel::from(filtered)));
                ui.set_notes_note_count(count);
            }
        }
    });

    // ── Content changed ──
    let ui_weak = ui.as_weak();
    ui.on_notes_content_changed(move |_text| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_is_modified(true);
        }
    });
}

/// Load notes list when navigating to screen 15.
pub fn load_notes_list(ui: &App) {
    let dir = notes_directory();
    let _ = std::fs::create_dir_all(&dir);
    let entries = scan_notes(&dir);
    let count = entries.len() as i32;
    ui.set_notes_list(ModelRc::new(VecModel::from(entries)));
    ui.set_notes_note_count(count);
    ui.set_notes_selected_index(-1);
    ui.set_notes_current_content("".into());
    ui.set_notes_current_title("".into());
    ui.set_notes_is_modified(false);
}

// ── Helpers ──

use crate::NoteEntry;

/// Get the notes storage directory.
fn notes_directory() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".local/share/yantrik/notes")
}

/// Scan notes directory and return entries sorted by modification time (newest first).
fn scan_notes(dir: &PathBuf) -> Vec<NoteEntry> {
    let mut entries = Vec::new();

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.extension().map_or(false, |e| e == "md") {
            continue;
        }

        let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();

        // Read first few lines for title and preview
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let title = extract_title(&content);
        let preview = content
            .lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .unwrap_or("")
            .chars()
            .take(80)
            .collect::<String>();

        // Get modification time
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let modified_text = format_relative_time(modified);

        entries.push(NoteEntry {
            title: title.into(),
            filename: filename.into(),
            modified: modified_text.into(),
            preview: preview.into(),
        });
    }

    // Sort by modification time (newest first) — use filename timestamp prefix as proxy
    entries.sort_by(|a, b| b.filename.cmp(&a.filename));

    entries
}

/// Extract title from markdown content (first # heading or first non-empty line).
fn extract_title(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return heading.trim().to_string();
        }
        // First non-empty line as fallback
        return trimmed.chars().take(50).collect();
    }
    "Untitled".to_string()
}

/// Convert a filename-friendly slug from a title.
fn slugify(title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse multiple dashes
    let mut result = String::new();
    let mut last_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !last_dash && !result.is_empty() {
                result.push('-');
            }
            last_dash = true;
        } else {
            result.push(c);
            last_dash = false;
        }
    }
    result.trim_end_matches('-').to_string()
}

/// Format a unix timestamp as relative time (e.g., "2m ago", "1h ago", "3d ago").
fn format_relative_time(timestamp: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if timestamp == 0 {
        return "—".to_string();
    }

    let diff = now.saturating_sub(timestamp);
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}
