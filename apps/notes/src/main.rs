//! Yantrik Notes — standalone app binary.
//!
//! Communicates with `notes-service` via JSON-RPC IPC.
//! Falls back to local filesystem if service is unavailable.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_transport::SyncRpcClient;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-notes");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("notes") else { return };

    let app = NotesApp::new().unwrap();

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    wire(&app);
    app.run().unwrap();
}

// ── Service wrappers ─────────────────────────────────────────────────

fn list_via_service(folder: Option<&str>) -> Result<Vec<NoteEntry>, String> {
    let client = SyncRpcClient::for_service("notes");
    let params = match folder {
        Some(f) => serde_json::json!({ "folder": f }),
        None => serde_json::json!({}),
    };
    let result = client.call("notes.list", params).map_err(|e| e.message)?;
    let summaries: Vec<yantrik_ipc_contracts::notes::NoteSummary> =
        serde_json::from_value(result).map_err(|e| e.to_string())?;
    Ok(summaries.into_iter().map(summary_to_entry).collect())
}

fn get_via_service(note_id: &str) -> Result<yantrik_ipc_contracts::notes::NoteContent, String> {
    let client = SyncRpcClient::for_service("notes");
    let result = client
        .call("notes.get", serde_json::json!({ "note_id": note_id }))
        .map_err(|e| e.message)?;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

fn create_via_service(title: &str, body: &str, tags: Vec<String>) -> Result<yantrik_ipc_contracts::notes::NoteContent, String> {
    let client = SyncRpcClient::for_service("notes");
    let result = client
        .call("notes.create", serde_json::json!({ "title": title, "body": body, "tags": tags }))
        .map_err(|e| e.message)?;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

fn update_via_service(note_id: &str, title: &str, body: &str) -> Result<(), String> {
    let client = SyncRpcClient::for_service("notes");
    client
        .call("notes.update", serde_json::json!({ "note_id": note_id, "title": title, "body": body }))
        .map_err(|e| e.message)?;
    Ok(())
}

fn delete_via_service(note_id: &str) -> Result<(), String> {
    let client = SyncRpcClient::for_service("notes");
    client
        .call("notes.delete", serde_json::json!({ "note_id": note_id }))
        .map_err(|e| e.message)?;
    Ok(())
}

fn set_pinned_via_service(note_id: &str, pinned: bool) -> Result<(), String> {
    let client = SyncRpcClient::for_service("notes");
    client
        .call("notes.set_pinned", serde_json::json!({ "note_id": note_id, "pinned": pinned }))
        .map_err(|e| e.message)?;
    Ok(())
}

fn set_tags_via_service(note_id: &str, tags: Vec<String>) -> Result<(), String> {
    let client = SyncRpcClient::for_service("notes");
    client
        .call("notes.set_tags", serde_json::json!({ "note_id": note_id, "tags": tags }))
        .map_err(|e| e.message)?;
    Ok(())
}

fn search_via_service(query: &str) -> Result<Vec<NoteEntry>, String> {
    let client = SyncRpcClient::for_service("notes");
    let result = client
        .call("notes.search", serde_json::json!({ "query": query }))
        .map_err(|e| e.message)?;
    let summaries: Vec<yantrik_ipc_contracts::notes::NoteSummary> =
        serde_json::from_value(result).map_err(|e| e.to_string())?;
    Ok(summaries.into_iter().map(summary_to_entry).collect())
}

fn summary_to_entry(s: yantrik_ipc_contracts::notes::NoteSummary) -> NoteEntry {
    let tag_preview = s.tags.first().cloned().unwrap_or_default();
    NoteEntry {
        title: s.title.into(),
        filename: s.id.into(),
        modified: s.modified_at.into(),
        preview: s.snippet.into(),
        is_pinned: s.pinned,
        tags: tag_preview.into(),
        created: s.created_at.into(),
        word_count: s.word_count as i32,
    }
}

fn folder_name(idx: i32) -> Option<&'static str> {
    match idx {
        0 => None,
        1 => Some("favorites"),
        2 => Some("recent"),
        _ => None,
    }
}

// ── Filesystem fallback ──────────────────────────────────────────────

fn notes_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/yantrik/notes")
}

