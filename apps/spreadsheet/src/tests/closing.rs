//! The session ending, and the window closed while a file is read. A logout asks nothing: the
//! unsaved sheet goes to its draft in the command that answers, and the next start offers it back.

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};

use super::{draft_file, enter, file, flush_drafts, killed_and_started_again, notice, raw, scratch, settle, with_file};
use crate::update::closing::CLOSING_WHEN_DONE;
use crate::YSheets;

#[test]
fn a_logout_asks_nothing_and_keeps_the_unsaved_sheet() {
    for signal in [true, false] {
        let (app, path) = with_file("logout", "1\r\n");
        enter(&app, "unsaved at logout");
        if signal {
            app.event(Event::ExitRequested(ExitReason::Logout));
        } else {
            app.display_gone();
        }
        assert!(app.exit_reason().is_some(), "ended in the command that answered");
        app.shutdown();
        let again = killed_and_started_again(app);
        assert_eq!(raw(&again, "A1"), "unsaved at logout", "signal={signal}");
        assert!(super::modified(&again));
        assert_eq!(file(&path), "1\r\n", "the file itself is as it was");
    }
}

#[test]
fn a_logout_with_the_draft_still_in_its_pause_hands_it_over_at_once() {
    use std::time::Duration;
    use yantrik_app::effects::{Effects, Request};

    use crate::command::Command;
    use crate::model::Model;

    let mut m = Model::new(scratch("logout-effects"), false);
    m.book_mut().put(super::cell("A1"), "typed just now").unwrap();
    let mut fx = Effects::<YSheets>::recording();
    crate::update::update(&mut m, Command::LoggingOut, &mut fx).unwrap();
    let names: Vec<&str> = fx.requests().iter().map(Request::name).collect();
    assert!(names.contains(&"exit"), "{names:?}");
    let at_once = fx.requests().iter().any(|r| matches!(r, Request::SaveDraft { delay, .. } if *delay == Duration::ZERO));
    assert!(at_once, "the process may be killed within the signal deadline, before a debounce: {names:?}");
}

#[test]
fn a_clean_sheet_is_not_given_a_draft_at_logout() {
    use yantrik_app::effects::{Effects, Request};

    use crate::command::Command;
    use crate::model::Model;

    let mut m = Model::new(scratch("logout-clean"), false);
    let mut fx = Effects::<YSheets>::recording();
    crate::update::update(&mut m, Command::LoggingOut, &mut fx).unwrap();
    assert!(fx.requests().iter().all(|r| !matches!(r, Request::SaveDraft { .. })));
    assert!(fx.requests().iter().any(|r| r.name() == "exit"));
}

#[test]
fn the_window_closed_while_a_file_is_read_waits_for_it_then_goes_on() {
    let (app, _) = with_file("close-reading", "1\r\n");
    let other = scratch("close-reading-2").join("other.csv");
    std::fs::write(&other, "other\r\n").unwrap();
    let pending = app.act("open", json!({ "path": other.display().to_string() })).unwrap();
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(notice(&app), CLOSING_WHEN_DONE);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY], "the window stays while the file is read");
    settle(&app);
    pending.settle().unwrap();
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn a_close_that_waited_on_a_read_over_unsaved_changes_asks_when_it_goes_on() {
    let (app, path) = with_file("close-reading-dirty", "1\r\n");
    let pending = app.act("open", json!({ "path": path.display().to_string() })).unwrap();
    app.close_requested(WindowId::PRIMARY);
    // Typed into while the file is read: the open is dropped, and the close asks about the typing.
    app.ui(|ui| {
        ui.set_cell_data("typed meanwhile".into());
        ui.invoke_formula_submitted("typed meanwhile".into(), "down".into());
    });
    settle(&app);
    pending.settle().expect_err("the open was dropped for the typing");
    assert!(app.exit_reason().is_none(), "unsaved changes are asked about, not dropped");
    assert_eq!(app.ui(|ui| ui.get_dialog()), crate::SheetDialog::Unsaved);
    assert_eq!(raw(&app, "A1"), "typed meanwhile");
}

