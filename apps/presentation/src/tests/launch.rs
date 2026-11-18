//! Launches: the deck the command line names, a second launch handing its file to the window
//! already open, and the window coming forward.

use serde_json::json;
use yantrik_app::app::WindowId;
use yantrik_app::launch::Launch;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{act, deck_folder, notice, outline, path_of, read_deck, scratch, settle, started, type_title, write_deck};
use crate::YPresent;

/// A second launch naming `file`, as the framework's hand-over takes it.
fn hand_over(app: &TestApp<YPresent>, file: &std::path::Path) {
    let profile = app.paths().profile().to_string();
    let launch = Launch::of_files(&profile, std::path::Path::new("/"), [file.to_path_buf()]);
    app.act("launch", json!({ "launch": launch })).unwrap().settle().expect("the hand-over is taken");
    settle(app);
}

#[test]
fn the_deck_on_the_command_line_opens_in_place_of_the_empty_one() {
    let file = scratch("argv").join("given.ydeck");
    write_deck(&file, "Given", &["from the command line"]);
    let app = TestApp::<YPresent>::start_at(Paths::temporary("presentation"), vec![file.clone().into()]);
    deck_folder(&app);
    settle(&app);
    assert_eq!(path_of(&app), Some(file));
    assert_eq!(outline(&app), vec!["from the command line"]);
    assert_eq!(notice(&app), "", "a successful open clears the notice");
}

#[test]
fn a_file_that_is_not_a_deck_on_the_command_line_is_said_and_costs_nothing() {
    let file = scratch("argv-bad").join("bad.ydeck");
    std::fs::write(&file, "not a deck").unwrap();
    let app = TestApp::<YPresent>::start_at(Paths::temporary("presentation"), vec![file.clone().into()]);
    settle(&app);
    assert!(notice(&app).starts_with(&file.display().to_string()), "{}", notice(&app));
    assert_eq!(outline(&app), vec!["Title Slide"]);
}

#[test]
fn a_second_launch_hands_its_file_over_and_the_window_comes_forward() {
    let app = started();
    let file = scratch("second").join("handed.ydeck");
    write_deck(&file, "Handed", &["handed"]);
    hand_over(&app, &file);
    assert_eq!(path_of(&app), Some(file));
    app.present().unwrap();
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
}

#[test]
fn a_second_launch_over_an_unsaved_deck_writes_that_deck_to_a_file_of_its_own_first() {
    let app = started();
    let dir = deck_folder(&app);
    type_title(&app, "my unsaved words");
    let file = scratch("second-dirty").join("handed.ydeck");
    write_deck(&file, "Handed", &["handed"]);
    hand_over(&app, &file);
    assert_eq!(outline(&app), vec!["handed"]);
    let kept = dir.join("untitled.ydeck");
    assert_eq!(read_deck(&kept).slides[0].title, "my unsaved words");
    assert_eq!(notice(&app), format!("The deck you were editing was saved to {}.", kept.display()));
}

#[test]
fn a_launch_waits_for_a_file_operation_to_finish() {
    let (app, _) = super::with_file("launch-wait", &["One"]);
    let saving = app.act("save", super::none()).unwrap();
    let file = scratch("second-wait").join("later.ydeck");
    write_deck(&file, "Later", &["later"]);
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
    type_title(&app, "stays");
    app.launch_again().expect("taken");
    settle(&app);
    assert_eq!(outline(&app), vec!["stays"]);
    act(&app, "show", super::none()).unwrap();
}
