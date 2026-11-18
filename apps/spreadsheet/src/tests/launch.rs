//! Launches: the file the command line names, a second launch handing its file to the window
//! already open, and the window coming forward.

use serde_json::json;
use yantrik_app::app::WindowId;
use yantrik_app::launch::Launch;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{act, enter, notice, path_of, raw, scratch, settle, started};
use crate::YSheets;

/// A second launch naming `file`, as the framework's hand-over takes it.
fn hand_over(app: &TestApp<YSheets>, file: &std::path::Path) {
    let profile = app.paths().profile().to_string();
    let launch = Launch::of_files(&profile, std::path::Path::new("/"), [file.to_path_buf()]);
    app.act("launch", json!({ "launch": launch })).unwrap().settle().expect("the hand-over is taken");
    settle(app);
}

fn csv(name: &str, text: &str) -> std::path::PathBuf {
    let file = scratch(name).join("given.csv");
    std::fs::write(&file, text).unwrap();
    file
}

#[test]
fn the_file_on_the_command_line_opens_in_place_of_the_empty_sheet() {
    let file = csv("argv", "from the command line\r\n");
    let app = TestApp::<YSheets>::start_at(Paths::temporary("spreadsheet"), vec![file.clone().into()]);
    settle(&app);
    assert_eq!(path_of(&app), Some(file));
    assert_eq!(raw(&app, "A1"), "from the command line");
    assert_eq!(notice(&app), "", "a successful open clears the notice");
}

#[test]
fn a_file_that_is_not_csv_on_the_command_line_is_said_and_costs_nothing() {
    let file = csv("argv-bad", "a,\"never closed\n");
    let app = TestApp::<YSheets>::start_at(Paths::temporary("spreadsheet"), vec![file.clone().into()]);
    settle(&app);
    assert!(super::status(&app).starts_with(&file.display().to_string()), "{}", super::status(&app));
    assert_eq!(path_of(&app), None);
}

#[test]
fn a_second_launch_hands_its_file_over_and_the_window_comes_forward() {
    let app = started();
    let file = csv("second", "handed\r\n");
    hand_over(&app, &file);
    assert_eq!(path_of(&app), Some(file));
    assert_eq!(raw(&app, "A1"), "handed");
    app.present().unwrap();
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
}

#[test]
fn a_second_launch_over_unsaved_changes_is_refused_and_the_changes_stay() {
    let app = started();
    enter(&app, "my unsaved words");
    let file = csv("second-dirty", "handed\r\n");
    hand_over(&app, &file);
    assert_eq!(raw(&app, "A1"), "my unsaved words");
    assert_eq!(path_of(&app), None);
    assert_eq!(notice(&app), "Untitled has unsaved changes, so nothing was opened. Save them (save, save_over) or undo them first.");
}

#[test]
fn a_launch_waits_for_a_file_operation_to_finish() {
    let (app, _) = super::with_file("launch-wait", "1\r\n");
    let saving = app.act("save_over", super::none()).unwrap();
    let file = csv("second-wait", "later\r\n");
    let profile = app.paths().profile().to_string();
    let launch = Launch::of_files(&profile, std::path::Path::new("/"), [file.clone()]);
    app.act("launch", json!({ "launch": launch })).unwrap().settle().unwrap();
    assert_ne!(path_of(&app), Some(file.clone()), "not while a file is being written");
    settle(&app);
    saving.settle().unwrap();
    assert_eq!(path_of(&app), Some(file), "once it is done");
}

#[test]
fn a_launch_with_no_file_opens_nothing() {
    let app = started();
    enter(&app, "stays");
    app.launch_again().expect("taken");
    settle(&app);
    assert_eq!(raw(&app, "A1"), "stays");
    act(&app, "select", json!({ "cell": "A1" })).unwrap();
}

#[test]
fn a_launch_in_a_window_for_a_mind_is_refused_and_reads_nothing() {
    use std::sync::Arc;
    use yantrik_app::mind::fake::FakeMind;
    let file = csv("mind-launch", "secret\r\n");
    let app = TestApp::<YSheets>::start_for_mind(Paths::temporary("spreadsheet"), vec![file.into()], Arc::new(FakeMind::new()));
    settle(&app);
    assert_eq!(path_of(&app), None, "the person's file is not read for a Mind");
}
