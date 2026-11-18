//! Gate row "Two document contexts" (design/notes-gate-2026-10-21.md §3): documents, views and
//! secondary windows, headless through the framework's window table. One note in two windows is
//! one document: what is typed in one is drawn in the other, and the two share its unsaved state,
//! its conflict and its draft; each window keeps its own undo history; save and undo go to the
//! focused window; closing one window leaves the app running and asks nothing about a note the
//! other still shows; the last window closing with unsaved text asks.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};
use yantrik_app::documents::OutsideState;
use yantrik_testing::TestApp;

use super::{act, autosave, draft_file, file, killed_and_started_again, note, notice, settle, state, with_note};
use crate::command::{Command, WindowAction};
use crate::update::closing::CLOSE_DIALOG;
use crate::update::said;
use crate::Notes;

/// A command as the window `window` sends it.
fn from(app: &TestApp<Notes>, window: WindowId, command: Command) {
    app.run(Command::From { window, command: Box::new(command) });
}

pub(super) fn type_in(app: &TestApp<Notes>, window: WindowId, text: &str) {
    from(app, window, Command::Edit(text.into()));
}

fn press_in(app: &TestApp<Notes>, window: WindowId, action: WindowAction) {
    from(app, window, Command::Window(action));
    settle(app);
}

/// The focused window's note in a window of its own, as its button opens it: the new window.
fn opened_beside(app: &TestApp<Notes>) -> WindowId {
    let before = app.windows();
    let focused = app.model(|m| m.pane.window);
    press_in(app, focused, WindowAction::NewWindow);
    let after = app.windows();
    assert_eq!(after.len(), before.len() + 1, "a window was opened");
    *after.iter().find(|w| !before.contains(w)).unwrap()
}

/// What a window's editor shows, and its footer.
fn editor(app: &TestApp<Notes>, window: WindowId) -> String {
    app.window(window, |ui| ui.get_content().to_string()).expect("the window is open")
}

fn footer(app: &TestApp<Notes>, window: WindowId) -> (String, bool) {
    app.window(window, |ui| (ui.get_status().to_string(), ui.get_modified())).expect("the window is open")
}

fn bar_in(app: &TestApp<Notes>, window: WindowId) -> i32 {
    app.window(window, |ui| ui.get_outside()).expect("the window is open")
}

fn note_in(app: &TestApp<Notes>, window: WindowId) -> Option<String> {
    app.model(move |m| m.pane_of(window).and_then(|p| m.current_in(p)).map(|c| c.id().to_string()))
}

fn text(app: &TestApp<Notes>) -> String {
    app.model(|m| m.current().unwrap().text().to_string())
}

/// Two saved notes, "Left" open in the first window and "Right" in a second: their windows and
/// files.
pub(super) fn two_notes_two_windows() -> (TestApp<Notes>, WindowId, PathBuf, WindowId, PathBuf) {
    let (app, right) = with_note("Right");
    let left = PathBuf::from(note(&app, "Left", "")["path"].as_str().unwrap());
    let answer = act(&app, "open_in_new_window", json!({ "title": "Right" })).expect("opened");
    let second = WindowId(answer["window"].as_u64().unwrap() as u32);
    (app, WindowId::PRIMARY, left, second, right)
}

#[test]
fn a_note_opens_in_a_new_window_from_its_button_and_from_the_intent() {
    let (app, path) = with_note("Shared");
    let second = opened_beside(&app);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY, second]);
    let id = path.file_name().unwrap().to_str().unwrap().to_string();
    assert_eq!(note_in(&app, second), Some(id.clone()), "the same note");
    assert_eq!(app.model(|m| m.documents.iter().count()), 1, "as one document");
    assert_eq!(editor(&app, second), file(&path), "drawn in the new window");
    // The intent runs the same command: the focused window's note, or the one it names.
    let answer = act(&app, "open_in_new_window", json!({})).expect("opened");
    assert_eq!((answer["open_note"].as_str(), answer["windows"].as_u64()), (Some(id.as_str()), Some(3)));
    note(&app, "Other", "");
    let answer = act(&app, "open_in_new_window", json!({ "title": "Shared" })).expect("opened by name");
    assert_eq!(answer["title"], "Shared");
    let refused = act(&app, "open_in_new_window", json!({ "title": "Nowhere" })).unwrap_err();
    assert!(refused.starts_with("No note is called \"Nowhere\""), "{refused}");
}

