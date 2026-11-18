//! Studio, driven headless through `yantrik-testing`: the window, the jobs and the clock run when
//! each test says, and the backends are fakes in this process (the placeholder drawer, and a
//! ComfyUI and an OpenAI-compatible server on real sockets).
//!
//! - `parity.rs`: the published actions and `describe`, pinned to what the app published before the
//!   framework, and the guide's rows.
//! - `surface.rs`: every action's answer, from its own result; the pictures it makes and files.
//! - `refusals.rs`: calls that cannot work are refused in words a caller can act on, said twice.
//! - `backends.rs`: `set_backend` and the grade that follows it.
//! - `wire.rs`: a ComfyUI server and a hosted service, their protocols and their failures.
//! - `queue.rs`: the queue: what runs at once, cancelling, closing with work in flight.
//! - `files.rs`: one picture's record, the Trash, the folder as it changes, the agents' files.
//! - `window.rs`: what the window shows, and the strings behind it.
//! - `for_mind.rs`: a window drawn for a Mind hides sentences and pictures and takes no input.
//! - `private_mode.rs`: Studio keeps none of its keys in Private mode.
//! - `units.rs`: the plan, the sentences, the summary.
//! - `server.rs`: the fake ComfyUI and OpenAI-compatible server.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yantrik_app::effects::Effects;
use yantrik_app::intents::Origin;
use yantrik_testing::{Call, Pending, TestApp};

use crate::command::Command;
use crate::config::Config;
use crate::model::{Model, Places};
use crate::Studio;

mod backends;
mod fetching;
mod drafts;
mod files;
mod for_mind;
mod parity;
mod private_mode;
mod queue;
mod refusals;
mod rest;
pub(crate) mod server;
mod surface;
mod units;
mod window;
mod wire;
pub(crate) mod world;

pub(crate) use world::World;

/// The token an agent's calls carry here.
pub(crate) const AGENT: &str = "tok-test-agent";

/// An app under test, and the world it works in.
pub(crate) struct Fixture {
    pub app: TestApp<Studio>,
    pub world: World,
}

/// Started with this configuration (as the file would hold it), the surface graded.
pub(crate) fn started(name: &str, config_text: &str) -> Fixture {
    with_config(name, Config::parse(config_text).unwrap())
}

/// The state of a machine nobody has set Studio up on.
pub(crate) fn unconfigured(name: &str) -> Fixture {
    with_config(name, Config::unconfigured())
}

pub(crate) fn with_config(name: &str, config: Config) -> Fixture {
    let f = just_started(name, config, |_| {});
    f.until("the grade to be published", |f| f.app.model(|m| m.graded));
    f
}

/// Started, and not yet waited for: the first command after the surface is served has not run.
pub(crate) fn just_started(name: &str, config: Config, places: impl FnOnce(&mut Places)) -> Fixture {
    let mut world = world::make(name, config.clone());
    places(&mut world.places);
    world::register(&world.paths, config, world.places.clone());
    let app = TestApp::<Studio>::start_at(world.paths.clone(), Vec::new());
    Fixture { app, world }
}

/// A machine with no home folder to write a settings file in.
pub(crate) fn homeless(name: &str, config_text: &str) -> Fixture {
    let f = just_started(name, Config::parse(config_text).unwrap(), |places| places.settings = None);
    f.until("the grade to be published", |f| f.app.model(|m| m.graded));
    f
}

/// A model with no window and no framework around it, for the tests that apply one command at a
/// time and read the effects it asked for.
pub(crate) fn bare(name: &str, config_text: &str) -> (World, Model) {
    let config = Config::parse(config_text).unwrap();
    let world = world::make(name, config.clone());
    let listed = crate::gallery::read_all(&world.places.gallery);
    let mut m = Model::new(config, world.places.clone(), listed, false);
    m.graded = true;
    (world, m)
}

/// Applies one command to a bare model as the person at the window, and returns what it asked for.
pub(crate) fn apply(m: &mut Model, command: Command) -> Effects<Studio> {
    apply_as(m, command, Effects::<Studio>::recording())
}

/// The same as a Mind's call: it runs under an invocation, so what it settles is recorded.
pub(crate) fn apply_as_agent(m: &mut Model, command: Command) -> Effects<Studio> {
    apply_as(m, command, Effects::<Studio>::recording_from(Origin::Surface { agent: true }))
}

