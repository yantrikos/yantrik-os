//! Notes service — markdown note CRUD, search, tagging via filesystem.
//!
//! Notes stored as `.md` files in `~/.local/share/yantrik/notes/`.
//! Metadata (pinned, tags) stored in `.meta` sidecar files.
//!
//! Methods:
//!   notes.list       { folder? }                → Vec<NoteSummary>
//!   notes.get        { id }                     → NoteContent
//!   notes.create     { title, body?, tags? }    → NoteContent
//!   notes.update     { id, title, body }        → ()
//!   notes.delete     { id }                     → ()
//!   notes.set_pinned { id, pinned }             → ()
//!   notes.set_tags   { id, tags }               → ()
//!   notes.search     { query }                  → Vec<NoteSummary>

use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::notes::*;
use yantrik_service_sdk::prelude::*;

fn main() {
    std::fs::create_dir_all(notes_dir()).ok();

    ServiceBuilder::new("notes")
        .handler(NotesHandler { dir: notes_dir() })
        .run();
}

fn notes_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".local/share/yantrik/notes")
    } else {
        PathBuf::from("/tmp/yantrik-notes")
    }
}

struct NotesHandler {
    dir: PathBuf,
}

impl ServiceHandler for NotesHandler {
    fn service_id(&self) -> &str {
        "notes"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "notes.list" => {
                let folder = params["folder"].as_str();
                let notes = self.list_notes(folder)?;
                Ok(serde_json::to_value(notes).unwrap())
            }
            "notes.get" => {
                let id = require_str(&params, "id")?;
                let note = self.get_note(id)?;
                Ok(serde_json::to_value(note).unwrap())
            }
            "notes.create" => {
                let title = require_str(&params, "title")?;
                let body = params["body"].as_str().unwrap_or("");
                let tags: Vec<String> = params["tags"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let note = self.create_note(title, body, tags)?;
                Ok(serde_json::to_value(note).unwrap())
            }
            "notes.update" => {
                let id = require_str(&params, "id")?;
                let title = require_str(&params, "title")?;
                let body = require_str(&params, "body")?;
                self.update_note(id, title, body)?;
                Ok(serde_json::json!(null))
            }
            "notes.delete" => {
                let id = require_str(&params, "id")?;
                self.delete_note(id)?;
                Ok(serde_json::json!(null))
            }
            "notes.set_pinned" => {
                let id = require_str(&params, "id")?;
                let pinned = params["pinned"].as_bool().unwrap_or(false);
                self.set_pinned(id, pinned)?;
                Ok(serde_json::json!(null))
            }
            "notes.set_tags" => {
                let id = require_str(&params, "id")?;
                let tags: Vec<String> = params["tags"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                self.set_tags(id, tags)?;
                Ok(serde_json::json!(null))
            }
            "notes.search" => {
                let query = require_str(&params, "query")?;
                let results = self.search_notes(query)?;
                Ok(serde_json::to_value(results).unwrap())
            }
            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method}"),
            }),
        }
    }
}

fn require_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, ServiceError> {
    params[key].as_str().ok_or_else(|| ServiceError {
        code: -32602,
        message: format!("Missing '{key}' parameter"),
    })
}

// ── Note metadata sidecar ────────────────────────────────────────────

#[derive(Default)]
struct NoteMeta {
    pinned: bool,
    tags: Vec<String>,
}

fn meta_path(md_path: &Path) -> PathBuf {
    md_path.with_extension("meta")
}

fn read_meta(md_path: &Path) -> NoteMeta {
    let mp = meta_path(md_path);
    let content = std::fs::read_to_string(&mp).unwrap_or_default();
    let mut meta = NoteMeta::default();
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("pinned:") {
            meta.pinned = v.trim() == "true";
        } else if let Some(v) = line.strip_prefix("tags:") {
            let tags_str = v.trim();
            if !tags_str.is_empty() {
                meta.tags = tags_str.split(',').map(|s| s.trim().to_string()).collect();
            }
        }
    }
    meta
}

fn write_meta(md_path: &Path, meta: &NoteMeta) {
    let mp = meta_path(md_path);
    let content = format!(
        "pinned:{}\ntags:{}\n",
        meta.pinned,
        meta.tags.join(",")
    );
    let _ = std::fs::write(&mp, content);
}

// ── CRUD implementation ──────────────────────────────────────────────