#[test]
fn with_nothing_open_a_new_window_is_refused_and_said() {
    let app = super::started();
    let refused = act(&app, "open_in_new_window", json!({})).unwrap_err();
    assert_eq!(refused, format!("{} {}", said::NOTHING_FOR_A_WINDOW, said::NAME_A_NOTE));
    press_in(&app, WindowId::PRIMARY, WindowAction::NewWindow);
    assert_eq!((app.windows().len(), notice(&app)), (1, said::NOTHING_FOR_A_WINDOW.to_string()));
}

#[test]
fn an_edit_in_one_window_is_drawn_in_the_other_with_one_unsaved_state_and_one_draft() {
    let (app, path) = with_note("Shared");
    let second = opened_beside(&app);
    let typed = "# Shared\n\ntyped in the first window";
    type_in(&app, WindowId::PRIMARY, typed);
    assert_eq!(editor(&app, second), typed, "the other window draws it");
    for window in [WindowId::PRIMARY, second] {
        assert_eq!(footer(&app, window), ("Unsaved · autosave pending".to_string(), true), "{window:?}");
    }
    let windows = state(&app)["windows"].clone();
    assert_eq!(windows.as_array().unwrap().iter().filter(|w| w["unsaved"] == true).count(), 2, "{windows}");
    app.run_writes();
    let drafts = std::fs::read_dir(draft_file(&app, &path).parent().unwrap()).unwrap().count();
    assert_eq!(drafts, 1, "one note, one draft");
    // The second window types on; the first draws that, and one save takes both.
    type_in(&app, second, &format!("{typed}, and the second"));
    assert_eq!(editor(&app, WindowId::PRIMARY), format!("{typed}, and the second"));
    autosave(&app);
    assert_eq!(file(&path), format!("{typed}, and the second"));
    for window in [WindowId::PRIMARY, second] {
        assert_eq!(footer(&app, window), ("All changes saved".to_string(), false), "{window:?}");
    }
}

#[test]
fn undo_takes_back_only_the_focused_windows_own_edits_and_never_over_the_others() {
    let (app, _) = with_note("Shared");
    let second = opened_beside(&app);
    type_in(&app, WindowId::PRIMARY, "# Shared\n\none");
    type_in(&app, second, "# Shared\n\none two");
    // The first window's history is of a text the second has changed since: dropped, not applied.
    press_in(&app, WindowId::PRIMARY, WindowAction::Undo);
    assert_eq!(text(&app), "# Shared\n\none two", "nothing was put back over the second window's words");
    assert_eq!(notice(&app), said::UNDO_DROPPED);
    let refused = act(&app, "undo", json!({})).unwrap_err();
    assert_eq!(refused, "There is nothing left to undo on this note in this window.", "dropped for good");
    // The second window's own edit is still its to undo.
    press_in(&app, second, WindowAction::Undo);
    assert_eq!(text(&app), "# Shared\n\none");
    assert_eq!(editor(&app, WindowId::PRIMARY), "# Shared\n\none", "and the first draws it");
    // Through the surface, in the window that is focused: the drop is said as the refusal.
    type_in(&app, second, "# Shared\n\none three");
    type_in(&app, WindowId::PRIMARY, "# Shared\n\none three four");
    from(&app, second, Command::Activated);
    assert_eq!(act(&app, "undo", json!({})).unwrap_err(), said::UNDO_DROPPED);
}

