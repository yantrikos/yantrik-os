//! Launches: the file the command line names, a second launch handing its file to the window
//! already open, and the window coming forward.

use yantrik_app::app::WindowId;
use yantrik_app::launch::Launch;
use serde_json::json;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{notice, path_of, press, scratch, settle, started, text, typed};
use crate::command::WindowAction;
use crate::YDoc;

/// A second launch naming `file`, as the framework's hand-over takes it.
fn hand_over(app: &TestApp<YDoc>, file: &std::path::Path) {
    let profile = app.paths().profile().to_string();
    let launch = Launch::of_files(&profile, std::path::Path::new("/"), [file.to_path_buf()]);
    app.act("launch", json!({ "launch": launch })).unwrap().settle().expect("the hand-over is taken");
    settle(app);
}

#[test]
fn the_file_on_the_command_line_opens_in_place_of_the_empty_document() {
    let file = scratch("argv").join("given.md");
    std::fs::write(&file, "from the command line\n").unwrap();
    let app = TestApp::<YDoc>::start_at(Paths::temporary("documents"), vec![file.clone().into()]);
    settle(&app);
    assert_eq!(path_of(&app), Some(file));
    assert_eq!(text(&app), "from the command line\n");
    assert_eq!(notice(&app), "", "a successful open clears the notice");
}

#[test]
fn a_second_launch_hands_its_file_over_and_the_window_comes_forward() {
    let app = started();
    let file = scratch("second").join("handed.md");
    std::fs::write(&file, "# Handed\n").unwrap();
    hand_over(&app, &file);
    assert_eq!(path_of(&app), Some(file));
    app.present().unwrap();
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
}

#[test]
fn a_second_launch_over_unsaved_text_is_refused_in_the_notice_and_the_draft_stays() {
    let app = started();
    typed(&app, "my unsaved words");
    let file = scratch("second-dirty").join("handed.md");
    std::fs::write(&file, "# Handed\n").unwrap();
    hand_over(&app, &file);
    assert_eq!(text(&app), "my unsaved words");
    assert!(
        notice(&app).starts_with(&format!("Opening {} would throw away 16 characters of", file.display())),
        "{}",
        notice(&app)
    );
}

#[test]
fn a_launch_waits_for_a_dialog_to_be_answered() {
    let app = started();
    press(&app, WindowAction::SaveAs);
    let file = scratch("second-wait").join("later.md");
    std::fs::write(&file, "# Later\n").unwrap();
    hand_over(&app, &file);
    assert_eq!(path_of(&app), None, "not while a dialog asks");
    press(&app, WindowAction::Cancel);
    assert_eq!(path_of(&app), Some(file), "once it is answered");
}

#[test]
fn a_launch_with_no_file_opens_nothing() {
    let app = started();
    typed(&app, "stays");
    app.launch_again().expect("taken");
    settle(&app);
    assert_eq!(text(&app), "stays");
}
