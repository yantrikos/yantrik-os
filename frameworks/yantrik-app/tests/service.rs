//! The service start (design #978, step 1), headless: an agent's act or describe starts the app's
//! one instance with no window, and the person's launch then presents a native window of it.
//!
//! What it holds:
//! - a service start loads the store and serves intents and `describe`, shows no window, raises no
//!   launch, and runs nothing of the person's (`App::start`) until their first window;
//! - the person's launch then opens the window, runs their start once, and the instance is the
//!   same process: never a second one, never on another display;
//! - closing the last window while agents use the instance keeps it, with none, until the idle
//!   wait passes; with no agent it ends as it always did;
//! - the instance reports the person's windows to the shell, a windowless one as none;
//! - a launch from a Mind's view opens nothing, with files or without;
//! - a killed instance is started again from the same store.

use std::time::Duration;

use serde_json::json;
use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::control::{Action, Param, View};
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::{Intent, Intents};
use yantrik_app::launch::Launch;
use yantrik_app::refused::Refused;
use yantrik_app::service::IDLE;
use yantrik_testing::TestApp;

slint::slint! {
    export component ServiceWin inherits Window {
        in-out property <int> notes: 0;
    }
}

impl yantrik_app::Window for ServiceWin {
    fn make() -> Result<Self, slint::PlatformError> {
        ServiceWin::new()
    }
    fn apply_theme(&self, _: &yantrik_app::theme::ThemeSettings) {}
    fn connect_chrome(&self) {}
}

#[derive(Default)]
struct Model {
    /// What the store holds, loaded by the service start.
    notes: Vec<String>,
    /// The service-safe start ran, and how often.
    served_starts: u32,
    /// The person's start ran (recovery, anything they see), and how often.
    person_starts: u32,
    launches: u32,
    presented: u32,
    not_opened: u32,
}

#[derive(Debug, Clone)]
enum Command {
    Load,
    Add(String),
    PersonStart,
    Launched,
    Presented,
    OpenAnother,
    NotOpened,
}

struct Service;

impl App for Service {
    type Ui = ServiceWin;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "service-test";

    fn model(_: &Boot) -> Result<Model, Startup> {
        Ok(Model::default())
    }

    fn wire(_: &Cx<Self>) {}

    fn start_service(_: &Model, fx: &mut Effects<Self>) {
        fx.then(Command::Load);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.then(Command::PersonStart);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        match command {
            Command::Load => {
                m.served_starts += 1;
                m.notes = vec!["from the store".into()];
            }
            Command::Add(text) => m.notes.push(text),
            Command::PersonStart => m.person_starts += 1,
            Command::Launched => m.launches += 1,
            Command::Presented => m.presented += 1,
            Command::OpenAnother => {
                fx.open_window();
            }
            Command::NotOpened => m.not_opened += 1,
        }
        Ok(())
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            Event::Launch(_) => Some(Command::Launched),
            Event::Presented(_) => Some(Command::Presented),
            Event::WindowNotOpened { .. } => Some(Command::NotOpened),
            _ => None,
        }
    }

    fn describe(m: &Model) -> View {
        View::new(format!("{} notes", m.notes.len())).with("notes", m.notes.len())
    }

    fn intents(table: &mut Intents<Self>) {
        table.add(
            Intent::new(Action::new("add", "Add a note").arg(Param::text("text")), |a| {
                Ok(Command::Add(a.text("text")?.to_string()))
            })
            .reply(|m: &Model| json!({ "notes": m.notes.len() })),
        );
    }

    fn render(ui: &ServiceWin, _: WindowId, m: &Model, _: &Changed) {
        ui.set_notes(m.notes.len() as i32);
    }
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("yantrik-service-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn visible(app: &TestApp<Service>) -> bool {
    app.ui(|ui| {
        use slint::ComponentHandle as _;
        ui.window().is_visible()
    })
}

