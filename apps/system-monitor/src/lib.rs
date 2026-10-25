//! Yantrik System Monitor, on YantrikApp (design/app-framework-2026-10-21.md, step 10: behaviour
//! unchanged).
//!
//! A reading is taken every two seconds in a job: the `system-monitor` service's snapshot and
//! process list, or the local `sysinfo` crate when the service does not answer, and then the
//! window and `describe` both say so, because a reading taken by the fallback is not the same
//! reading. A reading is drawn where it lands, in one frame, and only when it moves something
//! drawn; with the window off screen it is taken every thirty seconds instead, for `describe`. Ending a process is a job too, and its answer is read back from the process table.
//! The agent rail is the framework's; the three actions are intents, graded as they were; there
//! are no settings; and `main.rs` is three lines.

use std::time::Duration;

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::rail::RailCommand;
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

mod describe;
mod figures;
mod intents;
mod machine;
mod model;
mod outcome;
mod rail;
mod readout;
mod report;
mod sample;
mod update;
mod view;
#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod busy_tests;
#[cfg(test)]
mod rest_tests;
// The kill tests end children of their own and read `/proc`, as the old crate's did.
#[cfg(all(test, target_os = "linux"))]
mod outcome_tests;

pub use model::{Command, Model};

slint::include_modules!();
yantrik_app::window!(SystemMonitorApp);
yantrik_app::rail_window!(SystemMonitorApp);

/// A reading is taken this often while the window is on screen.
pub const POLL: TimerId = TimerId("system-monitor.poll");
pub const POLL_EVERY: Duration = Duration::from_secs(2);
/// And this often while it is not (minimised, covered, or put away by Show desktop): nothing is
/// drawn, and `describe` is never older than this (`yantrik_app::shown`).
pub const OFF_SCREEN_EVERY: Duration = Duration::from_secs(30);

pub struct SystemMonitor;

impl App for SystemMonitor {
    type Ui = SystemMonitorApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "system-monitor";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        Ok(Model::new(machine::for_paths(&boot.paths), boot.mind.clone()))
    }

    fn wire(cx: &Cx<Self>) {
        let ui = cx.ui();
        // The header has already set the in-out `sort-column`; the command keeps it.
        ui.on_sort_by_column(cx.cmd1(Command::SortBy));
        ui.on_process_search_changed(cx.cmd1(|query: slint::SharedString| Command::Search(query.to_string())));
        // End and Force Kill: the same command the `kill_process` action raises. The answer is
        // the notice strip, where a button reports itself.
        ui.on_kill_process(cx.cmd1(|pid: i32| Command::End { pid, force: false }));
        ui.on_force_kill_process(cx.cmd1(|pid: i32| Command::End { pid, force: true }));
        // AI Explain, in the header: the rail's question, drawn into the header's own panel.
        ui.on_ai_explain_pressed(cx.cmd(|| Command::Explain));
        ui.on_ai_dismiss(cx.cmd(|| Command::DismissAi));
        // No `back-pressed`: this is a window of its own with one screen, so there is nowhere to
        // go back to, and the wrapper sets `show-back: false` so the arrow is not drawn. No
        // `agent-context-activated`: the rail's context rows are plain rows, not buttons
        // (`context-opens` stays false). The proposal card draws Apply only for a proposal with
        // an impact, and every proposal here is an answer to read.
        drop(ui);
        yantrik_app::rail::wire(cx);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.every(POLL, POLL_EVERY, Command::Poll);
        fx.then(Command::Poll);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &SystemMonitorApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            Event::Activated(_) | Event::Presented(_) => Some(Command::Back),
            _ => None,
        }
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

    fn source() -> &'static str {
        include_str!("lib.rs").split("#[cfg(test)]").next().unwrap_or_default()
    }

    #[test]
    fn the_back_arrow_is_not_drawn_with_nowhere_to_go() {
        // There is no back handler because there is no screen to go back to. If the wrapper
        // ever draws the arrow, it must come with one.
        assert!(include_str!("../ui/app.slint").contains("show-back: false;"));
        assert!(!source().contains("on_back_pressed"));
    }

    #[test]
    fn the_rail_rows_are_not_offered_as_buttons() {
        assert!(!source().contains("on_agent_context_activated"));
        assert!(!include_str!("../ui/system_monitor.slint").contains("context-opens: true"));
    }

    #[test]
    fn no_proposal_here_draws_an_apply_button() {
        // The card draws Apply only for a proposal with an impact, and none here has one.
        let rail = include_str!("rail.rs").split("#[cfg(test)]").next().unwrap_or_default();
        assert!(!rail.contains("impact:"), "a proposal with an impact needs something to apply");
        assert!(!rail.contains("applies:"));
    }
}
