//! Download Manager, driven headless through `yantrik-testing`: the window, the jobs and the clock
//! run when each test says, against a web server on the loopback (`server.rs`) and the real
//! engine, with the real worker threads writing real files.
//!
//! - `parity.rs`: the published actions and `describe`, pinned to what the app published before
//!   the framework, and the guide's rows.
//! - `flow.rs`: a download from the add to the file, the checksum, and every action's answer.
//! - `order.rs`: answers per invocation, and commands that reach the engine in the order asked.
//! - `agents.rs`: where an agent may have a file saved, and what it may retry.
//! - `closing.rs`: a download half written when the window closes, on a logout, and when the app
//!   is killed: nothing lost, nothing corrupt, and the person's list intact.
//! - `rest.rs`: a window at rest draws nothing and a look at an unchanged engine draws nothing.
//! - `private_mode.rs`: Download Manager keeps none of its keys in Private mode.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yantrik_app::{App, Paths};
use yantrik_testing::{Call, Pending, TestApp};

use crate::engine::Engine;
use crate::Downloads;

mod agents;
mod closing;
mod flow;
mod hardening;
mod order;
mod parity;
mod private_mode;
mod rest;
pub(crate) mod server;

use server::{File, Server};

/// The token an agent's calls carry here.
pub(crate) const AGENT: &str = "tok-test-agent";

/// What the app started at `paths` keeps its list in, and saves into when no folder is named.
pub fn engine_for(paths: &Paths) -> Engine {
    Engine::open_in(paths.data_dir().join("list"), paths.data_dir().join("Downloads"))
}

/// A home of the tests' own, for the rule that says where an agent may save. Set once, before the
/// first app starts, so no test reads the machine's.
pub(crate) fn home() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("yantrik-dl-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        let home = home.canonicalize().unwrap();
        std::env::set_var("HOME", &home);
        home
    })
}

/// A body of `len` bytes that is not the same byte twice in a row, so a file cut anywhere or
/// stitched wrongly differs from it.
pub(crate) fn body(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 7 + i / 251) % 256) as u8).collect()
}

pub(crate) fn sha256_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// An app under test, the server it downloads from, and where its files are.
pub(crate) struct Fixture {
    pub app: TestApp<Downloads>,
    pub paths: Paths,
    pub server: Server,
}

/// Started over a server that serves `files`, with the list empty.
pub(crate) fn open(files: Vec<(&str, File)>) -> Fixture {
    home();
    let paths = Paths::temporary(Downloads::ID);
    let server = Server::start(files);
    let app = TestApp::<Downloads>::start_at(paths.clone(), Vec::new());
    let f = Fixture { app, paths, server };
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

    /// Runs jobs, lets the looks at the engine fire, and waits for the workers, until `done`
    /// holds or ten seconds pass.
    pub fn until(&self, what: &str, done: impl Fn(&Fixture) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.quiet();
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "{what}: not within ten seconds");
            std::thread::sleep(Duration::from_millis(5));
            self.app.advance(crate::update::TICK);
        }
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

    pub fn banner(&self) -> String {
        self.app.ui(|ui| ui.get_error_text().to_string())
    }

    /// Where a download lands when no folder is named.
    pub fn default_dir(&self) -> PathBuf {
        self.paths.data_dir().join("Downloads")
    }

    /// The row `describe` lists for download `id`.
    pub fn row(&self, id: i64) -> Value {
        self.state()["downloads"].as_array().and_then(|rows| rows.iter().find(|r| r["id"] == json!(id)).cloned()).unwrap_or(Value::Null)
    }

    /// Adds `path` from the server and runs it to the end: the id.
    pub fn download(&self, path: &str) -> i64 {
        let answer = self.act("add", json!({ "url": self.server.url(path) })).unwrap();
        let id = answer["id"].as_i64().unwrap();
        // Finished, and hashed: the digest is recorded just after the status says completed.
        self.until("the download finishes", |f| f.row(id)["status"] == "completed" && f.row(id).get("sha256").is_some());
        id
    }

    /// The same app started again over the same files, as after a restart.
    pub fn restart(self) -> Fixture {
        let Fixture { app, paths, server } = self;
        drop(app);
        let app = TestApp::<Downloads>::start_at(paths.clone(), Vec::new());
        let f = Fixture { app, paths, server };
        f.quiet();
        f
    }
}