#[test]
fn two_windows_on_two_notes_each_undo_and_save_their_own() {
    let (app, first, left, second, right) = two_notes_two_windows();
    type_in(&app, first, "# Left\n\nleft words");
    type_in(&app, second, "# Right\n\nright words");
    press_in(&app, first, WindowAction::Undo);
    assert_eq!(editor(&app, first), "# Left\n\n");
    assert_eq!(editor(&app, second), "# Right\n\nright words", "the other note is not touched");
    // Save goes to the focused window's note: the first's, which its undo left as its file holds it.
    let answer = act(&app, "save", json!({})).expect("nothing to write");
    assert_eq!(answer["title"], "Left");
    assert_eq!(file(&right), "# Right\n\n", "the other window's note is not saved by it");
    from(&app, second, Command::Activated);
    let answer = act(&app, "save", json!({})).expect("saved");
    assert_eq!(answer["title"], "Right");
    assert_eq!(file(&right), "# Right\n\nright words");
    assert_eq!(file(&left), "# Left\n\n");
}

#[test]
fn a_save_landing_for_one_window_does_not_call_the_others_note_saved() {
    let (app, first, left, second, right) = two_notes_two_windows();
    type_in(&app, first, "# Left\n\nunsaved");
    type_in(&app, second, "# Right\n\nsaved first");
    // The autosave saves the focused window's note first, then the other at its next debounce.
    autosave(&app);
    assert_eq!(file(&right), "# Right\n\nsaved first");
    assert_eq!(footer(&app, second), ("All changes saved".to_string(), false));
    assert_eq!(footer(&app, first), ("Unsaved · autosave pending".to_string(), true), "its own note is not saved");
    assert_eq!(file(&left), "# Left\n\n");
    autosave(&app);
    assert_eq!(file(&left), "# Left\n\nunsaved", "and then it is");
    assert_eq!(footer(&app, first), ("All changes saved".to_string(), false));
}

#[test]
fn closing_one_of_two_windows_on_a_note_asks_nothing_and_the_last_one_asks() {
    let (app, path) = with_note("Shared");
    let second = opened_beside(&app);
    type_in(&app, second, "# Shared\n\nnot saved yet");
    app.close_requested(second);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY], "it closed, with nothing asked");
    assert_eq!((app.exit_reason(), app.ui(|ui| ui.get_dialog())), (None, 0), "and the app runs on");
    assert_eq!(text(&app), "# Shared\n\nnot saved yet", "the note stays open, unsaved, in the other");
    assert!(app.watched().iter().any(|(_, p)| *p == path), "and watched");
    // The last window asks.
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.ui(|ui| ui.get_dialog()), CLOSE_DIALOG);
    assert_eq!(app.ui(|ui| ui.get_close_heading().to_string()), "Save \"Shared\" before closing?");
    app.ui(|ui| ui.invoke_action("close-save".into()));
    settle(&app);
    assert_eq!(file(&path), "# Shared\n\nnot saved yet");
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn closing_a_window_whose_note_no_other_window_shows_asks_about_it_and_the_app_runs_on() {
    let (app, first, _, second, right) = two_notes_two_windows();
    type_in(&app, second, "# Right\n\nonly here");
    app.close_requested(second);
    assert_eq!(app.window(second, |ui| ui.get_dialog()), Some(CLOSE_DIALOG), "asked in its own window");
    assert_eq!(app.window(first, |ui| ui.get_dialog()), Some(0), "and not in the other");
    app.window(second, |ui| ui.invoke_action("close-save".into()));
    settle(&app);
    assert_eq!(file(&right), "# Right\n\nonly here");
    assert_eq!((app.windows(), app.exit_reason()), (vec![first], None), "that window closed; the app runs on");
}

#[test]
fn a_notes_watch_is_dropped_when_its_last_window_lets_it_go() {
    let (app, first, left, second, right) = two_notes_two_windows();
    let watched = |app: &TestApp<Notes>, path: &PathBuf| app.watched().iter().any(|(_, p)| p == path);
    assert!(watched(&app, &left) && watched(&app, &right));
    app.close_requested(second);
    assert!(!watched(&app, &right), "the closed window's note is no longer watched");
    let third = opened_beside(&app);
    app.close_requested(third);
    assert!(watched(&app, &left), "a note the first window still shows is watched still");
    let _ = first;
}

