//! Private mode (#924, #926): Download Manager keeps none of its keys. A file name, a URL and a
//! folder each say what the person is fetching and where they keep it, so in Private mode
//! `describe` says Downloads is open and nothing more.

use serde_json::{json, Value};
use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};

use super::{body, open, server::File};

/// In a file's name and so its URL, its path and the summary.
const SENTINEL: &str = "SENTINEL-926";

fn leaks(answer: &Value) -> bool {
    answer.to_string().contains(SENTINEL)
}

#[test]
fn in_private_mode_describe_names_no_download_and_keeps_none_of_its_keys() {
    let name = format!("/for-her-{SENTINEL}.bin");
    let f = open(vec![(&name, File::plain(body(3_000)))]);
    f.download(&name);
    assert!(leaks(&f.app.describe()), "the download is in the model");
    pin(&f.app.paths(), true);
    let d = f.app.describe();
    assert_eq!(d["private"], json!(true));
    assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
    assert_eq!(d["state"], json!({ "private": true, "open": true, "windows_open": 1 }));
    assert!(!leaks(&d), "{d}");
    // An act's reply carries the same view; its `result` is the person's own call's answer.
    let pending = f.app.act("pause_all", json!({})).unwrap();
    f.quiet();
    let mut envelope = pending.envelope().clone();
    envelope.as_object_mut().unwrap().remove("result");
    assert_eq!(envelope["private"], json!(true));
    assert!(!leaks(&envelope), "{envelope}");
    pin(&f.app.paths(), false);
    assert!(leaks(&f.app.describe()), "and all of it once it is off");
}

#[test]
fn download_manager_lists_no_key_to_keep_in_private_mode() {
    let mut table = yantrik_app::intents::Intents::<crate::Downloads>::new();
    <crate::Downloads as yantrik_app::App>::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
