//! More than one window (design/notes-gate-2026-10-21.md §2, "Views and windows"), headless:
//! a window opened by a command is wired with its own id, drawn whole, and drawn again on every
//! change; closing one leaves the app running; the first window is not privileged; the last
//! window closing ends the process. A window the window manager activates raises one
//! `Event::Activated`, and only an open window that was not already active raises it. A second
//! launch's `present` raises `Event::Presented` for the window it showed, focus or not.

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::cx::{Cx, Inbound};
use yantrik_app::effects::Effects;
use yantrik_app::refused::Refused;
use yantrik_testing::TestApp;

slint::slint! {
    export component PairWin inherits Window {
        in-out property <int> count: 0;
        in-out property <int> shown-window: -1;
        callback bump();
    }
}

impl yantrik_app::Window for PairWin {
    fn make() -> Result<Self, slint::PlatformError> {
        PairWin::new()
    }
    fn apply_theme(&self, _: &yantrik_app::theme::ThemeSettings) {}
    fn connect_chrome(&self) {}
}

#[derive(Default)]
struct Model {
    count: i32,
    opened: Vec<WindowId>,
    /// Which window each bump came from.
    bumps: Vec<WindowId>,
    /// Each `Event::Activated`, in the order it was raised.
    activated: Vec<WindowId>,
    /// Each `Event::Presented`, in the order it was raised.
    presented: Vec<WindowId>,
}

#[derive(Debug, Clone)]
enum Command {
    Open,
    Bump(WindowId),
    /// Closes this window and opens another in one command: a window replaced.
    Replace(WindowId),
    /// Asks the process to end, then for a window.
    ExitThenOpen,
    Activated(WindowId),
    Presented(WindowId),
}

struct Pair;

impl App for Pair {
    type Ui = PairWin;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "pair-test";

    fn model(_: &Boot) -> Result<Model, Startup> {
        Ok(Model::default())
    }

    fn wire(cx: &Cx<Self>) {
        cx.ui().on_bump(cx.cmd(|| Command::Bump(WindowId::PRIMARY)));
    }

    fn wire_window(cx: &Cx<Self>, ui: &PairWin, window: WindowId) {
        ui.on_bump(cx.cmd(move || Command::Bump(window)));
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        match command {
            Command::Open => m.opened.push(fx.open_window()),
            Command::Bump(window) => {
                m.count += 1;
                m.bumps.push(window);
            }
            Command::Replace(window) => {
                fx.close(window);
                m.opened.push(fx.open_window());
            }
            Command::ExitThenOpen => {
                fx.exit();
                m.opened.push(fx.open_window());
            }
            Command::Activated(window) => m.activated.push(window),
            Command::Presented(window) => m.presented.push(window),
        }
        Ok(())
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            Event::Activated(window) => Some(Command::Activated(window)),
            Event::Presented(window) => Some(Command::Presented(window)),
            _ => None,
        }
    }

    fn render(ui: &PairWin, window: WindowId, m: &Model, _: &Changed) {
        ui.set_count(m.count);
        ui.set_shown_window(window.0 as i32);
    }
}

fn opened(app: &TestApp<Pair>) -> WindowId {
    app.run(Command::Open);
    *app.model(|m| m.opened.clone()).last().expect("a window was opened")
}

#[test]
fn a_window_opened_later_is_drawn_whole_and_told_its_id() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    assert_ne!(second, WindowId::PRIMARY);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY, second]);
    assert_eq!(app.window(second, |ui| ui.get_shown_window()), Some(second.0 as i32));
    assert_eq!(app.ui(|ui| ui.get_shown_window()), 0, "the first window keeps its own id");
}

#[test]
fn every_change_reaches_every_open_window() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    app.run(Command::Bump(WindowId::PRIMARY));
    app.run(Command::Bump(second));
    assert_eq!(app.ui(|ui| ui.get_count()), 2);
    assert_eq!(app.window(second, |ui| ui.get_count()), Some(2));
}

#[test]
fn a_window_opened_later_is_wired_with_its_own_id() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    app.window(second, |ui| ui.invoke_bump());
    app.ui(|ui| ui.invoke_bump());
    assert_eq!(app.model(|m| m.bumps.clone()), vec![second, WindowId::PRIMARY]);
}

