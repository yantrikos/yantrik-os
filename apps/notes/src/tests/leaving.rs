//! A note whose file is gone never traps the person (#925, decision 1). Clicking another note asks
//! *Save as new / Don't save / Cancel*, never refuses; closing its window asks the same, and *Save
//! as new* there keeps the text before the window closes; and the deleted bar's *Let it go*
//! (*Discard*, once it was edited) lets the note and its draft go at the person's word, said once,
//! so a restart offers nothing.

use std::path::PathBuf;

use yantrik_app::app::{ExitReason, WindowId};
use yantrik_testing::TestApp;

use super::windows::two_notes_two_windows;
use super::{bar, draft_file, file, killed_and_started_again, note, notice, settle, state, vault, with_note};
use crate::command::{Command, WindowAction};
use crate::update::closing::CLOSE_DIALOG;
use crate::update::said;
use crate::Notes;

/// "Other", saved, then "Gone", open, untouched, whose file another program deleted: the app and
/// Gone's file.
fn gone_beside_another() -> (TestApp<Notes>, PathBuf) {
    let (app, _) = with_note("Other");
    let path = PathBuf::from(note(&app, "Gone", "")["path"].as_str().unwrap());
    app.run_writes();
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    assert_eq!(bar(&app), 2);
    (app, path)
}

fn press(app: &TestApp<Notes>, id: &'static str) {
    app.ui(move |ui| ui.invoke_action(id.into()));
    settle(app);
}

fn other_id(app: &TestApp<Notes>) -> String {
    app.model(|m| m.library.iter().find(|n| n.title() == "Other").unwrap().id.clone())
}

fn click_other(app: &TestApp<Notes>) {
    let id = other_id(app);
    app.run(Command::Window(WindowAction::Open { id, trash: false }));
    settle(app);
}

/// The prompt as drawn: dialog, heading, sentence, save label.
fn prompt(app: &TestApp<Notes>) -> (i32, String, String, String) {
    app.ui(|ui| {
        (
            ui.get_dialog(),
            ui.get_close_heading().to_string(),
            ui.get_close_said().to_string(),
            ui.get_close_save_label().to_string(),
        )
    })
}

fn open_title(app: &TestApp<Notes>) -> Option<String> {
    app.model(|m| m.current().map(|c| c.title()))
}

fn listed(app: &TestApp<Notes>, title: &'static str) -> bool {
    app.model(move |m| m.library.iter().any(|n| n.title() == title && !n.trash))
}

#[test]
fn clicking_away_from_a_gone_note_asks_and_cancel_keeps_it() {
    let (app, _) = gone_beside_another();
    click_other(&app);
    let (dialog, heading, sentence, save) = prompt(&app);
    assert_eq!((dialog, heading.as_str()), (CLOSE_DIALOG, "Save \"Gone\" as a new note first?"), "asked, not refused");
    assert_eq!((sentence, save), (said::gone_prompt("Gone", said::Leaving::Note, false).1, said::SAVE_AS_NEW.to_string()));
    assert_eq!(notice(&app), "", "nothing was refused");
    press(&app, "close-cancel");
    assert_eq!((prompt(&app).0, open_title(&app)), (0, Some("Gone".to_string())), "it stays, as it was");
    assert_eq!(bar(&app), 2);
}

#[test]
fn dont_save_lets_the_gone_note_go_and_goes_on() {
    let (app, path) = gone_beside_another();
    click_other(&app);
    press(&app, "close-discard");
    assert_eq!(open_title(&app), Some("Other".to_string()), "the click went ahead");
    assert!(!listed(&app, "Gone"), "and the gone note is not listed");
    assert_eq!(notice(&app), said::let_go("Gone", false), "said once");
    app.run_writes();
    assert!(!draft_file(&app, &path).exists());
    let again = killed_and_started_again(app);
    assert!(state(&again)["recovery"].as_array().unwrap().is_empty(), "nothing is offered");
}

#[test]
fn save_as_new_keeps_the_text_and_then_goes_on() {
    let (app, path) = gone_beside_another();
    app.run(Command::Edit("# Gone\n\nworth keeping".into()));
    click_other(&app);
    assert_eq!(prompt(&app).2, said::gone_prompt("Gone", said::Leaving::Note, true).1, "worded for changes made");
    press(&app, "close-save");
    assert_eq!(open_title(&app), Some("Other".to_string()), "the click went ahead once the text was kept");
    let kept = app.model(|m| m.library.iter().find(|n| n.id.starts_with("copy-")).map(|n| n.id.clone()));
    assert_eq!(file(&vault(&app).join(kept.expect("a copy"))), "# Gone\n\nworth keeping");
    assert!(!path.exists(), "the deleted file is not made again");
    let again = killed_and_started_again(app);
    assert!(state(&again)["recovery"].as_array().unwrap().is_empty());
}