#[derive(Default, Clone)]
struct NoteMeta {
    pinned: bool,
    tags: String,
}

fn meta_path(md_path: &std::path::Path) -> PathBuf {
    md_path.with_extension("meta")
}

fn read_meta(md_path: &std::path::Path) -> NoteMeta {
    let mp = meta_path(md_path);
    let content = std::fs::read_to_string(&mp).unwrap_or_default();
    let mut meta = NoteMeta::default();
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("pinned:") {
            meta.pinned = v.trim() == "true";
        } else if let Some(v) = line.strip_prefix("tags:") {
            meta.tags = v.trim().to_string();
        }
    }
    meta
}

fn write_meta(md_path: &std::path::Path, meta: &NoteMeta) {
    let mp = meta_path(md_path);
    let content = format!("pinned:{}\ntags:{}\n", meta.pinned, meta.tags);
    let _ = std::fs::write(&mp, content);
}

fn scan_notes_fs() -> Vec<NoteEntry> {
    let dir = notes_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut entries = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for de in rd.flatten() {
            let path = de.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                // The first heading names the note; the filename is only the fallback.
                let title = content
                    .lines()
                    .find_map(|l| l.strip_prefix("# ").map(|t| t.trim().to_string()))
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| fname.trim_end_matches(".md").to_string());
                let preview: String = content
                    .lines()
                    .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                    .unwrap_or_default()
                    .chars()
                    .take(120)
                    .collect();
                let (modified_secs, modified) = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .map(|t| {
                        let secs = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                        let local: chrono::DateTime<chrono::Local> = t.into();
                        (secs, local.format("%b %-d, %H:%M").to_string())
                    })
                    .unwrap_or_default();
                let meta = read_meta(&path);
                let wc = content.split_whitespace().count();
                entries.push((modified_secs, NoteEntry {
                    title: title.into(),
                    filename: fname.into(),
                    created: modified.clone().into(),
                    modified: modified.into(),
                    preview: preview.into(),
                    is_pinned: meta.pinned,
                    tags: meta.tags.into(),
                    word_count: wc as i32,
                }));
            }
        }
    }
    // Sort: pinned first, then newest first
    entries.sort_by(|(sa, a), (sb, b)| b.is_pinned.cmp(&a.is_pinned).then_with(|| sb.cmp(sa)));
    entries.into_iter().map(|(_, e)| e).collect()
}

fn template_content(template: &str) -> &'static str {
    match template {
        "meeting" => "# Meeting Notes\n\n**Date:** \n**Attendees:** \n\n## Discussion Points\n\n1. \n\n## Action Items\n\n- [ ] \n",
        "project" => "# Project Brief\n\n## Overview\n\n\n## Objectives\n\n1. \n\n## Timeline\n\n| Milestone | Date | Status |\n|-----------|------|--------|\n",
        "decision" => "# Decision Log\n\n## Decision\n\n\n## Context\n\n\n## Options\n\n### Option A\n- **Pros:** \n- **Cons:** \n\n## Decision\n\n\n## Follow-up\n\n- [ ] \n",
        "todo" => "# TODO List\n\n## High Priority\n\n- [ ] \n\n## Medium Priority\n\n- [ ] \n\n## Low Priority\n\n- [ ] \n",
        _ => "# New Note\n\n",
    }
}

// ── Wire all callbacks ───────────────────────────────────────────────

/// Every `[[target]]` in the text, in order. `[[target|alias]]` yields `target`.
///
/// match_indices gives byte offsets at char boundaries, so the slicing below is safe on
/// non-ASCII note bodies.
fn wikilinks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (start, _) in text.match_indices("[[") {
        let rest = &text[start + 2..];
        if let Some(end) = rest.find("]]") {
            let target = rest[..end].split('|').next().unwrap_or("").trim();
            if !target.is_empty() && !target.contains('\n') {
                out.push(target.to_string());
            }
        }
    }
    out
}

/// A link resolves by title or by filename, case-insensitively, with or without the extension —
/// `[[Build times]]`, `[[build times]]` and `[[build-times.md]]` should all find the same note.
fn link_key(s: &str) -> String {
    s.trim().trim_end_matches(".md").to_lowercase()
}

