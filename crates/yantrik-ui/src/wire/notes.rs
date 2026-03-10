//! Notes Editor wiring — load, save, search, create, delete notes.
//!
//! Notes stored as `.md` files in `~/.local/share/yantrik/notes/`.
//! On save, also indexed in YantrikDB via bridge for semantic recall.
//! Supports folders (All/Favorites/Recent), tags, pinning, templates,
//! metadata display, content search, and markdown formatting insertion.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app_context::AppContext;
use crate::App;

/// Metadata sidecar for a note (stored as `<note>.meta` alongside the `.md` file).
/// Format: `pinned:<bool>\ntags:<comma-separated>\n`
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

/// Template content for each template type.
fn template_content(template: &str) -> &'static str {
    match template {
        "meeting" => "# Meeting Notes\n\n**Date:** \n**Attendees:** \n**Agenda:** \n\n## Discussion Points\n\n1. \n\n## Action Items\n\n- [ ] \n\n## Decisions Made\n\n- \n\n## Next Steps\n\n- \n",
        "project" => "# Project Brief\n\n## Overview\n\n\n## Objectives\n\n1. \n\n## Scope\n\n### In Scope\n- \n\n### Out of Scope\n- \n\n## Timeline\n\n| Milestone | Date | Status |\n|-----------|------|--------|\n|           |      |        |\n\n## Stakeholders\n\n- \n\n## Risks\n\n- \n\n## Success Criteria\n\n- \n",
        "decision" => "# Decision Log\n\n## Decision\n\n\n## Date\n\n\n## Context\n\nWhat is the issue that we need to decide on?\n\n## Options Considered\n\n### Option A\n- **Pros:** \n- **Cons:** \n\n### Option B\n- **Pros:** \n- **Cons:** \n\n## Decision\n\nWe decided to go with...\n\n## Rationale\n\n\n## Consequences\n\n- \n\n## Follow-up Actions\n\n- [ ] \n",
        "todo" => "# TODO List\n\n## High Priority\n\n- [ ] \n\n## Medium Priority\n\n- [ ] \n\n## Low Priority\n\n- [ ] \n\n## Completed\n\n- [x] \n",
        "sop" => "# Standard Operating Procedure\n\n## Purpose\n\n\n## Scope\n\n\n## Prerequisites\n\n- \n\n## Procedure\n\n### Step 1\n\n\n### Step 2\n\n\n### Step 3\n\n\n## Expected Outcome\n\n\n## Troubleshooting\n\n| Issue | Solution |\n|-------|----------|\n|       |          |\n\n## Revision History\n\n| Date | Author | Changes |\n|------|--------|---------|\n|      |        |         |\n",
        _ => "# New Note\n\n",
    }
}

