//! Gate row "Close cancelled" (design/notes-gate-2026-10-21.md §3): close as a document flow. A
//! clean close asks nothing; a dirty one asks *Save / Don't save / Cancel* once, and each answer
//! does exactly what it says. Closing one of two windows on a note is in tests/windows.rs; quitting
//! with two notes unsaved in two windows is here.

use std::path::PathBuf;

use yantrik_app::app::{Event, ExitReason, WindowId};
use yantrik_app::documents::disk::Step;
use yantrik_testing::TestApp;

use super::windows::{two_notes_two_windows, type_in};
use super::{draft_file, failing, file, notice, settle, unsaved, with_note};
use crate::command::{Command, WindowAction};
use crate::update::closing::CLOSE_DIALOG;
use crate::update::{said, AUTOSAVE_AFTER};
use crate::Notes;

const TYPED: &str = "# Closing\n\nlast words";

/// A saved note with text typed and not saved yet, its draft written, and its file.
fn typed() -> (TestApp<Notes>, std::path::PathBuf) {
    let (app, path) = with_note("Closing");
    app.run(Command::Edit(TYPED.into()));
    app.run_writes();
    assert!(draft_file(&app, &path).exists(), "the draft is kept");
    (app, path)
}

fn prompt(app: &TestApp<Notes>) -> (i32, String) {
    app.ui(|ui| (ui.get_dialog(), ui.get_close_heading().to_string()))
}

fn press(app: &TestApp<Notes>, id: &'static str) {
    app.ui(move |ui| ui.invoke_action(id.into()));
}

#[test]
fn a_clean_close_asks_nothing() {
    let (app, _) = with_note("Clean");
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    assert_eq!(prompt(&app).0, 0, "no prompt");
}

#[test]
fn a_dirty_close_asks_and_cancel_keeps_everything_and_saves_nothing() {
    let (app, path) = typed();
    let saved = file(&path);
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(prompt(&app), (CLOSE_DIALOG, "Save \"Closing\" before closing?".into()));
    assert_eq!(app.exit_reason(), None);
    assert_eq!(app.jobs_waiting(), 0, "nothing is saved while the person decides");
    press(&app, "close-cancel");
    settle(&app);
    assert_eq!(prompt(&app).0, 0, "the prompt goes");
    assert_eq!((app.exit_reason(), app.jobs_waiting()), (None, 0), "the window stays and nothing was saved");
    assert_eq!(file(&path), saved);
    assert!(unsaved(&app), "the text is kept as edits");
    assert_eq!(app.model(|m| m.current().unwrap().text().to_string()), TYPED);
    assert!(draft_file(&app, &path).exists(), "and so is its draft");
    // Escape is Cancel too.
    app.close_requested(WindowId::PRIMARY);
    app.run(Command::ViewSynced { focus_mode: false, dialog: 0 });
    assert!(app.model(|m| m.closing.is_none()));
    assert_eq!(app.exit_reason(), None);
}

#[test]
fn save_closes_only_once_the_save_lands() {
    let (app, path) = typed();
    app.close_requested(WindowId::PRIMARY);
    press(&app, "close-save");
    assert_eq!(app.jobs_waiting(), 1, "the save is in flight");
    assert_eq!(app.exit_reason(), None, "and the window waits for it");
    settle(&app);
    assert_eq!(file(&path), TYPED);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "the draft goes with the save");
}

#[test]
fn text_typed_while_the_closing_save_runs_is_saved_too_before_it_closes() {
    let (app, path) = typed();
    app.close_requested(WindowId::PRIMARY);
    press(&app, "close-save");
    app.run(Command::Edit(format!("{TYPED}, and more")));
    assert!(app.run_one_job(), "the first save lands, and its follow-up starts");
    assert_eq!(app.exit_reason(), None, "not closed over the newer text");
    settle(&app);
    assert_eq!(file(&path), format!("{TYPED}, and more"));
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn a_failed_save_keeps_the_window_and_says_why() {
    let (app, path) = typed();
    let saved = file(&path);
    failing(&app).fail(Step::Rename, ".md", None);
    app.close_requested(WindowId::PRIMARY);
    press(&app, "close-save");
    settle(&app);
    assert_eq!(app.exit_reason(), None, "the window stays");
    assert!(notice(&app).contains("injected failure at Rename"), "and says why: {}", notice(&app));
    assert!(unsaved(&app) && file(&path) == saved);
    assert!(app.model(|m| m.closing.is_none()), "the close is over; asking again asks again");
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(prompt(&app).0, CLOSE_DIALOG);
}

#[test]
fn a_save_an_outside_change_holds_back_keeps_the_window_and_says_why() {
    let (app, path) = typed();
    std::fs::write(&path, "theirs").unwrap();
    app.look_at_files();
    app.close_requested(WindowId::PRIMARY);
    press(&app, "close-save");
    settle(&app);
    assert_eq!(app.exit_reason(), None);
    assert_eq!(notice(&app), super::not_saved_over());
    assert_eq!(file(&path), "theirs", "nothing was saved over theirs");
}

#[test]
fn dont_save_drops_the_edits_and_the_draft() {
    let (app, path) = typed();
    let saved = file(&path);
    app.close_requested(WindowId::PRIMARY);
    press(&app, "close-discard");
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    assert_eq!(app.jobs_waiting(), 0, "nothing saved");
    assert_eq!(file(&path), saved);
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "the draft is gone too");
}

