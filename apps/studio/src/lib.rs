//! Yantrik Studio, on YantrikApp (design/app-framework-2026-10-21.md, section 11: behaviour
//! unchanged).
//!
//! Pictures from a sentence, made where the person says, saved as files. The window and the
//! control surface are two views of one model. Everything a person can press here is an action a
//! mind can name there, and both are the same commands, so an answer given to
//! `yos act studio generate` and the row that appears in the gallery are the same event seen
//! twice.
//!
//! The part that is not like the other apps is that making a picture takes seconds to minutes.
//! Nothing here waits for one: `generate` answers with a job id and declares itself deferred, each
//! picture is a worker, and the command it lands is what moves the queue on.
//!
//! # How it is put together
//!
//! - `model.rs` and `command.rs`: what the app is, and every change to it. `update/`: the only
//!   code that changes the model. `view.rs` and `words.rs`: the window and every string in it.
//!   `describe.rs`: what the Mind reads. `intents.rs`: the nine actions, as they were published.
//! - `jobs.rs`: the work that touches the disk or the network. `backend.rs`, `config.rs`,
//!   `gallery.rs`, `trash.rs` and `workflow.rs`: the backends, the configuration, the files.
//!
//! Four rules shaped the code:
//!
//! - NOTHING SLOW ON THE UI THREAD. A name is turned into a file, a picture is made and filed, the
//!   folder is read, a picture is moved to the Trash and a backend choice is written down on
//!   workers. The UI thread holds the model and draws it; the one read it makes at start (the
//!   configuration and the gallery) happens before any window exists.
//! - ANSWERS ARE THE CALL'S OWN. A call is told what its own target and its own worker found,
//!   written in the command that settles it (`answers.rs`), never what the person happens to have
//!   chosen or asked for meanwhile.
//! - A GRADE FOLLOWS THE BACKEND. `generate` and `variations` are regraded when the surface is
//!   served and when a change of backend lands; until then they are refused.
//! - ENDING IS AT ONCE. Studio starts no process of its own and holds nothing a close must wait
//!   for: a close, a quit or a logout cancels the pictures in flight, which a ComfyUI server is told
//!   of, and the app ends. A picture is filed whole with its record or not at all.

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::Refused;

pub mod agent_rule;
mod answers;
mod args;
pub mod backend;
mod command;
pub mod config;
mod describe;
mod fake;
mod fetch;
pub mod gallery;
mod intents;
mod jobs;
mod safe_write;
mod mind_view;
pub mod model;
pub mod plan;
mod said;
pub mod trash;
mod update;
mod view;
pub mod workflow;
mod words;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(StudioApp);

pub struct Studio;

impl App for Studio {
    type Ui = StudioApp;
    type Model = Model;
    type Command = Command;
    /// Studio keeps its one choice, where pictures are made, in its own file
    /// (`~/.config/yantrik/studio.json`, `config.rs`), as it always did; the framework's settings
    /// file is not used.
    type Settings = ();
    /// The control surface's id, as it always was: `yos act studio ...`.
    const ID: &'static str = "studio";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        // Before any window exists, so nothing waits on it: the configuration, and the gallery as
        // it is, which the first frame and the first `describe` both show.
        let (config, places) = world(boot);
        let listed = gallery::read_all(&places.gallery);
        let mut m = Model::new(config, places, listed, boot.for_mind);
        // The prompt the person had typed and not sent when Studio last closed comes back, and
        // says so.
        if let Some(kept) = boot.drafts.get(update::PROMPT_DRAFT).and_then(|b| std::str::from_utf8(b).ok()) {
            if !kept.trim().is_empty() && !boot.for_mind {
                m.form.prompt = kept.to_string();
                m.notice = "Your unsent prompt from last time is back in the box.".to_string();
            }
        }
        // A configuration file that exists and was not used says so in the window and in
        // `describe`, not only in the log: which file, and what is wrong with it.
        if let Some(refused) = m.config.refused.clone() {
            m.notice = if m.notice.is_empty() { refused } else { format!("{refused} {}", m.notice) };
        }
        Ok(m)
    }

    fn wire(cx: &Cx<Self>) {
        view::wire(cx);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        update::first(fx);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &StudioApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            // Brought to the front: the folder is looked at again, for what arrived meanwhile.
            Event::Activated(_) => Some(Command::Activated),
            // A close, a quit and a logout end the app at once (the pictures in flight are
            // cancelled), asking nothing; an unsent prompt is kept first, in the command that
            // answers the event.
            Event::CloseRequested(window) => Some(Command::Closing(window)),
            Event::ExitRequested(_) => Some(Command::Ending),
            // A launch opens the window and nothing more: Studio takes no file.
            _ => None,
        }
    }
}

/// The configuration and the places this app writes to.
#[cfg(not(test))]
fn world(_: &Boot<()>) -> (config::Config, model::Places) {
    let config = config::Config::load();
    let places = model::Places::real(&config);
    (config, places)
}

/// A test hands its app a world of its own (`tests/world.rs`), found by the folder its framework
/// files are in, which is one per test.
#[cfg(test)]
fn world(boot: &Boot<()>) -> (config::Config, model::Places) {
    tests::world::take(&boot.paths)
}