impl NotesHandler {
    fn note_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.md"))
    }

    fn list_notes(&self, _folder: Option<&str>) -> Result<Vec<NoteSummary>, ServiceError> {
        let entries = std::fs::read_dir(&self.dir).map_err(|e| ServiceError {
            code: -32000,
            message: format!("Cannot read notes dir: {e}"),
        })?;

        let mut notes = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            if let Some(summary) = self.read_summary(&path) {
                notes.push(summary);
            }
        }

        // Sort by modified_at descending
        notes.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        Ok(notes)
    }

    fn read_summary(&self, path: &Path) -> Option<NoteSummary> {
        let content = std::fs::read_to_string(path).ok()?;
        let meta = read_meta(path);
        let id = path.file_stem()?.to_string_lossy().to_string();

        let title = content
            .lines()
            .next()
            .unwrap_or("Untitled")
            .trim_start_matches('#')
            .trim()
            .to_string();

        let snippet: String = content
            .lines()
            .skip(1)
            .take(3)
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(200)
            .collect();

        let file_meta = std::fs::metadata(path).ok()?;
        let modified = file_meta
            .modified()
            .ok()
            .map(|t| {
                let d: chrono::DateTime<chrono::Utc> = t.into();
                d.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_default();

        let created = file_meta
            .created()
            .ok()
            .map(|t| {
                let d: chrono::DateTime<chrono::Utc> = t.into();
                d.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_default();

        let word_count = content.split_whitespace().count() as u32;

        Some(NoteSummary {
            id,
            title,
            snippet,
            modified_at: modified,
            created_at: created,
            pinned: meta.pinned,
            tags: meta.tags,
            word_count,
        })
    }

    fn get_note(&self, id: &str) -> Result<NoteContent, ServiceError> {
        let path = self.note_path(id);
        let body = std::fs::read_to_string(&path).map_err(|e| ServiceError {
            code: -32000,
            message: format!("Note not found: {e}"),
        })?;

        let meta = read_meta(&path);
        let file_meta = std::fs::metadata(&path).map_err(|e| ServiceError {
            code: -32000,
            message: format!("Cannot read note metadata: {e}"),
        })?;

        let title = body
            .lines()
            .next()
            .unwrap_or("Untitled")
            .trim_start_matches('#')
            .trim()
            .to_string();

        let modified = file_meta
            .modified()
            .ok()
            .map(|t| {
                let d: chrono::DateTime<chrono::Utc> = t.into();
                d.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_default();

        let created = file_meta
            .created()
            .ok()
            .map(|t| {
                let d: chrono::DateTime<chrono::Utc> = t.into();
                d.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_default();

        Ok(NoteContent {
            id: id.to_string(),
            title,
            body,
            modified_at: modified,
            created_at: created,
            pinned: meta.pinned,
            tags: meta.tags,
        })
    }

    fn create_note(
        &self,
        title: &str,
        body: &str,
        tags: Vec<String>,
    ) -> Result<NoteContent, ServiceError> {
        let id = uuid7::uuid7().to_string();
        let path = self.note_path(&id);

        let content = if body.is_empty() {
            format!("# {title}\n\n")
        } else {
            format!("# {title}\n\n{body}")
        };

        std::fs::write(&path, &content).map_err(|e| ServiceError {
            code: -32000,
            message: format!("Failed to create note: {e}"),
        })?;

        let meta = NoteMeta {
            pinned: false,
            tags: tags.clone(),
        };
        write_meta(&path, &meta);

        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string();

        Ok(NoteContent {
            id,
            title: title.to_string(),
            body: content,
            modified_at: now.clone(),
            created_at: now,
            pinned: false,
            tags,
        })
    }

    fn update_note(&self, id: &str, title: &str, body: &str) -> Result<(), ServiceError> {
        let path = self.note_path(id);
        if !path.exists() {
            return Err(ServiceError {
                code: -32000,
                message: format!("Note not found: {id}"),
            });
        }

        let content = format!("# {title}\n\n{body}");
        std::fs::write(&path, content).map_err(|e| ServiceError {
            code: -32000,
            message: format!("Failed to update note: {e}"),
        })?;
        Ok(())
    }

    fn delete_note(&self, id: &str) -> Result<(), ServiceError> {
        let path = self.note_path(id);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(meta_path(&path));
        Ok(())
    }

    fn set_pinned(&self, id: &str, pinned: bool) -> Result<(), ServiceError> {
        let path = self.note_path(id);
        if !path.exists() {
            return Err(ServiceError {
                code: -32000,
                message: format!("Note not found: {id}"),
            });
        }
        let mut meta = read_meta(&path);
        meta.pinned = pinned;
        write_meta(&path, &meta);
        Ok(())
    }

    fn set_tags(&self, id: &str, tags: Vec<String>) -> Result<(), ServiceError> {
        let path = self.note_path(id);
        if !path.exists() {
            return Err(ServiceError {
                code: -32000,
                message: format!("Note not found: {id}"),
            });
        }
        let mut meta = read_meta(&path);
        meta.tags = tags;
        write_meta(&path, &meta);
        Ok(())
    }

    fn search_notes(&self, query: &str) -> Result<Vec<NoteSummary>, ServiceError> {
        let all = self.list_notes(None)?;
        let query_lower = query.to_lowercase();

        Ok(all
            .into_iter()
            .filter(|n| {
                n.title.to_lowercase().contains(&query_lower)
                    || n.snippet.to_lowercase().contains(&query_lower)
                    || n.tags.iter().any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect())
    }
}