#[test]
fn one_conflict_is_shown_in_both_windows_and_taking_theirs_in_either_resolves_it_for_both() {
    let (app, path) = with_note("Shared");
    let second = opened_beside(&app);
    type_in(&app, WindowId::PRIMARY, "# Shared\n\nmine");
    std::fs::write(&path, "# Shared\n\ntheirs\n").unwrap();
    app.look_at_files();
    assert_eq!((bar_in(&app, WindowId::PRIMARY), bar_in(&app, second)), (1, 1), "the same bar in both");
    press_in(&app, second, WindowAction::TakeTheirs);
    assert_eq!(app.model(|m| m.current().unwrap().doc.outside.clone()), OutsideState::Clean);
    assert_eq!((bar_in(&app, WindowId::PRIMARY), bar_in(&app, second)), (0, 0), "resolved for both");
    for window in [WindowId::PRIMARY, second] {
        assert_eq!(editor(&app, window), "# Shared\n\ntheirs\n", "{window:?}");
    }
    // Every view's undo history started again: the first window's typing is not undone over theirs.
    press_in(&app, WindowId::PRIMARY, WindowAction::Undo);
    assert_eq!(text(&app), "# Shared\n\ntheirs\n");
}

#[test]
fn keep_mine_in_either_window_moves_both_to_the_copy() {
    let (app, path) = with_note("Shared");
    let second = opened_beside(&app);
    type_in(&app, second, "# Shared\n\nmine");
    std::fs::write(&path, "# Shared\n\ntheirs\n").unwrap();
    app.look_at_files();
    press_in(&app, WindowId::PRIMARY, WindowAction::KeepMine);
    let (first_note, second_note) = (note_in(&app, WindowId::PRIMARY), note_in(&app, second));
    assert!(first_note.as_deref().is_some_and(|id| id.starts_with("copy-")), "{first_note:?}");
    assert_eq!(first_note, second_note, "both windows show the copy, which holds the text");
    assert_eq!((bar_in(&app, WindowId::PRIMARY), bar_in(&app, second)), (0, 0));
    assert_eq!(file(&path), "# Shared\n\ntheirs\n", "the other program's file is left as it wrote it");
    assert!(!app.watched().iter().any(|(_, p)| *p == path), "and no window watches it any more");
}

#[test]
fn the_recovery_offer_appears_in_the_window_that_shows_its_note() {
    let (app, _) = with_note("Lost");
    note(&app, "Elsewhere", "");
    super::act(&app, "open_note", json!({ "title": "Lost" })).unwrap();
    app.run(Command::Edit("# Lost\n\ntyped before the crash".into()));
    app.run_writes();
    let app = killed_and_started_again(app);
    let recovery = |app: &TestApp<Notes>, w: WindowId| app.window(w, |ui| ui.get_recovery()).unwrap();
    assert_eq!(recovery(&app, WindowId::PRIMARY), 1, "the first window shows the note and its offer");
    let second = opened_beside(&app);
    assert_eq!(recovery(&app, second), 1, "a second window on that note shows it too");
    press_in(&app, WindowId::PRIMARY, WindowAction::Open { id: note_id(&app, "Elsewhere"), trash: false });
    assert_eq!((recovery(&app, WindowId::PRIMARY), recovery(&app, second)), (0, 1), "only where its note is");
    press_in(&app, second, WindowAction::RestoreDraft);
    assert_eq!(editor(&app, second), "# Lost\n\ntyped before the crash");
    assert_eq!(recovery(&app, second), 0);
}

fn note_id(app: &TestApp<Notes>, title: &'static str) -> String {
    app.model(move |m| m.library.iter().find(|n| n.title() == title).unwrap().id.clone())
}