/// Wire notes editor callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let notes_dir = notes_directory();
    let current_file = Rc::new(RefCell::new(String::new()));
    let bridge = ctx.bridge.clone();
    // Track active folder: 0=All, 1=Favorites, 2=Recent
    let active_folder = Rc::new(RefCell::new(0i32));

    // ── New note ──
    let nd = notes_dir.clone();
    let cf = current_file.clone();
    let af = active_folder.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_new(move || {
        let dir = nd.clone();
        let _ = std::fs::create_dir_all(&dir);

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let filename = format!("{}-untitled.md", ts);
        let path = dir.join(&filename);

        let content = "# New Note\n\n";
        if let Err(e) = std::fs::write(&path, content) {
            tracing::error!(error = %e, "Failed to create note");
            return;
        }

        // Create empty meta sidecar
        write_meta(&path, &NoteMeta::default());

        *cf.borrow_mut() = path.display().to_string();

        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_current_content(content.into());
            ui.set_notes_current_title("New Note".into());
            ui.set_notes_is_modified(false);
            ui.set_notes_current_tags("".into());
            update_metadata(&ui, &path, content);

            let folder = *af.borrow();
            refresh_list(&ui, &dir, folder);
            ui.set_notes_selected_index(0);
        }

        tracing::info!(path = %path.display(), "New note created");
    });

    // ── New from template ──
    let nd = notes_dir.clone();
    let cf = current_file.clone();
    let af = active_folder.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_new_from_template(move |template| {
        let dir = nd.clone();
        let _ = std::fs::create_dir_all(&dir);
        let template = template.to_string();

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let slug = slugify(&template);
        let filename = format!("{}-{}.md", ts, slug);
        let path = dir.join(&filename);

        let content = template_content(&template);
        if let Err(e) = std::fs::write(&path, content) {
            tracing::error!(error = %e, "Failed to create note from template");
            return;
        }

        // Create meta with template tag
        let meta = NoteMeta {
            pinned: false,
            tags: template.clone(),
        };
        write_meta(&path, &meta);

        *cf.borrow_mut() = path.display().to_string();

        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_current_content(content.into());
            ui.set_notes_current_title(extract_title(content).into());
            ui.set_notes_is_modified(false);
            ui.set_notes_current_tags(meta.tags.into());
            update_metadata(&ui, &path, content);

            let folder = *af.borrow();
            refresh_list(&ui, &dir, folder);
            ui.set_notes_selected_index(0);
        }

        tracing::info!(path = %path.display(), template = %template, "New note from template");
    });

    // ── Save note ──
    let cf = current_file.clone();
    let nd = notes_dir.clone();
    let af = active_folder.clone();
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

            // ── Create version snapshot before overwriting ──
            if path.exists() {
                if let Ok(old_content) = std::fs::read_to_string(&path) {
                    save_version(&path, &old_content);
                }
            }

            let title = extract_title(&content);

            // Save tags to meta
            let tags = ui.get_notes_current_tags().to_string();
            let mut meta = read_meta(&path);
            meta.tags = tags;
            write_meta(&path, &meta);

            // Rename file to match title if it's still "untitled"
            let final_path = if path.file_name().map_or(false, |f| f.to_string_lossy().contains("-untitled.md")) {
                let slug = slugify(&title);
                let stem = path.file_stem().unwrap().to_string_lossy();
                let ts_part = stem.split('-').next().unwrap_or("0");
                let new_name = format!("{}-{}.md", ts_part, slug);
                let new_path = path.with_file_name(new_name);
                if new_path != path {
                    let _ = std::fs::rename(&path, &new_path);
                    // Also rename meta file
                    let old_meta = meta_path(&path);
                    let new_meta = meta_path(&new_path);
                    let _ = std::fs::rename(&old_meta, &new_meta);
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
                    update_metadata(&ui, &final_path, &content);

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

                    let folder = *af.borrow();
                    refresh_list(&ui, &nd, folder);

                    // Update version history panel
                    let versions = load_version_history(&final_path);
                    ui.set_notes_version_history(ModelRc::new(VecModel::from(versions)));
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
    let af = active_folder.clone();
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
        // Also delete meta sidecar
        let _ = std::fs::remove_file(meta_path(std::path::Path::new(&path_str)));
        tracing::info!(path = %path_str, "Note deleted");

        *cf.borrow_mut() = String::new();

        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_current_content("".into());
            ui.set_notes_current_title("".into());
            ui.set_notes_is_modified(false);
            ui.set_notes_selected_index(-1);
            ui.set_notes_current_tags("".into());
            ui.set_notes_meta_created("".into());
            ui.set_notes_meta_modified("".into());
            ui.set_notes_meta_word_count(0);

            let folder = *af.borrow();
            refresh_list(&ui, &nd, folder);
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
                    let meta = read_meta(&path);
                    ui.set_notes_current_content(content.clone().into());
                    ui.set_notes_current_title(entry.title.to_string().into());
                    ui.set_notes_is_modified(false);
                    ui.set_notes_selected_index(index);
                    ui.set_notes_current_tags(meta.tags.into());
                    update_metadata(&ui, &path, &content);
                    // Load version history for selected note
                    let versions = load_version_history(&path);
                    ui.set_notes_version_history(ModelRc::new(VecModel::from(versions)));
                    // Clear export status
                    ui.set_notes_export_status("".into());
                }
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "Failed to read note");
                }
            }
        }
    });

    // ── Search notes (searches title AND content) ──
    let nd = notes_dir.clone();
    let af = active_folder.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_search(move |query| {
        let query = query.to_string();
        if let Some(ui) = ui_weak.upgrade() {
            let folder = *af.borrow();
            let all = scan_notes(&nd);
            let filtered_by_folder = filter_by_folder(all, &nd, folder);

            if query.is_empty() {
                let count = filtered_by_folder.len() as i32;
                ui.set_notes_list(ModelRc::new(VecModel::from(filtered_by_folder)));
                ui.set_notes_note_count(count);
            } else {
                let lower = query.to_lowercase();
                let filtered: Vec<_> = filtered_by_folder
                    .into_iter()
                    .filter(|e| {
                        let title_match = e.title.to_string().to_lowercase().contains(&lower);
                        let preview_match = e.preview.to_string().to_lowercase().contains(&lower);
                        // Also search full file content
                        let content_match = if !title_match && !preview_match {
                            let path = nd.join(&e.filename.to_string());
                            std::fs::read_to_string(&path)
                                .map(|c| c.to_lowercase().contains(&lower))
                                .unwrap_or(false)
                        } else {
                            false
                        };
                        title_match || preview_match || content_match
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
    ui.on_notes_content_changed(move |text| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_is_modified(true);
            // Update word count live
            let word_count = text.to_string().split_whitespace().count() as i32;
            ui.set_notes_meta_word_count(word_count);
        }
    });

    // ── Select folder ──
    let nd = notes_dir.clone();
    let af = active_folder.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_select_folder(move |folder_idx| {
        *af.borrow_mut() = folder_idx;
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_active_folder(folder_idx);
            refresh_list(&ui, &nd, folder_idx);
            // Deselect current note
            ui.set_notes_selected_index(-1);
            ui.set_notes_current_content("".into());
            ui.set_notes_current_title("".into());
        }
    });

    // ── Toggle pin ──
    let nd = notes_dir.clone();
    let af = active_folder.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_toggle_pin(move |index| {
        if index < 0 { return; }
        if let Some(ui) = ui_weak.upgrade() {
            let all = scan_notes(&nd);
            let idx = index as usize;
            if idx >= all.len() { return; }

            let entry = &all[idx];
            let path = nd.join(&entry.filename.to_string());
            let mut meta = read_meta(&path);
            meta.pinned = !meta.pinned;
            write_meta(&path, &meta);

            let folder = *af.borrow();
            refresh_list(&ui, &nd, folder);
        }
    });

    // ── Update tags ──
    let cf = current_file.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_update_tags(move |tags| {
        let path_str = cf.borrow().clone();
        if path_str.is_empty() { return; }
        let path = PathBuf::from(&path_str);
        let mut meta = read_meta(&path);
        meta.tags = tags.to_string();
        write_meta(&path, &meta);
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_is_modified(true);
        }
    });

    // ── Insert format (markdown syntax insertion) ──
    let ui_weak = ui.as_weak();
    ui.on_notes_insert_format(move |fmt| {
        if let Some(ui) = ui_weak.upgrade() {
            let content = ui.get_notes_current_content().to_string();
            let fmt = fmt.to_string();
            let insertion = match fmt.as_str() {
                "bold" => "**bold text**",
                "italic" => "*italic text*",
                "heading" => "\n## Heading\n",
                "bullet" => "\n- ",
                "checklist" => "\n- [ ] ",
                "code" => "\n```\ncode\n```\n",
                "divider" => "\n---\n",
                _ => return,
            };
            let new_content = format!("{}{}", content, insertion);
            ui.set_notes_current_content(new_content.into());
            ui.set_notes_is_modified(true);
        }
    });

    // ── AI Structure callback ──
    let bridge_ai = bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_notes_ai_structure(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let content = ui.get_notes_current_content().to_string();
        if content.trim().is_empty() { return; }

        let prompt = super::ai_assist::note_structure_prompt(&content);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge_ai,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 45,
                set_working: Box::new(|ui, v| ui.set_notes_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_notes_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_notes_ai_response().to_string()),
            },
        );
    });

    // ── AI Summarize callback ──
    let bridge_ai2 = bridge.clone();
    let ai_st2 = ai_state.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_ai_summarize(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let content = ui.get_notes_current_content().to_string();
        if content.trim().is_empty() { return; }

        let prompt = super::ai_assist::note_summarize_prompt(&content);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge_ai2,
            &ai_st2,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_notes_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_notes_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_notes_ai_response().to_string()),
            },
        );
    });

    // ── AI Apply callback (replace note content with AI response) ──
    let ui_weak = ui.as_weak();
    ui.on_notes_ai_apply(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let ai_text = ui.get_notes_ai_response().to_string();
            if !ai_text.is_empty() {
                ui.set_notes_current_content(ai_text.into());
                ui.set_notes_is_modified(true);
                ui.set_notes_ai_panel_open(false);
            }
        }
    });

    // ── AI Dismiss ──
    let ui_weak = ui.as_weak();
    ui.on_notes_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_ai_panel_open(false);
        }
    });

    // ── View version ──
    let cf = current_file.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_view_version(move |version_idx| {
        let path_str = cf.borrow().clone();
        if path_str.is_empty() { return; }
        let path = PathBuf::from(&path_str);
        let ver_path = version_path(&path, version_idx);
        if let Ok(content) = std::fs::read_to_string(&ver_path) {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_notes_current_content(content.into());
                // Don't mark as modified — this is read-only viewing
            }
        } else {
            tracing::warn!(version = version_idx, "Version file not found");
        }
    });

    // ── Restore version ──
    let cf = current_file.clone();
    let nd2 = notes_dir.clone();
    let af2 = active_folder.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_restore_version(move |version_idx| {
        let path_str = cf.borrow().clone();
        if path_str.is_empty() { return; }
        let path = PathBuf::from(&path_str);
        let ver_path = version_path(&path, version_idx);
        if let Ok(content) = std::fs::read_to_string(&ver_path) {
            // Overwrite current note with version content
            if let Err(e) = std::fs::write(&path, &content) {
                tracing::error!(error = %e, "Failed to restore version");
                return;
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_notes_current_content(content.clone().into());
                ui.set_notes_is_modified(false);
                update_metadata(&ui, &path, &content);
                let folder = *af2.borrow();
                refresh_list(&ui, &nd2, folder);
            }
            tracing::info!(version = version_idx, "Version restored");
        }
    });

    // ── Find backlinks ──
    let nd3 = notes_dir.clone();
    let cf = current_file.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_find_backlinks(move || {
        let path_str = cf.borrow().clone();
        if path_str.is_empty() { return; }
        let path = PathBuf::from(&path_str);
        let current_filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let current_title = {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            extract_title(&content)
        };

        let mut backlinks = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&nd3) {
            for entry in read_dir.flatten() {
                let ep = entry.path();
                if !ep.extension().map_or(false, |e| e == "md") { continue; }
                let fname = ep.file_name().unwrap_or_default().to_string_lossy().to_string();
                if fname == current_filename { continue; }

                if let Ok(content) = std::fs::read_to_string(&ep) {
                    // Check for [[title]] wiki-link or filename reference
                    let has_link = content.contains(&format!("[[{}]]", current_title))
                        || content.contains(&current_filename);
                    if has_link {
                        let title = extract_title(&content);
                        backlinks.push(crate::NoteBacklink {
                            title: title.into(),
                            filename: fname.into(),
                        });
                    }
                }
            }
        }

        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_backlinks(ModelRc::new(VecModel::from(backlinks)));
            ui.set_notes_backlinks_panel_open(true);
        }
        tracing::info!("Backlinks scan complete");
    });

    // ── Toggle meeting mode ──
    let cf = current_file.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_toggle_meeting_mode(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let current = ui.get_notes_meeting_mode();
            let new_mode = !current;
            ui.set_notes_meeting_mode(new_mode);

            if new_mode {
                // If current note is empty, pre-fill with meeting template
                let content = ui.get_notes_current_content().to_string();
                if content.trim().is_empty() || content.trim() == "# New Note" {
                    let today = {
                        let secs = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs();
                        let days = secs / 86400;
                        let (y, m, d) = days_to_ymd(days);
                        format!("{:04}-{:02}-{:02}", y, m, d)
                    };
                    let meeting_content = format!(
                        "# Meeting Notes\n\n**Date:** {}\n**Attendees:** \n\n## Agenda\n\n- \n\n## Notes\n\n\n\n## Action Items\n\n- [ ] \n\n## Decisions\n\n- ",
                        today
                    );
                    ui.set_notes_current_content(meeting_content.into());
                    ui.set_notes_is_modified(true);
                }
            }
        }
    });

    // ── Export MD ──
    let cf = current_file.clone();
    let ui_weak = ui.as_weak();
    ui.on_notes_export_md(move || {
        let path_str = cf.borrow().clone();
        if path_str.is_empty() { return; }
        if let Some(ui) = ui_weak.upgrade() {
            let content = ui.get_notes_current_content().to_string();
            let title = extract_title(&content);
            let slug = slugify(&title);

            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let docs_dir = PathBuf::from(&home).join("Documents");
            let _ = std::fs::create_dir_all(&docs_dir);

            let export_path = docs_dir.join(format!("{}.md", slug));
            match std::fs::write(&export_path, &content) {
                Ok(()) => {
                    tracing::info!(path = %export_path.display(), "Note exported");
                    ui.set_notes_export_status("Exported to ~/Documents/".into());

                    // Clear status after a brief moment (set via timer workaround)
                    // Note: Slint doesn't have timers in callbacks, so the status
                    // will persist until the next action clears it.
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to export note");
                    ui.set_notes_export_status("Export failed".into());
                }
            }
        }
    });

    // ── Import MD ──
    let ui_weak = ui.as_weak();
    ui.on_notes_import_md(move || {
        // Placeholder: real implementation would need a file picker dialog
        tracing::info!("Import MD requested — file picker not yet implemented");
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_notes_export_status("Import: file picker not yet available".into());
        }
    });
}

