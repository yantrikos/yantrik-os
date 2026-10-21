//! What a close or a quit asks, after the 520 run of #918: a prompt whose notes the autosave saved
//! under it answers itself (defect 3); a close refused while another window asks is said in its
//! own window only (defect 4); Ctrl+Q is a person's quit, asked once over every window, through
//! the framework's `fx.request_quit()` (#923); and a signal's quit (SIGTERM, SIGHUP) keeps every
//! unsaved text as a draft and ends at once, well inside the framework's deadline.

use std::time::Instant;

use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle as _, SharedString};
use yantrik_app::app::{ExitReason, WindowId};
use yantrik_app::cx::Inbound;
use yantrik_app::quit::{Signal, SIGNAL_DEADLINE};
use yantrik_testing::TestApp;

use super::windows::{two_notes_two_windows, type_in};
use super::{autosave, draft_file, file, killed_and_started_again, settle, state, with_note};
use crate::command::Command;
use crate::update::closing::CLOSE_DIALOG;
use crate::update::said;
use crate::Notes;

const LEFT: &str = "# Left\n\nleft words";
const RIGHT: &str = "# Right\n\nright words";

/// A window's dialog, the prompt's heading and sentence, and its notice line, as drawn.
fn drawn(app: &TestApp<Notes>, window: WindowId) -> (i32, String, String, String) {
    app.window(window, |ui| {
        (ui.get_dialog(), ui.get_close_heading().to_string(), ui.get_close_said().to_string(), ui.get_notice().to_string())
    })
    .expect("the window is open")
}

/// Ctrl+Q, pressed in the first window as the keyboard sends it.
fn ctrl_q(app: &TestApp<Notes>) {
    app.ui(|ui| {
        ui.show().unwrap();
        // The window's keys, as the window manager hands them to a window it activates.
        ui.invoke_focus_shortcuts();
        let key = |event| ui.window().dispatch_event(event);
        key(WindowEvent::KeyPressed { text: Key::Control.into() });
        key(WindowEvent::KeyPressed { text: SharedString::from("q") });
        key(WindowEvent::KeyReleased { text: SharedString::from("q") });
        key(WindowEvent::KeyReleased { text: Key::Control.into() });
    });
}

#[test]
fn a_close_prompt_whose_note_the_autosave_saved_answers_itself_and_closes() {
    let (app, path) = with_note("Closing");
    app.run(Command::Edit("# Closing\n\nlast words".into()));
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(drawn(&app, WindowId::PRIMARY).0, CLOSE_DIALOG);
    // The debounce runs out under the prompt, and its save lands.
    autosave(&app);
    assert_eq!(file(&path), "# Closing\n\nlast words");
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed), "nothing is left to ask, so it closes");
    assert!(app.model(|m| m.closing.is_none()));
}

/// Of two notes asked about at a quit, the one the autosave saved is no longer named: the prompt
/// never says "not saved yet" of a note that is.
#[test]
fn a_quit_prompt_names_only_the_notes_still_unsaved() {
    let (app, first, _, second, _) = two_notes_two_windows();
    type_in(&app, first, LEFT);
    type_in(&app, second, RIGHT);
    app.event(yantrik_app::app::Event::ExitRequested(ExitReason::Person));
    let (dialog, heading, ..) = drawn(&app, app.model(|m| m.pane.window));
    assert_eq!((dialog, heading.as_str()), (CLOSE_DIALOG, "Save 2 notes before Notes closes?"));
    // One debounce saves one note: the store does one thing at a time.
    autosave(&app);
    let (dialog, heading, sentence, _) = drawn(&app, app.model(|m| m.pane.window));
    assert_eq!(dialog, CLOSE_DIALOG, "one note is still unsaved");
    let left = state(&app)["windows"].as_array().unwrap().iter().find(|w| w["unsaved"] == true).cloned();
    let title = left.expect("one window still unsaved")["title"].as_str().unwrap().to_string();
    assert_eq!(heading, format!("Save \"{title}\" before Notes closes?"));
    assert_eq!(sentence, "Your changes to this note are not saved yet.");
    autosave(&app);
    assert_eq!(app.exit_reason(), Some(ExitReason::App), "both are saved, so Notes ends");
}

#[test]
fn a_close_refused_while_another_window_asks_is_said_only_in_its_own_window() {
    let (app, first, _, second, _) = two_notes_two_windows();
    type_in(&app, first, LEFT);
    type_in(&app, second, RIGHT);
    app.close_requested(second);
    app.close_requested(first);
    let asking = drawn(&app, second);
    assert_eq!((asking.0, asking.3.as_str()), (CLOSE_DIALOG, ""), "the asking window does not tell itself to answer");
    let refused = drawn(&app, first);
    assert_eq!((refused.0, refused.3.as_str()), (0, said::ANSWER_THE_OTHER_FIRST), "the refused window says why");
    // The next notice is every window's again.
    app.window(second, |ui| ui.invoke_action("close-cancel".into()));
    let e = super::act(&app, "compare", serde_json::json!({})).unwrap_err();
    assert_eq!((drawn(&app, first).3, drawn(&app, second).3), (e.clone(), e));
}

