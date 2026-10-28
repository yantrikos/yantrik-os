//! Arcade, driven headless through `yantrik-testing`: the window, the jobs and the clock run when
//! each test says, against a real library under the test's own directory and a stand-in for the
//! browser (`Fake`) that answers when the test lets it.
//!
//! - `parity.rs`: the published actions and `describe`, pinned to what the app published before
//!   the framework, and the guide's rows.
//! - `flow.rs`: every action's answer, the buttons, and the browser jobs.
//! - `order.rs`: answers per invocation, commands in the order asked, an older listing never
//!   replacing a newer one, and the lists read again when the window comes forward.
//! - `closing.rs`: the window closed, a logout and the display going, with a command on the lane
//!   and with text unsaved.
//! - `drafts.rs`: the editor's text in the draft lane.
//! - `rest.rs`: a window at rest draws nothing and arms nothing.
//! - `private_mode.rs`: Arcade keeps none of its keys in Private mode. `folders.rs`: the library's
//!   and the drafts' folders are the person's alone.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use yantrik_app::{App, Paths};
use yantrik_testing::{Call, Pending, TestApp};

use crate::command::Verb;
use crate::library::Library;
use crate::slow::Slow;
use crate::Arcade;

mod closing;
mod drafts;
mod flow;
mod folders;
mod mind_view;
mod order;
mod parity;
mod private_mode;
mod rest;
mod window;

/// The token an agent's calls carry here.
pub(crate) const AGENT: &str = "tok-test-agent";

pub(crate) const PIP: &str = r##"{
    "name": "Pip",
    "archetype": "critter",
    "ears": "pointy",
    "tail": "curl",
    "palette": { "base": "#44cc88", "belly": "#f2ead8", "accent": "#ff9f43", "nose": "#2f3640", "eye": "#2f3640" }
}"##;

pub(crate) fn meadow_run(title: &str) -> String {
    format!(
        r##"{{
            "title": "{title}",
            "player": {{ "character": {PIP} }},
            "collectible": {{ "kind": "berry", "count": 5 }},
            "hazards": [{{ "kind": "wanderer", "speed": 2.0, "count": 2 }}]
        }}"##
    )
}

/// The browser, as a test has it answer: every call is recorded, and the verdict is the test's.
/// It never blocks: the jobs of a test run on the one thread the windows share, so a worker that
/// waited would hold every other test. A test leaves a worker waiting by not running its job yet
/// (`Fixture::start_slow`), which is how a browser job is seen running.
pub(crate) struct Fake {
    pub calls: Mutex<Vec<(Verb, String)>>,
    pub verdict: Mutex<Result<String, String>>,
}

impl Fake {
    fn new() -> Fake {
        Fake { calls: Mutex::new(Vec::new()), verdict: Mutex::new(Ok("passed all 7 gates".into())) }
    }

    pub fn calls(&self) -> Vec<(Verb, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Slow for Fake {
    fn run(&self, verb: Verb, _: &Library, slug: &str, _: &Path) -> Result<String, String> {
        self.calls.lock().unwrap().push((verb, slug.to_string()));
        self.verdict.lock().unwrap().clone()
    }
}

fn fakes() -> &'static Mutex<HashMap<PathBuf, Arc<Fake>>> {
    static FAKES: OnceLock<Mutex<HashMap<PathBuf, Arc<Fake>>>> = OnceLock::new();
    FAKES.get_or_init(Default::default)
}

/// The browser an app started at `paths` reaches: the test's own, made on first ask.
pub fn slow_for(paths: &Paths) -> Arc<dyn Slow> {
    fake_of(paths)
}

pub(crate) fn fake_of(paths: &Paths) -> Arc<Fake> {
    fakes().lock().unwrap().entry(paths.data_dir().to_path_buf()).or_insert_with(|| Arc::new(Fake::new())).clone()
}

/// An app under test, and where its files are.
pub(crate) struct Fixture {
    pub app: TestApp<Arcade>,
    pub paths: Paths,
    pub browser: Arc<Fake>,
}

/// Started over an empty library.
pub(crate) fn open() -> Fixture {
    start_at(Paths::temporary(Arcade::ID))
}

/// Started at `paths` (a restart, when they were another fixture's).
pub(crate) fn start_at(paths: Paths) -> Fixture {
    let browser = fake_of(&paths);
    let app = TestApp::<Arcade>::start_at(paths.clone(), Vec::new());
    let f = Fixture { app, paths, browser };
    f.quiet();
    f
}

impl Fixture {
    /// Runs jobs, and what they posted, until none is left.
    pub fn quiet(&self) {
        const ROUNDS: usize = 32;
        for _ in 0..ROUNDS {
            self.app.tick();
            if self.app.jobs_waiting() == 0 {
                return;
            }
            self.app.run_jobs();
        }
        panic!("jobs still waiting after {ROUNDS} rounds");
    }

    /// Asks for browser actions and runs only the lane's part of each, so every worker is left
    /// waiting where the test can see its job running. Each answer is "started".
    pub fn start_slow(&self, calls: Vec<(&str, Value)>) -> Vec<Pending> {
        let pendings: Vec<Pending> = calls.iter().map(|(name, args)| self.app.act(name, args.clone()).unwrap()).collect();
        for _ in &calls {
            self.app.run_one_job();
        }
        self.app.tick();
        pendings
    }

    /// The person's own call of an intent, answered once its work has landed.
    pub fn act(&self, name: &str, args: Value) -> Result<Value, String> {
        let pending = self.app.act(name, args)?;
        self.settled(pending)
    }

    /// An agent's call.
    pub fn agent(&self, name: &str, args: Value) -> Result<Value, String> {
        let pending = self.app.act_as(Call::new(name, args).as_agent(AGENT))?;
        self.settled(pending)
    }

    pub fn settled(&self, pending: Pending) -> Result<Value, String> {
        self.quiet();
        pending.settle()
    }

    pub fn state(&self) -> Value {
        self.app.describe()["state"].clone()
    }

    pub fn summary(&self) -> String {
        self.app.describe()["summary"].as_str().unwrap_or_default().to_string()
    }

    pub fn notice(&self) -> String {
        self.app.ui(|ui| ui.get_notice().to_string())
    }

    /// The library on disk, read directly.
    pub fn library(&self) -> Library {
        Library::for_app(&self.paths)
    }

    /// Types `text` into the spec editor the way the window reports it.
    pub fn type_spec(&self, text: &str) {
        let text = text.to_string();
        self.app.ui(move |ui| {
            ui.set_spec_text(text.as_str().into());
            ui.invoke_spec_edited(text.as_str().into());
        });
        self.quiet();
    }

    pub fn press(&self, button: &str) {
        let id = button.to_string();
        self.app.ui(move |ui| ui.invoke_action(id.as_str().into()));
        self.quiet();
    }

    /// The same app started again over the same files, as after a restart.
    pub fn restart(self) -> Fixture {
        let Fixture { app, paths, .. } = self;
        drop(app);
        start_at(paths)
    }
}

/// A saved character and a saved game that casts it, through the intents.
pub(crate) fn seeded(f: &Fixture) {
    f.act("new_character", json!({ "spec": PIP })).unwrap();
    f.act("new_game", json!({ "spec": meadow_run("Meadow Run") })).unwrap();
}
