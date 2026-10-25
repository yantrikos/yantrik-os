//! A linger (`fx.linger`), headless: an app with a write in flight closes its last window and
//! runs on with none until the write lands and it runs `fx.exit()`, or its backstop passes. A
//! second launch during it opens the first window again and raises `Event::Presented`, where
//! the app cancels its exit; a logout still ends it within the signal deadline. Shaped as
//! Calendar's close (#921): a close with writes in flight waits for them, and a close with none
//! ends at once.

use std::time::Duration;

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::cx::{Cx, Inbound, MAX_LINGER};
use yantrik_app::effects::Effects;
use yantrik_app::quit::Signal;
use yantrik_app::refused::Refused;
use yantrik_app::timers::TimerId;
use yantrik_testing::TestApp;

slint::slint! {
    export component LingerWin inherits Window {
        in-out property <int> writes: 0;
    }
}

impl yantrik_app::Window for LingerWin {
    fn make() -> Result<Self, slint::PlatformError> {
        LingerWin::new()
    }
    fn apply_theme(&self, _: &yantrik_app::theme::ThemeSettings) {}
    fn connect_chrome(&self) {}
}

/// The app's own backstop for its writes.
const BACKSTOP: Duration = Duration::from_secs(10);

/// The name the framework arms a linger's backstop under, as an app could spell it.
const BACKSTOP_TIMER: TimerId = TimerId("yantrik-app.linger");

#[derive(Default)]
struct Model {
    /// Writes handed to a service whose outcome is not known yet.
    writes: u32,
    /// The last window closed with writes in flight: the app ends once they land.
    closing: bool,
    /// A logout asked the app to end: it ends once the writes land, and a window coming back
    /// does not cancel that.
    ending: bool,
    launches: u32,
    presented: Vec<WindowId>,
    opened: Vec<WindowId>,
    ticks: u32,
}

#[derive(Debug, Clone)]
enum Command {
    Write,
    Landed,
    Close(WindowId),
    /// Asks to linger without closing anything: ignored.
    LingerOnly,
    Open,
    Launched,
    Presented(WindowId),
    LoggingOut,
    /// Tries to stop the backstop and arm its own timer in its place.
    Disarm,
    Tick,
}

struct Lingerer;

impl App for Lingerer {
    type Ui = LingerWin;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "lingerer-test";

    fn model(_: &Boot) -> Result<Model, Startup> {
        Ok(Model::default())
    }

    fn wire(_: &Cx<Self>) {}

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        match command {
            Command::Write if m.closing || m.ending => {
                return Err(Refused::new("closing", "The app is closing, so nothing more is written."));
            }
            Command::Write => {
                m.writes += 1;
                fx.spawn(|_| Command::Landed);
            }
            Command::Landed => {
                m.writes -= 1;
                if (m.closing || m.ending) && m.writes == 0 {
                    fx.exit();
                }
            }
            Command::Close(window) => {
                fx.close(window);
                if m.writes > 0 {
                    m.closing = true;
                    fx.linger(BACKSTOP);
                }
            }
            Command::LingerOnly => fx.linger(BACKSTOP),
            Command::Open => m.opened.push(fx.open_window()),
            Command::Launched => m.launches += 1,
            // The person is back: the close is cancelled, and the writes land as any other. A
            // logout's end is not the person's to cancel.
            Command::Presented(window) => {
                m.presented.push(window);
                m.closing = false;
            }
            Command::Disarm => {
                fx.stop_timer(BACKSTOP_TIMER);
                fx.after(BACKSTOP_TIMER, Duration::from_secs(1), Command::Tick);
                fx.every(BACKSTOP_TIMER, Duration::from_secs(1), Command::Tick);
            }
            Command::Tick => m.ticks += 1,
            Command::LoggingOut => {
                m.ending = true;
                if m.writes == 0 {
                    fx.exit();
                }
            }
        }
        Ok(())
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            Event::CloseRequested(window) => Some(Command::Close(window)),
            Event::Launch(_) => Some(Command::Launched),
            Event::Presented(window) => Some(Command::Presented(window)),
            Event::ExitRequested(ExitReason::Logout) => Some(Command::LoggingOut),
            _ => None,
        }
    }

    fn render(ui: &LingerWin, _: WindowId, m: &Model, _: &Changed) {
        ui.set_writes(m.writes as i32);
    }
}