/// The first `# ` heading names a note; the filename is only the fallback.
fn title_of(content: &str, fname: &str) -> String {
    content
        .lines()
        .find_map(|l| l.strip_prefix("# ").map(|t| t.trim().to_string()))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| fname.trim_end_matches(".md").to_string())
}

/// Notes that link *to* `target_file`. Reads the vault each call: it is a flat directory of
/// small files, and a stale link graph is worse than a slow one.
fn backlinks_for(target_file: &str) -> Vec<NoteBacklink> {
    let dir = notes_dir();
    let target_content = std::fs::read_to_string(dir.join(target_file)).unwrap_or_default();
    let keys = [
        link_key(&title_of(&target_content, target_file)),
        link_key(target_file),
    ];

    let mut out: Vec<NoteBacklink> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "md").unwrap_or(true) {
            continue;
        }
        let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        // A note linking to itself is not a backlink.
        if fname == target_file {
            continue;
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        if wikilinks(&content).iter().any(|l| keys.contains(&link_key(l))) {
            out.push(NoteBacklink {
                title: title_of(&content, &fname).into(),
                filename: fname.into(),
            });
        }
    }
    out.sort_by_key(|b| b.title.to_lowercase());
    out
}

/// Load a note into the editor. Shared by the list and by following a backlink.
fn load_note(ui: &NotesApp, idx: i32, id: &str) {
    if let Ok(note) = get_via_service(id) {
        let wc = note.body.split_whitespace().count();
        ui.set_current_content(note.body.into());
        ui.set_current_title(note.title.into());
        ui.set_current_tags(note.tags.join(", ").into());
        ui.set_meta_word_count(wc as i32);
        ui.set_meta_created(note.created_at.into());
        ui.set_meta_modified(note.modified_at.into());
    } else {
        // Filesystem fallback
        let path = notes_dir().join(id);
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let meta = read_meta(&path);
        ui.set_current_title(title_of(&content, id).into());
        ui.set_current_content(content.clone().into());
        ui.set_current_tags(meta.tags.into());
        ui.set_meta_word_count(content.split_whitespace().count() as i32);
    }
    ui.set_selected_index(idx);
    ui.set_is_modified(false);
}