#[test]
fn closing_one_window_leaves_the_app_running_and_the_first_is_not_privileged() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.windows(), vec![second]);
    assert_eq!(app.exit_reason(), None, "a window is still open");
    // The window left still draws every change.
    app.run(Command::Bump(second));
    assert_eq!(app.window(second, |ui| ui.get_count()), Some(1));
    app.close_requested(second);
    assert!(app.windows().is_empty());
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn a_window_closed_is_gone_and_a_second_close_of_it_changes_nothing() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    let third = opened(&app);
    app.close_requested(second);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY, third]);
    assert_eq!(app.window(second, |ui| ui.get_count()), None);
    app.close_requested(second);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY, third]);
    assert_eq!(app.exit_reason(), None);
}

#[test]
fn a_refused_command_opens_no_window() {
    struct Refusing;
    impl App for Refusing {
        type Ui = PairWin;
        type Model = ();
        type Command = ();
        type Settings = ();
        const ID: &'static str = "pair-refusing-test";
        fn model(_: &Boot) -> Result<(), Startup> {
            Ok(())
        }
        fn wire(_: &Cx<Self>) {}
        fn update(_: &mut (), _: (), fx: &mut Effects<Self>) -> Result<(), Refused> {
            fx.open_window();
            Err(Refused::new("no", "Not now."))
        }
        fn render(_: &PairWin, _: WindowId, _: &(), _: &Changed) {}
    }
    let app = TestApp::<Refusing>::start();
    app.run(());
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
}

#[test]
fn a_second_launch_brings_forward_an_open_window_and_never_a_closed_one() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    app.close_requested(WindowId::PRIMARY);
    assert!(app.present().is_ok());
    assert_eq!(app.windows(), vec![second], "the closed first window stays closed");
    assert_eq!(app.model(|m| m.presented.clone()), vec![second], "the app is told which");
    // What is on screen, not only what the core lists.
    assert!(!app.ui(|ui| ui.window().is_visible()), "the first window is not shown again");
    assert_eq!(app.window(second, |ui| ui.window().is_visible()), Some(true));
    // So closing what is on screen ends the app, and nothing visible is left behind.
    app.close_requested(second);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn a_closed_first_window_is_not_drawn() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    app.close_requested(WindowId::PRIMARY);
    app.run(Command::Bump(second));
    assert_eq!(app.ui(|ui| ui.get_count()), 0, "closed before the change");
    assert_eq!(app.window(second, |ui| ui.get_count()), Some(1));
}

/// The core's judgement only: the headless platform runs no event loop. The real loop ends only
/// when the core asks it to (`event_loop::run_loop`), so the same holds there.
#[test]
fn closing_the_last_window_and_opening_one_in_one_command_replaces_it() {
    let app = TestApp::<Pair>::start();
    app.run(Command::Replace(WindowId::PRIMARY));
    let replacement = *app.model(|m| m.opened.clone()).last().unwrap();
    assert_eq!(app.windows(), vec![replacement]);
    assert_eq!(app.exit_reason(), None);
}

#[test]
fn no_window_is_made_once_the_process_is_ending() {
    let app = TestApp::<Pair>::start();
    app.run(Command::ExitThenOpen);
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
    assert_eq!(app.windows(), vec![WindowId::PRIMARY], "nothing flashes up while ending");
}

/// `present` used to show the window and say nothing: the app heard of it only if the window
/// manager then gave the window the keyboard, which Mind View, or a compositor that refuses focus,
/// may never do (#921).
#[test]
fn a_second_launch_tells_the_app_which_window_it_showed_with_no_focus_after_it() {
    let app = TestApp::<Pair>::start();
    assert!(app.present().is_ok());
    assert_eq!(app.model(|m| m.presented.clone()), vec![WindowId::PRIMARY]);
    assert!(activated(&app).is_empty(), "no activation is made up for it");
    // Every time, also for a window already in front: the person asked again.
    assert!(app.present().is_ok());
    assert_eq!(app.model(|m| m.presented.clone()), vec![WindowId::PRIMARY; 2]);
}