/// An app with one write in flight whose last window has just been closed.
fn lingering() -> TestApp<Lingerer> {
    let app = TestApp::<Lingerer>::start();
    app.run(Command::Write);
    app.close_requested(WindowId::PRIMARY);
    assert!(app.lingering(), "a close with a write in flight lingers");
    app
}

fn visible(app: &TestApp<Lingerer>) -> bool {
    app.ui(|ui| {
        use slint::ComponentHandle as _;
        ui.window().is_visible()
    })
}

/// Posts what the signal thread posts, from a thread of its own.
fn post(app: &TestApp<Lingerer>, inbound: Inbound<Lingerer>) {
    let post = app.post();
    let posted = std::thread::spawn(move || post.post(inbound).is_ok()).join();
    assert!(posted.expect("the posting thread"), "the app is running");
    app.tick();
}

#[test]
fn a_linger_keeps_the_app_running_with_no_window() {
    let app = lingering();
    assert!(app.windows().is_empty(), "the window is closed, not hidden and counted open");
    assert!(!visible(&app));
    assert_eq!(app.exit_reason(), None);
    assert_eq!(app.timers_armed(), 1, "the backstop is armed");
    // Short of the backstop it runs on, and its job is still waited on.
    app.advance(BACKSTOP - Duration::from_millis(1));
    assert_eq!(app.exit_reason(), None);
    assert_eq!(app.jobs_waiting(), 1);
    assert!(app.lingering());
}

#[test]
fn the_app_ending_itself_ends_the_linger() {
    let app = lingering();
    app.run_jobs();
    assert_eq!(app.model(|m| m.writes), 0, "the write landed and its outcome is known");
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
    assert!(!app.lingering());
}

#[test]
fn the_backstop_ends_an_app_that_never_ends_itself() {
    let app = lingering();
    app.advance(BACKSTOP);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    assert!(!app.lingering());
    // A second launch now finds the app ending: no window is opened for it.
    assert!(app.present().is_err());
    assert!(app.windows().is_empty());
}

#[test]
fn a_backstop_past_the_cap_is_held_to_it() {
    struct Long;
    impl App for Long {
        type Ui = LingerWin;
        type Model = ();
        type Command = ();
        type Settings = ();
        const ID: &'static str = "lingerer-long-test";
        fn model(_: &Boot) -> Result<(), Startup> {
            Ok(())
        }
        fn wire(_: &Cx<Self>) {}
        fn update(_: &mut (), _: (), fx: &mut Effects<Self>) -> Result<(), Refused> {
            fx.close(WindowId::PRIMARY);
            fx.linger(Duration::from_secs(3600));
            Ok(())
        }
        fn render(_: &LingerWin, _: WindowId, _: &(), _: &Changed) {}
    }
    let app = TestApp::<Long>::start();
    app.run(());
    assert!(app.lingering());
    app.advance(MAX_LINGER - Duration::from_millis(1));
    assert_eq!(app.exit_reason(), None);
    app.advance(Duration::from_millis(1));
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed), "an hour asked, the cap held");
}

