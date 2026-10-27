//! One instance, a window per display (#963), headless: a Mind's launch opens a window of the
//! Mind's beside the person's, only that window is masked, a launch never lands in the other's
//! window, and the intents answer as they do with the person's window alone.

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::control::Action;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::for_mind;
use yantrik_app::intents::{Intent, Intents};
use yantrik_app::refused::Refused;
use yantrik_testing::TestApp;

slint::slint! {
    export component SightWin inherits Window {
        in-out property <string> body;
        in-out property <int> shown-window: -1;
        callback edit();
    }
}

impl yantrik_app::Window for SightWin {
    fn make() -> Result<Self, slint::PlatformError> {
        SightWin::new()
    }
    fn apply_theme(&self, _: &yantrik_app::theme::ThemeSettings) {}
    fn connect_chrome(&self) {}
}

const SECRET: &str = "the person's private note";

#[derive(Default)]
struct Model {
    for_mind: bool,
    body: String,
    edits: Vec<WindowId>,
    presented: Vec<WindowId>,
    opened: Vec<WindowId>,
    closed: Vec<WindowId>,
}

#[derive(Debug, Clone)]
enum Command {
    /// An edit from a window: refused in a Mind's.
    Edit(WindowId),
    /// The intent: what the body is.
    Read,
    Presented(WindowId),
    Opened(WindowId),
    Closed(WindowId),
}

struct Sight;

impl App for Sight {
    type Ui = SightWin;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "sight-test";
    const MASKS_FOR_MIND: bool = true;

    fn model(boot: &Boot) -> Result<Model, Startup> {
        Ok(Model { for_mind: boot.for_mind, body: SECRET.into(), ..Model::default() })
    }

    fn wire(cx: &Cx<Self>) {
        cx.ui().on_edit(cx.cmd(|| Command::Edit(WindowId::PRIMARY)));
    }

    fn wire_window(cx: &Cx<Self>, ui: &SightWin, window: WindowId) {
        ui.on_edit(cx.cmd(move || Command::Edit(window)));
    }

    fn update(m: &mut Model, command: Command, _: &mut Effects<Self>) -> Result<(), Refused> {
        match command {
            Command::Edit(window) => {
                for_mind::refuse_if(for_mind::masked(m.for_mind, window), true)?;
                m.edits.push(window);
            }
            Command::Read => {}
            Command::Presented(window) => m.presented.push(window),
            Command::Opened(window) => m.opened.push(window),
            Command::Closed(window) => m.closed.push(window),
        }
        Ok(())
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            Event::Presented(window) => Some(Command::Presented(window)),
            Event::Opened(window) => Some(Command::Opened(window)),
            Event::Closed(window) => Some(Command::Closed(window)),
            _ => None,
        }
    }

    fn render(ui: &SightWin, window: WindowId, m: &Model, _: &Changed) {
        let masked = for_mind::masked(m.for_mind, window);
        ui.set_body(for_mind::shown_if_any(masked, &m.body).as_ref().into());
        ui.set_shown_window(window.0 as i32);
    }

    fn intents(table: &mut Intents<Self>) {
        table.add(
            Intent::new(Action::new("read", "What the note says").risk("safe"), |_| Ok(Command::Read))
                .reply(|m: &Model| serde_json::json!({ "body": m.body, "edits": m.edits.len() })),
        );
    }
}

fn body(app: &TestApp<Sight>, window: WindowId) -> String {
    app.window(window, |ui| ui.get_body().to_string()).expect("the window is open")
}

#[test]
fn only_the_minds_window_is_masked_and_the_persons_is_not() {
    let app = TestApp::<Sight>::start();
    let mind = app.open_for_mind("wayland-1");
    assert!(mind.for_mind() && !WindowId::PRIMARY.for_mind());
    assert_eq!(app.person_windows(), vec![WindowId::PRIMARY]);
    assert_eq!(app.mind_windows(), vec![mind]);
    assert_eq!(body(&app, mind), for_mind::HIDDEN);
    assert_eq!(body(&app, WindowId::PRIMARY), SECRET, "the person's window is never masked");
    assert_eq!(app.model(|m| m.opened.clone()), vec![mind], "the app heard of the Mind's window");
}

