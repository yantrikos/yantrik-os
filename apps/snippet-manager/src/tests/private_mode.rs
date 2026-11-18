//! Private mode (#924): Snippets keeps none of its keys. Every one of them names, counts or quotes
//! the person's snippets, and a snippet may be a secret, so in Private mode `describe` says
//! Snippets is open and nothing more.

use serde_json::json;
use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};
use yantrik_app::{App, Intents};

use super::open;
use crate::SnippetManager;

/// In a snippet's title and in its code's first line, so it reaches the summary, `open`, the
/// list's titles and the list's one-line preview.
const SENTINEL: &str = "SENTINEL-924-db-password";

#[test]
fn in_private_mode_describe_names_no_snippet_and_keeps_none_of_its_keys() {
    let f = open();
    f.act("new", json!({ "title": SENTINEL, "language": "Shell", "code": format!("export DB={SENTINEL}") })).unwrap();
    assert!(f.describe().to_string().contains(SENTINEL), "the snippet is in the model");
    pin(&f.paths, true);
    let d = f.describe();
    assert_eq!(d["private"], json!(true));
    assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
    assert_eq!(d["state"], json!({ "private": true, "open": true, "windows_open": 1 }));
    assert!(!d.to_string().contains(SENTINEL), "{d}");
    // An act's reply carries the same view; its `result` is the person's own call's answer.
    let pending = f.app.act("search", json!({ "query": "" })).unwrap();
    f.quiet();
    let mut envelope = pending.envelope().clone();
    envelope.as_object_mut().unwrap().remove("result");
    assert_eq!(envelope["private"], json!(true));
    assert!(!envelope.to_string().contains(SENTINEL), "{envelope}");
    pin(&f.paths, false);
    assert_eq!(f.state()["open"], json!(SENTINEL), "and all of it once it is off");
}

#[test]
fn snippets_lists_no_key_to_keep_in_private_mode() {
    let mut table = Intents::<SnippetManager>::new();
    SnippetManager::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
