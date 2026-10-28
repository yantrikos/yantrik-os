//! Private mode (#924, #926): Arcade keeps none of its keys. A game's title says what the person is
//! making, so in Private mode `describe` says Arcade is open and nothing more.

use serde_json::{json, Value};
use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};

use super::{meadow_run, open, PIP};

const SENTINEL: &str = "SENTINEL-926";

fn leaks(answer: &Value) -> bool {
    answer.to_string().contains(SENTINEL)
}

#[test]
fn in_private_mode_describe_names_no_game_and_keeps_none_of_its_keys() {
    let f = open();
    f.act("new_character", json!({ "spec": PIP })).unwrap();
    f.act("new_game", json!({ "spec": meadow_run(&format!("Secret {SENTINEL}")) })).unwrap();
    assert!(leaks(&f.app.describe()), "the game is in the model");
    pin(&f.app.paths(), true);
    let d = f.app.describe();
    assert_eq!(d["private"], json!(true));
    assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
    assert_eq!(d["state"], json!({ "private": true, "open": true, "windows_open": 1 }));
    assert!(!leaks(&d), "{d}");
    pin(&f.app.paths(), false);
    assert!(leaks(&f.app.describe()), "and all of it once it is off");
}

#[test]
fn arcade_lists_no_key_to_keep_in_private_mode() {
    let mut table = yantrik_app::intents::Intents::<crate::Arcade>::new();
    <crate::Arcade as yantrik_app::App>::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
