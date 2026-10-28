//! Private mode (#924, #926): Studio keeps none of its keys. The sentences a person typed and the
//! pictures they made are the content, so in Private mode `describe` says Studio is open and
//! nothing more. (That no agent acts at all while the person is private is the framework's, at the
//! socket: tests/private_mode.rs of yantrik-app.)

use serde_json::{json, Value};
use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};

use super::fake;

const SENTINEL: &str = "SENTINEL-926";

fn leaks(answer: &Value) -> bool {
    answer.to_string().contains(SENTINEL)
}

#[test]
fn in_private_mode_describe_shows_no_sentence_no_folder_and_none_of_its_keys() {
    let f = fake("private");
    f.act("generate", json!({ "prompt": format!("a picture of {SENTINEL}"), "width": 64, "height": 64 })).unwrap();
    assert_eq!(f.settled(), "");
    assert!(leaks(&f.describe()), "the sentence is in the model");
    pin(&f.app.paths(), true);
    let d = f.describe();
    assert_eq!(d["private"], json!(true));
    assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
    assert_eq!(d["state"], json!({ "private": true, "open": true, "windows_open": 1 }));
    assert!(!leaks(&d), "{d}");
    pin(&f.app.paths(), false);
    assert!(leaks(&f.describe()), "and all of it once it is off");
}

#[test]
fn studio_lists_no_key_to_keep_in_private_mode() {
    let mut table = yantrik_app::intents::Intents::<crate::Studio>::new();
    <crate::Studio as yantrik_app::App>::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