#[test]
fn describe_names_each_window_and_the_focused_one_and_the_actions_act_there() {
    let (app, first, _, second, _) = two_notes_two_windows();
    let s = state(&app);
    assert_eq!(s["focused_window"], second.0, "the new window is the focused one");
    assert_eq!(s["title"], "Right", "and every other key is about its note");
    let windows: Vec<(u64, bool, String)> = s["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["window"].as_u64().unwrap(), w["focused"] == true, w["title"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(windows, vec![(0, false, "Left".into()), (second.0 as u64, true, "Right".into())]);
    // The person moves to the first window: the window manager gives it the keys.
    app.event(Event::Activated(first));
    assert_eq!((state(&app)["focused_window"].clone(), state(&app)["title"].clone()), (json!(0), json!("Left")));
    act(&app, "append", json!({ "text": "\nadded" })).expect("appended");
    assert_eq!(editor(&app, first), "# Left\n\n\nadded", "to the focused window's note");
    assert_eq!(editor(&app, second), "# Right\n\n");
}

#[test]
fn a_window_that_could_not_be_made_takes_its_view_with_it_and_says_so() {
    let (app, path) = with_note("Shared");
    let second = opened_beside(&app);
    app.event(Event::WindowNotOpened { window: second, why: "No display".into() });
    assert_eq!(app.model(|m| m.windows()), vec![WindowId::PRIMARY], "its pane is gone");
    assert_eq!(notice(&app), said::window_not_opened("No display"));
    assert!(app.watched().iter().any(|(_, p)| *p == path), "the note is open where it was");
}

#[test]
fn a_window_with_nothing_to_ask_closes_while_another_windows_prompt_waits() {
    let (app, first, _, second, right) = two_notes_two_windows();
    type_in(&app, second, "# Right\n\nonly here");
    app.close_requested(second);
    assert_eq!(app.window(second, |ui| ui.get_dialog()), Some(CLOSE_DIALOG));
    // The person goes to the first window and opens its note beside it, then closes that.
    from(&app, first, Command::Activated);
    let third = opened_beside(&app);
    app.close_requested(third);
    assert_eq!(app.windows(), vec![first, second], "the window with nothing to ask closed");
    assert_eq!(app.window(second, |ui| ui.get_dialog()), Some(CLOSE_DIALOG), "and the prompt still waits");
    app.window(second, |ui| ui.invoke_action("close-cancel".into()));
    assert_eq!(file(&right), "# Right\n\n", "Cancel saved nothing");
    assert_eq!(editor(&app, second), "# Right\n\nonly here", "and kept everything");
}

/// Unsaved is one rule, whichever window asks (#909's deferred item): a note typed away from its
/// file in one window and back in the other is saved in both footers, in `describe`, to the
/// autosave and to the close, which asks nothing. Every test also checks the other direction:
/// after every command, a note with text not on disk must be a dirty document
/// (`Model::keep_unsaved_dirty`, which fails a test that breaks it).
#[test]
fn a_note_typed_back_to_its_file_in_another_window_is_saved_to_every_reader() {
    let (app, path) = with_note("Shared");
    let second = opened_beside(&app);
    let saved = file(&path);
    type_in(&app, WindowId::PRIMARY, "# Shared\n\nfor a moment");
    assert!(footer(&app, second).1, "unsaved, in the window that did not type it");
    type_in(&app, second, &saved);
    for window in [WindowId::PRIMARY, second] {
        assert_eq!(footer(&app, window), ("All changes saved".to_string(), false), "{window:?}");
    }
    assert_eq!(state(&app)["unsaved"], false);
    autosave(&app);
    assert_eq!(app.jobs_waiting(), 0, "nothing to write");
    app.close_requested(second);
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed), "and nothing to ask");
    assert_eq!(file(&path), saved);
}

/// #918's review, 1: a window closed from the taskbar or the window manager while the person types
/// in another leaves the focus, `describe` and the surface's calls in the person's window.
#[test]
fn a_window_closed_in_the_background_leaves_the_focus_where_the_person_is() {
    let (app, first, _, second, _) = two_notes_two_windows();
    let third = opened_beside(&app);
    type_in(&app, third, "# Right\n\ntyped in the third");
    app.close_requested(second);
    assert_eq!(app.windows(), vec![first, third], "it closed, with nothing to ask");
    assert_eq!(app.model(|m| m.pane.window), third, "the focus stays where the person types");
    assert_eq!(state(&app)["focused_window"], third.0);
}

