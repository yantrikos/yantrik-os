//! The window and the session ending. This screen has no dialog layer to ask in, so nothing is
//! asked: the deck goes to its draft in the command that answers, and the next start offers it
//! back. Nothing the person made is dropped, and no dialog shows without a question behind it.

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};

use super::{
    act, draft_file, file, killed_and_started_again, notice, outline, scratch, settle, started, state, type_title,
    with_file, write_deck,
};
use crate::YPresent;

#[test]
fn a_deck_with_nothing_unsaved_closes_at_once() {
    let app = started();
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    let (app, _) = with_file("close-clean", &["One"]);
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn closing_with_unsaved_changes_asks_nothing_and_keeps_the_deck_for_the_next_start() {
    let (app, path) = with_file("close-dirty", &["One"]);
    type_title(&app, "Unsaved at close");
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed), "no one was asked");
    assert!(draft_file(&app).exists() || app.writes_waiting() > 0, "the draft is handed to the lane");
    app.shutdown();
    let again = killed_and_started_again(app);
    assert_eq!(outline(&again), vec!["Unsaved at close"]);
    assert_eq!(state(&again)["recovered"], true);
    assert_eq!(notice(&again), format!("Recovered unsaved changes to {}. Save keeps them.", path.display()));
    assert_eq!(super::read_deck(&path).slides[0].title, "One", "the file itself is as it was");
}

#[test]
fn a_person_quitting_is_the_windows_close() {
    let app = started();
    type_title(&app, "x");
    app.event(Event::ExitRequested(ExitReason::Person));
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    app.shutdown();
    assert_eq!(outline(&killed_and_started_again(app)), vec!["x"]);
}

#[test]
fn a_logout_asks_nothing_and_keeps_the_unsaved_deck() {
    for signal in [true, false] {
        let (app, path) = with_file("logout", &["One"]);
        type_title(&app, "unsaved at logout");
        if signal {
            app.event(Event::ExitRequested(ExitReason::Logout));
        } else {
            app.display_gone();
        }
        assert!(app.exit_reason().is_some(), "ended in the command that answered");
        app.shutdown();
        let again = killed_and_started_again(app);
        assert_eq!(outline(&again), vec!["unsaved at logout"], "signal={signal}");
        assert_eq!(super::read_deck(&path).slides[0].title, "One");
    }
}

#[test]
fn the_window_closed_while_a_file_is_read_waits_for_it_then_goes_on() {
    let (app, _) = with_file("close-reading", &["One"]);
    let other = scratch("close-reading-2").join("other.ydeck");
    write_deck(&other, "Other", &["other"]);
    let pending = app.act("open", json!({ "path": other.display().to_string() })).unwrap();
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(notice(&app), crate::update::closing::CLOSING_WHEN_DONE);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY], "the window stays while the file is read");
    settle(&app);
    pending.settle().unwrap();
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn the_window_closed_while_a_copy_is_written_waits_and_stays_when_the_copy_fails() {
    let (app, path) = with_file("close-export", &["One"]);
    let disk = super::failing(&app);
    disk.fail(yantrik_app::documents::disk::Step::Write, "talk.md", Some(1));
    let pending = app.act("export_markdown", super::none()).unwrap();
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    pending.settle().unwrap_err();
    assert!(app.exit_reason().is_none(), "the window was not closed");
    assert!(notice(&app).ends_with(crate::update::closing::NOT_CLOSED), "{}", notice(&app));
    assert!(!path.with_file_name("talk.md").exists());
}

#[test]
fn nothing_the_close_does_leaves_a_dialog_or_a_question_up() {
    let app = started();
    type_title(&app, "a");
    app.close_requested(WindowId::PRIMARY);
    app.run_writes();
    assert!(state(&app).get("prompt_open").is_none(), "there is no prompt to be open");
    assert!(file(&draft_file(&app)).contains("\"a\""));
    act(&app, "show", super::none()).ok();
}

#[test]
fn a_logout_hands_the_draft_over_in_the_command_that_answers_not_after_its_debounce() {
    use std::sync::Arc;
    use std::time::Duration;
    use yantrik_app::effects::{Effects, Request};
    use yantrik_app::mind::fake::FakeMind;

    use crate::command::Command;
    use crate::model::Model;

    let mut m = Model::new(scratch("logout-effects"), Arc::new(FakeMind::new()), false);
    m.deck_mut().set_slide(0, Some("typed just now"), None, None).unwrap();
    let mut fx = Effects::<YPresent>::recording();
    crate::update::update(&mut m, Command::LoggingOut, &mut fx).unwrap();
    let names: Vec<&str> = fx.requests().iter().map(Request::name).collect();
    assert!(names.contains(&"exit"), "{names:?}");
    let at_once = fx.requests().iter().any(|r| matches!(r, Request::SaveDraft { delay, .. } if *delay == Duration::ZERO));
    assert!(at_once, "the process may be killed within the signal deadline, before a debounce: {names:?}");
}

#[test]
fn a_failed_open_with_a_close_pending_leaves_the_window_open_and_says_why() {
    let (app, _) = with_file("close-open-fails", &["One"]);
    let missing = scratch("close-open-fails-2").join("nothing.ydeck");
    let pending = app.act("open", json!({ "path": missing.display().to_string() })).unwrap();
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    pending.settle().unwrap_err();
    assert!(app.exit_reason().is_none(), "the window was not closed");
    assert!(notice(&app).ends_with(crate::update::closing::NOT_CLOSED), "{}", notice(&app));
}