#[test]
fn a_second_close_during_the_save_asks_nothing_and_saves_nothing_more() {
    let (app, path) = typed();
    app.close_requested(WindowId::PRIMARY);
    press(&app, "close-save");
    assert_eq!(app.jobs_waiting(), 1);
    app.close_requested(WindowId::PRIMARY);
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(prompt(&app).0, 0, "no second prompt");
    assert_eq!(app.jobs_waiting(), 1, "no second save");
    settle(&app);
    assert_eq!(file(&path), TYPED);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

/// Quitting asks once over every note with edits not on disk; with one window, one is asked about
/// (two windows on two notes are below).
#[test]
fn quitting_asks_once_and_ends_once_saved() {
    let (app, path) = typed();
    app.event(Event::ExitRequested(ExitReason::App));
    app.event(Event::ExitRequested(ExitReason::App));
    assert_eq!(prompt(&app), (CLOSE_DIALOG, "Save \"Closing\" before Notes closes?".into()), "asked once");
    press(&app, "close-save");
    settle(&app);
    assert_eq!(file(&path), TYPED);
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn logging_out_asks_nothing_and_writes_the_draft_now() {
    let (app, path) = with_note("Leaving");
    app.run(Command::Edit("# Leaving\n\nunsaved".into()));
    app.event(Event::ExitRequested(ExitReason::Logout));
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
    app.run_writes();
    assert!(draft_file(&app, &path).exists(), "the next start offers it back");
}

/// #914's open item: a library open read-only cannot save, so the prompt offers Don't save and
/// says Export keeps a copy, instead of waiting on a save the vault refuses.
#[test]
fn closing_a_read_only_library_with_unsaved_text_offers_dont_save() {
    let (app, _disk, path, saved, typed) = super::read_only::holding_unsaved_text();
    app.close_requested(WindowId::PRIMARY);
    let (heading, sentence) = app.ui(|ui| (ui.get_close_heading().to_string(), ui.get_close_said().to_string()));
    assert_eq!(app.ui(|ui| ui.get_dialog()), CLOSE_DIALOG);
    assert!(heading.starts_with("Save \""), "{heading}");
    assert_eq!(
        sentence,
        "This library is open read-only, so your changes cannot be saved here. Cancel, then Export, keeps a copy \
         of the text; Don't save closes without it.",
        "it says Export keeps a copy"
    );
    assert!(app.ui(|ui| ui.get_read_only()), "and the prompt's Save is not shown");
    // Export first, as the sentence says: Cancel, then the window's Export.
    press(&app, "close-cancel");
    let out = path.parent().unwrap().parent().unwrap().join(format!("closing-export-{}.md", std::process::id()));
    assert!(!out.starts_with(path.parent().unwrap()), "outside the library");
    let _ = std::fs::remove_file(&out);
    press(&app, "export");
    app.run(Command::Window(WindowAction::Confirm { path: out.display().to_string() }));
    settle(&app);
    assert_eq!(file(&out), typed, "the text got out");
    app.close_requested(WindowId::PRIMARY);
    press(&app, "close-discard");
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    assert_eq!(file(&path), saved, "nothing was written into the library");
}

/// Two windows on two notes, each with text not on disk, and the app asked to end.
fn two_unsaved_then_quit() -> (TestApp<Notes>, WindowId, PathBuf, WindowId, PathBuf) {
    let (app, first, left, second, right) = two_notes_two_windows();
    app.run(Command::From { window: first, command: Box::new(Command::Edit(LEFT.into())) });
    app.run(Command::From { window: second, command: Box::new(Command::Edit(RIGHT.into())) });
    app.event(Event::ExitRequested(ExitReason::App));
    app.event(Event::ExitRequested(ExitReason::App));
    (app, first, left, second, right)
}

const LEFT: &str = "# Left\n\nleft words";
const RIGHT: &str = "# Right\n\nright words";

/// Quitting with two notes unsaved in two windows asks once, over both, in the focused window;
/// Save saves one, then the other, and quits only when both have landed.
#[test]
fn quitting_with_two_unsaved_notes_in_two_windows_asks_once_and_saves_both() {
    let (app, first, left, second, right) = two_unsaved_then_quit();
    let asked = |w: WindowId| app.window(w, |ui| (ui.get_dialog(), ui.get_close_heading().to_string())).unwrap();
    assert_eq!(asked(second), (CLOSE_DIALOG, "Save 2 notes before Notes closes?".into()), "asked once, over both");
    assert_eq!(asked(first).0, 0, "in one window");
    let said = app.window(second, |ui| ui.get_close_said().to_string()).unwrap();
    assert!(said.contains("Left") && said.contains("Right"), "{said}");
    app.window(second, |ui| ui.invoke_action("close-save".into()));
    assert_eq!((app.jobs_waiting(), app.exit_reason()), (1, None), "one save at a time, and no quit yet");
    assert!(app.run_one_job(), "the first lands, and the second starts");
    assert_eq!((app.jobs_waiting(), app.exit_reason()), (1, None), "still not quit: one is not on disk");
    settle(&app);
    assert_eq!((file(&left), file(&right)), (LEFT.to_string(), RIGHT.to_string()));
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

/// A save that fails while quitting over two notes keeps both windows, both texts and the app,
/// and says which note did not save.
#[test]
fn a_failure_while_quitting_over_two_notes_keeps_everything_and_says_which() {
    let (app, first, left, second, right) = two_unsaved_then_quit();
    let name = right.file_name().unwrap().to_str().unwrap().to_string();
    failing(&app).fail(Step::Rename, &name, None);
    app.window(second, |ui| ui.invoke_action("close-save".into()));
    settle(&app);
    assert_eq!(app.exit_reason(), None, "Notes stays open");
    assert_eq!(app.windows(), vec![first, second], "with both windows");
    assert!(notice(&app).starts_with("\"Right\" was not saved, so Notes stays open."), "{}", notice(&app));
    assert!(notice(&app).contains("injected failure at Rename"), "and why: {}", notice(&app));
    assert_eq!(file(&left), LEFT, "the one that could be saved is");
    assert_eq!(file(&right), "# Right\n\n", "the other is not, and its file is as it was");
    let right_text = app.model(move |m| m.pane_of(second).and_then(|p| m.current_in(p)).unwrap().text().to_string());
    assert_eq!(right_text, RIGHT, "its text is kept, unsaved");
    assert!(app.model(|m| m.closing.is_none()), "the close is over; quitting again asks again");
}

/// #918's review, 3: another note's autosave failing while this window's close waits to save does
/// not end the close. This window's note is still saved and the window closes, and the failure is
/// said with the title of the note it is about.
#[test]
fn another_notes_failed_autosave_does_not_end_this_windows_close() {
    let (app, first, left, second, right) = two_notes_two_windows();
    type_in(&app, first, LEFT);
    type_in(&app, second, RIGHT);
    failing(&app).fail(Step::Rename, right.file_name().unwrap().to_str().unwrap(), None);
    app.advance(AUTOSAVE_AFTER);
    assert_eq!(app.jobs_waiting(), 1, "Right's autosave is in flight");
    app.close_requested(first);
    app.window(first, |ui| ui.invoke_action("close-save".into()));
    assert!(app.run_one_job(), "Right's save lands, failed");
    assert!(notice(&app).starts_with("\"Right\": "), "named: {}", notice(&app));
    assert!(notice(&app).contains("injected failure at Rename"), "{}", notice(&app));
    assert!(app.model(|m| m.closing.is_some()), "the close still waits on Left");
    settle(&app);
    assert_eq!(file(&left), LEFT, "Left is saved");
    assert_eq!(app.windows(), vec![second], "and its window closed");
    assert_eq!(file(&right), "# Right\n\n");
    let right_text = app.model(move |m| m.pane_of(second).and_then(|p| m.current_in(p)).unwrap().text().to_string());
    assert_eq!(right_text, RIGHT, "Right's text is kept, unsaved");
}

/// #918's review, 5: a window's close that would ask while another window's prompt waits says why
/// nothing happened.
#[test]
fn a_close_that_waits_for_another_windows_prompt_says_so() {
    let (app, first, _, second, _) = two_notes_two_windows();
    type_in(&app, first, LEFT);
    type_in(&app, second, RIGHT);
    app.close_requested(second);
    app.close_requested(first);
    assert_eq!(app.window(first, |ui| ui.get_dialog()), Some(0), "not asked over the other");
    assert_eq!(notice(&app), said::ANSWER_THE_OTHER_FIRST);
    assert_eq!(app.window(second, |ui| ui.get_dialog()), Some(CLOSE_DIALOG), "the other still asks");
    app.window(second, |ui| ui.invoke_action("close-cancel".into()));
    app.close_requested(first);
    assert_eq!(app.window(first, |ui| ui.get_dialog()), Some(CLOSE_DIALOG), "asked once the other is answered");
}
