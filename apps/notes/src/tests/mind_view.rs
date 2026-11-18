//! Notes drawn for a Mind (Mind View): `read_mind_view` reads the window as text, so the window
//! holds no note text. Titles stay; the editor takes no typing; Copy changes nothing; and a search
//! matches only what is shown.

use serde_json::json;
use slint::Model as _;
use yantrik_app::for_mind::{HIDDEN, READ_ONLY};
use yantrik_testing::TestApp;

use super::{act, note, settle, started};
use crate::command::{Command, WindowAction};
use crate::Notes;

const SENTINEL: &str = "SENTINEL-notes-mind-view-8e2b";

/// A library holding one note whose body carries the sentinel, and where it is kept.
fn kept() -> yantrik_app::Paths {
    let app = started();
    note(&app, "Plain title", &format!("# Plain title\n\nA body with {SENTINEL} in it.\n"));
    let paths = app.paths();
    let _ = app.shutdown();
    paths
}

fn start_for_mind(paths: yantrik_app::Paths) -> TestApp<Notes> {
    let app = TestApp::<Notes>::start_for_mind(paths, Vec::new(), std::sync::Arc::new(yantrik_app::mind::fake::FakeMind::new()));
    settle(&app);
    open_first(&app);
    app
}

/// The first card opened, and the preview turned on so every part of the window is drawn.
fn open_first(app: &TestApp<Notes>) {
    let id = app.ui(|ui| ui.get_notes().row_data(0).unwrap().id.to_string());
    app.run(Command::Window(WindowAction::Open { id, trash: false }));
    app.run(Command::Window(WindowAction::Preview));
    settle(app);
}

/// Every string the window holds that a screen reader or OCR could see.
fn window_text(app: &TestApp<Notes>) -> String {
    app.ui(|ui| {
        let mut all = vec![
            ui.get_content().to_string(),
            ui.get_note_title().to_string(),
            ui.get_notebook().to_string(),
            ui.get_tags().to_string(),
            ui.get_status().to_string(),
            ui.get_notice().to_string(),
            ui.get_theirs().to_string(),
            ui.get_outside_said().to_string(),
            ui.get_recovery_said().to_string(),
        ];
        let rows = ui.get_notes();
        for i in 0..rows.row_count() {
            let r = rows.row_data(i).unwrap();
            all.extend([r.title, r.preview, r.date, r.marker].map(|s| s.to_string()));
        }
        let blocks = ui.get_blocks();
        all.extend((0..blocks.row_count()).map(|i| blocks.row_data(i).unwrap().text.to_string()));
        all.join("\n")
    })
}

#[test]
fn a_mind_view_window_holds_no_note_text_but_keeps_titles() {
    let app = start_for_mind(kept());
    let text = window_text(&app);
    assert!(!text.contains(SENTINEL), "{text}");
    assert!(text.contains("Plain title"), "the title stays: {text}");
    assert_eq!(app.ui(|ui| ui.get_content().to_string()), HIDDEN);
    let preview = app.ui(|ui| ui.get_notes().row_data(0).unwrap().preview.to_string());
    assert_eq!(preview, HIDDEN);
    assert!(app.ui(|ui| ui.get_read_only()), "the editor takes no typing");
    assert_eq!(app.ui(|ui| ui.get_words()), 0, "and no count of the words it hides");
}

#[test]
fn a_normal_window_still_shows_the_note() {
    let paths = kept();
    let app = TestApp::<Notes>::start_at(paths, Vec::new());
    settle(&app);
    open_first(&app);
    assert!(window_text(&app).contains(SENTINEL));
    assert!(!app.ui(|ui| ui.get_read_only()));
}

#[test]
fn typing_and_copying_in_a_mind_view_window_change_nothing() {
    let paths = kept();
    let app = start_for_mind(paths.clone());
    let before = super::snapshot(paths.data_dir());
    app.run(Command::Edit(HIDDEN.into()));
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    app.run(Command::Window(WindowAction::Copy));
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    settle(&app);
    assert_eq!(super::snapshot(paths.data_dir()), before, "no note was written or made");
    assert!(!app.model(|m| m.unsaved()));
}