/// The launch handed over by a second copy shows the window through the same core.
#[test]
fn a_launch_handed_over_tells_the_app_which_window_it_showed() {
    let app = TestApp::<Pair>::start();
    let answer = app.launch_again().expect("the launch is handed over");
    assert_eq!(answer["window"], true, "{answer}");
    assert_eq!(app.model(|m| m.presented.clone()), vec![WindowId::PRIMARY]);
}

fn activated(app: &TestApp<Pair>) -> Vec<WindowId> {
    app.model(|m| m.activated.clone())
}

#[test]
fn a_window_gaining_activation_raises_one_activated_and_a_repeat_raises_none() {
    let app = TestApp::<Pair>::start();
    assert!(activated(&app).is_empty(), "nothing is raised before the window manager speaks");
    app.activate(WindowId::PRIMARY);
    assert_eq!(activated(&app), vec![WindowId::PRIMARY]);
    // The window manager saying it again, of the window already active, is not a new activation.
    app.activate(WindowId::PRIMARY);
    assert_eq!(activated(&app), vec![WindowId::PRIMARY]);
}

#[test]
fn a_second_windows_activation_raises_one_for_it() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    app.activate(WindowId::PRIMARY);
    // Wayland may tell the new window before the old one hears it lost the keyboard.
    app.activate(second);
    app.deactivate(WindowId::PRIMARY);
    assert_eq!(activated(&app), vec![WindowId::PRIMARY, second]);
    app.activate(WindowId::PRIMARY);
    app.deactivate(second);
    assert_eq!(activated(&app), vec![WindowId::PRIMARY, second, WindowId::PRIMARY]);
}

#[test]
fn losing_the_activation_raises_nothing_and_coming_back_raises_again() {
    let app = TestApp::<Pair>::start();
    app.activate(WindowId::PRIMARY);
    // The person switched to another app's window.
    app.deactivate(WindowId::PRIMARY);
    app.deactivate(WindowId::PRIMARY);
    assert_eq!(activated(&app), vec![WindowId::PRIMARY], "a loss is not an activation");
    app.activate(WindowId::PRIMARY);
    assert_eq!(activated(&app), vec![WindowId::PRIMARY, WindowId::PRIMARY]);
}

#[test]
fn a_closed_window_raises_none() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    app.activate(second);
    app.close_requested(second);
    // A late word from the window manager about the window that closed.
    app.activate(second);
    app.close_requested(WindowId::PRIMARY);
    app.activate(WindowId::PRIMARY);
    assert_eq!(activated(&app), vec![second]);
}

#[test]
fn an_activation_queued_behind_its_windows_close_is_dropped() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    // A close posted from elsewhere, not yet drained: the window is still open when the window
    // manager's word arrives, and closed by the time the drain reaches the activation.
    assert!(app.post().post(Inbound::Event(Event::CloseRequested(second))).is_ok());
    app.activate(second);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert!(activated(&app).is_empty(), "an activation of a window that closed reached the app");
}

#[test]
fn the_window_left_when_the_active_one_closes_raises_when_activated() {
    let app = TestApp::<Pair>::start();
    let second = opened(&app);
    app.activate(WindowId::PRIMARY);
    app.activate(second);
    app.close_requested(second);
    app.activate(WindowId::PRIMARY);
    assert_eq!(activated(&app), vec![WindowId::PRIMARY, second, WindowId::PRIMARY]);
}

#[test]
fn an_app_that_ignores_activation_is_unaffected() {
    struct Ignoring;
    impl App for Ignoring {
        type Ui = PairWin;
        type Model = u32;
        type Command = ();
        type Settings = ();
        const ID: &'static str = "pair-ignoring-test";
        fn model(_: &Boot) -> Result<u32, Startup> {
            Ok(0)
        }
        fn wire(_: &Cx<Self>) {}
        fn update(applied: &mut u32, _: (), _: &mut Effects<Self>) -> Result<(), Refused> {
            *applied += 1;
            Ok(())
        }
        fn render(_: &PairWin, _: WindowId, _: &u32, _: &Changed) {}
    }
    let app = TestApp::<Ignoring>::start();
    app.activate(WindowId::PRIMARY);
    app.deactivate(WindowId::PRIMARY);
    app.activate(WindowId::PRIMARY);
    assert_eq!(app.model(|applied| *applied), 0, "no command ran");
    assert!(app.last_refusal().is_none());
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert_eq!(app.exit_reason(), None);
}