/// Load notes list when navigating to screen 15.
pub fn load_notes_list(ui: &App) {
    let dir = notes_directory();
    let _ = std::fs::create_dir_all(&dir);
    refresh_list(ui, &dir, 0);
    ui.set_notes_selected_index(-1);
    ui.set_notes_current_content("".into());
    ui.set_notes_current_title("".into());
    ui.set_notes_is_modified(false);
    ui.set_notes_active_folder(0);
    ui.set_notes_current_tags("".into());
    ui.set_notes_meta_created("".into());
    ui.set_notes_meta_modified("".into());
    ui.set_notes_meta_word_count(0);
    ui.set_notes_version_history(ModelRc::new(VecModel::from(Vec::<NoteVersionEntry>::new())));
    ui.set_notes_backlinks(ModelRc::new(VecModel::from(Vec::<NoteBacklink>::new())));
    ui.set_notes_version_panel_open(false);
    ui.set_notes_backlinks_panel_open(false);
    ui.set_notes_meeting_mode(false);
    ui.set_notes_export_status("".into());
}

// ── Helpers ──

use crate::{NoteEntry, NoteVersionEntry, NoteBacklink};

/// Get the notes storage directory.
fn notes_directory() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".local/share/yantrik/notes")
}

/// Refresh the note list and folder counts for the UI.
fn refresh_list(ui: &App, dir: &PathBuf, active_folder: i32) {
    let all = scan_notes(dir);

    // Compute folder counts
    let all_count = all.len() as i32;
    let fav_count = all.iter().filter(|e| e.is_pinned).count() as i32;
    // Recent = modified in last 7 days (use the filename timestamp prefix)
    let recent_count = count_recent(&all, dir);

    ui.set_notes_folder_all_count(all_count);
    ui.set_notes_folder_fav_count(fav_count);
    ui.set_notes_folder_recent_count(recent_count);

    let filtered = filter_by_folder(all, dir, active_folder);
    let count = filtered.len() as i32;
    ui.set_notes_list(ModelRc::new(VecModel::from(filtered)));
    ui.set_notes_note_count(count);
}

