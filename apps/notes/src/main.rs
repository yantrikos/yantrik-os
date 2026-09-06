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

    // Held until the window closes: a dropped Timer stops firing.
    let _vault_watch = wire(&app);
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

/// Search the vault directly, for when notes-service is not running.
///
/// Without this, a failed service call left the list untouched: the search box showed a query and
/// the list showed everything, with nothing to say the search had not happened. Every other read
/// in this file already falls back to the filesystem; search was the one that did not.
fn search_fs(query: &str) -> Vec<NoteEntry> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return scan_notes_fs();
    }
    scan_notes_fs()
        .into_iter()
        .filter(|e| {
            if e.title.to_lowercase().contains(&needle) {
                return true;
            }
            // Body too — a note is usually easier to find by something it says than by its title.
            std::fs::read_to_string(notes_dir().join(e.filename.as_str()))
                .map(|body| body.to_lowercase().contains(&needle))
                .unwrap_or(false)
        })
        .collect()
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
        // `![[shot.png]]` is an image embed, not a link to a note.
        if start > 0 && text.as_bytes()[start - 1] == b'!' {
            continue;
        }
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

struct VaultFile {
    filename: String,
    title: String,
    content: String,
}

/// Read the whole vault once. Both link directions need titles, and one of them needs bodies,
/// so doing this per link would read every file N times.
fn read_vault() -> Vec<VaultFile> {
    let dir = notes_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().map(|e| e != "md").unwrap_or(true) {
                return None;
            }
            let filename = path.file_name()?.to_string_lossy().to_string();
            let content = std::fs::read_to_string(&path).ok()?;
            Some(VaultFile {
                title: title_of(&content, &filename),
                filename,
                content,
            })
        })
        .collect()
}

/// Notes that link *to* `target_file`.
fn backlinks_in(vault: &[VaultFile], target_file: &str) -> Vec<NoteBacklink> {
    let target = vault.iter().find(|f| f.filename == target_file);
    let keys = [
        link_key(target.map(|f| f.title.as_str()).unwrap_or(target_file)),
        link_key(target_file),
    ];

    let mut out: Vec<NoteBacklink> = vault
        .iter()
        // A note linking to itself is not a backlink.
        .filter(|f| f.filename != target_file)
        .filter(|f| wikilinks(&f.content).iter().any(|l| keys.contains(&link_key(l))))
        .map(|f| NoteBacklink {
            title: f.title.clone().into(),
            filename: f.filename.clone().into(),
        })
        .collect();
    out.sort_by_key(|b| b.title.to_lowercase());
    out
}

/// Where this note points. A target with no note yet is kept, with an empty `filename` — that is
/// a dangling link, and it is the most useful thing on the panel: it is work promised and not
/// yet done.
fn outbound_in(vault: &[VaultFile], content: &str) -> Vec<NoteBacklink> {
    let mut out: Vec<NoteBacklink> = Vec::new();
    for target in wikilinks(content) {
        let key = link_key(&target);
        let hit = vault
            .iter()
            .find(|f| link_key(&f.title) == key || link_key(&f.filename) == key);
        let entry = match hit {
            Some(f) => NoteBacklink {
                title: f.title.clone().into(),
                filename: f.filename.clone().into(),
            },
            None => NoteBacklink {
                title: target.clone().into(),
                filename: "".into(),
            },
        };
        if !out.iter().any(|e| e.title == entry.title && e.filename == entry.filename) {
            out.push(entry);
        }
    }
    out
}

/// Both directions for the note in front of you, from a single read of the vault.
fn links_for(target_file: &str, content: &str) -> (Vec<NoteBacklink>, Vec<NoteBacklink>) {
    let vault = read_vault();
    (backlinks_in(&vault, target_file), outbound_in(&vault, content))
}

/// What a directory scan can see without opening a file. Millisecond mtimes plus size and count
/// catch a new note, a deleted one, and an edit.
fn vault_fingerprint() -> (usize, u64, u64) {
    let (mut count, mut newest, mut bytes) = (0usize, 0u64, 0u64);
    if let Ok(entries) = std::fs::read_dir(notes_dir()) {
        for entry in entries.flatten() {
            if entry.path().extension().map(|e| e != "md").unwrap_or(true) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            count += 1;
            bytes += meta.len();
            if let Ok(t) = meta.modified() {
                if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                    newest = newest.max(d.as_millis() as u64);
                }
            }
        }
    }
    (count, newest, bytes)
}

