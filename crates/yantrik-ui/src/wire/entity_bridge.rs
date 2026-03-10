//! Entity Bridge — registers app objects in the unified entity graph.
//!
//! Each app's wire module calls these helpers after CRUD operations to keep
//! the entity graph in sync. This is the connective tissue between individual
//! apps and the cross-app intelligence layer.
//!
//! Usage from app wire modules:
//! ```ignore
//! // In wire/email.rs, after fetching emails:
//! entity_bridge::register_email(&graph, &email);
//!
//! // In wire/notes.rs, after saving a note:
//! entity_bridge::register_note(&graph, filename, title, content);
//! ```

use std::sync::{Arc, Mutex};

use yantrik_os::entity_graph::{EntityGraph, ObjectKind, RelationKind, UniversalObject};

/// Shared entity graph handle (Arc<Mutex<EntityGraph>>).
pub type SharedEntityGraph = Arc<Mutex<EntityGraph>>;

// ────────────────────────────────────────────────────────────────────────────
// Email
// ────────────────────────────────────────────────────────────────────────────

/// Register an email thread/message in the entity graph.
pub fn register_email(
    graph: &SharedEntityGraph,
    message_id: &str,
    from: &str,
    subject: &str,
    preview: &str,
    date: &str,
) -> Option<String> {
    let obj = UniversalObject::new(ObjectKind::Thread, subject, "email", message_id)
        .with_summary(format!("From: {} — {}", from, preview))
        .with_searchable_text(format!("{} {} {} {}", subject, from, preview, date))
        .with_metadata(serde_json::json!({
            "from": from,
            "date": date,
        }));

    if let Ok(g) = graph.lock() {
        match g.upsert_object(&obj) {
            Ok(id) => {
                // Auto-create Person entity for sender
                register_person_if_needed(&g, from);
                return Some(id);
            }
            Err(e) => tracing::warn!("entity_bridge: email register failed: {e}"),
        }
    }
    None
}

// ────────────────────────────────────────────────────────────────────────────
// Calendar
// ────────────────────────────────────────────────────────────────────────────

/// Register a calendar event in the entity graph.
pub fn register_calendar_event(
    graph: &SharedEntityGraph,
    event_id: &str,
    title: &str,
    date: &str,
    notes: &str,
    attendees: &[String],
) -> Option<String> {
    let attendee_text = if attendees.is_empty() {
        String::new()
    } else {
        format!(" Attendees: {}", attendees.join(", "))
    };

    let obj = UniversalObject::new(ObjectKind::Event, title, "calendar", event_id)
        .with_summary(format!("{} — {}", date, title))
        .with_searchable_text(format!("{} {} {}{}", title, date, notes, attendee_text))
        .with_metadata(serde_json::json!({
            "date": date,
            "attendees": attendees,
        }));

    if let Ok(g) = graph.lock() {
        match g.upsert_object(&obj) {
            Ok(id) => {
                // Create relations to attendee Person entities
                for attendee in attendees {
                    if let Some(person_id) = register_person_if_needed(&g, attendee) {
                        let _ = g.add_relation(&person_id, &id, RelationKind::Attendee);
                    }
                }
                return Some(id);
            }
            Err(e) => tracing::warn!("entity_bridge: calendar register failed: {e}"),
        }
    }
    None
}

// ────────────────────────────────────────────────────────────────────────────
// Notes
// ────────────────────────────────────────────────────────────────────────────

/// Register a note in the entity graph.
pub fn register_note(
    graph: &SharedEntityGraph,
    filename: &str,
    title: &str,
    content: &str,
) -> Option<String> {
    let preview: String = content.chars().take(200).collect();
    let obj = UniversalObject::new(ObjectKind::Note, title, "notes", filename)
        .with_summary(preview)
        .with_searchable_text(content.chars().take(2000).collect::<String>());

    if let Ok(g) = graph.lock() {
        match g.upsert_object(&obj) {
            Ok(id) => return Some(id),
            Err(e) => tracing::warn!("entity_bridge: note register failed: {e}"),
        }
    }
    None
}

// ────────────────────────────────────────────────────────────────────────────
// Files
// ────────────────────────────────────────────────────────────────────────────

/// Register a file in the entity graph (on open/access).
pub fn register_file(
    graph: &SharedEntityGraph,
    path: &str,
    name: &str,
    file_type: &str,
    size_bytes: u64,
) -> Option<String> {
    let obj = UniversalObject::new(ObjectKind::File, name, "files", path)
        .with_summary(format!("{} — {}", file_type, format_size(size_bytes)))
        .with_searchable_text(format!("{} {}", name, path))
        .with_metadata(serde_json::json!({
            "path": path,
            "type": file_type,
            "size": size_bytes,
        }));

    if let Ok(g) = graph.lock() {
        match g.upsert_object(&obj) {
            Ok(id) => return Some(id),
            Err(e) => tracing::warn!("entity_bridge: file register failed: {e}"),
        }
    }
    None
}

// ────────────────────────────────────────────────────────────────────────────
// Tasks (from notes checklists, etc.)
// ────────────────────────────────────────────────────────────────────────────

