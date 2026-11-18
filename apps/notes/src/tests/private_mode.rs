//! Private mode (#924): Notes keeps none of its keys. Every one of them names, counts or quotes
//! the person's notes, so in Private mode `describe` says Notes is open and nothing more.

use serde_json::json;
use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};

use super::{act, settle, started};

/// In a note's title and body, so it reaches the summary, `title`, `content` and the library.
const SENTINEL: &str = "Therapy, Thursday SENTINEL-924";

fn leaks(answer: &serde_json::Value) -> bool {
    answer.to_string().contains("SENTINEL-924")
}

#[test]
fn in_private_mode_describe_names_no_note_and_keeps_none_of_notes_keys() {
    let app = started();
    act(&app, "new_note", json!({ "title": SENTINEL, "text": format!("what was said: {SENTINEL}") })).unwrap();
    assert!(leaks(&app.describe()), "the note is in the model");
    pin(&app.paths(), true);
    let d = app.describe();
    assert_eq!(d["private"], json!(true));
    assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
    assert_eq!(d["state"], json!({ "private": true, "open": true, "windows_open": 1 }));
    assert!(!leaks(&d), "{d}");
    // An act's reply carries the same view; its `result` is the person's own call's answer.
    let pending = app.act("search", json!({ "query": "" })).unwrap();
    settle(&app);
    let mut envelope = pending.envelope().clone();
    envelope.as_object_mut().unwrap().remove("result");
    assert_eq!(envelope["private"], json!(true));
    assert!(!leaks(&envelope), "{envelope}");
    pin(&app.paths(), false);
    assert_eq!(app.describe()["state"]["title"], json!(SENTINEL), "and all of it once it is off");
}

#[test]
fn notes_lists_no_key_to_keep_in_private_mode() {
    let mut table = yantrik_app::intents::Intents::<crate::Notes>::new();
    <crate::Notes as yantrik_app::App>::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
