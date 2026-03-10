//! Cross-App Workflows — chain actions between apps.
//!
//! Enables workflows like: email → calendar event, email → note, note → task,
//! calendar → meeting notes, any → presentation slide.
//!
//! Each workflow: (1) creates an entity in the target app, (2) links it in the
//! entity graph, (3) navigates to the target app with data pre-populated.

use slint::{ComponentHandle, SharedString};

use crate::app_context::AppContext;
use crate::wire::entity_bridge;
use crate::App;

/// Wire cross-app action callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_cross_app_action(ui, ctx);
}

fn wire_cross_app_action(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let graph = ctx.entity_graph.clone();

    ui.on_cross_app_action(move |action_type, data_json| {
        let action = action_type.to_string();
        let data = data_json.to_string();
        tracing::info!(action = %action, "Cross-app action triggered");

        let ui = match ui_weak.upgrade() {
            Some(u) => u,
            None => return,
        };

        let parsed: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();

        match action.as_str() {
            "email_to_calendar" => {
                // Extract subject and date from email, pre-populate calendar event
                let subject = parsed["subject"].as_str().unwrap_or("New Event");
                let from = parsed["from"].as_str().unwrap_or("");

                // Set calendar pre-fill properties
                ui.set_cal_prefill_title(SharedString::from(subject));
                ui.set_cal_prefill_notes(SharedString::from(
                    format!("Created from email from {}", from),
                ));

                // Register cross-app link
                if let Some(email_source_id) = parsed["message_id"].as_str() {
                    if let Some(event_id) = entity_bridge::register_calendar_event(
                        &graph,
                        &format!("from-email-{}", email_source_id),
                        subject,
                        "",
                        &format!("Created from email: {}", subject),
                        &[],
                    ) {
                        if let Some(email_id) = parsed["entity_id"].as_str() {
                            entity_bridge::link_created_from(&graph, &event_id, email_id);
                        }
                    }
                }

                ui.set_current_screen(18);
                ui.invoke_navigate(18);
            }

            "email_to_note" => {
                let subject = parsed["subject"].as_str().unwrap_or("Email Note");
                let body = parsed["body"].as_str().unwrap_or("");
                let from = parsed["from"].as_str().unwrap_or("");

                // Pre-fill notes with email content
                let note_content = format!(
                    "# {}\n\n**From:** {}\n\n---\n\n{}",
                    subject, from, body
                );
                ui.set_notes_prefill_title(SharedString::from(subject));
                ui.set_notes_prefill_content(SharedString::from(note_content));

                // Register in entity graph
                if let Some(note_id) = entity_bridge::register_note(
                    &graph,
                    &format!("from-email-{}", parsed["message_id"].as_str().unwrap_or("unknown")),
                    subject,
                    &format!("Note created from email: {}", subject),
                ) {
                    if let Some(email_id) = parsed["entity_id"].as_str() {
                        entity_bridge::link_created_from(&graph, &note_id, email_id);
                    }
                }

                ui.set_current_screen(15);
                ui.invoke_navigate(15);
            }

            "email_to_task" => {
                let subject = parsed["subject"].as_str().unwrap_or("Follow up");
                let source = parsed["message_id"].as_str().unwrap_or("unknown");

                if let Some(task_id) = entity_bridge::register_task(
                    &graph,
                    &format!("task-from-email-{}", source),
                    subject,
                    false,
                    "email",
                ) {
                    if let Some(email_id) = parsed["entity_id"].as_str() {
                        entity_bridge::link_created_from(&graph, &task_id, email_id);
                    }
                }

                // Navigate to notes (tasks are managed in notes for now)
                ui.set_current_screen(15);
                ui.invoke_navigate(15);
            }

            "calendar_to_note" => {
                let title = parsed["title"].as_str().unwrap_or("Meeting Notes");
                let date = parsed["date"].as_str().unwrap_or("");
                let attendees: Vec<String> = parsed["attendees"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();

                let note_content = format!(
                    "# Meeting Notes: {}\n\n**Date:** {}\n**Attendees:** {}\n\n## Agenda\n\n- \n\n## Notes\n\n\n\n## Action Items\n\n- [ ] \n\n## Decisions\n\n- ",
                    title,
                    date,
                    if attendees.is_empty() { "N/A".to_string() } else { attendees.join(", ") },
                );
                ui.set_notes_prefill_title(SharedString::from(format!("Meeting: {}", title)));
                ui.set_notes_prefill_content(SharedString::from(note_content));

                // Link note to event in entity graph
                if let Some(note_id) = entity_bridge::register_note(
                    &graph,
                    &format!("meeting-notes-{}", parsed["event_id"].as_str().unwrap_or("unknown")),
                    &format!("Meeting: {}", title),
                    &format!("Meeting notes for {} on {}", title, date),
                ) {
                    if let Some(event_id) = parsed["entity_id"].as_str() {
                        entity_bridge::link_created_from(&graph, &note_id, event_id);
                    }
                }

                ui.set_current_screen(15);
                ui.invoke_navigate(15);
            }

            "note_to_presentation" => {
                let title = parsed["title"].as_str().unwrap_or("Presentation");
                let content = parsed["content"].as_str().unwrap_or("");

                ui.set_pres_prefill_title(SharedString::from(title));
                ui.set_pres_prefill_content(SharedString::from(content.to_string()));

                if let Some(pres_id) = entity_bridge::register_presentation(
                    &graph,
                    &format!("from-note-{}", parsed["note_id"].as_str().unwrap_or("unknown")),
                    title,
                    1,
                ) {
                    if let Some(note_id) = parsed["entity_id"].as_str() {
                        entity_bridge::link_created_from(&graph, &pres_id, note_id);
                    }
                }

                ui.set_current_screen(31);
                ui.invoke_navigate(31);
            }

            "file_to_editor" => {
                let path = parsed["path"].as_str().unwrap_or("");
                if !path.is_empty() {
                    ui.set_editor_open_path(SharedString::from(path));
                    ui.set_current_screen(12);
                    ui.invoke_navigate(12);
                }
            }

            "open_object" => {
                // Open any entity graph object in its source app
                let source_app = parsed["source_app"].as_str().unwrap_or("");
                let screen = match source_app {
                    "email" => 17,
                    "calendar" => 18,
                    "notes" => 15,
                    "files" => 8,
                    "spreadsheet" => 29,
                    "document" => 30,
                    "presentation" => 31,
                    "snippets" => 25,
                    _ => 1,
                };
                ui.set_current_screen(screen);
                ui.invoke_navigate(screen);
            }

            other => {
                tracing::warn!("Unknown cross-app action: {other}");
            }
        }
    });
}
