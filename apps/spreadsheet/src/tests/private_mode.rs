//! Private mode (#924): ySheets keeps none of its keys. Every one of them names, counts or quotes
//! the person's sheet, so in Private mode `describe` says ySheets is open and nothing more.

use serde_json::json;
use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};

use super::{act, scratch, settle, started};

/// In a cell, in the formula that quotes it, and in the file's name, so it reaches the summary, the
/// state, the selection and the notice.
const SENTINEL: &str = "Therapy-Thursday-SENTINEL-924";

fn leaks(answer: &serde_json::Value) -> bool {
    answer.to_string().contains("SENTINEL-924")
}

#[test]
fn in_private_mode_describe_names_no_cell_and_keeps_none_of_ysheets_keys() {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "A1", "text": SENTINEL })).unwrap();
    act(&app, "set_cell", json!({ "cell": "B1", "text": "=A1+1" })).unwrap();
    let path = scratch("private").join(format!("{SENTINEL}.csv"));
    act(&app, "save", json!({ "path": path.display().to_string() })).unwrap();
    act(&app, "select", json!({ "cell": "B1" })).unwrap();
    assert!(leaks(&app.describe()), "the sheet is in the model");
    pin(&app.paths(), true);
    let d = app.describe();
    assert_eq!(d["private"], json!(true));
    assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
    assert_eq!(d["state"], json!({ "private": true, "open": true, "windows_open": 1 }));
    assert!(!leaks(&d), "{d}");
    // An act's reply carries the same view; its `result` is the person's own call's answer.
    let pending = app.act("select", json!({ "cell": "A1" })).unwrap();
    settle(&app);
    let mut envelope = pending.envelope().clone();
    envelope.as_object_mut().unwrap().remove("result");
    assert_eq!(envelope["private"], json!(true));
    assert!(!leaks(&envelope), "{envelope}");
    pin(&app.paths(), false);
    assert_eq!(app.describe()["state"]["selection"]["typed"], json!(SENTINEL), "and all of it once it is off");
}

#[test]
fn ysheets_lists_no_key_to_keep_in_private_mode() {
    let mut table = yantrik_app::intents::Intents::<crate::YSheets>::new();
    <crate::YSheets as yantrik_app::App>::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
