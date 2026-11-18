//! One prompt at a time, of any kind (update/prompts.rs; the 520 run of #925). While one window's
//! close prompt or leaving prompt asks, another window's new prompt is refused, said in that
//! window only, and the first prompt stays whole and answerable. A close with nothing to ask still
//! closes. A dialog with no live prompt behind it is never shown. Each order is tested.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::app::WindowId;
use yantrik_app::documents::disk::Step;
use yantrik_testing::TestApp;

use super::windows::type_in;
use super::{act, autosave, failing, note, settle, with_note};
use crate::command::{Command, WindowAction};
use crate::update::closing::CLOSE_DIALOG;
use crate::update::said;
use crate::Notes;

const FIRST: WindowId = WindowId::PRIMARY;

/// "Other", "Left" open in the first window and "Right" in a second: the app, the second
/// window, Left's and Right's files, and Other's file name.
fn left_and_right() -> (TestApp<Notes>, WindowId, PathBuf, PathBuf, String) {
    let (app, other) = with_note("Other");
    let left = PathBuf::from(note(&app, "Left", "")["path"].as_str().unwrap());
    let right = PathBuf::from(note(&app, "Right", "")["path"].as_str().unwrap());
    let answer = act(&app, "open_in_new_window", json!({ "title": "Right" })).expect("opened");
    let second = WindowId(answer["window"].as_u64().unwrap() as u32);
    from(&app, FIRST, WindowAction::Open { id: name(&left), trash: false });
    (app, second, left, right, name(&other))
}

fn name(path: &std::path::Path) -> String {
    path.file_name().unwrap().to_str().unwrap().to_string()
}

fn from(app: &TestApp<Notes>, window: WindowId, action: WindowAction) {
    app.run(Command::From { window, command: Box::new(Command::Window(action)) });
    settle(app);
}

fn gone(app: &TestApp<Notes>, paths: &[&PathBuf]) {
    for path in paths {
        std::fs::remove_file(path).unwrap();
    }
    app.look_at_files();
}

/// A window as drawn: its dialog, the prompt's heading, and its notice line.
fn drawn(app: &TestApp<Notes>, window: WindowId) -> (i32, String, String) {
    app.window(window, |ui| (ui.get_dialog(), ui.get_close_heading().to_string(), ui.get_notice().to_string()))
        .expect("the window is open")
}

fn press_in(app: &TestApp<Notes>, window: WindowId, id: &'static str) {
    app.window(window, move |ui| ui.invoke_action(id.into()));
    settle(app);
}

fn leaves(app: &TestApp<Notes>, window: WindowId, other: &str) {
    from(app, window, WindowAction::Open { id: other.to_string(), trash: false });
}

const LEFT_ASKS: &str = "Save \"Left\" as a new note first?";
const RIGHT_ASKS: &str = "Save \"Right\" as a new note first?";

/// The 520 run's blocking defect: a leaving prompt, then another window's leave. The second is
/// refused, said there only, and the first stays whole and answers.
#[test]
fn a_leave_while_another_windows_leave_asks_is_refused_there() {
    let (app, second, left, right, other) = left_and_right();
    gone(&app, &[&left, &right]);
    leaves(&app, FIRST, &other);
    assert_eq!(drawn(&app, FIRST).1, LEFT_ASKS);
    leaves(&app, second, &other);
    assert_eq!(drawn(&app, second), (0, String::new(), said::ANSWER_THE_OTHER_FIRST.to_string()), "refused, said there");
    assert_eq!(drawn(&app, FIRST), (CLOSE_DIALOG, LEFT_ASKS.to_string(), String::new()), "the first still asks, whole");
    press_in(&app, FIRST, "close-cancel");
    assert_eq!(drawn(&app, FIRST).0, 0, "and its Cancel answers it");
    leaves(&app, second, &other);
    assert_eq!(drawn(&app, second).1, RIGHT_ASKS, "once answered, the other window may ask");
}

