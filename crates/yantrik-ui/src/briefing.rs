//! "What's going on": the note every turn the person starts with a mind begins with (#1416).
//!
//! The brain makes the text (`yantrik_companion::agenda::told`, plain code over the agenda); the
//! Think arm's tick keeps the latest here (`agenda_tick`), and the Lens's dispatch (`wire/chat.rs`)
//! reads it when the person sends a turn.
//!
//! One path to every harness: the note goes in the turn context's `notes` array, the field the
//! harness host already fills for the desktop's per-turn notes (`host.rs`, `with_notes`, which
//! appends after what is there) and every harness already puts in front of the person's message
//! (`notes_before`). The built-in companion has no `notes`: its turn text gets the same words in
//! front of the person's, the way a hand-over does (`builtin_text`).
//!
//! Only turns the person started carry it (`typed`); a program's `send_message` on the control
//! socket does not. Private mode and the Brain switch are read again here, at the turn, so a switch
//! turned off since the last tick takes the note away at once.

use std::sync::Mutex;

use chrono::{DateTime, Local, TimeZone};
use serde_json::{json, Value};
use yantrik_companion::agenda::Told;

static CURRENT: Mutex<Option<Told>> = Mutex::new(None);
/// What the last turn was told, until the Agents store has recorded it for Mind View.
static SENT: Mutex<Option<Told>> = Mutex::new(None);

fn lock(m: &Mutex<Option<Told>>) -> std::sync::MutexGuard<'_, Option<Told>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The tick's latest note (`None`: nothing to say, or a switch said no).
pub fn publish(told: Option<Told>) {
    *lock(&CURRENT) = told;
}

/// The note for a turn now: none while private mode is on or the Brain switch is off.
fn current() -> Option<Told> {
    if crate::private_mode::is_on() || !crate::agenda_tick::brain_on() {
        return None;
    }
    lock(&CURRENT).clone()
}

/// How a time reads to the person, always with the weekday and date in their zone, and a plain
/// "today" or "tomorrow" after it when that is true: "Fri 27 Nov, 15:00 (tomorrow)".
pub fn when_words(at: f64, now: DateTime<Local>) -> String {
    let Some(t) = Local.timestamp_opt(at as i64, 0).single() else { return String::new() };
    let days = (t.date_naive() - now.date_naive()).num_days();
    let near = match days {
        0 => " (today)",
        1 => " (tomorrow)",
        _ => "",
    };
    format!("{}{near}", t.format("%a %-d %b, %H:%M"))
}

/// `context` (a JSON object in a string) with `text` added to its `notes`, after any already there.
pub fn with_note(context: &str, text: &str) -> String {
    let mut value: Value = serde_json::from_str(context).unwrap_or_else(|_| json!({}));
    let Some(object) = value.as_object_mut() else { return context.to_string() };
    let mut notes = match object.remove("notes") {
        Some(Value::Array(earlier)) => earlier,
        _ => Vec::new(),
    };
    notes.push(Value::String(text.to_string()));
    object.insert("notes".into(), Value::Array(notes));
    value.to_string()
}

/// The turn the person starts with the active mind: the desk's own origin and, when the person
/// typed it, the note in the context's `notes`.
pub fn desk_turn(text: String, context: String, typed: bool) -> yantrik_harness::Turn {
    let told = if typed { current() } else { None };
    let context = match &told {
        Some(t) => with_note(&context, &t.text),
        None => context,
    };
    *lock(&SENT) = told;
    yantrik_harness::Turn::new(text).with_context(context).with_origin(yantrik_harness::protocol::Origin::desk())
}

/// The built-in companion's turn text: the person's words, with the note and any hand-over in front.
pub fn builtin_text(handover: Option<&crate::agents::handover::Handover>, text: &str, typed: bool) -> String {
    let said = match handover {
        Some(h) => crate::agents::handover::with_handover(h, text),
        None => text.to_string(),
    };
    match if typed { current() } else { None } {
        Some(t) => format!("{}\n\n{said}", t.text),
        None => said,
    }
}

/// What the turn just sent was told, for Mind View's line; once.
pub fn take_sent() -> Option<String> {
    lock(&SENT).take().map(|t| format!("{}\n{}", t.shown(), t.text))
}

/// A timeline note as drawn: the "Told:" one is its first line until opened, then the exact text.
pub fn as_shown(note: &str, open: bool) -> &str {
    match note.strip_prefix("Told: ").and(note.split_once('\n')) {
        Some((first, _)) if !open => first,
        _ => note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_goes_after_the_notes_already_in_the_context() {
        let c = with_note(r#"{"machine":{"home":"/h"},"notes":["a"]}"#, "b");
        let v: Value = serde_json::from_str(&c).unwrap();
        assert_eq!(v["notes"], json!(["a", "b"]));
        assert_eq!(v["machine"]["home"], "/h");
        assert_eq!(serde_json::from_str::<Value>(&with_note("{}", "x")).unwrap()["notes"], json!(["x"]));
    }

    #[test]
    fn a_turn_the_person_started_carries_the_note_and_one_a_program_started_does_not() {
        publish(Some(Told { text: "What's going on: x".into(), items: 1 }));
        let notes = |turn: yantrik_harness::Turn| {
            let v: Value = serde_json::from_str(turn.context.as_deref().unwrap()).unwrap();
            v.get("notes").cloned()
        };
        let carried = desk_turn("hi".into(), "{}".into(), true);
        // The switch reads are the shell's own; in a test neither is on.
        assert_eq!(notes(carried), Some(json!(["What's going on: x"])));
        assert_eq!(take_sent().unwrap(), "Told: what's going on (1 item)\nWhat's going on: x");
        assert_eq!(notes(desk_turn("hi".into(), "{}".into(), false)), None);
        assert!(take_sent().is_none());
        publish(None);
        assert_eq!(notes(desk_turn("hi".into(), "{}".into(), true)), None);
    }

    #[test]
    fn a_time_reads_with_its_weekday_and_date() {
        let now = Local.with_ymd_and_hms(2026, 11, 26, 9, 0, 0).unwrap();
        let at = |d, h| Local.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap().timestamp() as f64;
        assert_eq!(when_words(at(27, 15), now), "Fri 27 Nov, 15:00 (tomorrow)");
        assert_eq!(when_words(at(26, 18), now), "Thu 26 Nov, 18:00 (today)");
        assert_eq!(when_words(at(1, 14), now), "Sun 1 Nov, 14:00");
        assert_eq!(when_words(at(30, 14), now), "Mon 30 Nov, 14:00");
    }

    #[test]
    fn the_told_line_opens_to_the_exact_text() {
        let note = "Told: what's going on (1 item)\nWhat's going on: x";
        assert_eq!(as_shown(note, false), "Told: what's going on (1 item)");
        assert_eq!(as_shown(note, true), note);
        assert_eq!(as_shown("Erased 9 places.", false), "Erased 9 places.");
    }
}