/// Row of `filename` in the list as it currently stands.
fn index_of(ui: &NotesApp, filename: &str) -> Option<i32> {
    let model = ui.get_notes_list();
    (0..model.row_count()).find_map(|i| {
        let entry = model.row_data(i)?;
        (entry.filename.as_str() == filename).then_some(i as i32)
    })
}

/// Refresh whichever side panels are open for `id`.
fn refresh_panels(ui: &NotesApp, id: &str) {
    if ui.get_backlinks_panel_open() {
        let (inbound, outbound) = links_for(id, &ui.get_current_content().to_string());
        ui.set_backlinks(ModelRc::new(VecModel::from(inbound)));
        ui.set_outbound_links(ModelRc::new(VecModel::from(outbound)));
    }
    if ui.get_images_panel_open() {
        ui.set_note_images(ModelRc::new(VecModel::from(images_for(
            &ui.get_current_content().to_string(),
        ))));
    }
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

/// Where attached files live. Obsidian's convention, and it keeps the vault root readable.
fn attachments_dir() -> PathBuf {
    notes_dir().join("attachments")
}

/// A name that is free in `dir`: `shot.png`, then `shot-1.png`, and so on. Never overwrite an
/// existing attachment — a different note may already embed it.
fn unique_attachment_name(dir: &std::path::Path, base: &str) -> String {
    if !dir.join(base).exists() {
        return base.to_string();
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => (base, ""),
    };
    for n in 1.. {
        let candidate = if ext.is_empty() {
            format!("{stem}-{n}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn is_image_name(name: &str) -> bool {
    let n = name.to_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".svg"]
        .iter()
        .any(|e| n.ends_with(e))
}

/// Every image a note embeds, in order, as written. Understands both
/// `![[shot.png]]` and `![alt](attachments/shot.png)`.
fn image_refs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for (start, _) in text.match_indices("![[") {
        let rest = &text[start + 3..];
        if let Some(end) = rest.find("]]") {
            let name = rest[..end].split('|').next().unwrap_or("").trim();
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }

    // ![alt](target) — the alt text is decoration, the target is the file.
    for (start, _) in text.match_indices("](") {
        // Only when it is an image: the run before `](` has to open with `![`.
        let head = &text[..start];
        let Some(open) = head.rfind("![") else { continue };
        if head[open..].contains(']') {
            continue;
        }
        let rest = &text[start + 2..];
        if let Some(end) = rest.find(')') {
            let target = rest[..end].split_whitespace().next().unwrap_or("").trim();
            if !target.is_empty() && !target.starts_with("http") {
                out.push(target.to_string());
            }
        }
    }

    out.dedup();
    out
}

/// Resolve an embed against the vault: attachments first, then the vault root, then treat it
/// as a path in its own right.
fn resolve_image(reference: &str) -> Option<PathBuf> {
    let candidates = [
        attachments_dir().join(reference),
        notes_dir().join(reference),
        PathBuf::from(reference),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Load what the current note embeds. A reference that does not resolve is still listed, marked
/// `found: false` — a silently missing picture is worse than a visibly missing one.
fn images_for(content: &str) -> Vec<NoteImage> {
    image_refs(content)
        .into_iter()
        .map(|reference| {
            let resolved = resolve_image(&reference);
            let (source, found) = match resolved.as_ref() {
                Some(p) => match slint::Image::load_from_path(p) {
                    Ok(img) => (img, true),
                    Err(e) => {
                        tracing::warn!(image = %p.display(), error = ?e, "Could not decode image");
                        (slint::Image::default(), false)
                    }
                },
                None => (slint::Image::default(), false),
            };
            NoteImage {
                name: reference
                    .rsplit('/')
                    .next()
                    .unwrap_or(&reference)
                    .to_string()
                    .into(),
                path: resolved
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| reference.clone())
                    .into(),
                source,
                found,
            }
        })
        .collect()
}

/// The two things Notes asks the companion for.
#[derive(Clone, Copy)]
enum AiAction {
    Structure,
    Summarize,
}

impl AiAction {
    fn prompt(self, title: &str, body: &str) -> String {
        match self {
            AiAction::Structure => format!(
                "Reorganise the following note into clear markdown sections with headings. \
                 Keep every fact; do not invent anything. Reply with the note only.\n\n\
                 # {title}\n\n{body}"
            ),
            AiAction::Summarize => format!(
                "Summarise the following note in at most five bullet points. \
                 Use only what the note says. Reply with the bullets only.\n\n\
                 # {title}\n\n{body}"
            ),
        }
    }
}

/// Ask the companion on a worker thread and hand the answer back to the UI thread.
fn wire_ai_action(app: &NotesApp, current_file: Rc<RefCell<String>>, action: AiAction) {
    let weak = app.as_weak();
    let handler = move || {
        let Some(ui) = weak.upgrade() else { return };
        if current_file.borrow().is_empty() {
            return;
        }
        let title = ui.get_current_title().to_string();
        let body = ui.get_current_content().to_string();
        if body.trim().is_empty() {
            ui.set_ai_response("This note is empty.".into());
            ui.set_ai_panel_open(true);
            return;
        }

        ui.set_ai_is_working(true);
        ui.set_ai_panel_open(true);
        ui.set_ai_response("".into());

        let prompt = action.prompt(&title, &body);
        let back = ui.as_weak();
        std::thread::spawn(move || {
            let outcome = companion::ask(&prompt);
            // upgrade_in_event_loop hops back to the UI thread; touching the UI from here
            // would be a data race.
            let _ = back.upgrade_in_event_loop(move |ui| {
                ui.set_ai_is_working(false);
                match outcome {
                    Ok(text) => ui.set_ai_response(text.into()),
                    Err(e) => {
                        tracing::warn!(error = %e, "Companion call failed");
                        ui.set_ai_response(
                            format!("The companion did not answer: {e}\n\nIs the Yantrik shell running?")
                                .into(),
                        );
                    }
                }
            });
        });
    };

    match action {
        AiAction::Structure => app.on_ai_structure(handler),
        AiAction::Summarize => app.on_ai_summarize(handler),
    }
}

// ── The control surface ──────────────────────────────────────────────
//
// What the companion can see of Notes, and what it can ask Notes to do. Before this, the only
// way for it to know which note was open was to screenshot the window and send the pixels to a
// vision model — for our own software, which knows the answer exactly.
//
// Every action below calls the callback the button calls. That is deliberate: one code path, so
// an action cannot drift away from what the app actually does, and driving Notes needs no
// synthetic mouse.

/// Filename of the row whose title (or filename) matches `needle`, case-insensitively.
///
/// Callers name notes the way a person would — by title — while the app addresses them by
/// filename. Exact title first, then filename, then a contains-match, so a half-remembered name
/// still lands.
fn note_named(ui: &NotesApp, needle: &str) -> Option<String> {
    let want = needle.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    let model = ui.get_notes_list();
    let rows: Vec<NoteEntry> = (0..model.row_count()).filter_map(|i| model.row_data(i)).collect();

    let exact = rows.iter().find(|e| {
        e.title.to_lowercase() == want || e.filename.to_lowercase() == want
    });
    exact
        .or_else(|| rows.iter().find(|e| e.title.to_lowercase().contains(&want)))
        .map(|e| e.filename.to_string())
}

fn publish_control(app: &NotesApp, current_file: Rc<RefCell<String>>) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let weak = app.as_weak();
        let cf = current_file.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Notes — closing");
            };
            let open = cf.borrow().clone();
            let title = ui.get_current_title().to_string();
            let modified = ui.get_is_modified();
            let words = ui.get_meta_word_count();

            let summary = if open.is_empty() {
                format!("Notes — no note open, {} in the vault", ui.get_note_count())
            } else {
                format!(
                    "Notes — {}\u{201c}{title}\u{201d}, {words} words{}",
                    if modified { "editing " } else { "" },
                    if modified { ", unsaved" } else { "" }
                )
            };

            // The titles, not the bodies. A caller that wants a body asks for the note; this is
            // the glance, and it has to stay small enough to send on every turn.
            let model = ui.get_notes_list();
            let listed: Vec<serde_json::Value> = (0..model.row_count().min(50))
                .filter_map(|i| model.row_data(i))
                .map(|e| {
                    serde_json::json!({
                        "title": e.title.to_string(),
                        "filename": e.filename.to_string(),
                        "pinned": e.is_pinned,
                    })
                })
                .collect();

            View::new(summary)
                .with("open_note", if open.is_empty() { serde_json::Value::Null } else { open.clone().into() })
                .with("title", title)
                .with("unsaved", modified)
                .with("word_count", words)
                .with("note_count", ui.get_note_count())
                .with("folder", match ui.get_active_folder() {
                    1 => "favorites",
                    2 => "recent",
                    _ => "all",
                })
                .with("search_query", ui.get_search_query().to_string())
                .with("notes", serde_json::Value::Array(listed))
        }
    };

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "Notes window is gone".to_string());

    let open_ui = ui_for.clone();
    let new_ui = ui_for.clone();
    let save_ui = ui_for.clone();
    let append_ui = ui_for.clone();
    let search_ui = ui_for.clone();
    let folder_ui = ui_for;
    let append_file = current_file;

    App::new("notes")
        .describe(describe)
        .action(
            Action::new("open_note", "Open a note in the editor, by title or filename")
                .arg(Param::text("title").describe("The note's title, or its filename")),
            move |args| {
                let ui = open_ui()?;
                let want = args["title"].as_str().unwrap_or_default();
                let filename = note_named(&ui, want)
                    .ok_or_else(|| format!("no note here is called \"{want}\""))?;
                // The same callback the backlinks panel calls.
                ui.invoke_open_note(filename.clone().into());
                Ok(serde_json::json!({ "opened": filename, "title": ui.get_current_title().to_string() }))
            },
        )
        .action(
            Action::new("new_note", "Start a new note and open it for editing"),
            move |_args| {
                let ui = new_ui()?;
                ui.invoke_new_note();
                Ok(serde_json::json!({ "title": ui.get_current_title().to_string() }))
            },
        )
        .action(
            Action::new("save", "Write the open note to disk"),
            move |_args| {
                let ui = save_ui()?;
                if ui.get_current_title().is_empty() {
                    return Err("no note is open".into());
                }
                ui.invoke_save_note();
                Ok(serde_json::json!({ "saved": ui.get_current_title().to_string() }))
            },
        )
        .action(
            Action::new("append", "Add text to the end of the open note and save it")
                .arg(Param::text("text").describe("Markdown to append")),
            move |args| {
                let ui = append_ui()?;
                if append_file.borrow().is_empty() {
                    return Err("no note is open; call new_note or open_note first".into());
                }
                let addition = args["text"].as_str().unwrap_or_default();
                if addition.trim().is_empty() {
                    return Err("`text` is empty".into());
                }
                let mut content = ui.get_current_content().to_string();
                if !content.is_empty() && !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push_str(addition);
                if !content.ends_with('\n') {
                    content.push('\n');
                }
                ui.set_meta_word_count(content.split_whitespace().count() as i32);
                ui.set_current_content(content.into());
                ui.set_is_modified(true);
                // Saved, unlike the AI suggestions in the panel: this text was asked for
                // explicitly by name, not proposed for review.
                ui.invoke_save_note();
                Ok(serde_json::json!({ "appended_chars": addition.len() }))
            },
        )
        .action(
            Action::new("search", "Filter the note list, and report what matched")
                .arg(Param::text("query")),
            move |args| {
                let ui = search_ui()?;
                let query = args["query"].as_str().unwrap_or_default().to_string();
                ui.set_search_query(query.clone().into());
                ui.invoke_search_notes(query.into());
                let model = ui.get_notes_list();
                let hits: Vec<String> = (0..model.row_count().min(25))
                    .filter_map(|i| model.row_data(i))
                    .map(|e| e.title.to_string())
                    .collect();
                Ok(serde_json::json!({ "matched": ui.get_note_count(), "titles": hits }))
            },
        )
        .action(
            Action::new("set_folder", "Switch the list between all, favorites and recent")
                .arg(Param::text("folder").describe("all | favorites | recent")),
            move |args| {
                let ui = folder_ui()?;
                let index = match args["folder"].as_str().unwrap_or_default().to_lowercase().as_str() {
                    "all" => 0,
                    "favorites" | "favourites" | "pinned" => 1,
                    "recent" => 2,
                    other => return Err(format!("unknown folder `{other}`; use all, favorites or recent")),
                };
                ui.invoke_select_folder(index);
                Ok(serde_json::json!({ "showing": ui.get_note_count() }))
            },
        )
        .serve();
}

