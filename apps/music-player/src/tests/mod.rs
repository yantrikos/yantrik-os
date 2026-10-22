//! Music, driven headless through `yantrik-testing`: the window, the jobs and the clock run when
//! each test says, against the stand-in mpv (`fake.rs`), a real Python process on a real socket.
//!
//! - `parity.rs`: the published actions and `describe`, pinned to what the app published before
//!   the framework, and the guide's row.
//! - `playback.rs`: the transport and the intents that play, as mpv reports them.
//! - `files.rs`: `play_file`, a launch, and what a sound file that is not one does.
//! - `agents.rs`: what an agent is told, and what it is not, in every answer and in `describe`.
//! - `library.rs`: the scan, the folders and the settings file.
//! - `order.rs`: answers per invocation, and commands that reach mpv in the order they were asked.
//! - `private_mode.rs`: Music keeps none of its keys in Private mode.
//! - `busy.rs`: the player's commands never wait behind the library's scans.
//! - `closing.rs`: the player stopped and reaped on a close, a logout, the display going, and
//!   when the app is killed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yantrik_app::control::AgentTokenScope;
use yantrik_app::{App, Paths};
use yantrik_testing::{Call, Pending, TestApp};

use crate::player::Rig;
use crate::Music;

mod agents;
mod busy;
mod closing;
pub(crate) mod fake;
mod files;
mod library;
mod order;
mod parity;
mod playback;
mod private_mode;

/// The token an agent's calls carry here.
pub(crate) const AGENT: &str = "tok-test-agent";

fn installed() -> &'static Mutex<HashMap<PathBuf, Rig>> {
    static INSTALLED: OnceLock<Mutex<HashMap<PathBuf, Rig>>> = OnceLock::new();
    INSTALLED.get_or_init(Default::default)
}

/// What the app started at `paths` plays through: what its test installed, or no player at all.
pub fn rig_for(paths: &Paths) -> Rig {
    let found = installed().lock().unwrap_or_else(|p| p.into_inner()).get(paths.config_file()).cloned();
    found.unwrap_or(Rig { program: "/nonexistent/no-mpv-here".into(), runtime: None, music: None })
}

/// A directory of its own for one test.
pub(crate) fn scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("yantrik-music-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A library of 300 tracks in three albums, a symlink loop, and a file that is not audio.
pub(crate) fn library_in(dir: &Path) -> PathBuf {
    let music = dir.join("Music");
    for album in ["Alpha", "Beta", "Gamma"] {
        std::fs::create_dir_all(music.join(album)).unwrap();
        for n in 0..100 {
            std::fs::write(music.join(album).join(format!("{album} {n:03}.mp3")), b"").unwrap();
        }
    }
    std::os::unix::fs::symlink(&music, music.join("Alpha/loop")).unwrap();
    std::fs::write(music.join("cover.jpg"), b"").unwrap();
    music
}

/// The session's runtime directory, private, as the socket needs it.
pub(crate) fn runtime_in(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let runtime = dir.join("run");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    runtime
}

/// An app under test, the directory its files are in, and its library.
pub(crate) struct Fixture {
    pub app: TestApp<Music>,
    pub dir: PathBuf,
    pub music: PathBuf,
}

/// What a test starts the app with.
pub(crate) struct Setup {
    /// The stand-in mpv, or a program that is not there.
    pub player: bool,
    /// The settings file from before the framework, as the old app wrote it at the old place.
    pub kept: Option<Value>,
    pub args: Vec<std::ffi::OsString>,
}

impl Default for Setup {
    fn default() -> Setup {
        Setup { player: true, kept: None, args: Vec::new() }
    }
}

/// Started over the 300-track library, with the volume kept at 40 and the first scan landed;
/// `None` where there is no python3 for the stand-in.
pub(crate) fn open(tag: &str) -> Option<Fixture> {
    open_with(tag, Setup::default())
}

pub(crate) fn open_with(tag: &str, setup: Setup) -> Option<Fixture> {
    if !fake::python() {
        eprintln!("no python3 for the stand-in player; skipped");
        return None;
    }
    let dir = scratch(tag);
    let music = library_in(&dir);
    let program = if setup.player { fake::install(&dir) } else { dir.join("no-mpv-here") };
    let paths = Paths::temporary(Music::ID);
    // The file the app kept before the framework, where it kept it.
    let kept = setup.kept.unwrap_or_else(|| json!({ "folders": [music], "volume": 40.0 }));
    let former = paths.former_config_file("music.json").expect("Music's own file from before");
    std::fs::create_dir_all(former.parent().unwrap()).unwrap();
    std::fs::write(&former, serde_json::to_vec(&kept).unwrap()).unwrap();
    let rig = Rig { program: program.to_string_lossy().into_owned(), runtime: Some(runtime_in(&dir)), music: None };
    installed().lock().unwrap_or_else(|p| p.into_inner()).insert(paths.config_file().to_path_buf(), rig);
    let app = TestApp::<Music>::start_at(paths, setup.args);
    let f = Fixture { app, dir, music };
    f.quiet();
    Some(f)
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
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

    /// Runs jobs and takes what the player said until `done` holds, or six seconds pass.
    pub fn until(&self, what: &str, done: impl Fn(&Fixture) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            self.quiet();
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "{what}: not within six seconds");
            std::thread::sleep(Duration::from_millis(5));
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

    /// `describe`'s state as an agent reads it: inside the dispatch, with its token in scope.
    pub fn agent_state(&self) -> Value {
        self.app.model(|m| {
            let _agent = AgentTokenScope::enter(Some(AGENT.into()));
            crate::describe::view(m).state
        })
    }

    pub fn notice(&self) -> String {
        self.app.ui(|ui| ui.get_notice().to_string())
    }

    pub fn status(&self) -> String {
        self.app.ui(|ui| ui.get_status_text().to_string())
    }

    pub fn playing(&self) -> bool {
        self.app.ui(|ui| ui.get_is_playing())
    }

    /// The path the stand-in reported last, as the model holds it.
    pub fn playing_path(&self) -> Option<String> {
        self.app.model(|m| m.observed().path)
    }

    /// The running player's process id.
    pub fn pid(&self) -> Option<u32> {
        self.app.model(|m| m.player.pid())
    }

    pub fn track(&self, album: &str, n: u32) -> PathBuf {
        self.music.join(album).join(format!("{album} {n:03}.mp3"))
    }

    /// The library list's rows, as the window draws them.
    pub fn rows(&self) -> usize {
        use slint::Model as _;
        self.app.ui(|ui| ui.get_library_tracks().row_count())
    }

    /// The temporary directory's path as text, which no agent's answer may contain.
    pub fn dir_text(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }
}