#[test]
fn a_minds_window_runs_nothing_as_the_person_and_the_persons_window_does() {
    let app = TestApp::<Sight>::start();
    let mind = app.open_for_mind("wayland-1");
    app.window(mind, |ui| ui.invoke_edit());
    assert_eq!(app.last_refusal().map(|r| r.sentence), Some(for_mind::READ_ONLY.to_string()));
    app.ui(|ui| ui.invoke_edit());
    assert!(app.last_refusal().is_none());
    assert_eq!(app.model(|m| m.edits.clone()), vec![WindowId::PRIMARY]);
}

/// The person's launch while a Mind's window is open: their own window comes forward, and nothing
/// is handed into the Mind's.
#[test]
fn the_persons_launch_beside_a_minds_window_goes_to_the_persons_window() {
    let app = TestApp::<Sight>::start();
    let mind = app.open_for_mind("wayland-1");
    let answer = app.launch_from_person().unwrap();
    assert_eq!(answer["window"], true);
    assert_eq!(app.model(|m| m.presented.clone()), vec![WindowId::PRIMARY]);
    assert_eq!(app.mind_windows(), vec![mind], "no window of the Mind's opened or shown for it");
    assert_eq!(body(&app, WindowId::PRIMARY), SECRET);
}

/// The reverse: a Mind's launch while the person's window is open opens the Mind's own window,
/// and the person's is neither shown nor handed the launch. A second launch from the same
/// display raises that window; another Mind's display gets its own.
#[test]
fn a_minds_launch_beside_the_persons_window_opens_the_minds_own() {
    let app = TestApp::<Sight>::start();
    let mind = app.open_for_mind("wayland-1");
    assert!(app.model(|m| m.presented.is_empty()), "the person's window was not brought forward");
    assert_eq!(app.open_for_mind("wayland-1"), mind, "the same display, the same window");
    let other = app.open_for_mind("wayland-2");
    assert_ne!(other, mind);
    assert_eq!(app.mind_windows(), vec![mind, other]);
    assert_eq!(app.person_windows(), vec![WindowId::PRIMARY]);
    // A launch that lost its marker on the way is still a Mind's, by its display alone.
    let answer = app
        .act("launch", serde_json::json!({ "launch": yantrik_app::Launch::empty("default"), "display": "wayland-1" }))
        .unwrap()
        .settle()
        .unwrap();
    assert_eq!(answer["mind_window"], mind.0);
}

/// With the person's window closed and a Mind's still open, the app runs on; the person's next
/// launch opens their window again (the app hears `Opened` first), not the Mind's.
#[test]
fn a_minds_window_keeps_the_app_running_and_the_person_gets_their_window_back() {
    let app = TestApp::<Sight>::start();
    let mind = app.open_for_mind("wayland-1");
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.windows(), vec![mind]);
    assert!(app.exit_reason().is_none(), "the Mind's window keeps the app running");
    app.launch_from_person().unwrap();
    assert_eq!(app.person_windows(), vec![WindowId::PRIMARY]);
    assert_eq!(app.model(|m| m.opened.clone()), vec![mind, WindowId::PRIMARY]);
    assert_eq!(app.model(|m| m.presented.clone()), vec![WindowId::PRIMARY]);
    assert_eq!(body(&app, WindowId::PRIMARY), SECRET);
}

