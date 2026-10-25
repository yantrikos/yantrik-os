//! Yantrik Snippets, on YantrikApp (design/app-framework-2026-10-21.md, §11 step 10: behaviour
//! unchanged).
//!
//! It once could not keep a snippet: `on_snip_save` dropped what the editor sent, Copy was a log
//! line and nothing was ever written. Now it keeps snippets in `~/.local/share/yantrik/snippets/
//! snippets.json`, puts a snippet on the clipboard with `wl-copy`, searches and filters what it
//! has, and answers on a control surface so the launcher, a file on the command line and a mind
//! all reach the same window.
//!
//! # How it is put together
//!
//! - `store/`: the snippets and collections as plain data (`mod.rs`), their one file (`file.rs`).
//! - `model.rs` and `command.rs`: what the app is, and every change to it.
//! - `update/`: the only code that changes the model, by concern.
//! - `lane.rs` and `disk.rs`: writing the file off the UI thread, in order, never a change the
//!   file refused left on screen. `clipboard.rs`: `wl-copy`, from a job.
//! - `view.rs`: the window, bound and drawn. `describe.rs` and `answers.rs`: what the Mind reads.
//! - `intents.rs`: the nine actions, as they were published. `rail.rs`: the agent rail's source.

use std::path::PathBuf;

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::control::View;
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Intents;
use yantrik_app::rail::{Rail, RailCommand};
use yantrik_app::{Paths, Refused};

mod answers;
mod clipboard;
mod command;
mod describe;
mod disk;
mod intents;
mod lane;
mod model;
mod rail;
pub mod store;
mod update;
mod view;
#[cfg(test)]
mod tests;

pub use command::Command;
pub use model::{Editor, Model};

slint::include_modules!();
yantrik_app::window!(SnippetManagerApp);
yantrik_app::rail_window!(SnippetManagerApp);

pub struct SnippetManager;

/// Where the library is: `$YANTRIK_SNIPPETS_DIR` when it is set (a conformance probe needs
/// somewhere to work that is not the person's real collection), the app's data folder otherwise,
/// which is the folder the app always used. Never the variable under `cargo test`, so no test can
/// touch the snippets of the person running it.
fn store_dir(paths: &Paths) -> PathBuf {
    match std::env::var_os("YANTRIK_SNIPPETS_DIR") {
        Some(dir) if !cfg!(test) => {
            // The design's one path rule (§6, ★3): the per-app overrides warn for one release.
            tracing::warn!(dir = %PathBuf::from(&dir).display(), "YANTRIK_SNIPPETS_DIR is set; it will be removed after this release");
            PathBuf::from(dir)
        }
        _ => paths.data_dir().to_path_buf(),
    }
}

impl App for SnippetManager {
    type Ui = SnippetManagerApp;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    /// `snippets`, not `snippet-manager`: the id the launcher's route carries, so `yos act
    /// snippets …` and the dock reach the same process.
    const ID: &'static str = "snippets";

    fn model(boot: &Boot<()>) -> Result<Model, Startup> {
        let path = store::file::state_path(&store_dir(&boot.paths));
        // Read once, before any window exists: the window opens on what is kept.
        let loaded = store::file::load(&path);
        let mut m = Model {
            library: loaded.library,
            query: String::new(),
            tag: String::new(),
            collection: store::ALL,
            selected: -1,
            editor: Editor::none(),
            notice: loaded.notice,
            copied: false,
            lane: lane::Lane::new(disk::for_app(&boot.paths), path),
            work: 0,
            closing: false,
            ending: false,
            answer: serde_json::Value::Null,
            rail: Rail::new(boot.mind.clone()),
            clipboard: clipboard::for_app(&boot.paths),
            drafts: Default::default(),
            typed: 0,
            offers: Vec::new(),
        };
        recover(&mut m, &boot.drafts);
        Ok(m)
    }

    fn wire(cx: &Cx<Self>) {
        view::wire(cx);
    }

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        update::update(m, command, fx)
    }

    fn render(ui: &SnippetManagerApp, _: WindowId, m: &Model, changed: &Changed) {
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
            // A file on the command line, the first launch's or one handed over by a second.
            Event::Launch(launch) if !launch.files.is_empty() => Some(Command::Launch(launch.files)),
            // Work in flight, or typing unsaved: the typing goes to the draft lane and the window
            // closes, and the app runs on until the work lands. With neither, the framework closes
            // the window and ends, as before.
            Event::CloseRequested(window) if m.work > 0 || m.editor_dirty() => Some(Command::Closing(window)),
            // A logout, a signal, the display gone, or a person's quit.
            Event::ExitRequested(_) => Some(Command::Ending),
            // A second launch opened the window again while a close waited: the close is over.
            Event::Presented(_) if m.closing => Some(Command::Reopened),
            // Said, so the person knows the text is only in the window; no other copy goes.
            Event::DraftNotSaved { name, why } => Some(Command::DraftNotKept { name, why }),
            _ => None,
        }
    }

    fn rail(command: RailCommand) -> Option<Command> {
        Some(Command::Rail(command))
    }
}

/// Text the last run did not keep, offered back on the recovery bar: each draft whose text the
/// store does not hold. The first one's snippet is opened with the text in the editor, so the
/// person sees what Restore keeps; nothing is put back until they choose. A draft whose text the
/// store already holds is let go by the first command (`update::drafts::keep`).
fn recover(m: &mut Model, drafts: &yantrik_app::Drafts) {
    for name in drafts.names() {
        let Some(id) = name.strip_prefix(model::DRAFT_PREFIX).and_then(|n| n.parse::<i32>().ok()) else { continue };
        let Some(text) = drafts.json::<Editor>(name).filter(|t| t.id == id) else { continue };
        let kept = m.library.get(id).is_some() && !m.library.differs(id, &text.patch());
        // A draft for a snippet no longer kept names an id a refused create may have used and the
        // file never counted past: it is never given to another snippet.
        m.library.reserve(id);
        let mut draft = model::Draft::of(text, 0);
        if kept {
            // The file just read holds it: the first command lets it go (`update::drafts`).
            draft.pending = Some(0);
        } else {
            m.offers.push(model::Offer { id, why: model::Why::Closed });
        }
        m.drafts.insert(id, draft);
    }
    if let Some(first) = m.offers.first().map(|o| o.id) {
        m.select(first);
    }
}