fn apply_as(m: &mut Model, command: Command, mut fx: Effects<Studio>) -> Effects<Studio> {
    crate::update::update(m, command, &mut fx).unwrap();
    fx
}

/// A fake backend, which every machine has.
pub(crate) fn fake(name: &str) -> Fixture {
    started(name, r#"{"backend":{"kind":"fake"}}"#)
}

/// A gallery row as the window holds it, in plain data: the window's own row carries an image,
/// which does not cross to the test's thread.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub name: String,
    pub prompt: String,
    pub meta: String,
    pub path: String,
    pub place: String,
    pub has_thumb: bool,
    pub thumb: (u32, u32),
    pub selected: bool,
}

impl Fixture {
    /// The gallery rows on screen, top first.
    pub fn rows(&self) -> Vec<Row> {
        use slint::Model as _;
        self.app.ui(|ui| {
            ui.get_shots()
                .iter()
                .map(|row| Row {
                    name: row.name.to_string(),
                    prompt: row.prompt.to_string(),
                    meta: row.meta.to_string(),
                    path: row.path.to_string(),
                    place: row.place.to_string(),
                    has_thumb: row.has_thumb,
                    thumb: (row.thumb.size().width, row.thumb.size().height),
                    selected: row.selected,
                })
                .collect()
        })
    }

    /// Applies what was posted, runs the waiting jobs and moves the clock a little.
    pub fn step(&self) {
        self.app.tick();
        self.app.run_jobs();
        self.app.advance(Duration::from_millis(20));
    }

    /// Steps until `done`, or fails after twenty seconds of real time.
    pub fn until(&self, what: &str, mut done: impl FnMut(&Fixture) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.step();
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
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

    /// Waits for the queue to empty the way a caller watching `describe` would, and returns the
    /// notice it left behind. An empty notice means nothing went wrong: once a job is out of the
    /// queue, the notice is the only place a failure has to go.
    pub fn settled(&self) -> String {
        self.until("the queue to empty", |f| f.app.model(|m| m.queue.is_empty()));
        self.notice()
    }

    pub fn notice(&self) -> String {
        self.app.model(|m| m.notice.clone())
    }

    /// The person's own call of an action, answered once its work has landed.
    pub fn act(&self, name: &str, args: Value) -> Result<Value, String> {
        let pending = self.app.act(name, args)?;
        self.settle(pending)
    }

    /// The person's own call, answered as soon as its command has landed, with no worker run: for
    /// the tests that look at the app between a call and the work it started.
    pub fn ask(&self, name: &str, args: Value) -> Result<Value, String> {
        self.app.act(name, args)?.settle()
    }

    /// An agent's call.
    pub fn agent(&self, name: &str, args: Value) -> Result<Value, String> {
        let pending = self.app.act_as(Call::new(name, args).as_agent(AGENT))?;
        self.settle(pending)
    }

    pub fn settle(&self, pending: Pending) -> Result<Value, String> {
        self.quiet();
        pending.settle()
    }

    pub fn describe(&self) -> Value {
        self.app.describe()
    }

    pub fn state(&self) -> Value {
        self.app.describe()["state"].clone()
    }

    pub fn summary(&self) -> String {
        self.app.describe()["summary"].as_str().unwrap_or_default().to_string()
    }

    pub fn gallery(&self) -> PathBuf {
        self.world.places.gallery.clone()
    }

    /// The paths in the gallery, newest first, as the state reports them.
    pub fn paths(&self) -> Vec<String> {
        self.state()["gallery"]["newest"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["path"].as_str().unwrap().to_string())
            .collect()
    }

    /// One picture, made by the fake backend, so the tests that are about something else start from
    /// a gallery that has one. Returns its filename.
    pub fn one_picture(&self, prompt: &str) -> String {
        self.act("generate", json!({ "prompt": prompt, "seed": 11, "width": 256, "height": 256, "steps": 4 }))
            .unwrap();
        assert_eq!(self.settled(), "", "the setup picture was not made");
        self.app.model(|m| m.gallery.first().expect("the picture is in the gallery").name.clone())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.world.dir);
    }
}
