//! Yantrik Containers, on YantrikApp (design/app-framework-2026-10-21.md, step 10: behaviour
//! unchanged).
//!
//! Manages Docker/Podman containers by running the runtime and reading what it says back.
//! Everything about *what the runtime said* lives in [`runtime`], which has no window and no
//! process in it; every process is started in a job, through [`machine`]; the button and the
//! Mind's action are the same command, applied by [`update`]; the agent rail is the framework's;
//! the seven actions are intents, graded as they were; and `main.rs` is three lines.

use yantrik_app::app::{App, Boot, Changed, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::rail::{Rail, RailCommand};
use yantrik_app::Refused;

mod describe;
mod intents;
mod machine;
mod model;
mod rail;
mod runtime;
mod update;
mod verdict;
mod view;
#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod rest_tests;
#[cfg(test)]
mod runtime_tests;

pub use model::{Command, Model};

slint::include_modules!();
yantrik_app::window!(ContainerManagerApp);
yantrik_app::rail_window!(ContainerManagerApp);

pub struct Containers;

impl App for Containers {
    type Ui = ContainerManagerApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    /// The id the control surface has always published (`app-containers.sock`); the binary and
    /// the launcher call it `container-manager`, and the shell maps that name to this one.
    const ID: &'static str = "containers";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        Ok(Model {
            // The screen's own default until the first listing says which runtime it asked.
            runtime: "docker".into(),
            // Nothing has been asked yet, so nothing is known; the first refresh decides.
            health: runtime::Availability::Unreachable("not asked yet".into()),
            containers: Vec::new(),
            images: Vec::new(),
            volumes: Vec::new(),
            listed_turn: 0,
            notice: String::new(),
            tab: 0,
            logs: Default::default(),
            ai: Default::default(),
            in_flight: Vec::new(),
            answer: serde_json::Value::Null,
            rail: Rail::new(boot.mind.clone()),
            machine: machine::Machine::for_app(&boot.paths),
            mind: boot.mind.clone(),
        })
    }

    fn wire(cx: &Cx<Self>) {
        view::wire(cx);
    }

    /// The first read of the machine. There is no timer of the app's own: the list is re-read
    /// after every command and on Refresh, as it always was.
    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.then(Command::Refresh);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &ContainerManagerApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn rail(command: RailCommand) -> Option<Command> {
        Some(Command::Rail(command))
    }
}

#[cfg(test)]
mod dead_control_tests {
    //! Nothing drawn does nothing (the repo's rule for every control).

    /// The source above the tests, so a test mentioning a name cannot satisfy itself.
    fn source(text: &str) -> &str {
        text.split("#[cfg(test)]").next().unwrap_or_default()
    }

    #[test]
    fn the_tab_bar_switches_the_list_on_its_own() {
        // `ct-tab` only tells the model, which is right only while the screen does the work.
        let screen = include_str!("../ui/container_manager.slint");
        for (i, id) in ["tab-containers", "tab-images", "tab-volumes"].iter().enumerate() {
            let line = format!("(id == \"{id}\") {{ root.active-tab = {i}; root.ct-tab({i}); }}");
            assert!(screen.contains(&line), "the tab bar no longer sets active-tab for {id}");
            assert!(screen.contains(&format!("if active-tab == {i} : VerticalLayout")), "nothing draws tab {i}");
        }
    }

    #[test]
    fn the_rail_rows_are_not_offered_as_buttons() {
        assert!(!source(include_str!("view.rs")).contains("on_agent_context_activated"));
        assert!(!include_str!("../ui/container_manager.slint").contains("context-opens: true"));
    }

    #[test]
    fn no_proposal_here_draws_an_apply_button() {
        // The card draws Apply only for a proposal with an impact; every proposal here is an
        // answer to read. (`app_tests` checks the card the rail actually makes.)
        let rail = source(include_str!("rail.rs"));
        assert!(!rail.contains("impact:") && !rail.contains("applies:"));
    }

    #[test]
    fn the_screen_tells_the_model_when_its_log_pane_closes() {
        let app = include_str!("../ui/app.slint");
        assert!(app.contains("changed show-logs =>") && app.contains("root.logs-closed()"));
        assert!(source(include_str!("view.rs")).contains("on_logs_closed"));
    }
}