#[test]
fn a_minds_window_closes_without_asking_the_app_and_the_last_one_ends_it() {
    let app = TestApp::<Sight>::start();
    let mind = app.open_for_mind("wayland-1");
    app.close_mind_window(mind);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert_eq!(app.model(|m| m.closed.clone()), vec![mind]);
    let mind = app.open_for_mind("wayland-1");
    app.close_requested(WindowId::PRIMARY);
    app.close_mind_window(mind);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

/// The intents reach the one instance and answer the same with a Mind's window open beside the
/// person's as with the person's alone (#959's equality, per window).
#[test]
fn the_intents_answer_the_same_with_a_minds_window_open() {
    let alone = TestApp::<Sight>::start();
    let beside = TestApp::<Sight>::start();
    beside.open_for_mind("wayland-1");
    let read = |app: &TestApp<Sight>| app.act("read", serde_json::json!({})).unwrap().settle().unwrap();
    assert_eq!(read(&alone), read(&beside));
    assert_eq!(read(&beside)["body"], SECRET, "an intent reads the person's data, unmasked");
    assert_eq!(alone.describe(), beside.describe());
}

/// A copy that runs on a Mind's display itself (no person's display to run on) masks every
/// window it draws, the first included: #959's fail-closed rule, kept.
#[test]
fn a_copy_on_a_minds_display_masks_all_it_draws() {
    let app = TestApp::<Sight>::start_for_mind(
        yantrik_app::Paths::temporary(Sight::ID),
        Vec::new(),
        std::sync::Arc::new(yantrik_app::mind::fake::FakeMind::new()),
    );
    assert_eq!(app.ui(|ui| ui.get_body().to_string()), for_mind::HIDDEN);
}

/// An app that does not mask for a Mind (#972 H3): the same window and model as Sight, and no
/// declaration, so its render would show a Mind the person's state.
struct Plain;

impl App for Plain {
    type Ui = SightWin;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "plain-test";

    fn model(boot: &Boot) -> Result<Model, Startup> {
        Sight::model(boot)
    }

    fn wire(cx: &Cx<Self>) {
        cx.ui().on_edit(cx.cmd(|| Command::Edit(WindowId::PRIMARY)));
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        match command {
            Command::Opened(window) => m.opened.push(window),
            Command::Closed(window) => m.closed.push(window),
            Command::Presented(window) => m.presented.push(window),
            _ => {}
        }
        let _ = fx;
        Ok(())
    }

    fn event(event: Event<()>, m: &Model) -> Option<Command> {
        Sight::event(event, m)
    }

    fn render(ui: &SightWin, _: WindowId, m: &Model, _: &Changed) {
        ui.set_body(m.body.as_str().into());
    }
}

const REFUSED_BESIDE: &str = "plain-test is open on the person's screen and cannot also open in your view; use its actions and os_describe plain-test instead.";

#[test]
fn an_app_that_does_not_mask_gives_a_mind_no_window_beside_the_persons() {
    let app = TestApp::<Plain>::start();
    assert_eq!(app.launch_from_mind("wayland-1").unwrap_err(), REFUSED_BESIDE);
    assert!(app.mind_windows().is_empty());
    assert_eq!(app.person_windows(), vec![WindowId::PRIMARY]);
}

/// With no window of the person's open, a Mind may open it and see only what it opens itself; the
/// person's launch then closes the Mind's window before theirs is shown.
#[test]
fn the_persons_launch_closes_a_minds_window_of_an_app_that_does_not_mask() {
    let app = TestApp::<Plain>::start();
    app.as_if_started_windowless();
    let mind = app.open_for_mind("wayland-1");
    assert_eq!(app.windows(), vec![mind]);
    app.launch_from_person().unwrap();
    assert_eq!(app.windows(), vec![WindowId::PRIMARY], "the Mind's window closed before the person's opened");
    assert_eq!(app.model(|m| m.closed.clone()), vec![mind]);
    assert!(app.exit_reason().is_none());
}

/// M1: a Mind's launch opens nothing in the app, which would open it in the person's model.
#[test]
fn a_minds_launch_opens_no_file_or_link() {
    let app = TestApp::<Sight>::start();
    let launch = yantrik_app::Launch::of_files("default", std::path::Path::new("/"), [std::path::PathBuf::from("/home/p/plan.txt")]);
    let refused = app.launch_from_mind_with("wayland-1", &launch).unwrap_err();
    assert!(refused.contains("opens no file or link in sight-test"), "{refused}");
    assert!(app.mind_windows().is_empty(), "and no window either");
    // The same launch from the person is applied as ever.
    let person = yantrik_app::Launch::of_files("default", std::path::Path::new("/"), [std::path::PathBuf::from("/home/p/plan.txt")]);
    let args = serde_json::json!({ "launch": person, "display": yantrik_app::for_mind::PERSON_DISPLAY });
    assert!(app.act("launch", args).unwrap().settle().is_ok());
}
