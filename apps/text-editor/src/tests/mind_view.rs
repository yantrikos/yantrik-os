//! The Text Editor drawn for a Mind (Mind View): `read_mind_view` reads the window as text, so the
//! window holds no document text. File names stay; the field takes no typing; a search finds
//! nothing; and the gutter, colours and counts say nothing about the text.

use yantrik_app::for_mind::{HIDDEN, READ_ONLY};
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App as _, Paths};
use yantrik_testing::TestApp;

use super::{scratch, settle, typed};
use crate::command::Command;
use crate::TextEditor;

const SENTINEL: &str = "SENTINEL-editor-mind-view-3d9c";

/// A Text Editor drawn for a Mind (or not) with a file holding the sentinel open.
fn with_document(for_mind: bool) -> (TestApp<TextEditor>, std::path::PathBuf) {
    // The file reaches the window as a launch argument: a masked window refuses the Open dialog.
    let path = scratch("mind-view").join("plan.rs");
    std::fs::write(&path, format!("fn main() {{
    // {SENTINEL}
    let s = \"{SENTINEL}\";
}}
")).unwrap();
    let paths = Paths::temporary(TextEditor::ID);
    let app = if for_mind {
        TestApp::<TextEditor>::start_for_mind(paths, vec![path.clone().into_os_string()], std::sync::Arc::new(FakeMind::new()))
    } else {
        TestApp::<TextEditor>::start_at(paths, vec![path.clone().into_os_string()])
    };
    settle(&app);
    (app, path)
}

/// Every string the window holds that a screen reader or OCR could see.
fn window_text(app: &TestApp<TextEditor>) -> String {
    app.ui(|ui| {
        let tabs = ui.get_tabs();
        let mut all = vec![
            ui.get_content().to_string(),
            ui.get_numbers().to_string(),
            ui.get_keywords().to_string(),
            ui.get_strings().to_string(),
            ui.get_comments().to_string(),
            ui.get_document_title().to_string(),
            ui.get_path_label().to_string(),
            ui.get_language().to_string(),
            ui.get_ending().to_string(),
            ui.get_view_status().to_string(),
            ui.get_notice().to_string(),
            ui.get_recovery_status().to_string(),
            ui.get_query().to_string(),
        ];
        all.extend((0..slint::Model::row_count(&tabs)).map(|i| slint::Model::row_data(&tabs, i).unwrap().title.to_string()));
        all.join("\n")
    })
}

#[test]
fn a_mind_view_window_holds_no_document_text_but_keeps_the_file_name() {
    let (app, _) = with_document(true);
    let text = window_text(&app);
    assert!(!text.contains(SENTINEL), "{text}");
    assert!(text.contains("plan.rs"), "the name stays: {text}");
    assert_eq!(app.ui(|ui| ui.get_content().to_string()), HIDDEN);
    assert!(app.ui(|ui| ui.get_masked()), "the field takes no keys");
    assert!(!app.ui(|ui| ui.get_has_highlights()), "and no colours spell the text");
    assert_eq!(app.ui(|ui| ui.get_line_count()), 0);
}

#[test]
fn a_normal_window_still_shows_the_document() {
    let (app, _) = with_document(false);
    assert!(window_text(&app).contains(SENTINEL));
    assert!(!app.ui(|ui| ui.get_masked()));
}

#[test]
fn typing_in_a_mind_view_window_changes_nothing() {
    let (app, path) = with_document(true);
    let before = super::file(&path);
    typed(&app, HIDDEN);
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    assert!(!super::modified(&app));
    assert!(super::text(&app).contains(SENTINEL), "the document is as it was");
    assert_eq!(super::file(&path), before);
}

#[test]
fn a_search_in_a_mind_view_window_finds_nothing() {
    let (app, _) = with_document(true);
    app.run(Command::Find { query: SENTINEL.into(), match_case: false });
    assert_eq!(app.ui(|ui| ui.get_match_count()), 0);
    assert!(!window_text(&app).contains(SENTINEL), "and the query is the only place the word is");
}

#[test]
fn masking_is_visual_only_describe_and_calls_are_as_for_a_normal_instance() {
    let mut got = Vec::new();
    for for_mind in [false, true] {
        let (app, path) = with_document(for_mind);
        let d = app.describe();
        let read = super::act(&app, "read", super::none()).unwrap();
        let find = super::act(&app, "find", serde_json::json!({ "text": SENTINEL })).unwrap();
        // The scratch folder differs per run; the name of the file does not.
        let text = serde_json::json!([d["summary"], d["state"], read, find]).to_string();
        got.push(text.replace(&path.parent().unwrap().display().to_string(), "<dir>"));
    }
    assert!(got[0].contains(SENTINEL), "a Mind still reads the document through the calls");
    assert_eq!(got[0], got[1]);
}

#[test]
fn every_edit_path_in_a_mind_view_window_is_refused_before_the_document_changes() {
    use crate::command::WindowAction as W;
    let (app, path) = with_document(true);
    let disk = super::file(&path);
    let before = super::text(&app);
    app.run(Command::Find { query: SENTINEL.into(), match_case: false });
    let actions = [
        W::Replace { with: "PROBE-REPLACED".into() },
        W::ReplaceAll { with: "PROBE-REPLACED".into() },
        W::Undo,
        W::Redo,
        W::Open,
        W::Save,
        W::SaveAs,
        W::SaveClose,
        W::Goto,
        W::Discard,
        W::Confirm { path: path.display().to_string() },
        W::Confirm { path: "/etc/hostname".into() },
    ];
    for action in actions {
        app.run(Command::Window(action.clone()));
        assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY, "{action:?}");
    }
    settle(&app);
    assert_eq!(super::text(&app), before, "the document is as it was");
    assert!(!super::modified(&app));
    assert_eq!(super::file(&path), disk, "nothing was written");
    assert_eq!(super::tabs(&app), 1, "and nothing was opened");
    assert!(!window_text(&app).contains("PROBE-REPLACED"));
}

#[test]
fn a_mind_view_window_gives_no_measure_of_the_text_in_the_caret() {
    let (app, _) = with_document(true);
    // An offset into the mask, past its first line break in the real text.
    app.run(Command::Cursor(20));
    settle(&app);
    assert_eq!((app.ui(|ui| ui.get_cursor_line()), app.ui(|ui| ui.get_cursor_column())), (1, 1));
    let (normal, _) = with_document(false);
    normal.run(Command::Cursor(20));
    settle(&normal);
    assert!(normal.ui(|ui| ui.get_cursor_line()) > 1, "a normal window still follows the caret");
}
