//! Yantrik Spreadsheet (ySheets), on YantrikApp (design/app-framework-2026-10-21.md, step 10:
//! behaviour unchanged).
//!
//! One sheet, typed into, worked out, opened from and saved to CSV, with undo and redo, and a
//! control surface that does the same things by name. The sheet is a framework `Document`: opening
//! and saving are jobs, one file at a time; a save replaces its file by an exchange that keeps
//! whatever it took the place of; the file is watched, so another program's edit is followed, a
//! conflict is said, and a moved file is followed (#86); the unsaved sheet is in the framework's
//! draft lane, so a crash or a logout loses nothing, and the next start opens it again.
//!
//! Working the sheet out is a job too. A formula can read twenty million cells, so typing changes
//! the text at once and the values arrive from a job, taken only if the sheet is still the one it
//! was made for. The actions are intents, worded and graded as they were, and `main.rs` is three
//! lines.
//!
//! A path on the command line (`%f` in the .desktop file) is opened at start; a second launch with
//! a path hands it to the window that is already open.

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::Refused;

mod address;
mod agent_rule;
mod book;
mod caller;
mod command;
mod csv;
mod describe;
mod files;
mod formula;
mod history;
mod intents;
mod mind_view;
mod model;
mod recovery;
mod sheet;
mod update;
mod view;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(SpreadsheetApp);

/// The app: ySheets.
pub struct YSheets;

impl App for YSheets {
    type Ui = SpreadsheetApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    /// The id the launcher routes `spreadsheet`, `ysheets` and `sheets` to, and the socket's name,
    /// `app-spreadsheet.sock`.
    const ID: &'static str = "spreadsheet";

    /// The model, with the draft the last run left opened again.
    fn model(boot: &Boot) -> Result<Model, Startup> {
        let mut m = Model::new(boot.paths.state_dir().to_path_buf(), boot.for_mind);
        let notice = recovery::restore(&mut m, &boot.drafts, !boot.launch.files.is_empty());
        m.notice.set(notice);
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

    fn render(ui: &SpreadsheetApp, _: WindowId, m: &Model, changed: &Changed) {
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
            // Every launch, the first one included; one with no file opens nothing. ySheets takes
            // the first file named, as it always did.
            Event::Launch(launch) => launch.files.into_iter().next().map(Command::Launch),
            Event::CloseRequested(_) => Some(Command::CloseRequested),
            // No one to ask (a signal, the display gone): the draft, then the end.
            Event::ExitRequested(ExitReason::Logout) => Some(Command::LoggingOut),
            // A person's quit is the window's close: the same question, once.
            Event::ExitRequested(ExitReason::Person) => Some(Command::Quit),
            Event::Presented(_) => Some(Command::Presented),
            // The window coming forward asks nothing of ySheets: it follows no clock and no
            // service, and its file is watched whether or not it is in front.
            Event::Activated(_) => None,
            Event::DraftSaved(name) => Some(Command::DraftSaved(name)),
            Event::DraftNotSaved { name, why } => Some(Command::DraftNotSaved { name, why }),
            Event::FileChanged { key, seen } => Some(Command::FileChanged { key, seen }),
            _ => None,
        }
    }
}
