//! Terminal, driven headless through `yantrik-testing` against real shells: the window, the jobs
//! and the clock run when each test says, and each tab is a `/bin/sh -i` on its own PTY.
//!
//! - `parity.rs`: the published actions and `describe`, pinned to what the app published before
//!   the framework, and the guide's rows.
//! - `tabs.rs`: the 32-tab cap (#683), new tabs and their folders, switching, restart.
//! - `surface.rs`: every action's answer, from its own shell; refusals said twice.
//! - `closing.rs`: the questions on a close and a quit, one at a time, and the end of the shells
//!   on a close, a quit and a logout, with nothing left running.
//! - `flood.rs`: a shell that prints without pause costs one command a frame and bounded memory.
//! - `rest.rs`: a window at rest draws nothing and arms nothing.
//! - `for_mind.rs`: a window drawn for a Mind hides the screen and takes no input.
//! - `private_mode.rs`: Terminal keeps none of its keys in Private mode, and a launch types
//!   nothing.

use std::path::PathBuf;
use std::sync::Once;
use std::time::{Duration, Instant};

use serde_json::Value;
use yantrik_app::{App, Paths};
use yantrik_testing::{Call, Pending, TestApp};

use crate::command::Command;
use crate::Terminal;

mod closing;
mod flood;
mod for_mind;
mod parity;
mod private_mode;
mod rest;
mod surface;
mod tabs;

/// The token an agent's calls carry here.
pub(crate) const AGENT: &str = "tok-test-agent";

/// The prompt every test shell shows.
pub(crate) const PROMPT: &str = "yantrik $";

fn environment() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Every shell a test starts is a plain sh with a prompt the tests can wait for.
        std::env::set_var("SHELL", "/bin/sh");
        std::env::set_var("PS1", format!("{PROMPT} "));
        // And they start in a folder of their own, so nothing a job writes lands in the tree.
        let home = std::env::temp_dir().join(format!("yantrik-terminal-tests-{}", unsafe { libc::getuid() }));
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_current_dir(home).unwrap();
    });
}

/// An app under test, and where its files are.
pub(crate) struct Fixture {
    pub app: TestApp<Terminal>,
    pub paths: Paths,
}

/// Started, with its first shell at a prompt.
pub(crate) fn open() -> Fixture {
    start_with(Vec::new())
}

pub(crate) fn start_with(args: Vec<std::ffi::OsString>) -> Fixture {
    environment();
    let paths = Paths::temporary(Terminal::ID);
    let f = Fixture { app: TestApp::<Terminal>::start_at(paths.clone(), args), paths };
    f.until("the first shell", |f| f.tabs() == 1 && f.screen().contains(PROMPT));
    f
}

impl Fixture {
    /// Applies what was posted, runs the waiting jobs and moves the clock a little.
    pub fn step(&self) {
        self.app.tick();
        self.app.run_jobs();
        self.app.advance(Duration::from_millis(20));
    }

    /// Steps until `done`, or fails after ten seconds of real time.
    pub fn until(&self, what: &str, mut done: impl FnMut(&Fixture) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.step();
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Runs jobs, and what they posted, until none is left.
    pub fn quiet(&self) {
        for _ in 0..64 {
            self.app.tick();
            if self.app.jobs_waiting() == 0 {
                return;
            }
            self.app.run_jobs();
        }
        panic!("jobs still waiting after 64 rounds");
    }

    pub fn tabs(&self) -> usize {
        self.app.model(|m| m.tabs.len())
    }

    /// The active shell's screen text.
    pub fn screen(&self) -> String {
        self.app.model(|m| m.session().map(|s| s.snapshot().text).unwrap_or_default())
    }

    pub fn notice(&self) -> String {
        self.app.ui(|ui| ui.get_notice().to_string())
    }

    /// The shells' pids, tab by tab.
    pub fn pids(&self) -> Vec<u32> {
        self.app.model(|m| m.tabs.iter().map(|t| t.session.pid()).collect())
    }

    pub fn active_pid(&self) -> u32 {
        self.app.model(|m| m.session().map(|s| s.pid()).unwrap_or(0))
    }

    /// Types `text` the way a key would, and waits for `seen` on the screen.
    pub fn type_until(&self, text: &str, seen: &str) {
        self.app.run(Command::Input(text.as_bytes().to_vec()));
        self.until(seen, |f| f.screen().contains(seen));
    }

    /// Starts `command` in the active shell and waits until it is running there.
    pub fn start_job(&self, command: &str) {
        self.app.run(Command::Input(format!("{command}\r").into_bytes()));
        self.until("a job running", |f| f.app.model(|m| m.session().is_some_and(|s| s.has_children())));
    }

    /// The person's own call of an action, answered once its work has landed.
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

    /// Opens `count` more tabs through the action, one after another.
    pub fn open_tabs(&self, count: usize) {
        for _ in 0..count {
            self.act("new_tab", serde_json::json!({})).unwrap();
        }
    }
}

/// Whether a process is still there. A zombie is not: it only waits to be reaped.
pub(crate) fn exists(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !stat.rsplit(')').next().unwrap_or("").trim_start().starts_with('Z'),
        Err(_) => false,
    }
}

/// The processes the shell with this pid has started and that are still its children.
pub(crate) fn children_of(pid: u32) -> Vec<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// A folder of the test's own, with spaces and a semicolon in its name.
pub(crate) fn folder(paths: &Paths, name: &str) -> PathBuf {
    let dir = paths.data_dir().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}