/// Filter entries by folder type.
fn filter_by_folder(mut entries: Vec<NoteEntry>, dir: &PathBuf, folder: i32) -> Vec<NoteEntry> {
    match folder {
        1 => {
            // Favorites: only pinned
            entries.retain(|e| e.is_pinned);
        }
        2 => {
            // Recent: modified in last 7 days
            let cutoff = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .saturating_sub(7 * 86400);
            entries.retain(|e| {
                let path = dir.join(&e.filename.to_string());
                path.metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() >= cutoff)
                    .unwrap_or(false)
            });
        }
        _ => {} // All notes — no filter
    }
    // Pinned notes always sort to top
    entries.sort_by(|a, b| {
        b.is_pinned.cmp(&a.is_pinned)
            .then_with(|| b.filename.cmp(&a.filename))
    });
    entries
}

/// Count notes modified in the last 7 days.
fn count_recent(entries: &[NoteEntry], dir: &PathBuf) -> i32 {
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(7 * 86400);
    entries.iter().filter(|e| {
        let path = dir.join(&e.filename.to_string());
        path.metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() >= cutoff)
            .unwrap_or(false)
    }).count() as i32
}

/// Update metadata display properties.
fn update_metadata(ui: &App, path: &std::path::Path, content: &str) {
    let word_count = content.split_whitespace().count() as i32;
    ui.set_notes_meta_word_count(word_count);

    if let Ok(metadata) = path.metadata() {
        if let Ok(modified) = metadata.modified() {
            ui.set_notes_meta_modified(format_datetime(modified).into());
        }
        // On some platforms created() is not available, fall back to modified
        let created = metadata.created().or_else(|_| metadata.modified());
        if let Ok(created) = created {
            ui.set_notes_meta_created(format_datetime(created).into());
        }
    }
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

        // Read meta sidecar
        let meta = read_meta(&path);

        // Get created time
        let created = entry
            .metadata()
            .ok()
            .and_then(|m| m.created().ok().or_else(|| m.modified().ok()))
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let word_count = content.split_whitespace().count() as i32;

        // Show first tag as chip preview in list
        let tag_preview: String = meta.tags.split(',').next().unwrap_or("").trim().to_string();

        entries.push(NoteEntry {
            title: title.into(),
            filename: filename.into(),
            modified: modified_text.into(),
            preview: preview.into(),
            is_pinned: meta.pinned,
            tags: tag_preview.into(),
            created: format_relative_time(created).into(),
            word_count,
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
        return "\u{2014}".to_string();
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

/// Format a SystemTime as a short date-time string (e.g., "2026-03-09 14:30").
fn format_datetime(time: std::time::SystemTime) -> String {
    let secs = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple UTC formatting (no chrono dependency)
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;

    // Calculate year/month/day from days since epoch
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", year, month, day, hours, minutes)
}

// ── Version management helpers ──

/// Get the path for a specific version of a note.
fn version_path(md_path: &std::path::Path, version: i32) -> PathBuf {
    let stem = md_path.file_stem().unwrap_or_default().to_string_lossy();
    let dir = md_path.parent().unwrap_or(std::path::Path::new("."));
    dir.join(format!("{}.v{}", stem, version))
}

/// Save a version snapshot before overwriting. Keeps max 10 versions, rotating old ones.
fn save_version(md_path: &std::path::Path, content: &str) {
    // Find next version number
    let stem = md_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let dir = md_path.parent().unwrap_or(std::path::Path::new("."));

    let mut max_ver = 0i32;
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(rest) = fname.strip_prefix(&format!("{}.", stem)) {
                if let Some(num_str) = rest.strip_prefix('v') {
                    if let Ok(n) = num_str.parse::<i32>() {
                        max_ver = max_ver.max(n);
                    }
                }
            }
        }
    }

    let next_ver = max_ver + 1;

    // If we'd exceed 10 versions, remove the oldest
    if next_ver > 10 {
        // Delete version 1 and shift all down
        for i in 1..next_ver {
            let old = version_path(md_path, i);
            let new = version_path(md_path, i - 1);
            if i == 1 {
                let _ = std::fs::remove_file(&old);
            } else {
                let _ = std::fs::rename(&old, &new);
            }
        }
        // Write as the last slot
        let ver_path = version_path(md_path, std::cmp::min(next_ver - 1, 10));
        let _ = std::fs::write(&ver_path, content);
    } else {
        let ver_path = version_path(md_path, next_ver);
        let _ = std::fs::write(&ver_path, content);
    }

    tracing::debug!(version = next_ver, "Version snapshot saved");
}

/// Load version history entries for a note file.
fn load_version_history(md_path: &std::path::Path) -> Vec<crate::NoteVersionEntry> {
    let stem = md_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let dir = md_path.parent().unwrap_or(std::path::Path::new("."));

    let mut versions = Vec::new();

    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(rest) = fname.strip_prefix(&format!("{}.", stem)) {
                if let Some(num_str) = rest.strip_prefix('v') {
                    if let Ok(n) = num_str.parse::<i32>() {
                        let ts = entry
                            .metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .map(|t| format_datetime(t))
                            .unwrap_or_else(|| "\u{2014}".to_string());

                        versions.push(crate::NoteVersionEntry {
                            label: format!("Version {}", n).into(),
                            timestamp: ts.into(),
                            index: n,
                        });
                    }
                }
            }
        }
    }

    // Sort by version number descending (newest first)
    versions.sort_by(|a, b| b.index.cmp(&a.index));
    versions
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from Howard Hinnant
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
