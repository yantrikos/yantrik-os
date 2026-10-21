//! Yantrik Notes, on YantrikApp (design/notes-gate-2026-10-21.md, step 8c: behaviour unchanged).
//!
//! A Markdown vault at `Paths::data_dir` for the id `notes`, which is where Notes has always kept
//! it (`~/.local/share/yantrik/notes`). The store keeps its bounded reads, its journal and its
//! Trash, and writes through the framework's disk seam; the open note is a document, saved by
//! jobs one at a time with the edits made meanwhile as one follow-up, its file watched while it
//! is open; the autosave is a framework timer; the twenty-one actions are intents, worded and
//! graded as they were; and `main.rs` is three lines.
//!
//! Step 8d-3b: a note can be open in more than one window. Each window is a view with a pane of
//! its own (pane.rs), and a note two windows show is one document (update/windows.rs).

use std::path::PathBuf;

use yantrik_app::app::{App, Boot, Changed, Event, ExitReason, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::paths::Paths;
use yantrik_app::Refused;

use crate::store::version::Format;

mod claim;
mod command;
mod describe;
mod intents;
mod model;
mod note;
mod notice;
mod pane;
mod recovery;
mod store;
mod undo;
mod update;
mod view;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::Model;

slint::include_modules!();
yantrik_app::window!(NotesApp);

pub struct Notes;

/// Where the vault is: `$YANTRIK_NOTES_DIR` when it is set (a test machine's probe, a second
/// library), the app's data folder otherwise. Never the variable under `cargo test`, so no test
/// can touch the library of the person running it.
fn vault_dir(paths: &Paths) -> PathBuf {
    match std::env::var_os("YANTRIK_NOTES_DIR") {
        Some(dir) if !cfg!(test) => PathBuf::from(dir),
        _ => paths.data_dir().to_path_buf(),
    }
}

/// Where the old builds' lock is. Under `cargo test`, inside the test's own root: the tests'
/// apps run side by side in one process, and none may stand in for the person's running Notes.
fn legacy_lock(paths: &Paths) -> Option<PathBuf> {
    if cfg!(test) {
        let dir = paths.state_dir();
        let _ = std::fs::create_dir_all(dir);
        Some(dir.join(claim::LEGACY_NAME))
    } else {
        claim::legacy_path()
    }
}

impl App for Notes {
    type Ui = NotesApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "notes";

    /// The model, once this copy holds the library (`claim`). A copy that does not is the second
    /// one: the first is asked to show its window, as the old build did, and this one stops as a
    /// hand-over, not a failure.
    ///
    /// A library a newer Notes wrote is refused first, before the claim makes its lock: nothing
    /// in that vault is touched by a build that does not know its format.
    fn model(boot: &Boot) -> Result<Model, Startup> {
        let vault = store::Vault::real(vault_dir(&boot.paths));
        if let Format::Newer(found) = vault.format() {
            return Err(Startup::new(store::version::newer_at_start(found)));
        }
        let claim = claim::hold(legacy_lock(&boot.paths).as_deref(), vault.dir()).map_err(|claim::Held| {
            // Never from a test: it would ask the person's own Notes to come forward.
            if !cfg!(test) {
                yantrik_app::instance::present_running(Self::ID);
            }
            Startup::handed_over(claim::ALREADY_RUNNING)
        })?;
        let recovery = recovery::Recovery::read(&boot.drafts, vault.dir(), boot.paths.state_dir());
        Ok(Model::new(vault, claim, recovery))
    }

    fn wire(cx: &Cx<Self>) {
        let ui = cx.ui();
        view::wire(cx, &ui, WindowId::PRIMARY);
    }

    fn wire_window(cx: &Cx<Self>, ui: &NotesApp, window: WindowId) {
        view::wire(cx, ui, window);
    }

    fn start(_: &Model, fx: &mut Effects<Self>) {
        fx.then(Command::Load);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &NotesApp, window: WindowId, m: &Model, changed: &Changed) {
        view::render::render(ui, window, m, changed);
    }

    fn describe(m: &Model) -> View {
        describe::view(m)
    }

    fn intents(table: &mut Intents<Self>) {
        intents::table(table);
    }

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            Event::CloseRequested(window) => Some(Command::CloseRequested(window)),
            Event::Activated(window) => Some(Command::From { window, command: Box::new(Command::Activated) }),
            Event::WindowNotOpened { window, why } => Some(Command::WindowNotOpened { window, why }),
            // No one to ask (a signal): drafts, then the end. A person (Ctrl+Q): asked once.
            Event::ExitRequested(ExitReason::Logout) => Some(Command::LoggingOut),
            Event::ExitRequested(_) => Some(Command::Quit),
            Event::DraftSaved(name) => Some(Command::DraftSaved(name)),
            Event::DraftNotSaved { name, why } => Some(Command::DraftNotSaved { name, why }),
            Event::FileChanged { key, seen } => Some(Command::FileChanged { key, seen }),
            _ => None,
        }
    }
}