#[test]
fn a_second_launch_during_a_linger_opens_the_window_and_tells_the_app() {
    let app = lingering();
    let answer = app.launch_again().expect("the launch is handed over");
    assert_eq!(answer["window"], true, "the second copy is told a window was shown: {answer}");
    assert_eq!(app.model(|m| m.launches), 2, "the first launch and the one handed over");
    assert_eq!(app.model(|m| m.presented.clone()), vec![WindowId::PRIMARY]);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert!(visible(&app));
    assert_eq!(app.ui(|ui| ui.get_writes()), 1, "drawn whole from the model");
    assert!(!app.lingering());
    assert_eq!(app.timers_armed(), 0, "the backstop is stopped");
    // The close is cancelled: writes are taken again, and nothing ends the app under the person.
    app.run(Command::Write);
    assert!(app.last_refusal().is_none());
    app.advance(BACKSTOP * 2);
    app.run_jobs();
    assert_eq!(app.model(|m| m.writes), 0);
    assert_eq!(app.exit_reason(), None, "the person's window is still open");
    // And the usual rule is back: closing it with nothing in flight ends the app.
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn the_desktops_present_during_a_linger_opens_the_window_and_tells_the_app() {
    let app = lingering();
    assert!(app.present().is_ok());
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert_eq!(app.model(|m| (m.presented.clone(), m.closing)), (vec![WindowId::PRIMARY], false));
    assert!(!app.lingering());
}

#[test]
fn a_window_the_app_opens_during_a_linger_ends_it() {
    let app = lingering();
    app.run(Command::Open);
    let opened = *app.model(|m| m.opened.clone()).last().expect("a window was opened");
    assert_eq!(app.windows(), vec![opened]);
    assert!(!app.lingering());
    app.advance(BACKSTOP * 2);
    assert_eq!(app.exit_reason(), None, "the backstop went with the linger");
}

#[test]
fn a_logout_during_a_linger_still_ends_the_app() {
    // The app's logout flow: it ends once the write lands.
    let app = lingering();
    post(&app, Inbound::Signal(Signal::Terminate));
    assert!(app.model(|m| m.ending), "the logout reached the app");
    assert_eq!(app.exit_reason(), None, "its write is still in flight");
    app.run_jobs();
    assert_eq!(app.exit_reason(), Some(ExitReason::App));

    // A write that never lands: the signal deadline ends it, well before the backstop.
    let app = lingering();
    post(&app, Inbound::Signal(Signal::Terminate));
    post(&app, Inbound::Overdue(Signal::Terminate));
    assert_eq!(app.exit_reason(), Some(ExitReason::Logout));
}

#[test]
fn a_window_coming_back_during_a_logout_does_not_cancel_it() {
    let app = lingering();
    post(&app, Inbound::Signal(Signal::HangUp));
    assert!(app.present().is_ok(), "the window is still the person's to see");
    app.run(Command::Write);
    assert_eq!(app.last_refusal().map(|r| r.kind), Some("closing"), "the logout refuses writes");
    app.run_jobs();
    assert_eq!(app.exit_reason(), Some(ExitReason::App), "and ends once the write lands");
}

#[test]
fn a_linger_is_taken_only_from_the_command_that_closes_the_last_window() {
    let app = TestApp::<Lingerer>::start();
    app.run(Command::Write);
    // No window closed: ignored.
    app.run(Command::LingerOnly);
    assert!(!app.lingering());
    assert_eq!(app.timers_armed(), 0);
    // A window is still open after the close: ignored, and the app runs on as ever.
    app.run(Command::Open);
    let second = *app.model(|m| m.opened.clone()).last().unwrap();
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.windows(), vec![second]);
    assert!(!app.lingering());
    assert_eq!(app.exit_reason(), None);
    // The last window: taken.
    app.close_requested(second);
    assert!(app.lingering());
    assert_eq!(app.exit_reason(), None);
}

#[test]
fn a_close_with_nothing_in_flight_ends_at_once() {
    let app = TestApp::<Lingerer>::start();
    app.close_requested(WindowId::PRIMARY);
    assert!(!app.lingering());
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

/// #932 review, Low 1: the backstop shared the app's timer names, so an app's `stop_timer` or a
/// re-arm under its name left a process with no window running for ever.
#[test]
fn the_app_cannot_stop_or_replace_the_backstop() {
    assert!(BACKSTOP_TIMER.is_framework());
    let mut fx = Effects::<Lingerer>::recording();
    fx.stop_timer(BACKSTOP_TIMER);
    fx.after(BACKSTOP_TIMER, Duration::from_secs(1), Command::Tick);
    assert!(fx.requests().is_empty(), "the framework's timer is not the app's to ask for");

    let app = lingering();
    app.run(Command::Disarm);
    assert_eq!(app.timers_armed(), 1, "the backstop, and nothing of the app's under its name");
    app.advance(BACKSTOP);
    assert_eq!(app.model(|m| m.ticks), 0);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed), "the backstop still ends it");
}

/// #932 review, Low 2: the process is ending for the logout, and says so, when the backstop
/// cuts the logout's wait short.
#[test]
fn a_backstop_during_a_logout_ends_the_app_for_the_logout() {
    let app = lingering();
    post(&app, Inbound::Signal(Signal::Terminate));
    assert_eq!(app.exit_reason(), None, "the logout waits for its write");
    app.advance(BACKSTOP);
    assert_eq!(app.exit_reason(), Some(ExitReason::Logout));
}

/// #932 review, Info 3: a launch handed to an app the backstop has ended applies nothing.
#[test]
fn a_launch_after_the_backstop_is_refused_and_applies_nothing() {
    let app = lingering();
    app.advance(BACKSTOP);
    let refused = app.launch_again().expect_err("the app is ending");
    assert!(refused.contains("ending"), "{refused}");
    assert_eq!(app.model(|m| (m.launches, m.presented.len())), (1, 0), "only the first launch");
    assert!(app.windows().is_empty());
}
