//! Private mode (#924, #926): Music keeps none of its keys. The track, its path, the library, the
//! folders and the status line all name what the person listens to and where they keep it, so
//! in Private mode `describe` says Music is open and nothing more.

use serde_json::{json, Value};
use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};

use super::open;

/// In a folder's name and a track's, so it reaches the summary, `track`, `path`, the library,
/// the folders and the status line.
const SENTINEL: &str = "SENTINEL-924";

fn leaks(answer: &Value) -> bool {
    answer.to_string().contains(SENTINEL)
}

#[test]
fn in_private_mode_describe_names_no_track_and_keeps_none_of_musics_keys() {
    let Some(f) = open("private") else { return };
    let folder = f.dir.join(format!("Late night {SENTINEL}"));
    std::fs::create_dir_all(&folder).unwrap();
    let track = folder.join(format!("For her {SENTINEL}.mp3"));
    std::fs::write(&track, b"").unwrap();
    f.app.ui({
        let folder = folder.to_string_lossy().into_owned();
        move |ui| ui.invoke_scan_add_folder(folder.into())
    });
    f.quiet();
    f.act("play_file", json!({ "path": track })).unwrap();
    assert!(leaks(&f.app.describe()), "the track is in the model");
    pin(&f.app.paths(), true);
    let d = f.app.describe();
    assert_eq!(d["private"], json!(true));
    assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
    assert_eq!(d["state"], json!({ "private": true, "open": true, "windows_open": 1 }));
    assert!(!leaks(&d), "{d}");
    // An act's reply carries the same view; its `result` is the person's own call's answer.
    let pending = f.app.act("pause", json!({})).unwrap();
    f.quiet();
    let mut envelope = pending.envelope().clone();
    envelope.as_object_mut().unwrap().remove("result");
    assert_eq!(envelope["private"], json!(true));
    assert!(!leaks(&envelope), "{envelope}");
    pin(&f.app.paths(), false);
    assert!(f.state()["track"].as_str().is_some_and(|t| t.contains(SENTINEL)), "and all of it once it is off");
}

#[test]
fn music_lists_no_key_to_keep_in_private_mode() {
    let mut table = yantrik_app::intents::Intents::<crate::Music>::new();
    <crate::Music as yantrik_app::App>::intents(&mut table);
    assert!(table.shown_in_private_keys().is_empty(), "a key kept in Private mode is a decision for a reader");
}