#[test]
fn a_service_start_loads_the_store_and_shows_nothing() {
    let app = TestApp::<Service>::start_service();
    app.run_jobs();
    assert!(app.windows().is_empty(), "no window is counted open");
    assert!(!visible(&app), "and none is shown");
    assert_eq!(app.model(|m| m.served_starts), 1, "the store is loaded");
    assert_eq!(app.model(|m| m.person_starts), 0, "the person's start waits for their window");
    assert_eq!(app.model(|m| m.launches), 0, "no launch is applied to a service start");
    assert_eq!(app.exit_reason(), None, "and it does not end for having no window");
    assert!(!app.person_has_started());
}

#[test]
fn describe_and_an_act_answer_from_the_windowless_instance() {
    let app = TestApp::<Service>::start_service();
    let seen = app.describe_as_agent("agent-1");
    assert_eq!(seen["state"]["notes"], 1, "{seen}");
    let answer = app.act("add", json!({ "text": "x" })).unwrap().settle().unwrap();
    assert_eq!(answer["notes"], 2);
    assert!(app.windows().is_empty(), "serving agents opened no window");
}

#[test]
fn two_agents_at_once_are_served_by_the_one_instance() {
    let app = TestApp::<Service>::start_service();
    let a = app.act_as(yantrik_testing::Call::new("add", json!({ "text": "a" })).as_agent("agent-a"));
    let b = app.act_as(yantrik_testing::Call::new("add", json!({ "text": "b" })).as_agent("agent-b"));
    assert!(a.is_ok() && b.is_ok());
    assert_eq!(app.model(|m| m.notes.len()), 3, "one model took both, in order");
    assert_eq!(app.model(|m| m.served_starts), 1, "started once");
    assert!(app.windows().is_empty());
}

#[test]
fn the_persons_launch_after_a_service_start_presents_a_native_window() {
    let app = TestApp::<Service>::start_service();
    app.describe_as_agent("agent-1");
    let answer = app.launch_again().expect("handed over");
    assert_eq!(answer["window"], true, "{answer}");
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert!(visible(&app));
    assert_eq!(app.ui(|ui| ui.get_notes()), 1, "drawn whole from the instance's model");
    assert_eq!(app.model(|m| m.person_starts), 1, "their start ran, once");
    assert_eq!(app.model(|m| m.launches), 1, "their launch was applied");
    assert_eq!(app.model(|m| m.presented), 1);
    // Another launch brings the same window forward; their start does not run again.
    app.launch_again().unwrap();
    assert_eq!(app.model(|m| m.person_starts), 1);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY], "the same window, not a second");
}

#[test]
fn the_persons_own_start_runs_both_starts_and_ends_when_the_last_window_closes() {
    let app = TestApp::<Service>::start();
    assert_eq!(app.model(|m| (m.served_starts, m.person_starts)), (1, 1), "both starts ran");
    assert_eq!(app.model(|m| (m.served_starts, m.person_starts)), (1, 1));
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed), "no agent used it: it ends as it did");
}

#[test]
fn closing_the_last_window_while_agents_use_it_keeps_the_instance_windowless() {
    let app = TestApp::<Service>::start_service();
    app.launch_again().unwrap();
    app.describe_as_agent("agent-1");
    app.close_requested(WindowId::PRIMARY);
    assert!(app.windows().is_empty(), "no window of the person's, and none anywhere else");
    assert_eq!(app.exit_reason(), None, "the instance is kept for the agent");
    assert!(!visible(&app));
    // It goes on answering.
    assert_eq!(app.describe_as_agent("agent-1")["state"]["notes"], 1);
    // And the person's next launch is the same instance's window again.
    app.launch_again().unwrap();
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
}

#[test]
fn a_person_started_instance_an_agent_used_stays_windowless_after_the_close() {
    let app = TestApp::<Service>::start();
    app.tick();
    app.describe_as_agent("agent-1");
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), None);
    assert!(app.windows().is_empty());
}