#[test]
fn nothing_the_close_does_leaves_a_dialog_up_with_no_question_behind_it() {
    let app = super::started();
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(app.ui(|ui| ui.get_dialog()), crate::SheetDialog::None);
    assert!(app.model(|m| m.pending.is_none() && !m.asking()));
}

#[test]
fn the_draft_is_made_when_typing_pauses_and_not_at_every_key() {
    let app = super::started();
    enter(&app, "a");
    assert!(!draft_file(&app).exists(), "not yet: typing may go on");
    app.advance(crate::update::drafts::PAUSE / 2);
    enter(&app, "b");
    app.advance(crate::update::drafts::PAUSE / 2);
    app.run_writes();
    assert!(!draft_file(&app).exists(), "another key restarted the pause");
    flush_drafts(&app);
    assert!(draft_file(&app).exists());
    assert!(file(&draft_file(&app)).contains("\"b\""));
}

#[test]
fn a_close_that_waited_on_a_read_that_failed_goes_on_with_nothing_lost() {
    let (app, _) = with_file("close-reading-bad", "1\r\n");
    let bad = scratch("close-reading-bad-2").join("bad.csv");
    std::fs::write(&bad, "a,\"never closed\n").unwrap();
    let pending = app.act("open", json!({ "path": bad.display().to_string() })).unwrap();
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    pending.settle().unwrap_err();
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed), "nothing was unsaved, so the close goes on");
}

#[test]
fn a_person_quitting_with_nothing_unsaved_is_not_asked() {
    let (app, _) = with_file("quit-clean", "1\r\n");
    app.event(Event::ExitRequested(ExitReason::Person));
    settle(&app);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn the_window_coming_forward_asks_nothing_of_the_app() {
    let (app, _) = with_file("activate", "1\r\n");
    enter(&app, "typing");
    let before = app.model(|m| (m.sheet().used_rows(), m.active, m.notice.window().to_string()));
    app.activate(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(app.model(|m| (m.sheet().used_rows(), m.active, m.notice.window().to_string())), before);
    assert_eq!(app.pending_commands(), 0);
    assert!(app.last_refusal().is_none());
}

#[test]
fn text_in_the_formula_bar_not_yet_entered_is_in_the_draft_a_logout_writes() {
    for signal in [true, false] {
        let (app, path) = with_file("logout-bar", "1\r\n");
        // Typed, and not entered: the sheet itself is unchanged.
        super::type_in_bar(&app, "typed in the bar, never entered");
        assert!(!super::modified(&app));
        if signal {
            app.event(Event::ExitRequested(ExitReason::Logout));
        } else {
            app.display_gone();
        }
        assert!(app.exit_reason().is_some(), "ended in the command that answered");
        app.shutdown();
        let again = killed_and_started_again(app);
        assert_eq!(raw(&again, "A1"), "typed in the bar, never entered", "signal={signal}");
        assert!(super::modified(&again), "it is an edit nobody has saved");
        assert_eq!(
            notice(&again),
            format!(
                "Recovered unsaved changes to {}. The text you were typing in A1 is in that cell. Save keeps it.",
                path.display()
            ),
            "signal={signal}"
        );
        assert_eq!(file(&path), "1\r\n", "and the saved file is never touched by it");
    }
}

#[test]
fn text_cleared_from_the_formula_bar_at_logout_comes_back_cleared() {
    let (app, _) = with_file("logout-bar-cleared", "keep me\r\n");
    super::type_in_bar(&app, "");
    app.event(Event::ExitRequested(ExitReason::Logout));
    app.shutdown();
    let again = killed_and_started_again(app);
    assert_eq!(raw(&again, "A1"), "");
    assert!(notice(&again).contains("The text you were typing in A1 is in that cell"));
}

#[test]
fn a_bar_that_shows_the_cell_as_it_is_adds_nothing_to_a_logout() {
    let (app, _) = with_file("logout-bar-same", "same\r\n");
    app.event(Event::ExitRequested(ExitReason::Logout));
    app.shutdown();
    assert_eq!(notice(&killed_and_started_again(app)), "", "nothing was unsaved, so nothing is offered back");
}
