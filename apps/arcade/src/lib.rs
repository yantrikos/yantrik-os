//! Yantrik Arcade, on YantrikApp (design/app-framework-2026-10-21.md, §11: behaviour unchanged).
//!
//! A game construction kit. A mind writes small specs in two bounded JSON grammars (a character, a
//! game); a deterministic compiler turns them into one self-contained HTML file with the game loop,
//! juice, sound and a chunky stylised creature already inside it. The window is the workbench,
//! library on the left and spec editor on the right, and the control surface publishes the same
//! nine commands the buttons call, so a mind can drive the whole kit without a synthetic mouse.
//!
//! # How it is put together
//!
//! - `library.rs`: the characters and games as plain files under one directory, each spec, build
//!   and verdict written whole or not at all. `spec.rs`, `compile.rs` and `engine.js`: the two
//!   grammars and the compiler. `verify.rs` and `play.rs`: the headless verifier and the Browser.
//! - the lane: the line of commands to the library, in the order they were asked for (`yantrik_app::lane`).
//! - `model.rs` and `command.rs`: what the app is, and every change to it. `update/`: the only code
//!   that changes the model. `view.rs`: the window. `describe.rs`: what the Mind reads.
//! - `intents.rs`: the nine actions, as they were published. `answers.rs`: what each is told.
//!   `work.rs`: the commands against the library. `slow.rs`: the three that reach a browser.
//!
//! Three rules shaped the code:
//!
//! - ONE PATH. Every button and every surface action is a `Request` that lands in `update`; there
//!   is no private reimplementation for either caller.
//! - NOTHING SLOW ON THE UI THREAD. Every read and write of the library runs on the lane. Verify,
//!   screenshot and play each launch or reach a browser, seconds to minutes, so they run on a job
//!   of their own and answer "started"; the verdict lands in the library's verify.json and in
//!   `describe` when it is ready.
//! - REFUSALS ARE SENTENCES. A bad spec, an unknown game, a missing browser: each says in one
//!   sentence what is wrong and what to do, because the reader is usually a mind.
//!
//! The same binary has a headless CLI (`yantrik-arcade build|verify|screenshot …`, `cli.rs`) for
//! scripted runs and for testing on a machine with no display. Nothing here runs on a clock: the
//! window changes when a command moves it, and at rest it draws nothing.

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::Refused;

mod answers;
mod browser;
pub mod cli;
mod command;
pub mod compile;
mod describe;
mod intents;
pub mod library;
pub mod play;
mod model;
pub mod spec;
mod slow;
mod update;
pub mod verify;
mod view;
mod work;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(ArcadeApp);

pub struct Arcade;

impl App for Arcade {
    type Ui = ArcadeApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    /// The control surface's id, as it always was: `yos act arcade …`.
    const ID: &'static str = "arcade";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        let library = library::Library::for_app(&boot.paths);
        // The library's folder and the drafts', the person's alone before anything is written in
        // them. A folder that cannot be made is said by the load and the save.
        let state = boot.paths.state_dir();
        for folder in [library.root().to_path_buf(), state.to_path_buf(), state.join(yantrik_app::drafts::DIR)] {
            if let Err(e) = library::private_folder(&folder) {
                tracing::warn!(folder = %folder.display(), error = %e, "could not make the folder private");
            }
        }
        // Read once, before any window exists, so the first `describe` and the first paint both
        // see what is on disk.
        let listing = work::list(&library);
        let mut m = Model {
            lane: yantrik_app::lane::Lane::start("arcade-library", library.clone()),
            slow: slow::for_app(&boot.paths),
            library,
            listing,
            asked: 0,
            applied: 0,
            work: 0,
            reload_asked: false,
            slow_asked: 0,
            jobs: Default::default(),
            spec_text: String::new(),
            kept_text: String::new(),
            game_name: String::new(),
            notice: String::new(),
            drafted: false,
            for_mind: boot.for_mind,
            closing: false,
            ending: false,
            answer: serde_json::Value::Null,
        };
        // Text the last run did not keep is back in the editor, and says so.
        // Not in a Mind's copy: the draft is the person's own, unsaved, and not for it to read.
        if let Some(text) = update::drafts::recovered(&boot.drafts).filter(|_| !boot.for_mind) {
            m.spec_text = text;
            m.drafted = true;
            m.notice = "The spec you were typing is back in the editor; save it as a game or a character to keep it.".into();
        }
        Ok(m)
    }

    fn wire(cx: &Cx<Self>) {
        view::wire(cx);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &ArcadeApp, _: WindowId, m: &Model, changed: &Changed) {
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
            // A command on the lane, or typing unsaved: the typing goes to the draft lane and the
            // window closes, and the app runs on until the command lands. With neither, the
            // framework closes the window and ends, as before.
            Event::CloseRequested(window) if m.busy() || m.unsaved() || !m.jobs.running().is_empty() => Some(Command::Closing(window)),
            // A second launch opened the window again while that close waits: it is cancelled.
            Event::Presented(_) if m.closing => Some(Command::Reopened),
            // Brought forward again (a second launch, or the shell's present): the library is read
            // again, as when the window is activated.
            Event::Presented(_) => Some(Command::Reload),
            // A logout (a signal, the display going) or a person's quit.
            Event::ExitRequested(_) => Some(Command::Ending),
            // Brought to the front: the library is read again, so the lists are the disk's word.
            Event::Activated(_) => Some(Command::Reload),
            Event::DraftNotSaved { why, .. } => Some(Command::DraftNotKept { why }),
            _ => None,
        }
    }
}
