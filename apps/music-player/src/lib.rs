//! Yantrik Music, on YantrikApp (design/app-framework-2026-10-21.md, §11 step 10: behaviour
//! unchanged).
//!
//! A library scanned from the folders you list, a queue, and playback through one headless mpv,
//! driven over its JSON IPC socket (`mpv/`). The screen draws what mpv reports — position,
//! length, paused, volume — and the control surface answers with the same reports, so "playing"
//! is never the app's own opinion.
//!
//! # How it is put together
//!
//! - `model.rs` and `command.rs`: what the app is, and every change to it.
//! - `update/`: the only code that changes the model.
//! - `player/`: the player as the jobs drive it, one command at a time in the order they were
//!   asked for. `mpv/`: the socket and the process.
//! - `view.rs`: the window, bound and drawn. `describe.rs`: what the Mind reads.
//! - `intents.rs`: the eight actions, as they were published. `answers.rs`: what each is told.
//! - `library.rs` and `queue.rs`: the scan and the queue, pure. `kept.rs`: the settings file.
//!   `seen.rs`: which paths an agent is told.
//!
//! Nothing that speaks to mpv or reads the disk runs on the UI thread. The audio stops when the
//! window closes, as it always did: the app lingers without a window only until the player is
//! stopped and reaped, and a logout does the same before the app ends.

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::Refused;

mod answers;
mod command;
mod describe;
mod intents;
mod kept;
mod library;
mod model;
mod mpv;
mod player;
mod queue;
mod seen;
mod update;
mod view;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use kept::Kept;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(MusicPlayerApp);

pub struct Music;

impl App for Music {
    type Ui = MusicPlayerApp;
    type Model = Model;
    type Command = Command;
    type Settings = Kept;
    /// The control surface's id, as it always was: `yos act music …`, `app-music.sock`.
    const ID: &'static str = "music";

    fn model(boot: &Boot<Kept>) -> Result<Model, Startup> {
        let rig = player::for_app(&boot.paths);
        Ok(Model::new(&rig, &boot.settings, boot.paths.config_file().to_path_buf()))
    }

    fn wire(cx: &Cx<Self>) {
        view::wire(cx);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        // The library is read again at every start: it is whatever the folders hold.
        fx.then(Command::Rescan);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &MusicPlayerApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn event(event: Event<Kept>, m: &Model) -> Option<Command> {
        match event {
            // `%f` from the launcher or Files, a path typed after the command, or either from a
            // second launch: played as the whole queue. One file, as before; a deep link's intent
            // is not something this app has ever taken.
            Event::Launch(launch) => launch.files.into_iter().next().map(Command::Open),
            // The window closes and the audio stops with it, as before. With a player running,
            // or a command on its way to one, the app lingers until mpv is stopped and reaped;
            // with neither, the framework closes it and ends.
            Event::CloseRequested(window) if m.running() || m.player_jobs > 0 => Some(Command::Closing(window)),
            // A second launch opened the window again while that close waits: it is cancelled.
            Event::Presented(_) if m.closing => Some(Command::Reopened),
            // A logout (a signal, the display going) or a person's quit: the player is stopped
            // and reaped first, so no mpv outlives the app.
            Event::ExitRequested(_) => Some(Command::Ending),
            Event::SettingsChangedOutside(kept) => Some(Command::KeptOutside(kept)),
            Event::SettingsNotSaved(why) => Some(Command::NotSaved(why)),
            // Brought to the front: nothing to read again. The player pushes what it does as it
            // does it, and the library is read at start and on Rescan, as before.
            Event::Activated(_) => None,
            _ => None,
        }
    }
}