/// The other order.
#[test]
fn the_same_the_other_way_round() {
    let (app, second, left, right, other) = left_and_right();
    gone(&app, &[&left, &right]);
    leaves(&app, second, &other);
    leaves(&app, FIRST, &other);
    assert_eq!(drawn(&app, FIRST), (0, String::new(), said::ANSWER_THE_OTHER_FIRST.to_string()));
    assert_eq!(drawn(&app, second), (CLOSE_DIALOG, RIGHT_ASKS.to_string(), String::new()));
    press_in(&app, second, "close-discard");
    assert_eq!(app.model(move |m| m.pane_of(second).and_then(|p| m.current_in(p)).map(|c| c.title())), Some("Other".into()));
    leaves(&app, FIRST, &other);
    assert_eq!(drawn(&app, FIRST).1, LEFT_ASKS);
}

/// A close prompt, then another window's leave: refused; the close prompt answers.
#[test]
fn a_leave_while_another_windows_close_asks_is_refused_there() {
    let (app, second, left, _, other) = left_and_right();
    type_in(&app, second, "# Right\n\nunsaved");
    gone(&app, &[&left]);
    app.close_requested(second);
    leaves(&app, FIRST, &other);
    assert_eq!(drawn(&app, FIRST), (0, String::new(), said::ANSWER_THE_OTHER_FIRST.to_string()));
    assert_eq!(drawn(&app, second).1, "Save \"Right\" before closing?");
    press_in(&app, second, "close-discard");
    assert_eq!(app.windows(), vec![FIRST], "the close went ahead");
    leaves(&app, FIRST, &other);
    assert_eq!(drawn(&app, FIRST).1, LEFT_ASKS);
}

/// A leaving prompt, then another window's close that would ask: refused; a close with nothing to
/// ask still closes.
#[test]
fn a_close_while_another_windows_leave_asks_is_refused_unless_it_asks_nothing() {
    let (app, second, left, _, other) = left_and_right();
    gone(&app, &[&left]);
    leaves(&app, FIRST, &other);
    type_in(&app, second, "# Right\n\nunsaved");
    app.close_requested(second);
    assert_eq!(drawn(&app, second), (0, String::new(), said::ANSWER_THE_OTHER_FIRST.to_string()));
    assert_eq!(drawn(&app, FIRST), (CLOSE_DIALOG, LEFT_ASKS.to_string(), String::new()));
    // Saved, it has nothing to ask, so it closes while the other window asks.
    autosave(&app);
    app.close_requested(second);
    assert_eq!(app.windows(), vec![FIRST]);
    assert_eq!(drawn(&app, FIRST).1, LEFT_ASKS);
}

/// A dialog with no prompt behind it is put away and never drawn.
#[test]
fn a_prompt_dialog_with_no_prompt_behind_it_is_never_shown() {
    let (app, _) = with_note("Plain");
    app.run(Command::ViewSynced { focus_mode: false, dialog: CLOSE_DIALOG });
    assert_eq!(app.model(|m| m.pane.dialog), 0);
    assert_eq!(app.ui(|ui| ui.get_dialog()), 0);
}

/// The refusal is said in the refused window; the asking window keeps showing its save failure.
#[test]
fn a_refusal_does_not_hide_the_asking_windows_save_failure() {
    let (app, second, _, _, _) = left_and_right();
    failing(&app).fail(Step::Write, ".md", None);
    type_in(&app, FIRST, "# Left\n\nunsaved");
    type_in(&app, second, "# Right\n\nunsaved");
    autosave(&app);
    autosave(&app);
    app.close_requested(FIRST);
    let failure = drawn(&app, FIRST).2;
    assert!(failure.contains("injected failure at Write"), "{failure}");
    app.close_requested(second);
    assert_eq!(drawn(&app, second).2, said::ANSWER_THE_OTHER_FIRST);
    assert_eq!(drawn(&app, FIRST).2, failure, "still in view where it was");
}

/// *Let it go* is said in the windows that showed the note, not in one showing another note.
#[test]
fn let_it_go_is_said_only_where_the_note_was_shown() {
    let (app, second, left, _, _) = left_and_right();
    gone(&app, &[&left]);
    press_in(&app, FIRST, "let-go");
    assert_eq!(drawn(&app, FIRST).2, said::let_go("Left", false));
    assert_eq!(drawn(&app, second).2, "", "the window on Right says nothing of Left");
}