#[test]
fn ctrl_q_asks_once_over_every_window_and_saves_both() {
    let (app, first, left, second, right) = two_notes_two_windows();
    type_in(&app, first, LEFT);
    type_in(&app, second, RIGHT);
    app.run(Command::From { window: first, command: Box::new(Command::Activated) });
    ctrl_q(&app);
    let (dialog, heading, ..) = drawn(&app, first);
    assert_eq!((dialog, heading.as_str()), (CLOSE_DIALOG, "Save 2 notes before Notes closes?"), "asked once");
    assert_eq!(drawn(&app, second).0, 0, "in one window");
    app.ui(|ui| ui.invoke_action("close-save".into()));
    settle(&app);
    assert_eq!((file(&left), file(&right)), (LEFT.to_string(), RIGHT.to_string()));
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn ctrl_q_cancelled_keeps_notes_running_with_its_text() {
    let (app, path) = with_note("Stays");
    app.run(Command::Edit("# Stays\n\nunsaved".into()));
    ctrl_q(&app);
    assert_eq!(drawn(&app, WindowId::PRIMARY).1, "Save \"Stays\" before Notes closes?");
    app.ui(|ui| ui.invoke_action("close-cancel".into()));
    settle(&app);
    assert_eq!(app.exit_reason(), None);
    assert_eq!(file(&path), "# Stays\n\n", "nothing saved");
    assert_eq!(app.model(|m| m.current().unwrap().text().to_string()), "# Stays\n\nunsaved");
    // With nothing unsaved, Ctrl+Q ends Notes without a word.
    super::act(&app, "save", serde_json::json!({})).expect("saved");
    ctrl_q(&app);
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

/// Two notes typed in two windows, their drafts not written yet (the debounce has not run out).
fn typed_in_two_windows() -> (TestApp<Notes>, std::path::PathBuf, std::path::PathBuf) {
    let (app, first, left, second, right) = two_notes_two_windows();
    type_in(&app, first, LEFT);
    type_in(&app, second, RIGHT);
    assert!(!draft_file(&app, &left).exists() && !draft_file(&app, &right).exists(), "nothing written per key");
    (app, left, right)
}

/// The compositor goes, with no signal and no one asking: the framework owes Notes its logout,
/// and every unsaved text is on disk as a draft once the runner's shutdown has finished.
#[test]
fn the_display_going_keeps_every_unsaved_text_as_a_draft() {
    let (app, left, right) = typed_in_two_windows();
    app.display_gone();
    assert_eq!(app.ui(|ui| ui.get_dialog()), 0, "no one was asked");
    app.shutdown();
    assert!(draft_file(&app, &left).exists() && draft_file(&app, &right).exists(), "both drafts are on disk");
    assert_eq!(file(&left), "# Left\n\n", "and nothing was saved unasked");
    let again = killed_and_started_again(app);
    assert_eq!(state(&again)["recovery"].as_array().unwrap().len(), 2, "both are offered back");
}

/// A signal still queued when the display goes: one logout, not two, and the drafts are kept.
#[test]
fn a_signal_still_queued_when_the_display_goes_is_one_logout() {
    let (app, left, right) = typed_in_two_windows();
    app.post().post(Inbound::Signal(Signal::Terminate)).expect("the app is running");
    app.display_gone();
    app.shutdown();
    assert_eq!(app.ui(|ui| ui.get_dialog()), 0);
    assert!(draft_file(&app, &left).exists() && draft_file(&app, &right).exists());
}

/// After a signal, Ctrl+Q asks nothing: no one is there to answer.
#[test]
fn ctrl_q_after_a_signal_asks_nothing() {
    let (app, path) = with_note("Signalled");
    app.run(Command::Edit("# Signalled\n\nunsaved".into()));
    app.post().post(Inbound::Signal(Signal::Terminate)).expect("the app is running");
    app.tick();
    ctrl_q(&app);
    assert_eq!(app.ui(|ui| ui.get_dialog()), 0, "no prompt");
    app.run_writes();
    assert!(draft_file(&app, &path).exists());
}

/// SIGTERM (a shutdown, a logout) with two notes unsaved in two windows and a save in flight: no
/// one is asked, both texts are kept as drafts, and Notes ends in the one command, not at the
/// framework's deadline. The next start offers both back.
#[test]
fn a_signal_keeps_every_unsaved_text_as_a_draft_and_ends_at_once() {
    let (app, first, left, second, right) = two_notes_two_windows();
    type_in(&app, first, LEFT);
    type_in(&app, second, RIGHT);
    app.advance(crate::update::AUTOSAVE_AFTER);
    assert_eq!(app.jobs_waiting(), 1, "a save is in flight");
    let signalled = Instant::now();
    app.post().post(Inbound::Signal(Signal::Terminate)).expect("the app is running");
    app.tick();
    // Ended in the one command the signal raised: no job ran and no timer moved. The wall time
    // includes the test harness's hop to its UI thread, on a loaded test machine.
    assert_eq!(app.exit_reason(), Some(ExitReason::App), "Notes ended itself, not the deadline");
    assert_eq!(app.jobs_waiting(), 1, "it did not wait for the save in flight");
    assert!(signalled.elapsed() < SIGNAL_DEADLINE, "inside the deadline: {:?}", signalled.elapsed());
    assert_eq!(app.ui(|ui| ui.get_dialog()), 0, "no one was asked");
    app.run_writes();
    assert!(draft_file(&app, &left).exists() && draft_file(&app, &right).exists(), "both drafts are written");
    let again = killed_and_started_again(app);
    let offered = state(&again)["recovery"].as_array().unwrap().len();
    assert_eq!(offered, 2, "and both are offered back");
}