/// Register a task in the entity graph.
pub fn register_task(
    graph: &SharedEntityGraph,
    task_id: &str,
    title: &str,
    done: bool,
    source_app: &str,
) -> Option<String> {
    let obj = UniversalObject::new(ObjectKind::Task, title, source_app, task_id)
        .with_summary(if done { "Completed" } else { "Open" }.to_string())
        .with_searchable_text(title.to_string())
        .with_metadata(serde_json::json!({ "done": done }));

    if let Ok(g) = graph.lock() {
        match g.upsert_object(&obj) {
            Ok(id) => return Some(id),
            Err(e) => tracing::warn!("entity_bridge: task register failed: {e}"),
        }
    }
    None
}

// ────────────────────────────────────────────────────────────────────────────
// Spreadsheet / Document / Presentation
// ────────────────────────────────────────────────────────────────────────────

/// Register a spreadsheet in the entity graph.
pub fn register_spreadsheet(
    graph: &SharedEntityGraph,
    filename: &str,
    title: &str,
    sheet_count: usize,
) -> Option<String> {
    let obj = UniversalObject::new(ObjectKind::Spreadsheet, title, "spreadsheet", filename)
        .with_summary(format!("{} sheets", sheet_count))
        .with_searchable_text(title.to_string());

    upsert_locked(graph, &obj)
}

/// Register a document in the entity graph.
pub fn register_document(
    graph: &SharedEntityGraph,
    filename: &str,
    title: &str,
    word_count: usize,
) -> Option<String> {
    let obj = UniversalObject::new(ObjectKind::Document, title, "document", filename)
        .with_summary(format!("{} words", word_count))
        .with_searchable_text(title.to_string());

    upsert_locked(graph, &obj)
}

/// Register a presentation in the entity graph.
pub fn register_presentation(
    graph: &SharedEntityGraph,
    filename: &str,
    title: &str,
    slide_count: usize,
) -> Option<String> {
    let obj = UniversalObject::new(ObjectKind::Presentation, title, "presentation", filename)
        .with_summary(format!("{} slides", slide_count))
        .with_searchable_text(title.to_string());

    upsert_locked(graph, &obj)
}

// ────────────────────────────────────────────────────────────────────────────
// Snippets
// ────────────────────────────────────────────────────────────────────────────

/// Register a snippet in the entity graph.
pub fn register_snippet(
    graph: &SharedEntityGraph,
    snippet_id: &str,
    title: &str,
    language: &str,
    code: &str,
) -> Option<String> {
    let preview: String = code.chars().take(200).collect();
    let obj = UniversalObject::new(ObjectKind::Snippet, title, "snippets", snippet_id)
        .with_summary(format!("{} — {}", language, preview))
        .with_searchable_text(format!("{} {} {}", title, language, code.chars().take(1000).collect::<String>()))
        .with_metadata(serde_json::json!({ "language": language }));

    upsert_locked(graph, &obj)
}

// ────────────────────────────────────────────────────────────────────────────
// Cross-App Relations
// ────────────────────────────────────────────────────────────────────────────

/// Create a "created from" relation (e.g., note created from email).
pub fn link_created_from(
    graph: &SharedEntityGraph,
    new_object_id: &str,
    source_object_id: &str,
) {
    if let Ok(g) = graph.lock() {
        let _ = g.add_relation(new_object_id, source_object_id, RelationKind::CreatedFrom);
    }
}

/// Create a "references" relation between two objects.
pub fn link_references(
    graph: &SharedEntityGraph,
    object_id: &str,
    referenced_id: &str,
) {
    if let Ok(g) = graph.lock() {
        let _ = g.add_relation(object_id, referenced_id, RelationKind::References);
    }
}

/// Remove an object from the graph (e.g., when a note is deleted).
pub fn unregister(graph: &SharedEntityGraph, source_app: &str, source_id: &str) {
    if let Ok(g) = graph.lock() {
        if let Ok(Some(obj)) = g.find_by_source(source_app, source_id) {
            let _ = g.delete_object(&obj.id);
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ────────────────────────────────────────────────────────────────────────────

/// Register a person entity if they don't already exist.
/// Returns the person's graph ID.
fn register_person_if_needed(graph: &EntityGraph, name_or_email: &str) -> Option<String> {
    let clean = name_or_email.trim();
    if clean.is_empty() {
        return None;
    }

    // Use the name/email as the source_id for dedup
    if let Ok(Some(existing)) = graph.find_by_source("people", clean) {
        return Some(existing.id);
    }

    let obj = UniversalObject::new(ObjectKind::Person, clean, "people", clean)
        .with_searchable_text(clean.to_string());

    match graph.upsert_object(&obj) {
        Ok(id) => Some(id),
        Err(_) => None,
    }
}

/// Lock-and-upsert helper.
fn upsert_locked(graph: &SharedEntityGraph, obj: &UniversalObject) -> Option<String> {
    if let Ok(g) = graph.lock() {
        match g.upsert_object(obj) {
            Ok(id) => return Some(id),
            Err(e) => tracing::warn!("entity_bridge: upsert failed: {e}"),
        }
    }
    None
}

/// Format bytes into human-readable size.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
