//! Private mode (#924): yPresent keeps none of its keys. Every one of them names, counts or
//! quotes the person's deck, so in Private mode `describe` says yPresent is open and nothing more.

use serde_json::json;
use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};

use super::{act, settle, started};

/// In a deck's name and a slide's title, body and notes, so it reaches the summary, the state, the
/// outline and the current slide.
const SENTINEL: &str = "Therapy, Thursday SENTINEL-924";

fn leaks(answer: &serde_json::Value) -> bool {
    answer.to_string().contains("SENTINEL-924")
}

#[test]
fn in_private_mode_describe_names_no_slide_and_keeps_none_of_ypresents_keys() {
    let app = started();
    act(&app, "new_deck", json!({ "title": SENTINEL })).unwrap();
    act(&app, "set_slide", json!({ "index": 0, "title": SENTINEL, "body": SENTINEL, "notes": SENTINEL })).unwrap();
    assert!(leaks(&app.describe()), "the deck is in the model");
    pin(&app.paths(), true);
    let d = app.describe();
    assert_eq!(d["private"], json!(true));
    assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
    assert_eq!(d["state"], json!({ "private": true, "open": true, "windows_open": 1 }));
    assert!(!leaks(&d), "{d}");
    // An act's reply carries the same view; its `result` is the person's own call's answer.
    let pending = app.act("go_to", json!({ "index": 0 })).unwrap();
    settle(&app);
    let mut envelope = pending.envelope().clone();
    envelope.as_object_mut().unwrap().remove("result");
    assert_eq!(envelope["private"], json!(true));
    assert!(!leaks(&envelope), "{envelope}");
    pin(&app.paths(), false);
    assert_eq!(app.describe()["state"]["deck_title"], json!(SENTINEL), "and all of it once it is off");
}

#[test]
fn ypresent_lists_no_key_to_keep_in_private_mode() {
    let mut table = yantrik_app::intents::Intents::<crate::YPresent>::new();
    <crate::YPresent as yantrik_app::App>::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