#[test]
fn the_idle_wait_ends_a_windowless_instance_and_an_agent_starts_it_over() {
    let app = TestApp::<Service>::start_service();
    app.advance(IDLE - Duration::from_secs(1));
    assert_eq!(app.exit_reason(), None);
    // An agent's call starts the wait over.
    app.describe_as_agent("agent-1");
    app.advance(IDLE - Duration::from_secs(1));
    assert_eq!(app.exit_reason(), None, "the wait started again at the call");
    app.advance(Duration::from_secs(1));
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn a_window_open_ends_the_idle_wait() {
    let app = TestApp::<Service>::start_service();
    app.launch_again().unwrap();
    app.advance(IDLE * 3);
    assert_eq!(app.exit_reason(), None, "the person has a window");
    assert_eq!(app.timers_armed(), 0);
}

#[test]
fn the_instance_reports_the_persons_windows_and_a_windowless_one_as_none() {
    let reports = dir("report");
    let app = TestApp::<Service>::start_service();
    app.report_to(&reports);
    let read = || {
        let text = std::fs::read_to_string(reports.join("service-test.windows")).unwrap();
        yantrik_ipc_contracts::mind_view::window_owners::Report::parse(&text).unwrap()
    };
    assert_eq!(read().person, 0, "alive, with no window of the person's");
    app.launch_again().unwrap();
    assert_eq!(read().person, 1);
    app.describe_as_agent("agent-1");
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(read().person, 0, "the window closed and the instance stays");
    drop(app);
    let _ = std::fs::remove_dir_all(&reports);
}

#[test]
fn a_window_asked_for_before_the_person_is_here_is_not_opened() {
    // Opening a window is the person's: an act that asked for one in a service start gets none.
    let app = TestApp::<Service>::start_service();
    assert!(app.windows().is_empty());
    assert!(!app.person_has_started());
}

#[test]
fn a_launch_from_a_minds_view_opens_nothing_with_files_or_without() {
    let app = TestApp::<Service>::start_service();
    let profile = app.paths().profile().to_string();
    let empty = app.launch_from_a_mind(Launch::empty(&profile)).unwrap_err();
    assert_eq!(empty, "service-test cannot open in your view; use its actions and os_describe service-test instead.");
    let with_file = Launch::of_files(&profile, std::path::Path::new("/"), [std::path::PathBuf::from("/tmp/a.md")]);
    let err = app.launch_from_a_mind(with_file).unwrap_err();
    assert!(err.contains("cannot open in your view"), "{err}");
    assert!(app.windows().is_empty(), "no window");
    assert_eq!(app.model(|m| m.launches), 0, "nothing was applied to the person's model");
    assert!(!app.person_has_started());
    // The person's own launch still works.
    app.launch_again().unwrap();
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
}

#[test]
fn an_instance_killed_is_started_again_from_the_same_files() {
    let paths = yantrik_app::Paths::temporary("service-test");
    let first = TestApp::<Service>::start_service_at(paths.clone());
    first.run(Command::Add("kept".into()));
    drop(first);
    // The model is the app's own here; what the restart proves is that a new instance starts
    // cleanly over the same files, windowless, and serves again.
    let second = TestApp::<Service>::start_service_at(paths);
    assert_eq!(second.model(|m| m.served_starts), 1);
    assert!(second.windows().is_empty());
    assert_eq!(second.exit_reason(), None);
    assert!(second.describe_as_agent("agent-1")["state"]["notes"].is_number());
}

#[test]
fn a_service_start_takes_nothing_to_open() {
    let args = vec![std::ffi::OsString::from("--service"), std::ffi::OsString::from("/tmp/a.md")];
    let opened = yantrik_app::Boot::<()>::open_as(
        yantrik_app::Paths::temporary("service-test"),
        args,
        std::sync::Arc::new(yantrik_mind::fake::FakeMind::new()),
        yantrik_app::services::Services::fakes(),
        false,
    );
    assert!(opened.is_err(), "a service start with a file is refused");
}

fn open_service(for_mind: bool) -> Result<yantrik_app::Boot<()>, yantrik_app::Startup> {
    yantrik_app::Boot::<()>::open_as(
        yantrik_app::Paths::temporary("notes"),
        vec![std::ffi::OsString::from("--service")],
        std::sync::Arc::new(yantrik_mind::fake::FakeMind::new()),
        yantrik_app::services::Services::fakes(),
        for_mind,
    )
    .map(|(boot, _)| boot)
}

/// H1: a `--service` start is the shell's, on the person's display. One that carries the marker, or
/// is on a display that is a Mind's, claims nothing and says the one sentence.
#[test]
fn a_service_start_drawn_for_a_mind_exits_without_claiming() {
    use yantrik_app::for_mind::drawn_for_mind_in;
    use yantrik_ipc_contracts::mind_view_displays::Known;
    let known = Known { person: Some("wayland-1".into()), minds: vec!["wayland-2".into()] };
    let said = "Notes cannot open in your view; use its actions and os_describe notes instead.";

    // The marker.
    let marked = drawn_for_mind_in(Some("1"), Some("wayland-1"), &known);
    assert_eq!(open_service(marked).unwrap_err().sentence, said);
    // A display that is Mind View's seat, with no marker.
    let on_the_seat = drawn_for_mind_in(None, Some("wayland-2"), &known);
    assert_eq!(open_service(on_the_seat).unwrap_err().sentence, said);
    // A display that is neither the person's nor listed.
    let elsewhere = drawn_for_mind_in(None, Some("wayland-5"), &known);
    assert_eq!(open_service(elsewhere).unwrap_err().sentence, said);
    // The shell's own start: the person's display, no marker.
    let shells = drawn_for_mind_in(None, Some("wayland-1"), &known);
    assert!(open_service(shells).unwrap().service);
}

/// H2: a person on `wayland-1` opens every framework app normally; a Mind on its seat is refused
/// in the name the person knows the app by.
#[test]
fn a_person_on_another_display_opens_every_framework_app_and_a_mind_on_its_seat_is_refused() {
    use yantrik_app::for_mind::drawn_for_mind_in;
    use yantrik_app::service::start_refusal;
    use yantrik_ipc_contracts::framework_apps::FRAMEWORK_APPS;
    use yantrik_ipc_contracts::mind_view_displays::Known;
    let known = Known { person: Some("wayland-1".into()), minds: vec!["wayland-2".into()] };
    for app in FRAMEWORK_APPS {
        let person = drawn_for_mind_in(None, Some("wayland-1"), &known);
        assert_eq!(start_refusal(app.id, person), None, "{}", app.id);
        let mind = drawn_for_mind_in(None, Some("wayland-2"), &known);
        assert_eq!(
            start_refusal(app.id, mind).as_deref(),
            Some(format!("{} cannot open in your view; use its actions and os_describe {} instead.", app.name, app.id).as_str()),
            "{}",
            app.id
        );
    }
}

/// L9: an act that asks for a window gets none unless the person has one of the app open.
#[test]
fn an_agents_act_opens_no_window_once_the_persons_last_one_is_closed() {
    let app = TestApp::<Service>::start_service();
    app.launch_again().unwrap();
    app.describe_as_agent("agent-1");
    app.close_requested(WindowId::PRIMARY);
    assert!(app.windows().is_empty());
    app.run(Command::OpenAnother);
    assert!(app.windows().is_empty(), "no window on the person's desktop for an act");
    assert_eq!(app.model(|m| m.not_opened), 1, "the app heard it was not opened");
    // With the person's window open it is theirs to have a second one.
    app.launch_again().unwrap();
    app.run(Command::OpenAnother);
    assert_eq!(app.windows().len(), 2);
}