#[test]
fn the_search_box_in_a_mind_view_window_matches_titles_not_text() {
    let app = start_for_mind(kept());
    let shown = |app: &TestApp<Notes>, query: &str| {
        app.run(Command::Filter { folder: "all".into(), query: query.into() });
        app.ui(|ui| ui.get_notes().row_count())
    };
    assert_eq!(shown(&app, SENTINEL), 0, "the text is not searched");
    assert_eq!(shown(&app, "plain title"), 1, "the title still finds it");
}

/// What a caller gets from a Notes over `paths`: `describe` and the answers of the calls that
/// read notes, with the note open.
fn what_a_caller_gets(app: &TestApp<Notes>) -> serde_json::Value {
    let d = app.describe();
    let search = act(app, "search", json!({ "query": SENTINEL })).unwrap();
    let read = act(app, "read_notes", json!({ "query": SENTINEL })).unwrap();
    json!({ "summary": d["summary"], "state": d["state"], "search": search, "read": read })
}

#[test]
fn masking_is_visual_only_describe_and_calls_are_as_for_a_normal_instance() {
    let paths = kept();
    let normal = TestApp::<Notes>::start_at(paths.clone(), Vec::new());
    settle(&normal);
    open_first(&normal);
    let expected = what_a_caller_gets(&normal);
    let _ = normal.shutdown();
    drop(normal);
    let masked = start_for_mind(paths);
    let got = what_a_caller_gets(&masked);
    assert!(expected.to_string().contains(SENTINEL), "a Mind still reads notes through the calls");
    assert_eq!(got, expected);
}

#[test]
fn every_edit_and_file_path_in_a_mind_view_window_is_refused_before_anything_changes() {
    let paths = kept();
    let app = start_for_mind(paths.clone());
    let before = super::snapshot(paths.data_dir());
    let title = app.ui(|ui| ui.get_note_title().to_string());
    let actions = [
        WindowAction::Undo,
        WindowAction::Redo,
        WindowAction::Save,
        WindowAction::Import,
        WindowAction::Export,
        WindowAction::Confirm { path: "/tmp/out.md".into() },
        WindowAction::KeepMine,
        WindowAction::TakeTheirs,
        WindowAction::RestoreDraft,
        WindowAction::DiscardDraft,
        WindowAction::Trash,
        WindowAction::Restore,
        WindowAction::Pin,
        WindowAction::Copy,
    ];
    for action in actions {
        app.run(Command::Window(action.clone()));
        assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY, "{action:?}");
    }
    app.run(Command::Metadata { notebook: "elsewhere".into(), tags: "x".into() });
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    settle(&app);
    assert_eq!(super::snapshot(paths.data_dir()), before, "no note was written, made or exported");
    assert!(!app.model(|m| m.unsaved()));
    assert_eq!(app.ui(|ui| ui.get_note_title().to_string()), title);
    assert!(!std::path::Path::new("/tmp/out.md").exists());
}

#[test]
fn the_recovery_bar_in_a_mind_view_window_gives_no_length_of_the_text() {
    // A note typed in, killed, and its file gone: the bar describes the unsaved text.
    let (app, path) = super::with_note("Lost");
    app.run(Command::Edit("# Lost

words typed before the crash".into()));
    app.run_writes();
    let paths = app.paths();
    drop(app);
    std::fs::remove_file(&path).unwrap();
    let mut bars = Vec::new();
    for for_mind in [false, true] {
        let app = if for_mind {
            let mind = std::sync::Arc::new(yantrik_app::mind::fake::FakeMind::new());
            TestApp::<Notes>::start_for_mind(paths.clone(), Vec::new(), mind)
        } else {
            TestApp::<Notes>::start_at(paths.clone(), Vec::new())
        };
        settle(&app);
        bars.push(app.ui(|ui| ui.get_recovery_said().to_string()));
    }
    assert!(bars[0].contains("characters"), "{}", bars[0]);
    assert!(!bars[1].is_empty() && !bars[1].contains("characters"), "{}", bars[1]);
}
