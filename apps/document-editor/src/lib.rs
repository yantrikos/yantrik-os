//! Yantrik Document Editor (yDoc), on YantrikApp (design/app-framework-2026-10-21.md, step 10:
//! behaviour unchanged).
//!
//! A Markdown document a person and a mind can both write in, in one window. The document is a
//! framework `Document`: opening, saving and exporting are jobs, one file at a time; a save
//! replaces its file by an exchange that keeps whatever it took the place of; the file is
//! watched, so another program's edit is followed, a conflict is said, and a moved file is
//! followed (#86); the unsaved text is in the framework's draft lane, so a crash or a logout loses
//! nothing, and the next start opens it again. The ten actions are intents, worded and graded as
//! they were, and `main.rs` is three lines.

use std::path::PathBuf;

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::paths::Paths;
use yantrik_app::rail::RailCommand;
use yantrik_app::Refused;

mod agent_rule;
mod caller;
mod claim;
mod command;
mod describe;
mod files;
mod intents;
mod limits;
mod markdown;
mod model;
mod notice;
mod prompt;
mod rail;
mod recovery;
mod update;
mod view;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(DocumentEditorApp);
yantrik_app::rail_window!(DocumentEditorApp);

/// The app: yDoc.
pub struct YDoc;

/// Where the old builds' lock is. Under `cargo test`, inside the test's own root: the tests' apps
/// run side by side in one process, and none may stand in for the person's running yDoc.
fn legacy_lock(paths: &Paths) -> PathBuf {
    if cfg!(test) {
        let dir = paths.state_dir();
        let _ = std::fs::create_dir_all(dir);
        dir.join(claim::LEGACY_NAME)
    } else {
        claim::legacy_path()
    }
}

impl App for YDoc {
    type Ui = DocumentEditorApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    /// The id the launcher routes `documents` to (`Launch::Program { id: "documents", bin:
    /// "yantrik-document-editor" }`) and the socket's name, `app-documents.sock`.
    const ID: &'static str = "documents";

    /// The model, once this copy holds the old builds' lock (claim.rs), with the draft the last
    /// run left opened again, and the old build's recovery file read once.
    fn model(boot: &Boot) -> Result<Model, Startup> {
        let claim = claim::hold(Some(&legacy_lock(&boot.paths))).map_err(|claim::Held| {
            // Never from a test: it would ask the person's own yDoc to come forward. Nor for an
            // agent's start: it shows the person nothing (#978).
            if !cfg!(test) && !boot.service {
                yantrik_app::instance::present_running(Self::ID);
            }
            Startup::handed_over(claim::ALREADY_RUNNING)
        })?;
        let state = boot.paths.state_dir().to_path_buf();
        let mut m = Model::new(claim, state.clone(), boot.mind.clone(), boot.for_mind);
        let old = files::legacy::read(&files::legacy::old_dir(&state), None);
        let notice = recovery::restore(&mut m, &boot.drafts, old, !boot.launch.files.is_empty());
        m.notice.set(notice);
        Ok(m)
    }

    fn wire(cx: &Cx<Self>) {
        let ui = cx.ui();
        view::wire(cx, &ui);
    }

    fn start_service(_: &Model, _: &mut Effects<Self>) {
        // Nothing: the document editor's start is its recovery of the person's unsaved work, which
        // waits for their first window. An agent's start serves the model as it is.
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.then(Command::Started);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &DocumentEditorApp, _: WindowId, m: &Model, changed: &Changed) {
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
            // Every launch, the first one included; one with no file opens nothing. yDoc takes
            // the first file named, as it always did.
            Event::Launch(launch) => launch.files.into_iter().next().map(Command::Launch),
            Event::CloseRequested(_) => Some(Command::CloseRequested),
            // No one to ask (a signal, the display gone): the draft, then the end.
            Event::ExitRequested(ExitReason::Logout) => Some(Command::LoggingOut),
            // A person's quit is the window's close: asked about once.
            Event::ExitRequested(ExitReason::Person) => Some(Command::Quit),
            Event::Presented(_) => Some(Command::Presented),
            // The window coming forward asks nothing of yDoc: it follows no clock and no
            // service, and its file is watched whether or not it is in front.
            Event::Activated(_) => None,
            Event::DraftSaved(name) => Some(Command::DraftSaved(name)),
            Event::DraftNotSaved { name, why } => Some(Command::DraftNotSaved { name, why }),
            Event::FileChanged { key, seen } => Some(Command::FileChanged { key, seen }),
            _ => None,
        }
    }

    fn rail(command: RailCommand) -> Option<Command> {
        Some(Command::Rail(command))
    }
}
