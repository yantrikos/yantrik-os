//! Snippets, driven headless through `yantrik-testing`: the window, the jobs and the clock run when
//! each test says, against a real file under the test's own directory, a disk that can be told to
//! refuse (`disk::testing`), a clipboard that keeps what it is given (`clipboard::testing`) and a
//! fake Mind.
//!
//! - `parity.rs`: the published actions and `describe`, pinned to what the app published before
//!   the framework.
//! - `editing.rs`: the window's buttons and the editor's typing.
//! - `writes.rs`: the write lane: a refused write, writes landing out of order, answers per call.
//! - `copying.rs` and `files.rs`: the clipboard, import, export and a file on the command line.
//! - `ending.rs`: the window closed, a logout, the display going. `recovery.rs`: unsaved text kept
//!   in the draft lane and offered back on the recovery bar, after a refused save or a restart.
//!   `refused.rs`: a refused or unfinished write never loses the typing or offers it wrongly.
//!   `kept.rs`: a draft goes only on a landed write or a Discard, under random failures too.
//! - `agent_exposure.rs`: no snippet code in `describe` or an agent's search, and an agent's
//!   import only of a file in the person's home.
//! - `secrets.rs`: a snippet's code is never in a log, an answer, `describe` or a debug print.
//! - `rail.rs`: the agent rail and the AI button. `private_mode.rs`: `describe` in Private mode.
//! - `folders.rs`: the library's and the drafts' folders are the person's alone (0700).

use std::sync::Arc;

use serde_json::{json, Value};
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App, Paths, Services};
use yantrik_testing::TestApp;

use crate::clipboard::testing::{self as clip, Kept};
use crate::disk::testing::{self as faulty, Faulty};
use crate::store::{file, Library};
use crate::SnippetManager;

mod agent_exposure;
mod copying;
mod editing;
mod ending;
mod files;
mod folders;
mod kept;
mod parity;
mod private_mode;
mod rail;
mod recovery;
mod refused;
mod secrets;
mod writes;

/// An app under test, with what it talks to.
pub(crate) struct Fixture {
    pub app: TestApp<SnippetManager>,
    pub paths: Paths,
    pub disk: Arc<Faulty>,
    pub clipboard: Arc<Kept>,
    pub mind: Arc<FakeMind>,
}

/// Started over an empty store.
pub(crate) fn open() -> Fixture {
    start_at(Paths::temporary(SnippetManager::ID), FakeMind::new(), Vec::new())
}

/// Started at `paths` (a restart, when the paths were another fixture's), with `args` on its
/// command line.
pub(crate) fn start_at(paths: Paths, mind: FakeMind, args: Vec<std::ffi::OsString>) -> Fixture {
    let disk = faulty::install(&paths);
    let clipboard = clip::of(&paths);
    let mind = Arc::new(mind);
    let app = TestApp::<SnippetManager>::start_at_with(paths.clone(), args, mind.clone(), Services::fakes());
    let f = Fixture { app, paths, disk, clipboard, mind };
    f.quiet();
    f
}

impl Fixture {
    /// Runs jobs, timers due now and what they posted, until nothing is left.
    pub fn quiet(&self) {
        for _ in 0..16 {
            if self.app.jobs_waiting() == 0 && self.app.pending_commands() == 0 {
                return;
            }
            self.app.run_jobs();
        }
    }

    /// The person's own call through the real dispatch, answered once its work has landed.
    pub fn act(&self, name: &str, args: Value) -> Result<Value, String> {
        let pending = self.app.act(name, args)?;
        self.quiet();
        pending.settle()
    }

    /// A snippet kept through `new`, with its id.
    pub fn keep(&self, title: &str, language: &str, code: &str, tags: &str) -> i32 {
        let answer = self
            .act("new", json!({ "title": title, "language": language, "code": code, "tags": tags }))
            .unwrap_or_else(|e| panic!("{title} was not kept: {e}"));
        answer["id"].as_i64().unwrap() as i32
    }

    /// Two snippets, as the store tests seed them.
    pub fn seeded(&self) -> (i32, i32) {
        let first = self.keep("Tail the journal", "Shell", "journalctl -fu yantrik-ui", "logs, systemd");
        let second = self.keep("Borrow checker dance", "Rust", "let mut guard = state.borrow_mut();", "rust, borrow");
        (first, second)
    }

    /// The state file's path.
    pub fn path(&self) -> std::path::PathBuf {
        file::state_path(self.paths.data_dir())
    }

    /// What the file holds now, read as a restart would read it.
    pub fn on_disk(&self) -> Library {
        file::read_back(&self.path()).expect("the state file reads")
    }

    pub fn file_bytes(&self) -> Option<Vec<u8>> {
        std::fs::read(self.path()).ok()
    }

    pub fn describe(&self) -> Value {
        self.app.describe()
    }

    pub fn state(&self) -> Value {
        self.describe()["state"].clone()
    }

    pub fn notice(&self) -> String {
        self.app.model(|m| m.notice.clone())
    }

    /// The person typing into the editor: the fields as the window would hold them, then the
    /// screen's `detail-edited`.
    pub fn type_into(&self, title: &str, code: &str) {
        let (title, code) = (title.to_string(), code.to_string());
        self.app.ui(move |ui| {
            ui.set_detail_title(title.into());
            ui.set_detail_code(code.into());
            ui.invoke_detail_edited();
        });
    }

    /// The app again at the same paths, as after a restart. This one is shut down first.
    pub fn restart(self) -> Fixture {
        let _ = self.app.shutdown();
        let paths = self.paths.clone();
        drop(self);
        start_at(paths, FakeMind::new(), Vec::new())
    }
}
