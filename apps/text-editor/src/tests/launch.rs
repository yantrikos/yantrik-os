//! Launches: the file the command line names, a second launch handing its file to the window
//! already open, and the window coming forward.

use serde_json::json;
use yantrik_app::app::WindowId;
use yantrik_app::launch::Launch;
use yantrik_testing::TestApp;

use super::{press, scratch, settle, state, tabs, text};
use crate::command::WindowAction;
use crate::TextEditor;

fn path_in_front(app: &TestApp<TextEditor>) -> Option<std::path::PathBuf> {
    app.model(|m| m.path(m.active_tab()).map(|p| p.to_path_buf()))
}

#[test]
fn the_file_on_the_command_line_opens_in_place_of_the_empty_tab() {
    let file = scratch("argv").join("given.txt");
    std::fs::write(&file, "from the command line\n").unwrap();
    let app = TestApp::<TextEditor>::start_at(yantrik_app::paths::Paths::temporary("editor"), vec![file.clone().into()]);
    settle(&app);
    assert_eq!(tabs(&app), 1, "the empty tab is replaced");
    assert_eq!(path_in_front(&app), Some(file));
    assert_eq!(text(&app), "from the command line\n");
}

#[test]
fn a_second_launch_hands_its_file_over_and_waits_for_a_dialog_to_be_answered() {
    let app = super::started();
    let file = scratch("second").join("handed.txt");
    std::fs::write(&file, "handed over\n").unwrap();
    press(&app, WindowAction::Open);
    let profile = app.paths().profile().to_string();
    let launch = Launch::of_files(&profile, std::path::Path::new("/"), [file.clone()]);
    app.act("launch", json!({ "launch": launch })).unwrap().settle().expect("the hand-over is taken");
    settle(&app);
    assert_eq!(path_in_front(&app), None, "not while the Open dialog asks");
    press(&app, WindowAction::Cancel);
    settle(&app);
    assert_eq!(path_in_front(&app), Some(file), "opened once the dialog was answered");
    assert_eq!(state(&app)["dialog"], 0);
}

#[test]
fn the_window_coming_forward_changes_nothing() {
    let app = super::started();
    super::typed(&app, "as it was");
    let before = state(&app);
    app.deactivate(WindowId::PRIMARY);
    app.activate(WindowId::PRIMARY);
    app.present().unwrap();
    settle(&app);
    assert_eq!(state(&app), before);
    assert!(app.exit_reason().is_none() && !app.lingering());
}
