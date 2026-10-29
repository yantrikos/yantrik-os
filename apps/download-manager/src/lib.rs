//! Yantrik Download Manager, on YantrikApp (design/app-framework-2026-10-21.md, §11: behaviour
//! unchanged).
//!
//! Fetches files over HTTP, shows what each transfer is doing, verifies it against a checksum, and
//! publishes all of that on the control surface so an agent can fetch a file without a terminal
//! and read back where it landed and what its hash was.
//!
//! # How it is put together
//!
//! - `engine/`: the downloads. One handle onto shared state, which its worker threads write as
//!   they go (one thread a transfer, writing straight to the file). The list is kept in
//!   `~/.local/share/yantrik/downloads/state.json` and reconciled against the disk on every start.
//! - the desk: the line of commands to the engine, in the order they were asked for (`yantrik_app::lane`).
//! - `model.rs` and `command.rs`: what the app is, and every change to it. `update/`: the only code
//!   that changes the model. `view.rs`: the window. `describe.rs`: what the Mind reads.
//! - `intents.rs`: the ten actions, as they were published. `answers.rs`: what each is told.
//!   `agent_rule.rs`: where an agent may have a file saved. `rail.rs`: the agent rail.
//!
//! Nothing that reads or writes the disk, or starts a transfer, runs on the UI thread. Closing the
//! window pauses what is downloading, as it always did, and the app runs on without its window
//! only until each writer has stopped at a whole chunk and the list is written (`update/life.rs`).

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::rail::{Rail, RailCommand};
use yantrik_app::Refused;

mod agent_rule;
mod answers;
mod command;
mod describe;
pub mod engine;
mod intents;
mod model;
mod rail;
mod update;
mod view;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(DownloadManagerApp);
yantrik_app::rail_window!(DownloadManagerApp);

pub struct Downloads;

/// The engine this app runs: this machine's list, or under a test the one that test installed.
#[cfg(not(test))]
fn engine_for(_: &yantrik_app::Paths) -> engine::Engine {
    engine::Engine::new()
}

#[cfg(test)]
use tests::engine_for;

impl App for Downloads {
    type Ui = DownloadManagerApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    /// The control surface's id, as it always was: `yos act download-manager …`.
    const ID: &'static str = "download-manager";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        // Read once, before any window exists, so the first `describe` and the first paint both
        // see what actually survived the last run.
        Ok(Model::new(engine_for(&boot.paths), Rail::new(boot.mind.clone())))
    }

    fn wire(cx: &Cx<Self>) {
        view::wire(cx);
    }

    fn start_service(_: &Model, _: &mut Effects<Self>) {
        // Nothing: the first look raises what the engine has to say as notifications, which are
        // the person's. The list is the engine's, read when `describe` asks.
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        // The first look draws the empty list's counters and raises what the engine has to say.
        fx.then(Command::Tick);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &DownloadManagerApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn event(event: Event<()>, m: &Model) -> Option<Command> {
        match event {
            // With a download being written, or a command on its way to the engine, the window
            // closes and the app runs on until the writers have stopped; with neither, the
            // framework closes it and ends, as before.
            Event::CloseRequested(window) if m.busy() => Some(Command::Closing(window)),
            // A second launch opened the window again while that close waits: it is cancelled.
            Event::Presented(_) if m.closing => Some(Command::Reopened),
            // A logout (a signal, the display going) or a person's quit: the writers are paused
            // and the list is written first, so no partial file is left mid-write.
            Event::ExitRequested(_) => Some(Command::Ending),
            // Brought to the front: the list is read again, so it is the engine's word and not
            // the last look's. Nothing is drawn if the workers moved nothing.
            Event::Activated(_) => Some(Command::Tick),
            _ => None,
        }
    }

    fn rail(command: RailCommand) -> Option<Command> {
        Some(Command::Rail(command))
    }
}
