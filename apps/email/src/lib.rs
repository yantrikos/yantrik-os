//! Yantrik Email, on YantrikApp (design/app-framework-2026-10-21.md, §11 step 9: behaviour
//! unchanged).
//!
//! The mail service is asked from jobs, never on the UI thread (`backend`); every button and the
//! action of the same name raise one command, applied by `update`; the window is drawn by
//! `view::render` and described by `describe`, both from the same model; the seven actions are
//! intents, worded and graded as they were; the unsent draft is the app's own file under the
//! framework's paths (`files`); and `main.rs` is three lines.
//!
//! What this app never holds: a mail password beyond the one command that carries it from the
//! setup form to the service, or any token at all. The service keeps the credentials.

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::Refused;

mod backend;
mod command;
mod demo;
mod describe;
mod files;
mod intents;
mod model;
mod rows;
/// The half of the app that has no window in it. `tests/email-core` compiles it on its own.
pub mod state;
mod update;
mod view;
mod words;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(EmailApp);

pub struct Email;

impl App for Email {
    type Ui = EmailApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "email";

    fn model(boot: &Boot) -> Result<Model, Startup> {
        let files = files::Files::for_app(&boot.paths);
        files::import_legacy_draft(&boot.paths, &files, files::legacy_draft().as_deref());
        let mut kept = state::load_draft(&files.draft);
        // What the lanes held when Email last closed is the newest text there is: the composer at
        // a logout first (an edit made after Send included), else a message being sent then.
        let lane = |name: &str| boot.drafts.json::<state::Draft>(name).filter(|d| !d.is_empty());
        let (sending, composing) = (lane(model::SENDING), lane(model::COMPOSING));
        let from_lanes = composing.or_else(|| sending.clone());
        if let Some(text) = &from_lanes {
            kept = Ok(Some(text.clone()));
        }
        let mut model = Model::new(backend::for_app(&boot.paths), boot.mind.clone(), files, kept);
        model.composer.kept_unsure = sending.is_some() || files::is_unsure(&model.files.draft);
        // Said once the first look lands, unless the person has chosen about the draft first.
        model.composer.announce_unsent = from_lanes.as_ref().map(|_| match sending {
            Some(_) => update::drafts::UNSENT_AT_START,
            None => update::drafts::WRITING_AT_START,
        });
        model.composer.unsent_at_start = from_lanes;
        if demo::asked() {
            demo::populate(&mut model);
        }
        Ok(model)
    }

    fn wire(cx: &Cx<Self>) {
        let ui = cx.ui();
        view::wire(cx, &ui);
    }

    fn start(m: &Model, fx: &mut Effects<Self>) {
        // One look at the mail service, which starts it if it is not running. Not for the
        // fixture, which has no service behind it.
        if !m.looked {
            fx.then(Command::Look(command::Look::Start));
        }
        // Applied within `start`, before the window takes a press: whatever the person then does
        // to the draft is a file operation queued after this one, so their choice is the last
        // word.
        if m.composer.unsent_at_start.is_some() {
            fx.then(Command::OfferUnsent);
        }
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &EmailApp, _: WindowId, m: &Model, changed: &Changed) {
        view::render::render(ui, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    /// The app ending, whoever ends it (the session, a person's quit, the window closing): the
    /// composer's text is handed to the draft lane without asking, and the app ends. A send in
    /// flight already left its text there when Send was pressed. Nothing is sent.
    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            Event::ExitRequested(ExitReason::Logout) => Some(Command::Ending(command::Ending::Logout)),
            Event::ExitRequested(ExitReason::Person) => Some(Command::Ending(command::Ending::Quit)),
            Event::CloseRequested(window) => Some(Command::Ending(command::Ending::Window(window))),
            _ => None,
        }
    }
}
