//! What the 520 run of #918 saw drawn, drawn again: a conflict said once in full (the bar), with
//! the footer's short state and no notice repeating it (defect 5); a note whose file is gone,
//! said in the footer in amber (defect 2); and a copy or a recovered note told apart on its card
//! from the note it was kept from, whose title it has (defect 6). Each is written as a headless
//! screenshot beside the others (`ui-screenshots/`).

use std::path::PathBuf;

use slint::{ComponentHandle as _, Model as _};
use yantrik_testing::TestApp;

use super::window::screenshot;
use super::{not_saved_over, notice, settle, with_note};
use crate::command::{Command, WindowAction};
use crate::update::said;
use crate::Notes;

const MINE: &str = "# Outside test\n\nSaved body.\n\nMy local edit.";

/// A note, shown at the size the 520 run's screenshots were, edited, and changed by another
/// program; the person's Save is then refused over it.
fn refused_over_a_change() -> (TestApp<Notes>, PathBuf) {
    let (app, path) = with_note("Outside test");
    app.ui(|ui| {
        ui.show().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1120, 680));
    });
    app.run(Command::Edit(MINE.into()));
    std::fs::write(&path, "# Outside test\n\nSaved body.\n\nAppended by an outside editor.\n").unwrap();
    app.look_at_files();
    app.run(Command::Window(WindowAction::Save));
    settle(&app);
    (app, path)
}

/// The window's notice line, its bar's sentence, and its footer.
fn said_in_window(app: &TestApp<Notes>) -> (String, String, String) {
    app.ui(|ui| (ui.get_notice().to_string(), ui.get_outside_said().to_string(), ui.get_status().to_string()))
}

/// The library cards as drawn: title and marker.
fn cards(app: &TestApp<Notes>) -> Vec<(String, String)> {
    app.ui(|ui| {
        let rows = ui.get_notes();
        (0..rows.row_count())
            .filter_map(|i| rows.row_data(i))
            .map(|r| (r.title.to_string(), r.marker.to_string()))
            .collect()
    })
}

#[test]
fn a_conflict_is_said_once_in_full_and_the_footer_is_a_short_state() {
    let (app, _) = refused_over_a_change();
    assert_eq!(notice(&app), not_saved_over(), "the refusal is the model's notice, for describe");
    let (line, bar, footer) = said_in_window(&app);
    assert_eq!(line, "", "the window's notice line does not repeat the bar");
    assert_eq!(bar, said::outside(&yantrik_app::documents::OutsideState::Conflict { now: None }).unwrap());
    assert_eq!(footer, said::WAITING_ON_CHOICE);
    screenshot(&app, "notes-conflict-said-once.png");
    // Put away with Not now, the bar no longer says it, so the notice line does.
    app.ui(|ui| ui.invoke_action("not-now".into()));
    assert_eq!(said_in_window(&app).0, not_saved_over());
}

#[test]
fn a_note_whose_file_is_gone_says_so_in_the_footer() {
    let (app, path) = with_note("Kill test");
    app.ui(|ui| {
        ui.show().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1120, 680));
    });
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    let footer = app.ui(|ui| (ui.get_status().to_string(), ui.get_modified()));
    assert_eq!(footer, (said::FILE_GONE.to_string(), true), "in amber");
    screenshot(&app, "notes-file-gone.png");
}

#[test]
fn a_copy_and_a_recovered_note_say_so_on_their_cards() {
    let (app, path) = refused_over_a_change();
    app.ui(|ui| ui.invoke_action("keep-mine".into()));
    settle(&app);
    let mut drawn = cards(&app);
    drawn.sort();
    let both = vec![("Outside test".to_string(), String::new()), ("Outside test".to_string(), "Copy".to_string())];
    assert_eq!(drawn, both, "two notes of one title, told apart");
    screenshot(&app, "notes-kept-copy.png");
    // Take theirs keeps the window's text as a recovered note, which says so too.
    app.run(Command::Window(WindowAction::Open { id: path.file_name().unwrap().to_str().unwrap().into(), trash: false }));
    app.run(Command::Edit("# Outside test\n\nmine again".into()));
    std::fs::write(&path, "# Outside test\n\ntheirs again\n").unwrap();
    app.look_at_files();
    app.ui(|ui| ui.invoke_action("take-theirs".into()));
    settle(&app);
    let markers: Vec<String> = cards(&app).into_iter().map(|(_, marker)| marker).filter(|m| !m.is_empty()).collect();
    assert_eq!(markers.len(), 2, "{markers:?}");
    assert!(markers.contains(&"Copy".to_string()) && markers.contains(&"Recovered".to_string()), "{markers:?}");
}

/// The empty state in a half-width window: its heading wraps rather than running off the edge.
#[test]
fn the_empty_state_fits_a_narrow_window() {
    let app = super::started();
    app.ui(|ui| {
        ui.show().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(640, 680));
    });
    assert!(!app.ui(|ui| ui.get_opened()));
    screenshot(&app, "notes-empty-narrow.png");
}