/// The same with a close that asks: its prompt shows in its own window, and what the surface
/// asks for meanwhile is done to the note the person is in.
#[test]
fn a_close_prompt_raised_in_the_background_leaves_the_surfaces_calls_on_the_persons_note() {
    let (app, first, _, second, _) = two_notes_two_windows();
    type_in(&app, second, "# Right\n\nonly here");
    app.event(Event::Activated(first));
    app.close_requested(second);
    assert_eq!(app.window(second, |ui| ui.get_dialog()), Some(CLOSE_DIALOG), "asked in its own window");
    assert_eq!(app.window(first, |ui| ui.get_dialog()), Some(0), "and not in the person's");
    assert_eq!(state(&app)["focused_window"], first.0, "where the focus stays");
    let answer = act(&app, "append", json!({ "text": "\nadded" })).expect("appended");
    assert_eq!(answer["title"], "Left", "to the note the person is in");
    assert_eq!(editor(&app, first), "# Left\n\n\nadded");
    assert_eq!(editor(&app, second), "# Right\n\nonly here");
    // The prompt's own buttons act in its window.
    app.window(second, |ui| ui.invoke_action("close-cancel".into()));
    assert_eq!(app.window(second, |ui| ui.get_dialog()), Some(0));
    assert_eq!(app.windows(), vec![first, second], "Cancel kept it open");
}

/// Closing the focused window gives the focus back to the window the person was in before it, not
/// to the lowest numbered one; and when that one closes too, to the one before that.
#[test]
fn closing_the_focused_window_gives_the_focus_back_to_the_window_the_person_was_in_before() {
    let (app, first, _, second, _) = two_notes_two_windows();
    let third = opened_beside(&app);
    app.event(Event::Activated(second));
    app.close_requested(second);
    assert_eq!((app.windows(), app.model(|m| m.pane.window)), (vec![first, third], third));
    app.close_requested(third);
    assert_eq!((app.windows(), app.model(|m| m.pane.window)), (vec![first], first));
}

/// #918's review, 4: an offer for a note no window shows is on one window's bar, the focused
/// one's, and moves with the person; it is never offered in two windows at once.
#[test]
fn an_offer_for_a_note_no_window_shows_is_on_one_windows_bar_only() {
    let (app, _) = with_note("Third");
    note(&app, "Left", "");
    note(&app, "Right", "");
    act(&app, "open_note", json!({ "title": "Third" })).unwrap();
    app.run(Command::Edit("# Third\n\ntyped before the crash".into()));
    app.run_writes();
    let app = killed_and_started_again(app);
    press_in(&app, WindowId::PRIMARY, WindowAction::Open { id: note_id(&app, "Left"), trash: false });
    let answer = act(&app, "open_in_new_window", json!({ "title": "Right" })).expect("opened");
    let second = WindowId(answer["window"].as_u64().unwrap() as u32);
    let bars = |app: &TestApp<Notes>| {
        app.windows().into_iter().map(|w| app.window(w, |ui| ui.get_recovery()).unwrap()).collect::<Vec<_>>()
    };
    assert_eq!(app.windows(), vec![WindowId::PRIMARY, second]);
    assert_eq!(bars(&app), vec![0, 2], "on the focused window's bar only");
    app.event(Event::Activated(WindowId::PRIMARY));
    assert_eq!(bars(&app), vec![2, 0], "and it moves with the person");
    press_in(&app, WindowId::PRIMARY, WindowAction::RestoreDraft);
    assert_eq!(editor(&app, WindowId::PRIMARY), "# Third\n\ntyped before the crash");
    assert_eq!(bars(&app), vec![0, 0], "restored once, offered nowhere after");
}
