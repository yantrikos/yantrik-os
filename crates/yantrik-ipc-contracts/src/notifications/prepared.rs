//! What the shell sends the notifications service when a "Prepare for me" job has finished: one
//! place builds the request and one place reads it, so a shape the service refuses is a test
//! failure here and not a card that never shows (#1416, slice 5, round 6).

use serde_json::{json, Value};

/// The longest id the service takes.
pub const MAX_ITEM_ID: usize = 64;

/// The id the card is filed under: the calendar's own id, without the agenda's `calendar:` prefix
/// (the agenda keys an item `<source>:<id>`, and the earlier "prepare for what's next" card is
/// `prep:<id>`, so the same bare id lets this card replace it). Only letters, digits and `-`.
pub fn card_item_id(agenda_id: &str) -> String {
    let bare = agenda_id.split_once(':').map_or(agenda_id, |(_, id)| id);
    bare.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).take(MAX_ITEM_ID).collect()
}

/// The request for `notifications.notice_prepared`. `missing` is what did not happen, in plain
/// words; the card says it.
pub fn done_params(agenda_id: &str, line: &str, folder: &str, has_summary: bool, why: &str, missing: &[String]) -> Value {
    json!({
        "item_id": card_item_id(agenda_id),
        "line": line,
        "folder": folder,
        "summary": has_summary,
        "why": why,
        "missing": missing.iter().take(4).cloned().collect::<Vec<_>>().join("; "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agenda_key_becomes_the_calendars_own_id() {
        assert_eq!(card_item_id("calendar:abc-123"), "abc-123");
        assert_eq!(card_item_id("calendar:a_b@c.d"), "a-b-c-d");
        assert_eq!(card_item_id("plain"), "plain");
        assert!(card_item_id(&format!("calendar:{}", "x".repeat(200))).len() <= MAX_ITEM_ID);
    }
}
