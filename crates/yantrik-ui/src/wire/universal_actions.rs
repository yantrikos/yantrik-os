//! Universal Actions — "Open in...", "Convert to...", "Export..." menus.
//!
//! Given an object kind, returns the available actions. These are used by
//! the command palette, context menus, and global search results.

use yantrik_os::entity_graph::ObjectKind;

/// A universal action that can be performed on an entity graph object.
#[derive(Debug, Clone)]
pub struct UniversalAction {
    pub label: String,
    pub icon_char: String,
    pub action_id: String,
    pub category: ActionCategory,
}

/// Action categories for grouping in menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionCategory {
    OpenIn,
    ConvertTo,
    Export,
    AI,
}

impl ActionCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenIn => "Open In",
            Self::ConvertTo => "Convert To",
            Self::Export => "Export",
            Self::AI => "AI",
        }
    }
}

/// Get available actions for an object kind.
pub fn actions_for_kind(kind: ObjectKind) -> Vec<UniversalAction> {
    match kind {
        ObjectKind::Thread => vec![
            open_in("Email", "@", "email"),
            convert("Calendar Event", "▦", "email_to_calendar"),
            convert("Note", "✎", "email_to_note"),
            convert("Task", "☐", "email_to_task"),
            ai("Summarize", "summarize"),
        ],
        ObjectKind::Event => vec![
            open_in("Calendar", "▦", "calendar"),
            convert("Meeting Notes", "✎", "calendar_to_note"),
            ai("Prepare for Meeting", "meeting_prep"),
        ],
        ObjectKind::Note => vec![
            open_in("Notes", "✎", "notes"),
            open_in("Text Editor", "≡", "editor"),
            convert("Presentation", "YP", "note_to_presentation"),
            convert("Document", "YD", "note_to_document"),
            export("Markdown", "md"),
        ],
        ObjectKind::File => vec![
            open_in("File Browser", "F", "files"),
            open_in("Text Editor", "≡", "editor"),
            open_in("Image Viewer", "I", "image_viewer"),
            ai("Summarize", "summarize"),
        ],
        ObjectKind::Task => vec![
            open_in("Notes", "✎", "notes"),
            ai("Break Down", "break_down"),
        ],
        ObjectKind::Decision => vec![
            open_in("Notes", "✎", "notes"),
        ],
        ObjectKind::Person => vec![
            action(ActionCategory::OpenIn, "Find in Email", "@", "person:email"),
            action(ActionCategory::OpenIn, "Find in Calendar", "▦", "person:calendar"),
        ],
        ObjectKind::Spreadsheet => vec![
            open_in("Spreadsheet", "YS", "spreadsheet"),
            export("CSV", "csv"),
        ],
        ObjectKind::Document => vec![
            open_in("Document Editor", "YD", "document"),
            export("Markdown", "md"),
            export("HTML", "html"),
        ],
        ObjectKind::Presentation => vec![
            open_in("Presentation", "YP", "presentation"),
            export("Markdown", "md"),
        ],
        ObjectKind::Snippet => vec![
            open_in("Snippet Manager", "<>", "snippets"),
            action(ActionCategory::OpenIn, "Open in Editor", "≡", "snippet:editor"),
        ],
    }
}

/// Get all actions for a specific object (by ID), to be shown in a context menu.
/// Returns actions filtered to what's applicable.
pub fn actions_for_object(
    kind: ObjectKind,
    object_id: &str,
    source_app: &str,
) -> Vec<(String, String, String)> {
    // Returns (label, icon, action_id) tuples with object ID embedded
    actions_for_kind(kind)
        .into_iter()
        .map(|a| {
            let action_with_id = format!("{}:{}", a.action_id, object_id);
            (a.label, a.icon_char, action_with_id)
        })
        .collect()
}

// ── Helpers ──

fn open_in(app_name: &str, icon: &str, target: &str) -> UniversalAction {
    action(
        ActionCategory::OpenIn,
        &format!("Open in {}", app_name),
        icon,
        &format!("open_in:{}", target),
    )
}

fn convert(target_name: &str, icon: &str, workflow: &str) -> UniversalAction {
    action(
        ActionCategory::ConvertTo,
        &format!("Create {}", target_name),
        icon,
        &format!("convert:{}", workflow),
    )
}

fn export(format: &str, fmt_id: &str) -> UniversalAction {
    action(
        ActionCategory::Export,
        &format!("Export as {}", format),
        "⬇",
        &format!("export:{}", fmt_id),
    )
}

fn ai(label: &str, action_id: &str) -> UniversalAction {
    action(
        ActionCategory::AI,
        &format!("AI: {}", label),
        "◈",
        &format!("ai:{}", action_id),
    )
}

fn action(category: ActionCategory, label: &str, icon: &str, action_id: &str) -> UniversalAction {
    UniversalAction {
        label: label.to_string(),
        icon_char: icon.to_string(),
        action_id: action_id.to_string(),
        category,
    }
}