fn wire(app: &NotesApp) {
    let current_file: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    // Initial load
    let notes = list_via_service(None).unwrap_or_else(|_| scan_notes_fs());
    let count = notes.len() as i32;
    app.set_notes_list(ModelRc::new(VecModel::from(notes)));
    app.set_note_count(count);
    app.set_folder_all_count(count);

    // ── New note ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_new_note(move || {
            let title = "Untitled";
            let body = "# Untitled\n\n";
            if let Ok(note) = create_via_service(title, body, vec![]) {
                *cf.borrow_mut() = note.id.clone();
                if let Some(ui) = weak.upgrade() {
                    ui.set_current_content(note.body.into());
                    ui.set_current_title(note.title.into());
                    ui.set_is_modified(false);
                    refresh_list(&ui, 0);
                }
            } else {
                // Filesystem fallback
                let dir = notes_dir();
                let _ = std::fs::create_dir_all(&dir);
                let fname = format!("untitled-{}.md", uuid7::uuid7());
                let path = dir.join(&fname);
                let _ = std::fs::write(&path, body);
                *cf.borrow_mut() = fname;
                if let Some(ui) = weak.upgrade() {
                    ui.set_current_content(body.into());
                    ui.set_current_title(title.into());
                    ui.set_is_modified(false);
                    refresh_list(&ui, 0);
                }
            }
        });
    }

    // ── New from template ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_new_from_template(move |template| {
            let tmpl = template.to_string();
            let title = format!("{} Note", tmpl.chars().next().unwrap_or('N').to_uppercase().collect::<String>() + &tmpl[1..]);
            let body = template_content(&tmpl);
            if let Ok(note) = create_via_service(&title, body, vec![tmpl.clone()]) {
                *cf.borrow_mut() = note.id.clone();
                if let Some(ui) = weak.upgrade() {
                    ui.set_current_content(note.body.into());
                    ui.set_current_title(note.title.into());
                    ui.set_is_modified(false);
                    refresh_list(&ui, 0);
                }
            } else {
                let dir = notes_dir();
                let _ = std::fs::create_dir_all(&dir);
                let fname = format!("{}-{}.md", tmpl, uuid7::uuid7());
                let path = dir.join(&fname);
                let _ = std::fs::write(&path, body);
                *cf.borrow_mut() = fname;
                if let Some(ui) = weak.upgrade() {
                    ui.set_current_content(body.into());
                    ui.set_current_title(title.into());
                    ui.set_is_modified(false);
                    refresh_list(&ui, 0);
                }
            }
        });
    }

    // ── Save note ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_save_note(move || {
            let Some(ui) = weak.upgrade() else { return };
            let content = ui.get_current_content().to_string();
            let title = ui.get_current_title().to_string();
            let id = cf.borrow().clone();
            if id.is_empty() { return; }

            if update_via_service(&id, &title, &content).is_ok() {
                // Also update tags if set
                let tags_str = ui.get_current_tags().to_string();
                if !tags_str.is_empty() {
                    let tags: Vec<String> = tags_str.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
                    let _ = set_tags_via_service(&id, tags);
                }
            } else {
                // Filesystem fallback
                let path = notes_dir().join(&id);
                let _ = std::fs::write(&path, &content);
            }
            ui.set_is_modified(false);
            let wc = content.split_whitespace().count();
            ui.set_meta_word_count(wc as i32);
        });
    }

    // ── Delete note ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_delete_note(move || {
            let id = cf.borrow().clone();
            if id.is_empty() { return; }

            if delete_via_service(&id).is_err() {
                let path = notes_dir().join(&id);
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_file(meta_path(&path));
            }
            *cf.borrow_mut() = String::new();
            if let Some(ui) = weak.upgrade() {
                ui.set_current_content("".into());
                ui.set_current_title("".into());
                ui.set_selected_index(-1);
                refresh_list(&ui, ui.get_active_folder());
            }
        });
    }

    // ── Select note ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_select_note(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            let model = ui.get_notes_list();
            if idx < 0 || idx as usize >= model.row_count() { return; }
            let entry = model.row_data(idx as usize).unwrap();
            let id = entry.filename.to_string();
            *cf.borrow_mut() = id.clone();
            load_note(&ui, idx, &id);
            if ui.get_backlinks_panel_open() {
                ui.set_backlinks(ModelRc::new(VecModel::from(backlinks_for(&id))));
            }
        });
    }

    // ── Search ──
    {
        let weak = app.as_weak();
        app.on_search_notes(move |query| {
            let Some(ui) = weak.upgrade() else { return };
            let q = query.to_string();
            if q.is_empty() {
                refresh_list(&ui, ui.get_active_folder());
                return;
            }
            if let Ok(results) = search_via_service(&q) {
                let count = results.len() as i32;
                ui.set_notes_list(ModelRc::new(VecModel::from(results)));
                ui.set_note_count(count);
            }
        });
    }

    // ── Content changed ──
    {
        let weak = app.as_weak();
        app.on_content_changed(move |_content| {
            if let Some(ui) = weak.upgrade() {
                ui.set_is_modified(true);
            }
        });
    }

    // ── Select folder ──
    {
        let weak = app.as_weak();
        app.on_select_folder(move |idx| {
            if let Some(ui) = weak.upgrade() {
                ui.set_active_folder(idx);
                refresh_list(&ui, idx);
            }
        });
    }

    // ── Toggle pin ──
    {
        let weak = app.as_weak();
        app.on_toggle_pin(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            let model = ui.get_notes_list();
            if idx < 0 || idx as usize >= model.row_count() { return; }
            let entry = model.row_data(idx as usize).unwrap();
            let id = entry.filename.to_string();
            let new_pinned = !entry.is_pinned;

            if set_pinned_via_service(&id, new_pinned).is_err() {
                let path = notes_dir().join(&id);
                let mut meta = read_meta(&path);
                meta.pinned = new_pinned;
                write_meta(&path, &meta);
            }
            refresh_list(&ui, ui.get_active_folder());
        });
    }

    // ── Update tags ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_update_tags(move |tags_str| {
            let id = cf.borrow().clone();
            if id.is_empty() { return; }
            let tags: Vec<String> = tags_str.to_string().split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();

            if set_tags_via_service(&id, tags).is_err() {
                let path = notes_dir().join(&id);
                let mut meta = read_meta(&path);
                meta.tags = tags_str.to_string();
                write_meta(&path, &meta);
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_current_tags(tags_str);
            }
        });
    }

    // ── Insert formatting ──
    {
        let weak = app.as_weak();
        app.on_insert_format(move |fmt| {
            let Some(ui) = weak.upgrade() else { return };
            let current = ui.get_current_content().to_string();
            let insertion = match fmt.as_str() {
                "bold" => "**bold**",
                "italic" => "*italic*",
                "code" => "`code`",
                "heading" => "\n## Heading\n",
                "list" => "\n- Item\n",
                "checkbox" => "\n- [ ] Task\n",
                "link" => "[link text](url)",
                "quote" => "\n> Quote\n",
                "divider" => "\n---\n",
                "table" => "\n| Col 1 | Col 2 |\n|-------|-------|\n|       |       |\n",
                _ => "",
            };
            if !insertion.is_empty() {
                let new_content = format!("{}{}", current, insertion);
                ui.set_current_content(new_content.into());
                ui.set_is_modified(true);
            }
        });
    }

    // ── Close note ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_close_note(move || {
            *cf.borrow_mut() = String::new();
            if let Some(ui) = weak.upgrade() {
                ui.set_current_content("".into());
                ui.set_current_title("".into());
                ui.set_current_tags("".into());
                ui.set_selected_index(-1);
                ui.set_is_modified(false);
            }
        });
    }

    // ── Export ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_export_md(move || {
            let Some(ui) = weak.upgrade() else { return };
            let id = cf.borrow().clone();
            if id.is_empty() { return; }
            let content = ui.get_current_content().to_string();
            let export_dir = notes_dir().join("exports");
            let _ = std::fs::create_dir_all(&export_dir);
            let export_path = export_dir.join(&id);
            match std::fs::write(&export_path, &content) {
                Ok(_) => ui.set_export_status(format!("Exported to {}", export_path.display()).into()),
                Err(e) => ui.set_export_status(format!("Export failed: {e}").into()),
            }
        });
    }

    // Stubs for AI features (need companion bridge in standalone mode)
    app.on_ai_structure(|| { tracing::info!("AI structure requested (standalone mode)"); });
    app.on_ai_summarize(|| { tracing::info!("AI summarize requested (standalone mode)"); });
    app.on_ai_apply(|| {});
    app.on_ai_dismiss(|| {});
    app.on_view_version(|_| {});
    app.on_restore_version(|_| {});
    // ── Backlinks ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_find_backlinks(move || {
            let Some(ui) = weak.upgrade() else { return };
            let id = cf.borrow().clone();
            if id.is_empty() {
                return;
            }
            let found = backlinks_for(&id);
            tracing::info!(note = %id, backlinks = found.len(), "Scanned vault for backlinks");
            ui.set_backlinks(ModelRc::new(VecModel::from(found)));
        });
    }

    // ── Follow a backlink ──
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_open_note(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            let name = name.to_string();
            let model = ui.get_notes_list();
            for i in 0..model.row_count() {
                let Some(entry) = model.row_data(i) else { continue };
                if entry.filename.as_str() == name {
                    *cf.borrow_mut() = name.clone();
                    load_note(&ui, i as i32, &name);
                    ui.set_backlinks(ModelRc::new(VecModel::from(backlinks_for(&name))));
                    return;
                }
            }
            tracing::warn!(note = %name, "Backlink target is not in the current list");
        });
    }
    app.on_toggle_meeting_mode(|| {});
    app.on_import_md(|| {});
}

fn refresh_list(ui: &NotesApp, folder: i32) {
    let folder_str = folder_name(folder);
    let notes = list_via_service(folder_str).unwrap_or_else(|_| scan_notes_fs());
    let count = notes.len() as i32;
    ui.set_notes_list(ModelRc::new(VecModel::from(notes)));
    ui.set_note_count(count);
    if folder == 0 {
        ui.set_folder_all_count(count);
    }
}
