//! Yantrik Presentation (yPresent), on YantrikApp (design/app-framework-2026-10-21.md, step 10:
//! behaviour unchanged).
//!
//! A deck of slides a person and a mind can both build, in one window. The deck is a framework
//! `Document`: opening, saving and exporting are jobs, one file at a time; a save replaces its
//! file by an exchange that keeps whatever it took the place of; the file is watched, so another
//! program's edit is followed, a conflict is said, and a moved file is followed (#86); the unsaved
//! deck is in the framework's draft lane, so a crash or a logout loses nothing, and the next
//! start opens it again. The actions are intents, worded and graded as they were, and `main.rs`
//! is three lines.

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::Refused;

mod agent_rule;
mod caller;
mod command;
mod deck;
mod describe;
mod files;
mod intents;
mod mind_view;
mod model;
mod recovery;
mod update;
mod view;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(PresentationApp);

/// The app: yPresent.
pub struct YPresent;

impl App for YPresent {
    type Ui = PresentationApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    /// The id the launcher routes `presentation` and `slides` to (`Launch::Program { id:
    /// "presentation", bin: "yantrik-presentation" }`) and the socket's name,
    /// `app-presentation.sock`.
    const ID: &'static str = "presentation";

    /// The model, with the draft the last run left opened again, and the old build's recovery
    /// file read once.
    fn model(boot: &Boot) -> Result<Model, Startup> {
        let state = boot.paths.state_dir().to_path_buf();
        let mut m = Model::new(state.clone(), boot.mind.clone(), boot.for_mind);
        let old = files::legacy::read(&state);
        let notice = recovery::restore(&mut m, &boot.drafts, old, !boot.launch.files.is_empty());
        // The sentence names the deck's file; for a deck an agent is not shown, an agent reads it
        // without that name.
        let hidden = m.doc.hidden;
        m.notice.set_about(hidden, notice, || {
            format!("Recovered unsaved changes to {}. Save keeps them.", agent_rule::HIDDEN)
        });
        Ok(m)
    }

    fn wire(cx: &Cx<Self>) {
        let ui = cx.ui();
        view::wire(cx, &ui);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.then(Command::Started);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &PresentationApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            // Every launch, the first one included; one with no file opens nothing. yPresent takes
            // the first file named, as it always did.
            Event::Launch(launch) => launch.files.into_iter().next().map(Command::Launch),
            Event::CloseRequested(_) => Some(Command::CloseRequested),
            // No one to ask (a signal, the display gone): the draft, then the end.
            Event::ExitRequested(ExitReason::Logout) => Some(Command::LoggingOut),
            // A person's quit is the window's close: nothing is asked, the draft keeps the deck.
            Event::ExitRequested(ExitReason::Person) => Some(Command::Quit),
            Event::Presented(_) => Some(Command::Presented),
            // The window coming forward asks nothing of yPresent: it follows no clock and no
            // service, and its file is watched whether or not it is in front.
            Event::Activated(_) => None,
            Event::DraftSaved(name) => Some(Command::DraftSaved(name)),
            Event::DraftNotSaved { name, why } => Some(Command::DraftNotSaved { name, why }),
            Event::FileChanged { key, seen } => Some(Command::FileChanged { key, seen }),
            _ => None,
        }
    }
}
