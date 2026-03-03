//! Offline responder — handles user messages when the LLM backend is unavailable.
//!
//! Uses pattern matching, memory recall, and system context parsing to provide
//! basic intelligence without an LLM. The companion feels present even when
//! the AI backend is down.

use yantrikdb_core::types::RecallResult;
use yantrikdb_core::YantrikDB;

use crate::types::Urge;

/// Stateless offline responder. All methods are pure functions.
pub struct OfflineResponder;

impl OfflineResponder {
    /// Generate a response without the LLM.
    ///
    /// Tries pattern matching first, then memory recall, then falls back
    /// to a default message. Always returns a non-empty string.
    pub fn respond(
        db: &YantrikDB,
        user_text: &str,
        system_context: &str,
        memories: &[RecallResult],
        urges: &[Urge],
        user_name: &str,
    ) -> String {
        let lower = user_text.to_lowercase();

        // Pattern: Remember something
        if let Some(content) = try_extract_remember(&lower, user_text) {
            return match db.record_text(
                &content,
                "episodic",
                0.5,
                0.0,
                604800.0,
                &serde_json::json!({}),
                "default",
                0.9,
                "general",
                "companion",
                None,
            ) {
                Ok(_) => "Got it, I'll remember that.".to_string(),
                Err(_) => {
                    "I tried to save that but ran into an issue. I'll try again later."
                        .to_string()
                }
            };
        }

        // Pattern: What time is it
        if lower.contains("what time") || lower.contains("current time") {
            if let Some(val) = extract_field(system_context, "Time:") {
                return format!("It's {}.", val);
            }
        }

        // Pattern: Battery
        if lower.contains("battery") || lower.contains("charge") || lower.contains("power level")
        {
            if let Some(val) = extract_field(system_context, "Battery:") {
                return format!("Battery is at {}.", val);
            }
        }

        // Pattern: Disk / storage
        if lower.contains("disk") || lower.contains("storage") || lower.contains("space") {
            if let Some(val) = extract_field(system_context, "Disk:") {
                return format!("{}.", val);
            }
        }

        // Pattern: Network / wifi
        if lower.contains("network") || lower.contains("wifi") || lower.contains("internet") {
            if let Some(val) = extract_field(system_context, "WiFi:") {
                return format!("{}.", val);
            }
        }

        // Pattern: Explicit memory recall
        if lower.contains("what do you know")
            || lower.contains("what do you remember")
            || lower.contains("do you remember")
        {
            if !memories.is_empty() {
                return format_memory_response(memories, false);
            }
            return "I don't have any memories about that topic yet.".to_string();
        }

        // Pattern: Who am I
        if lower.contains("who am i") || lower.contains("my name") {
            return format!("You're {}.", user_name);
        }

        // Pattern: Greeting
        if lower.starts_with("hi")
            || lower.starts_with("hey")
            || lower.starts_with("hello")
            || lower.starts_with("good morning")
            || lower.starts_with("good evening")
        {
            return format!(
                "Hey {}! I'm running in local mode right now — the AI backend is offline. \
                 But I can still remember things, check your system, and recall what I know.",
                user_name
            );
        }

        // Default: show memories if relevant ones were found
        if !memories.is_empty() && memories[0].score > 0.4 {
            return format_memory_response(memories, true);
        }

        // Urge-aware fallback
        if !urges.is_empty() {
            let urge_text = urges
                .iter()
                .take(2)
                .map(|u| format!("- {}", u.reason))
                .collect::<Vec<_>>()
                .join("\n");
            return format!(
                "I can't think deeply right now — the AI backend is offline. \
                 But I had some things on my mind:\n{}\n\
                 Your files, apps, and settings still work normally.",
                urge_text
            );
        }

        // Bare fallback
        "I can't think deeply right now — the AI backend is offline. \
         But I'm still here. Your files, apps, and settings all work normally."
            .to_string()
    }
}

/// Try to extract content from a "remember that/this/to" pattern.
fn try_extract_remember(lower: &str, original: &str) -> Option<String> {
    for prefix in &[
        "remember that ",
        "remember this: ",
        "remember to ",
        "remember ",
        "please remember ",
        "don't forget ",
    ] {
        if lower.starts_with(prefix) {
            let content = &original[prefix.len()..];
            if content.len() >= 3 {
                return Some(content.trim().to_string());
            }
        }
    }
    None
}

/// Extract a field value from the system context string.
///
/// System context format: "Battery: 72% (charging) | WiFi: connected | ..."
fn extract_field<'a>(context: &'a str, field: &str) -> Option<&'a str> {
    let start = context.find(field)?;
    let after = &context[start + field.len()..];
    let after = after.trim_start();
    // Take until next pipe separator or end of string
    let end = after.find('|').unwrap_or(after.len());
    let value = after[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Format recalled memories into a natural response.
fn format_memory_response(memories: &[RecallResult], is_fallback: bool) -> String {
    let prefix = if is_fallback {
        "I'm running in local mode right now. Here's what I recall about that:"
    } else {
        "Here's what I remember:"
    };

    let items: Vec<String> = memories
        .iter()
        .take(3)
        .map(|m| format!("- {}", m.text))
        .collect();

    let suffix = if is_fallback {
        "\nI'll be able to think more deeply once the AI backend is back."
    } else {
        ""
    };

    format!("{}\n{}{}", prefix, items.join("\n"), suffix)
}
