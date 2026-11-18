//! Quitting while windows are open (`yantrik_app::quit`), headless: a person's quit
//! (`fx.request_quit()`) runs the app's close flow, which may cancel; a signal's reaches the app
//! on its UI thread as a logout, once however many arrive, and the framework ends an app that
//! has not ended by the deadline; an app with nothing unsaved, or that takes the default, ends.

mod common;

use yantrik_app::app::{App, Boot, Changed, ExitReason, Startup, WindowId};
use yantrik_app::cx::{Cx, Inbound};
use yantrik_app::effects::Effects;
use yantrik_app::quit::Signal;
use yantrik_app::refused::Refused;
use yantrik_testing::TestApp;

use common::quitter::{ui_thread, Answer, Command, Quitter, QuitterWin};

/// Posts `inbound` from a thread of its own, as the signal thread does.
fn post_from_another_thread(app: &TestApp<Quitter>, inbound: Inbound<Quitter>) {
    let post = app.post();
    std::thread::spawn(move || post.post(inbound).map_err(|_| "the app is running"))
        .join()
        .expect("the posting thread")
        .unwrap();
}

fn signal(app: &TestApp<Quitter>, signal: Signal) {
    post_from_another_thread(app, Inbound::Signal(signal));
}

#[test]
fn request_quit_runs_the_close_flow_and_cancelling_keeps_the_app_running() {
    let app = TestApp::<Quitter>::start();
    app.run(Command::OpenWindow);
    app.run(Command::Edit);
    // Ctrl+Q, in the first window: the app's flow asks once over both windows.
    app.ui(|ui| ui.invoke_quit_pressed());
    assert_eq!(app.model(|m| (m.asking, m.asked)), (true, 1));
    assert_eq!(app.exit_reason(), None, "nothing ends while the person is asked");
    // Asked again while the prompt is up: the app's flow asks nothing more.
    app.run(Command::QuitPressed);
    assert_eq!(app.model(|m| m.asked), 1);
    app.run(Command::Answer(Answer::Cancel));
    assert_eq!(app.exit_reason(), None, "Cancel keeps the app running");
    assert_eq!(app.windows().len(), 2, "and every window open");
    // Asked again after a cancel: asked again, and Don't save ends it.
    app.run(Command::QuitPressed);
    assert_eq!(app.model(|m| m.asked), 2);
    app.run(Command::Answer(Answer::DontSave));
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn a_quit_with_nothing_unsaved_ends_the_app() {
    let app = TestApp::<Quitter>::start();
    app.run(Command::OpenWindow);
    app.run(Command::QuitPressed);
    assert_eq!(app.model(|m| m.asked), 0, "nothing to ask about");
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn a_signal_reaches_the_app_on_its_ui_thread_as_a_logout_once() {
    let app = TestApp::<Quitter>::start();
    app.run(Command::Edit);
    signal(&app, Signal::Terminate);
    signal(&app, Signal::Terminate);
    assert_eq!(app.model(|m| m.logouts), 0, "posted, not yet applied");
    app.tick();
    assert_eq!(app.model(|m| m.logouts), 1, "two signals, one logout");
    assert_eq!(app.model(|m| m.logout_thread), Some(ui_thread(&app)), "on the UI thread");
    assert_eq!(app.model(|m| m.asked), 0, "no one is asked at a logout");
    // Another while the logout's save is under way: not raised again.
    signal(&app, Signal::HangUp);
    app.tick();
    assert_eq!(app.model(|m| m.logouts), 1);
    assert_eq!(app.exit_reason(), None, "the app is saving");
    app.run(Command::Saved);
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn a_signal_while_a_person_is_asked_saves_without_asking() {
    let app = TestApp::<Quitter>::start();
    app.run(Command::Edit);
    app.run(Command::QuitPressed);
    assert!(app.model(|m| m.asking));
    signal(&app, Signal::HangUp);
    app.tick();
    assert_eq!(app.model(|m| (m.logouts, m.asking, m.asked)), (1, false, 1));
}

#[test]
fn an_app_that_has_not_ended_by_the_deadline_is_ended() {
    let app = TestApp::<Quitter>::start();
    app.run(Command::Edit);
    signal(&app, Signal::Terminate);
    app.tick();
    assert!(app.model(|m| m.saving), "the save that would end it never lands");
    post_from_another_thread(&app, Inbound::Overdue(Signal::Terminate));
    app.tick();
    assert_eq!(app.exit_reason(), Some(ExitReason::Logout));
}

#[test]
fn the_deadline_after_the_app_ended_changes_nothing() {
    let app = TestApp::<Quitter>::start();
    signal(&app, Signal::Terminate);
    app.tick();
    app.run(Command::Saved);
    post_from_another_thread(&app, Inbound::Overdue(Signal::Terminate));
    app.tick();
    assert_eq!(app.exit_reason(), Some(ExitReason::App), "the app's own end stands");
}

/// An app that takes every default: `Event::ExitRequested` ends it as asked.
struct Plain;

impl App for Plain {
    type Ui = QuitterWin;
    type Model = ();
    type Command = bool;
    type Settings = ();
    const ID: &'static str = "plain-quit-test";

    fn model(_: &Boot) -> Result<(), Startup> {
        Ok(())
    }

    fn wire(_: &Cx<Self>) {}

    fn update(_: &mut (), quit: bool, fx: &mut Effects<Self>) -> Result<(), Refused> {
        if quit {
            fx.request_quit();
        }
        Ok(())
    }

    fn render(_: &QuitterWin, _: WindowId, _: &(), _: &Changed) {}
}

#[test]
fn an_app_that_takes_the_default_ends_at_a_persons_quit() {
    let app = TestApp::<Plain>::start();
    app.run(false);
    assert_eq!(app.exit_reason(), None);
    app.run(true);
    assert_eq!(app.exit_reason(), Some(ExitReason::Person));
}

#[test]
fn an_app_that_takes_the_default_ends_at_a_signal() {
    let app = TestApp::<Plain>::start();
    let post = app.post();
    std::thread::spawn(move || post.post(Inbound::Signal(Signal::HangUp)).is_ok())
        .join()
        .unwrap();
    app.tick();
    assert_eq!(app.exit_reason(), Some(ExitReason::Logout));
}

#[test]
fn request_quit_is_recorded_as_an_effect() {
    let mut fx = Effects::<Plain>::recording();
    fx.request_quit();
    let names: Vec<&str> = fx.requests().iter().map(|r| r.name()).collect();
    assert_eq!(names, ["request_quit"]);
}