#[test]
fn let_it_go_clears_the_document_and_its_draft_and_a_restart_offers_nothing() {
    let (app, path) = gone_beside_another();
    app.run_writes();
    assert!(draft_file(&app, &path).exists(), "kept while the file is gone");
    assert!(!app.ui(|ui| ui.get_outside_edited()), "untouched: the button reads Let it go");
    press(&app, "let-go");
    assert_eq!(open_title(&app), None, "the document is gone");
    assert!(app.model(|m| m.documents.iter().count() == 0 && m.files.is_empty()));
    assert!(!listed(&app, "Gone"));
    assert_eq!(notice(&app), said::let_go("Gone", false));
    assert_eq!(bar(&app), 0);
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "and its draft");
    let again = killed_and_started_again(app);
    assert!(state(&again)["recovery"].as_array().unwrap().is_empty(), "a restart offers nothing");
    assert_eq!(notice(&again), "");
}

#[test]
fn an_edited_gone_note_offers_discard_and_says_its_changes_went() {
    let (app, _) = gone_beside_another();
    app.run(Command::Edit("# Gone\n\nchanged".into()));
    assert!(app.ui(|ui| ui.get_outside_edited()), "edited: the button reads Discard");
    press(&app, "let-go");
    assert_eq!(notice(&app), said::let_go("Gone", true));
    assert!(!listed(&app, "Gone"));
}

/// A note one window has open and another shows too: letting it go in one lets it go in both.
#[test]
fn let_it_go_in_one_window_lets_the_note_go_in_every_window() {
    let (app, first, left, second, _) = two_notes_two_windows();
    app.run(Command::From { window: first, command: Box::new(Command::Window(WindowAction::NewWindow)) });
    settle(&app);
    let third = *app.windows().last().unwrap();
    std::fs::remove_file(&left).unwrap();
    app.look_at_files();
    app.window(third, |ui| ui.invoke_action("let-go".into()));
    settle(&app);
    let shown = |w: WindowId| app.model(move |m| m.pane_of(w).and_then(|p| m.current_in(p)).map(|c| c.title()));
    assert_eq!((shown(first), shown(third), shown(second)), (None, None, Some("Right".to_string())));
}

#[test]
fn closing_a_gone_note_with_save_as_new_keeps_it_then_closes() {
    let (app, path) = gone_beside_another();
    app.close_requested(WindowId::PRIMARY);
    let (dialog, heading, _, save) = prompt(&app);
    assert_eq!((dialog, heading.as_str(), save.as_str()), (CLOSE_DIALOG, "Save \"Gone\" as a new note before closing?", "Save as new"));
    press(&app, "close-save");
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    let kept = app.model(|m| m.library.iter().any(|n| n.id.starts_with("copy-") && n.title() == "Gone"));
    assert!(kept, "kept as a new note before the window closed");
    assert!(!path.exists());
}

/// A keep that cannot be written keeps the window, says why, and lets the person choose again.
#[test]
fn a_close_whose_keep_cannot_be_written_stays_and_says_why() {
    let (app, _) = gone_beside_another();
    super::failing(&app).fail(yantrik_app::documents::disk::Step::Write, ".md", None);
    app.close_requested(WindowId::PRIMARY);
    press(&app, "close-save");
    assert_eq!(app.exit_reason(), None, "the window stays");
    assert!(notice(&app).starts_with("Your text could not be kept as a separate note"), "{}", notice(&app));
    assert!(app.model(|m| m.closing.is_none()), "the close is over; asking again asks again");
    assert_eq!(open_title(&app), Some("Gone".to_string()));
}

/// A window's prompt as it draws it: dialog, heading, save label.
fn drawn(app: &TestApp<Notes>, window: WindowId) -> (i32, String, String) {
    app.window(window, |ui| (ui.get_dialog(), ui.get_close_heading().to_string(), ui.get_close_save_label().to_string()))
        .expect("the window is open")
}

fn from(app: &TestApp<Notes>, window: WindowId, action: WindowAction) {
    app.run(Command::From { window, command: Box::new(Command::Window(action)) });
    settle(app);
}

/// P4: the file comes back while the prompt asks to leave it. The question is no longer true,
/// so it goes.
#[test]
fn a_leaving_prompt_goes_when_the_file_comes_back() {
    let (app, path) = gone_beside_another();
    click_other(&app);
    assert_eq!(prompt(&app).0, CLOSE_DIALOG);
    std::fs::write(&path, "# Gone\n\nback from elsewhere\n").unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!((prompt(&app).0, bar(&app)), (0, 0), "no stale question");
    assert!(app.model(|m| m.leaving.is_none()));
    assert_eq!(open_title(&app), Some("Gone".to_string()));
}

/// P5: let go in another window while one asks to leave it: the asking window's question goes.
#[test]
fn a_leaving_prompt_goes_when_its_note_is_let_go_in_another_window() {
    let (app, _) = gone_beside_another();
    from(&app, WindowId::PRIMARY, WindowAction::NewWindow);
    let third = *app.windows().last().unwrap();
    let id = other_id(&app);
    from(&app, WindowId::PRIMARY, WindowAction::Open { id, trash: false });
    assert_eq!(drawn(&app, WindowId::PRIMARY).0, CLOSE_DIALOG);
    app.window(third, |ui| ui.invoke_action("let-go".into()));
    settle(&app);
    assert_eq!(drawn(&app, WindowId::PRIMARY).0, 0, "the question it asked is gone with the note");
    assert!(app.model(|m| m.leaving.is_none()));
}
