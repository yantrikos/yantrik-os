//! Yantrik Text Editor, on YantrikApp (design/app-framework-2026-10-21.md, step 10: behaviour
//! unchanged).
//!
//! A bounded, multi-tab text workbench in one window. Each tab is a view on a document in the
//! framework's `Documents` table; opening and saving are jobs, one file at a time; a save replaces
//! its file by an exchange that keeps whatever it took the place of; every open tab's file is
//! watched, so another program's edit is followed, a conflict is said, and a moved file is
//! followed (#86); every unsaved tab's text is in the framework's draft lane, so a crash or a
//! logout loses nothing, and the next start opens it again. The twenty actions are intents,
//! worded and graded as they were, and `main.rs` is three lines.

use std::path::PathBuf;

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::paths::Paths;
use yantrik_app::Refused;

mod agent_rule;
mod caller;
mod claim;
mod command;
mod describe;
mod files;
mod intents;
mod model;
mod notice;
mod owner;
mod prompt;
mod reading;
mod recovery;
mod tab;
mod text;
mod update;
mod view;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(TextEditorApp);

pub struct TextEditor;

/// Where the old builds' lock is. Under `cargo test`, inside the test's own root: the tests' apps
/// run side by side in one process, and none may stand in for the person's running editor.
fn legacy_lock(paths: &Paths) -> Option<PathBuf> {
    if cfg!(test) {
        let dir = paths.state_dir();
        let _ = std::fs::create_dir_all(dir);
        Some(dir.join(claim::LEGACY_NAME))
    } else {
        claim::legacy_path()
    }
}

impl App for TextEditor {
    type Ui = TextEditorApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "editor";

    /// The model, once this copy holds the old builds' lock (claim.rs), with every draft the last
    /// run left opened again, and the old build's recovery file read once.
    fn model(boot: &Boot) -> Result<Model, Startup> {
        let claim = claim::hold(legacy_lock(&boot.paths).as_deref()).map_err(|claim::Held| {
            // Never from a test: it would ask the person's own editor to come forward.
            if !cfg!(test) {
                yantrik_app::instance::present_running(Self::ID);
            }
            Startup::handed_over(claim::ALREADY_RUNNING)
        })?;
        let state = boot.paths.state_dir().to_path_buf();
        let mut m = Model::new(claim, state.clone());
        let notice = recovery::restore(&mut m, &boot.drafts, files::legacy::read(&state, None));
        if m.tabs.is_empty() {
            update::tabs::blank(&mut m, owner::Opener::Person);
        }
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

    fn render(ui: &TextEditorApp, _: WindowId, m: &Model, changed: &Changed) {
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
            // Every launch, the first one included; one with no file opens nothing.
            Event::Launch(launch) if !launch.files.is_empty() => Some(Command::Launch(launch.files)),
            Event::CloseRequested(_) => Some(Command::CloseRequested),
            // No one to ask (a signal, the display gone): drafts, then the end.
            Event::ExitRequested(ExitReason::Logout) => Some(Command::LoggingOut),
            // A person's quit is the window's close: each unsaved tab asked about in turn.
            Event::ExitRequested(ExitReason::Person) => Some(Command::Quit),
            Event::Presented(_) => Some(Command::Presented),
            // The window coming forward asks nothing of the editor: it follows no clock and no
            // service, and its tabs are watched whether or not it is in front.
            Event::Activated(_) => None,
            Event::DraftSaved(name) => Some(Command::DraftSaved(name)),
            Event::DraftNotSaved { name, why } => Some(Command::DraftNotSaved { name, why }),
            Event::FileChanged { key, seen } => Some(Command::FileChanged { key, seen }),
            _ => None,
        }
    }
}