fn wire(app: &NotesApp) -> slint::Timer {
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
            ui.set_image_preview_index(0);
            refresh_panels(&ui, &id);
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
            let results = search_via_service(&q).unwrap_or_else(|_| search_fs(&q));
            let count = results.len() as i32;
            ui.set_notes_list(ModelRc::new(VecModel::from(results)));
            ui.set_note_count(count);
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
    // ── AI, via the companion in the shell ──
    //
    // These were stubs: the model, the memory and the bond live in the shell process. They are
    // now RPC calls on the same bus the services use. The work happens on a worker thread —
    // asking an LLM on the UI thread would freeze the window for the length of the answer.
    wire_ai_action(app, current_file.clone(), AiAction::Structure);
    wire_ai_action(app, current_file.clone(), AiAction::Summarize);

    // Apply: drop the suggestion into the note, where the author can edit or undo it.
    {
        let weak = app.as_weak();
        app.on_ai_apply(move || {
            let Some(ui) = weak.upgrade() else { return };
            let suggestion = ui.get_ai_response().to_string();
            if suggestion.is_empty() {
                return;
            }
            let mut content = ui.get_current_content().to_string();
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&format!("\n{suggestion}\n"));
            ui.set_current_content(content.clone().into());
            ui.set_meta_word_count(content.split_whitespace().count() as i32);
            // Left unsaved on purpose: generated text should be looked at before it is kept.
            ui.set_is_modified(true);
            ui.set_ai_panel_open(false);
            ui.set_ai_response("".into());
        });
    }

    {
        let weak = app.as_weak();
        app.on_ai_dismiss(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_ai_response("".into());
            ui.set_ai_panel_open(false);
        });
    }
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
            let (inbound, outbound) = links_for(&id, &ui.get_current_content().to_string());
            tracing::info!(
                note = %id,
                links_in = inbound.len(),
                links_out = outbound.len(),
                "Scanned the vault for links"
            );
            ui.set_backlinks(ModelRc::new(VecModel::from(inbound)));
            ui.set_outbound_links(ModelRc::new(VecModel::from(outbound)));
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
                    ui.set_image_preview_index(0);
                    refresh_panels(&ui, &name);
                    return;
                }
            }
            tracing::warn!(note = %name, "Backlink target is not in the current list");
        });
    }
    // ── Images ──
    {
        let weak = app.as_weak();
        app.on_scan_images(move || {
            let Some(ui) = weak.upgrade() else { return };
            let found = images_for(&ui.get_current_content().to_string());
            tracing::info!(images = found.len(), "Scanned note for images");
            if ui.get_image_preview_index() as usize >= found.len() {
                ui.set_image_preview_index(0);
            }
            ui.set_note_images(ModelRc::new(VecModel::from(found)));
        });
    }

    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        app.on_attach_image(move |raw_path| {
            let Some(ui) = weak.upgrade() else { return };
            let id = cf.borrow().clone();
            if id.is_empty() {
                return;
            }

            let mut given = raw_path.to_string().trim().to_string();
            if let Some(rest) = given.strip_prefix("~/") {
                if let Ok(home) = std::env::var("HOME") {
                    given = format!("{home}/{rest}");
                }
            }
            let src = PathBuf::from(&given);
            if !src.is_file() {
                tracing::warn!(path = %given, "Attach: no such file");
                return;
            }
            let Some(base) = src.file_name().map(|n| n.to_string_lossy().to_string()) else { return };
            if !is_image_name(&base) {
                tracing::warn!(path = %given, "Attach: not an image");
                return;
            }

            // Never overwrite an existing attachment; a note elsewhere may embed it.
            let dir = attachments_dir();
            if std::fs::create_dir_all(&dir).is_err() {
                tracing::error!(dir = %dir.display(), "Attach: cannot create attachments dir");
                return;
            }
            let name = unique_attachment_name(&dir, &base);
            if let Err(e) = std::fs::copy(&src, dir.join(&name)) {
                tracing::error!(error = ?e, "Attach: copy failed");
                return;
            }

            // Embed it, then save through the same path the Save button uses, so the reference
            // and the file land together.
            let mut content = ui.get_current_content().to_string();
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&format!("\n![[{name}]]\n"));
            let title = ui.get_current_title().to_string();
            if update_via_service(&id, &title, &content).is_err() {
                let _ = std::fs::write(notes_dir().join(&id), &content);
            }
            ui.set_current_content(content.clone().into());
            ui.set_meta_word_count(content.split_whitespace().count() as i32);
            ui.set_is_modified(false);
            ui.set_note_images(ModelRc::new(VecModel::from(images_for(&content))));
            tracing::info!(image = %name, "Attached image");
        });
    }

    app.on_toggle_meeting_mode(|| {});
    app.on_import_md(|| {});

    // Published last: everything the surface reports is wired by now, so the first
    // `app.describe` cannot catch a half-built window.
    publish_control(app, current_file.clone());

    // ── Watch the vault ──
    //
    // Something other than this app writes here: an agent, a sync tool, another editor. A poll
    // is enough — a directory stat costs nothing next to a note that never shows up — and it
    // keeps everything on the UI thread, with no channel to drain.
    let watch = slint::Timer::default();
    {
        let weak = app.as_weak();
        let cf = current_file.clone();
        let seen = std::cell::RefCell::new(vault_fingerprint());
        watch.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(2),
            move || {
                let now = vault_fingerprint();
                if *seen.borrow() == now {
                    return;
                }
                *seen.borrow_mut() = now;

                let Some(ui) = weak.upgrade() else { return };
                tracing::info!("Vault changed on disk, reloading");
                refresh_list(&ui, ui.get_active_folder());

                let id = cf.borrow().clone();
                if id.is_empty() {
                    return;
                }
                match index_of(&ui, &id) {
                    // Never overwrite an unsaved buffer: the person typing wins over the file.
                    // Keep the selection pointing at the right row, which may have moved.
                    Some(i) if ui.get_is_modified() => ui.set_selected_index(i),
                    Some(i) => {
                        load_note(&ui, i, &id);
                        refresh_panels(&ui, &id);
                    }
                    None => {
                        // The open note was deleted from under us.
                        tracing::info!(note = %id, "Open note disappeared from the vault");
                        cf.borrow_mut().clear();
                        ui.set_selected_index(-1);
                        ui.set_current_content("".into());
                        ui.set_current_title("".into());
                    }
                }
            },
        );
    }
    watch
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wikilinks_reads_targets_and_aliases() {
        let links = wikilinks("see [[Build times]] and [[notes/other.md|that one]]");
        assert_eq!(links, vec!["Build times", "notes/other.md"]);
    }

    #[test]
    fn an_image_embed_is_not_a_note_link() {
        // `![[shot.png]]` shares its opening bracket run with a wikilink; treating it as one
        // would put a phantom note in every backlink list.
        let links = wikilinks("![[shot.png]] but [[Real note]] counts");
        assert_eq!(links, vec!["Real note"]);
    }

    #[test]
    fn links_resolve_regardless_of_case_or_extension() {
        assert_eq!(link_key("Build times"), link_key("build TIMES"));
        assert_eq!(link_key("build-times.md"), link_key("build-times"));
    }

    #[test]
    fn title_comes_from_the_first_heading() {
        assert_eq!(title_of("# Real title\n\nbody", "slug.md"), "Real title");
        assert_eq!(title_of("no heading here", "slug.md"), "slug");
        // An empty heading is not a title.
        assert_eq!(title_of("# \n\nbody", "slug.md"), "slug");
    }

    #[test]
    fn image_refs_understands_both_syntaxes() {
        let refs = image_refs("![[a.png]] then ![alt text](attachments/b.jpg) done");
        assert_eq!(refs, vec!["a.png", "attachments/b.jpg"]);
    }

    #[test]
    fn image_refs_skips_remote_and_plain_links() {
        // A plain link is not an embed, and a remote image is not in the vault.
        let refs = image_refs("[a note](other.md) and ![remote](https://example.com/x.png)");
        assert!(refs.is_empty(), "got {refs:?}");
    }

    #[test]
    fn attachment_names_never_collide() {
        let dir = std::env::temp_dir().join(format!("yantrik-notes-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(unique_attachment_name(&dir, "shot.png"), "shot.png");
        std::fs::write(dir.join("shot.png"), b"x").unwrap();
        assert_eq!(unique_attachment_name(&dir, "shot.png"), "shot-1.png");
        std::fs::write(dir.join("shot-1.png"), b"x").unwrap();
        assert_eq!(unique_attachment_name(&dir, "shot.png"), "shot-2.png");

        std::fs::remove_dir_all(&dir).ok();
    }
}
