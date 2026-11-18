//! Private mode (#924, #926): Terminal keeps none of its keys. A screen can hold anything the
//! person typed or was shown, so in Private mode `describe` says Terminal is open and nothing
//! more. (That no agent acts at all while the person is private is the framework's, at the
//! socket: tests/private_mode.rs of yantrik-app.)

use serde_json::{json, Value};
use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};

use super::open;

const SENTINEL: &str = "SENTINEL-926";

fn leaks(answer: &Value) -> bool {
    answer.to_string().contains(SENTINEL)
}

#[test]
fn in_private_mode_describe_shows_no_screen_no_folder_and_none_of_its_keys() {
    let f = open();
    f.act("run", json!({ "command": format!("echo {SENTINEL}") })).unwrap();
    f.until("the output", |f| f.screen().contains(SENTINEL));
    f.until("the probe", |f| f.summary().contains("idle at a prompt"));
    assert!(leaks(&f.app.describe()), "the screen is in the model");
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
fn terminal_lists_no_key_to_keep_in_private_mode() {
    let mut table = yantrik_app::intents::Intents::<crate::Terminal>::new();
    <crate::Terminal as yantrik_app::App>::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
