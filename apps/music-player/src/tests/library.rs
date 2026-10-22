//! The scan, off the UI thread, and the folder list with its refusals, kept in the settings
//! file; the file from before the framework, read once and kept beside the new one.

use serde_json::{json, Value};

use super::{open, open_with, Setup};
use crate::kept::Kept;

#[test]
fn the_scan_runs_off_the_ui_thread_and_its_last_sentence_stays_on_screen() {
    let Some(f) = open("scan") else { return };
    assert_eq!(f.status(), "300 tracks in 1 folder");
    assert_eq!(f.app.ui(|ui| ui.get_library_track_count()), 300, "each song once, the loop not walked");
    let folder = f.app.ui(|ui| slint::Model::row_data(&ui.get_scan_folders(), 0).unwrap().track_count);
    assert_eq!(folder, 300, "the folder's count is real");
    // A scan the person starts: the window says it is scanning while the job walks.
    f.app.ui(|ui| ui.invoke_scan_rescan());
    assert!(f.app.ui(|ui| ui.get_is_scanning()));
    assert!(f.status().contains("Scanning"), "{}", f.status());
    assert_eq!(f.state()["scanning"], true);
    f.quiet();
    assert!(!f.app.ui(|ui| ui.get_is_scanning()));
    assert_eq!(f.status(), "300 tracks in 1 folder");
    // A rescan through the surface answers with what was found.
    let found = f.act("rescan", json!({})).unwrap();
    assert_eq!(found["library_size"], 300);
    assert_eq!(found["folders"][0]["tracks"], 300);
    assert_eq!(found["summary"], "300 tracks in 1 folder");
    assert_eq!(found["unreadable"], json!([]));
}

/// A `rescan` while a scan runs is told so and starts nothing; once that scan lands, the next
/// one is answered with what it found.
#[test]
fn a_rescan_while_a_scan_runs_is_told_so_and_the_next_one_finds_the_new_file() {
    let Some(f) = open("two-scans") else { return };
    let first = f.app.act("rescan", json!({})).unwrap();
    let second = f.app.act("rescan", json!({})).unwrap().settle();
    assert_eq!(second.unwrap_err(), crate::update::SCANNING);
    assert_eq!(f.app.jobs_waiting(), 1, "the second started nothing");
    assert!(f.notice().is_empty() || f.notice() == crate::update::SCANNING);
    f.quiet();
    assert_eq!(first.settle().unwrap()["library_size"], 300);
    std::fs::write(f.music.join("Gamma/zz new.mp3"), b"").unwrap();
    assert_eq!(f.act("rescan", json!({})).unwrap()["library_size"], 301);
    assert_eq!(f.state()["library_size"], 301);
}

#[test]
fn folders_are_added_and_removed_in_the_dialogs_words_and_kept() {
    let Some(f) = open("folders") else { return };
    let add = |path: String| {
        f.app.ui(move |ui| ui.invoke_scan_add_folder(path.into()));
        f.quiet();
        f.notice()
    };
    let music = std::fs::canonicalize(&f.music).unwrap().to_string_lossy().into_owned();
    assert!(add(music).ends_with("is already in the list."));
    assert!(add("relative/path".into()).contains("full path"));
    assert!(add(f.music.join("cover.jpg").to_string_lossy().into_owned()).starts_with("Not a folder"));
    let solo = f.dir.join("Solo");
    std::fs::create_dir_all(&solo).unwrap();
    std::fs::write(solo.join("only.flac"), b"").unwrap();
    let before = f.app.model(|m| m.folders.len());
    assert!(add(solo.to_string_lossy().into_owned()).is_empty(), "added, and no complaint");
    assert_eq!(f.app.model(|m| m.folders.len()), before + 1);
    assert!(f.status().ends_with(&format!("in {} folders", before + 1)), "{}", f.status());
    f.app.run_writes();
    let kept = f.app.committed_settings();
    assert_eq!(kept.folders.as_ref().map(Vec::len), Some(before + 1), "the folder list is kept");
    assert!(kept.folders.unwrap().iter().any(|p| p.ends_with("/Solo")));
    // Removed from the top: the rest are scanned again.
    while f.app.model(|m| m.folders.len()) > 1 {
        f.app.ui(|ui| ui.invoke_scan_remove_folder(0));
        f.quiet();
    }
    assert_eq!(f.status(), "1 track in 1 folder");
    f.app.ui(|ui| ui.invoke_scan_remove_folder(5));
    f.quiet();
    assert_eq!(f.notice(), "That folder is not in the list.");
}

/// `~/.config/yantrik/music.json`, as the old app wrote it: brought across once to the app's own
/// file, and left exactly as it was.
#[test]
fn the_file_from_before_the_framework_is_brought_across_and_left_as_it_was() {
    let Some(f) = open("legacy") else { return };
    let paths = f.app.paths();
    let former = paths.former_config_file("music.json").unwrap();
    let old: Value = serde_json::from_slice(&std::fs::read(&former).unwrap()).unwrap();
    assert_eq!(old, json!({"folders": [f.music], "volume": 40.0}), "the old file is as it was");
    assert_eq!(f.state()["volume"], 40.0);
    assert_eq!(f.state()["folders"][0]["path"], json!(f.music));
    assert!(paths.config_file().ends_with("apps/music/settings.json"), "{}", paths.config_file().display());
    let now: Value = serde_json::from_slice(&std::fs::read(paths.config_file()).unwrap()).unwrap();
    assert_eq!((now["app"].clone(), now["schema"].clone()), (json!("music"), json!(1)), "{now}");
    assert_eq!(now["settings"]["volume"], 40.0);
    // A change is written to the app's own file, never the old one.
    f.act("set_volume", json!({"percent": 55})).unwrap();
    f.app.run_writes();
    let now: Value = serde_json::from_slice(&std::fs::read(paths.config_file()).unwrap()).unwrap();
    assert_eq!(now["settings"]["volume"], 55.0);
    assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&former).unwrap()).unwrap(), old);
}

#[test]
fn a_first_run_lists_nothing_it_was_not_given_and_writes_nothing() {
    let Some(f) = open_with("first-run", Setup { kept: Some(json!({"folders": []})), ..Setup::default() }) else { return };
    assert_eq!(f.status(), "No library folders. Add one under Folders.");
    let err = f.act("play", json!({})).unwrap_err();
    assert_eq!(err, "Nothing to play: the library is empty. Add a folder that holds music under Folders.");
}

#[test]
fn the_settings_file_edited_outside_the_app_is_adopted() {
    let Some(f) = open("outside") else { return };
    let solo = f.dir.join("Elsewhere");
    std::fs::create_dir_all(&solo).unwrap();
    std::fs::write(solo.join("one.ogg"), b"").unwrap();
    let kept = Kept { folders: Some(vec![solo.to_string_lossy().into_owned()]), volume: 12.0 };
    f.app.event(yantrik_app::app::Event::SettingsChangedOutside(kept));
    f.quiet();
    assert_eq!(f.status(), "1 track in 1 folder");
    assert_eq!(f.state()["volume"], 12.0);
}
