//! Yantrik Music Player — standalone app binary.
//!
//! A library scanned from the folders you list, a queue, and playback through one headless mpv
//! per window, driven over its JSON IPC socket (`mpv.rs`). The screen draws what mpv reports —
//! position, length, paused, volume — and the control surface answers with the same reports, so
//! "playing" is never the app's own opinion.
//!
//! Threads: the window and everything in `Music` live on the UI thread. The library scan runs on
//! a worker and hands its result back with `slint::invoke_from_event_loop`; mpv's reader thread
//! does the same with every event. Nothing blocks the window for longer than one IPC round trip.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::control::{agent_is_calling, answer_later, Action, App, Param, View};
use yantrik_app_runtime::prelude::*;

mod library;
mod mpv;
mod queue;
#[cfg(test)]
mod tests;

use library::Track;
use queue::Queue;

slint::include_modules!();

/// The id this app's control surface publishes: `yos act music …`.
const SURFACE: &str = "music";

/// How long an action waits for mpv to say a file it was handed is playing.
const LOAD_WAIT: Duration = Duration::from_secs(4);

fn main() {
    init_tracing("yantrik-music-player");
    if std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref() == Ok("1")
        && matches!(std::env::var("SLINT_BACKEND").as_deref(), Ok("winit") | Err(_))
    {
        std::env::set_var("SLINT_BACKEND", "winit-software");
    }

    // `%f` from the launcher or Files, or a path typed after the command.
    let file = std::env::args_os().nth(1).map(PathBuf::from);

    // One window per app. A second launch hands its file to the running one.
    let Some(_instance) = instance::claim("music-player") else {
        if let Some(path) = file {
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            let client = yantrik_app_runtime::SyncRpcClient::for_service(
                &yantrik_app_runtime::control::service_id_for(SURFACE),
            )
            .with_timeout(Duration::from_secs(6));
            let request = json!({"action": "play_file", "args": {"path": path}});
            for _ in 0..20 {
                if let Ok(reply) = client.call("app.act", request.clone()) {
                    if reply["accepted"] != true {
                        eprintln!("Music did not play it: {reply}");
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            eprintln!("Music is already starting; open the file again in a moment.");
        }
        return;
    };

    let app = MusicPlayerApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    let state = wire(&app, Music::new(app.as_weak(), store_path(), "mpv"));
    publish(&app, &state);
    let _ = start_scan(&state, None);
    if let Some(path) = file {
        if let Err(e) = play_file(&state, &path) {
            tracing::warn!(path = %path.display(), "{e}");
        }
    }

    run_until_closed(&app, "yantrik-music-player");

    // mpv exits with the app: dropping the engine asks it to quit, then kills it.
    let engine = state.borrow_mut().engine.take();
    drop(engine);
}

// ── State ────────────────────────────────────────────────────────────

/// Whether there is a player, in the words `describe` uses.
#[derive(Debug, Clone, PartialEq)]
enum Player {
    NotStarted,
    Running,
    /// mpv could not be started; the sentence says why.
    Unavailable(String),
    /// mpv was running and went away.
    Stopped,
}

impl Player {
    fn word(&self) -> &'static str {
        match self {
            Player::NotStarted => "not started",
            Player::Running => "running",
            Player::Unavailable(_) => "unavailable",
            Player::Stopped => "stopped",
        }
    }
}

struct Music {
    ui: slint::Weak<MusicPlayerApp>,
    /// Where the folder list and the volume are kept.
    store: PathBuf,
    /// The program started as the player. `mpv`, or a stand-in in the tests.
    program: String,

    folders: Vec<String>,
    per_folder: Vec<usize>,
    library: Vec<Track>,
    filter: String,
    /// Indices into `library` that match `filter`, in the order the window lists them.
    shown: Vec<usize>,
    scanning: bool,
    /// A folder changed while a scan ran: scan again when it ends.
    rescan_after: bool,
    /// Callers of `rescan` waiting for its result, each with whether it is an agent's.
    scan_waiters: Vec<(mpsc::Sender<Value>, bool)>,

    queue: Queue,
    engine: Option<Arc<mpv::Engine>>,
    /// Bumped on every start, so an event from a player that has since gone is ignored.
    generation: u64,
    player: Player,
    /// The volume, 0–100: what mpv reported last, or what it will start with.
    volume: f64,
    muted: bool,

    status: String,
    /// What an agent reads in place of `status` when it names folders the agent may not see.
    agent_status: Option<String>,
    notice: String,
}

type State = Rc<RefCell<Music>>;

thread_local! {
    /// The UI thread's handle on the state, for closures that arrive by `invoke_from_event_loop`
    /// from the scan worker and mpv's reader thread: they must be `Send`, and `State` is not.
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Run `f` on the UI thread against the state, if the window is still there.
fn on_ui(f: impl FnOnce(&State) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        let state = STATE.with(|s| s.borrow().clone());
        if let Some(state) = state {
            f(&state);
        }
    });
}

impl Music {
    fn new(ui: slint::Weak<MusicPlayerApp>, store: PathBuf, program: &str) -> Music {
        let saved = Saved::load(&store);
        Music {
            ui,
            store,
            program: program.to_string(),
            per_folder: vec![0; saved.folders.len()],
            folders: saved.folders,
            library: Vec::new(),
            filter: String::new(),
            shown: Vec::new(),
            scanning: false,
            rescan_after: false,
            scan_waiters: Vec::new(),
            queue: Queue::default(),
            engine: None,
            generation: 0,
            player: Player::NotStarted,
            volume: saved.volume,
            muted: false,
            status: String::new(),
            agent_status: None,
            notice: String::new(),
        }
    }

    fn live_engine(&self) -> Option<Arc<mpv::Engine>> {
        self.engine.as_ref().filter(|e| e.is_alive()).cloned()
    }

    fn observed(&self) -> mpv::Observed {
        self.live_engine().map(|e| e.observed.snapshot()).unwrap_or_default()
    }

    fn shown_tracks(&self) -> Vec<Track> {
        self.shown.iter().filter_map(|&i| self.library.get(i).cloned()).collect()
    }

    fn refilter(&mut self) {
        let needle = self.filter.trim().to_lowercase();
        self.shown = self
            .library
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                needle.is_empty()
                    || t.title.to_lowercase().contains(&needle)
                    || t.album.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
    }
}

// ── What is kept between runs ────────────────────────────────────────

/// The folder list and the volume, in `~/.config/yantrik/music.json`. The library itself is
/// not kept: it is whatever the folders hold, and a scan on start reads it again.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    folders: Vec<String>,
    #[serde(default = "default_volume")]
    volume: f64,
}

fn default_volume() -> f64 {
    80.0
}

fn store_path() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    config.join("yantrik/music.json")
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

impl Saved {
    /// What was kept, or — on a first run — `~/Music` if there is one.
    fn load(path: &Path) -> Saved {
        if let Some(saved) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            return saved;
        }
        let music = home().join("Music");
        Saved {
            folders: if music.is_dir() { vec![music.to_string_lossy().into_owned()] } else { vec![] },
            volume: default_volume(),
        }
    }
}

fn save(m: &Music) -> Result<(), String> {
    let saved = Saved { folders: m.folders.clone(), volume: m.volume };
    let text = serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?;
    if let Some(dir) = m.store.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let tmp = m.store.with_extension("json.tmp");
    std::fs::write(&tmp, text)
        .and_then(|_| std::fs::rename(&tmp, &m.store))
        .map_err(|e| format!("could not save the folder list to {}: {e}", m.store.display()))
}

// ── Drawing ──────────────────────────────────────────────────────────

fn row(t: &Track, current: bool) -> MusicTrackData {
    MusicTrackData {
        title: t.title.as_str().into(),
        album: t.album.as_str().into(),
        path: t.path.as_str().into(),
        is_current: current,
    }
}

/// The library list, the queue and the folders: redrawn when they change, not on every tick.
fn draw_lists(s: &State) {
    let m = s.borrow();
    let Some(ui) = m.ui.upgrade() else { return };
    let playing_path = m.queue.current().map(|t| t.path.clone());
    let rows: Vec<MusicTrackData> = m
        .shown
        .iter()
        .filter_map(|&i| m.library.get(i))
        .map(|t| row(t, playing_path.as_deref() == Some(t.path.as_str())))
        .collect();
    ui.set_library_tracks(ModelRc::new(VecModel::from(rows)));
    ui.set_library_track_count(m.library.len() as i32);
    let current = m.queue.current_index();
    let queue: Vec<MusicTrackData> =
        m.queue.tracks().iter().enumerate().map(|(i, t)| row(t, Some(i) == current)).collect();
    ui.set_queue_tracks(ModelRc::new(VecModel::from(queue)));
    ui.set_queue_current_index(current.map(|i| i as i32).unwrap_or(-1));
    let folders: Vec<MusicScanFolderData> = m
        .folders
        .iter()
        .enumerate()
        .map(|(i, f)| MusicScanFolderData {
            path: f.as_str().into(),
            track_count: m.per_folder.get(i).copied().unwrap_or(0) as i32,
        })
        .collect();
    ui.set_scan_folders(ModelRc::new(VecModel::from(folders)));
    drop(m);
    draw_status(s);
}

/// The status line, the notice and the scanning flag: cheap, so a scan's progress redraws only
/// this and never rebuilds the library list.
fn draw_status(s: &State) {
    let m = s.borrow();
    let Some(ui) = m.ui.upgrade() else { return };
    ui.set_is_scanning(m.scanning);
    ui.set_status_text(m.status.as_str().into());
    ui.set_notice(m.notice.as_str().into());
}

/// The transport, from what mpv last reported.
fn draw_transport(s: &State) {
    let m = s.borrow();
    let Some(ui) = m.ui.upgrade() else { return };
    let o = m.observed();
    let loaded = o.path.is_some() && !o.idle && !o.gone;
    let (title, album) = match (loaded, m.queue.current()) {
        (true, Some(t)) if o.path.as_deref() == Some(t.path.as_str()) => (t.title.clone(), t.album.clone()),
        (true, _) => (title_of(o.path.as_deref().unwrap_or_default()), String::new()),
        (false, _) => (String::new(), String::new()),
    };
    ui.set_now_title(title.as_str().into());
    ui.set_now_album(album.into());
    ui.set_is_playing(o.playing());
    let (pos, dur) = if loaded { (o.time_pos, o.duration) } else { (None, None) };
    ui.set_time_current(library::clock(pos).into());
    ui.set_time_total(library::clock(dur).into());
    ui.set_progress(match (pos, dur) {
        (Some(p), Some(d)) if d > 0.0 => (p / d) as f32,
        _ => 0.0,
    });
    ui.set_volume((o.volume.unwrap_or(m.volume) / 100.0) as f32);
    ui.set_volume_muted(if m.live_engine().is_some() { o.muted } else { m.muted });
    let context = if loaded {
        vec![AgentContextItem {
            id: "track".into(),
            label: title.into(),
            detail: match (pos, dur) {
                (Some(p), Some(d)) => format!("{} of {}", library::clock(Some(p)), library::clock(Some(d))),
                _ => String::new(),
            }
            .into(),
            source: "file".into(),
        }]
    } else {
        Vec::new()
    };
    ui.set_agent_context(ModelRc::new(VecModel::from(context)));
}

fn draw(s: &State) {
    draw_lists(s);
    draw_transport(s);
}

fn title_of(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Contract point 4: failure is said twice — to the caller, and on screen in `notice`, which is
/// also `describe.notice`.
fn refuse(s: &State, message: impl Into<String>) -> String {
    let message = message.into();
    s.borrow_mut().notice = message.clone();
    draw_status(s);
    message
}

fn say(s: &State, status: impl Into<String>) {
    {
        let mut m = s.borrow_mut();
        m.status = status.into();
        m.agent_status = None;
    }
    draw_status(s);
}

// ── The player ───────────────────────────────────────────────────────

/// The running mpv, started now if there is none. A player that died is started again here,
/// which is what "restarts on the next play" means.
fn engine(s: &State) -> Result<Arc<mpv::Engine>, String> {
    if let Some(e) = s.borrow().live_engine() {
        return Ok(e);
    }
    let (program, volume, muted, generation) = {
        let mut m = s.borrow_mut();
        m.engine = None;
        m.generation += 1;
        (m.program.clone(), m.volume, m.muted, m.generation)
    };
    let socket = mpv::socket_path().map_err(|e| {
        s.borrow_mut().player = Player::Unavailable(e.to_string());
        refuse(s, e.to_string())
    })?;
    // mpv reports time-pos many times a second; the window needs it about twice a second.
    let last_tick = Mutex::new(f64::NEG_INFINITY);
    let forward = move |event: mpv::Event| {
        if let mpv::Event::Property(name, v) = &event {
            if name == "time-pos" {
                let t = v.as_f64().unwrap_or(0.0);
                let mut last = last_tick.lock().unwrap();
                if (t - *last).abs() < 0.5 {
                    return;
                }
                *last = t;
            }
        }
        on_ui(move |s| on_event(s, generation, event));
    };
    match mpv::Engine::start(&program, socket, volume, forward) {
        Ok(e) => {
            let e = Arc::new(e);
            if muted {
                let _ = e.client.set("mute", json!(true));
            }
            let mut m = s.borrow_mut();
            m.engine = Some(e.clone());
            m.player = Player::Running;
            if m.notice.contains("mpv") {
                m.notice.clear();
            }
            Ok(e)
        }
        Err(e) => {
            s.borrow_mut().player = Player::Unavailable(e.to_string());
            Err(refuse(s, e.to_string()))
        }
    }
}

fn on_event(s: &State, generation: u64, event: mpv::Event) {
    if s.borrow().generation != generation {
        return;
    }
    match event {
        mpv::Event::Property(name, v) => {
            if name == "volume" {
                if let Some(v) = v.as_f64() {
                    s.borrow_mut().volume = v;
                }
            }
            if name == "mute" {
                s.borrow_mut().muted = v.as_bool().unwrap_or(false);
            }
            draw_transport(s);
        }
        mpv::Event::FileLoaded => draw(s),
        mpv::Event::EndFile { reason, error } => match reason.as_str() {
            // Auto-advance: the end of a track plays the next one.
            "eof" => {
                let next = s.borrow_mut().queue.next().cloned();
                match next {
                    Some(t) => {
                        let _ = load(s, &t);
                    }
                    None => say(s, "End of the queue."),
                }
                draw(s);
            }
            "error" => {
                let title = s.borrow().queue.current().map(|t| t.title.clone()).unwrap_or_default();
                let why = error.unwrap_or_else(|| "mpv could not read it".into());
                refuse(s, format!("Could not play \u{201c}{title}\u{201d}: {why}."));
                draw_transport(s);
            }
            // The "sound file" was a playlist and mpv is moving on to what it lists. mpv is told
            // not to (`--access-references=no`); if it does anyway, it is stopped here and the
            // file is refused rather than followed.
            "redirect" => {
                if let Some(e) = s.borrow().live_engine() {
                    let _ = e.client.command(&[json!("stop")]);
                }
                let title = s.borrow().queue.current().map(|t| t.title.clone()).unwrap_or_default();
                refuse(s, format!("Could not play \u{201c}{title}\u{201d}: {}.", mpv::REDIRECTED));
                draw_transport(s);
            }
            _ => draw_transport(s),
        },
        mpv::Event::Gone => {
            {
                let mut m = s.borrow_mut();
                m.engine = None;
                m.player = Player::Stopped;
            }
            refuse(s, "The player (mpv) stopped. Press play to start it again.");
            draw_transport(s);
        }
    }
}

/// Hand `track` to mpv and unpause. Returns once mpv has accepted the command, not once it plays.
fn load(s: &State, track: &Track) -> Result<Arc<mpv::Engine>, String> {
    let e = engine(s)?;
    e.client.load(&track.path).map_err(|why| refuse(s, format!("Could not play \u{201c}{}\u{201d}: {why}.", track.title)))?;
    let _ = e.client.set("pause", json!(false));
    {
        let mut m = s.borrow_mut();
        if m.notice.starts_with("Could not play") || m.notice.starts_with("Nothing") {
            m.notice.clear();
        }
        m.status = format!("Playing \u{201c}{}\u{201d}", track.title);
        m.agent_status = None;
    }
    draw(s);
    Ok(e)
}

/// Start the library at `at` (an index into what the window lists) with the list as the queue.
fn play_shown(s: &State, at: usize) -> Result<(Arc<mpv::Engine>, Track), String> {
    let track = {
        let mut m = s.borrow_mut();
        let tracks = m.shown_tracks();
        m.queue.start(tracks, at).cloned()
    };
    let track = track.ok_or_else(|| refuse(s, format!("There is no track {} in the library list.", at + 1)))?;
    Ok((load(s, &track)?, track))
}

/// Play or resume. `None` when it resumed rather than loading a file.
fn play(s: &State) -> Result<(Arc<mpv::Engine>, Option<Track>), String> {
    let live = s.borrow().live_engine();
    if let Some(e) = live {
        let o = e.observed.snapshot();
        if o.path.is_some() && !o.idle {
            e.client.set("pause", json!(false)).map_err(|why| refuse(s, format!("Could not resume: {why}.")))?;
            return Ok((e, None));
        }
    }
    let current = s.borrow().queue.current().cloned();
    if let Some(t) = current {
        return Ok((load(s, &t)?, Some(t)));
    }
    if s.borrow().shown.is_empty() {
        return Err(refuse(
            s,
            "Nothing to play: the library is empty. Add a folder that holds music under Folders.",
        ));
    }
    let (e, t) = play_shown(s, 0)?;
    Ok((e, Some(t)))
}

fn pause(s: &State) -> Result<Arc<mpv::Engine>, String> {
    let e = playing_engine(s, "pause")?;
    e.client.set("pause", json!(true)).map_err(|why| refuse(s, format!("Could not pause: {why}.")))?;
    Ok(e)
}

/// The engine, when a file is loaded in it; a refusal naming `what` otherwise.
fn playing_engine(s: &State, what: &str) -> Result<Arc<mpv::Engine>, String> {
    let e = s.borrow().live_engine();
    match e {
        Some(e) if e.observed.snapshot().path.is_some() && !e.observed.snapshot().idle => Ok(e),
        _ => Err(refuse(s, format!("Nothing is playing, so there is nothing to {what}."))),
    }
}

fn next(s: &State) -> Result<(Arc<mpv::Engine>, Track), String> {
    let t = s.borrow_mut().queue.next().cloned();
    let t = t.ok_or_else(|| {
        if s.borrow().queue.is_empty() {
            refuse(s, "The queue is empty: pick a track in the library first.")
        } else {
            refuse(s, "This is the last track in the queue.")
        }
    })?;
    Ok((load(s, &t)?, t))
}

/// Back to the start of the track if it has played for more than three seconds, else the one
/// before it.
fn previous(s: &State) -> Result<(Arc<mpv::Engine>, Option<Track>), String> {
    let live = s.borrow().live_engine();
    if let Some(e) = live {
        let o = e.observed.snapshot();
        if o.path.is_some() && o.time_pos.unwrap_or(0.0) > 3.0 {
            e.client.command(&[json!("seek"), json!(0), json!("absolute")]).map_err(|w| refuse(s, w))?;
            return Ok((e, None));
        }
    }
    let t = s.borrow_mut().queue.previous().cloned();
    let empty = s.borrow().queue.is_empty();
    match t {
        Some(t) => Ok((load(s, &t)?, Some(t))),
        None if empty => Err(refuse(s, "The queue is empty: pick a track in the library first.")),
        None => {
            let e = playing_engine(s, "go back to")?;
            e.client.command(&[json!("seek"), json!(0), json!("absolute")]).map_err(|w| refuse(s, w))?;
            Ok((e, None))
        }
    }
}

fn seek_to(s: &State, seconds: f64) -> Result<Arc<mpv::Engine>, String> {
    let e = playing_engine(s, "seek in")?;
    e.client
        .command(&[json!("seek"), json!(seconds.max(0.0)), json!("absolute")])
        .map_err(|why| refuse(s, format!("Could not seek: {why}.")))?;
    Ok(e)
}

fn set_volume(s: &State, percent: f64) -> Result<Option<Arc<mpv::Engine>>, String> {
    let percent = percent.clamp(0.0, 100.0);
    s.borrow_mut().volume = percent;
    let e = s.borrow().live_engine();
    if let Some(e) = &e {
        e.client.set("volume", json!(percent)).map_err(|why| refuse(s, format!("Could not set the volume: {why}.")))?;
    }
    let saved = save(&s.borrow());
    if let Err(why) = saved {
        tracing::warn!("{why}");
    }
    draw_transport(s);
    Ok(e)
}

/// An audio file on disk, by its absolute path — never anything mpv would read as a URL, a
/// playlist or an option.
fn audio_file(path: &Path) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(path).map_err(|e| format!("There is no file at {}: {e}.", path.display()))?;
    if !path.is_file() {
        return Err(format!("{} is not a file.", path.display()));
    }
    if !library::is_audio(&path) {
        return Err(format!(
            "{} is not a sound file Music plays (mp3, flac, ogg, opus, wav, m4a).",
            path.display()
        ));
    }
    Ok(path)
}

/// Make `path` the whole queue and play it.
fn play_file(s: &State, path: &Path) -> Result<(Arc<mpv::Engine>, Track), String> {
    let path = audio_file(path).map_err(|why| refuse(s, why))?;
    let track = library::track_from(&path);
    s.borrow_mut().queue.start(vec![track.clone()], 0);
    Ok((load(s, &track)?, track))
}

// ── The scan ─────────────────────────────────────────────────────────

/// Scan every folder on a worker thread. The window stays live; the result comes back through
/// `invoke_from_event_loop`. `waiter` gets the summary when it is done, with the paths an agent
/// may not read hidden if it is an agent's.
fn start_scan(s: &State, waiter: Option<(mpsc::Sender<Value>, bool)>) -> Result<(), String> {
    let folders = {
        let mut m = s.borrow_mut();
        if let Some(w) = waiter {
            m.scan_waiters.push(w);
        }
        if m.scanning {
            m.rescan_after = true;
            return Ok(());
        }
        m.scanning = true;
        m.status = if m.folders.is_empty() {
            String::new()
        } else {
            format!("Scanning {}\u{2026}", library::count(m.folders.len(), "folder", "folders"))
        };
        m.agent_status = None;
        m.folders.clone()
    };
    draw_lists(s);
    let spawned = std::thread::Builder::new().name("music-scan".into()).spawn(move || {
        let mut progress = |n: usize| {
            on_ui(move |s| say(s, format!("Scanning\u{2026} {} so far", library::count(n, "track", "tracks"))));
        };
        let found = library::scan_all(&folders, &mut progress);
        on_ui(move |s| finish_scan(s, folders, found));
    });
    if let Err(e) = spawned {
        s.borrow_mut().scanning = false;
        return Err(refuse(s, format!("Could not start the scan: {e}.")));
    }
    Ok(())
}

fn finish_scan(s: &State, scanned: Vec<String>, found: library::Library) {
    let (again, waiters, (person, agent)) = {
        let mut m = s.borrow_mut();
        m.scanning = false;
        // Counts belong to the folders that were scanned; a folder added meanwhile reads 0
        // until the scan that follows.
        m.per_folder = m
            .folders
            .iter()
            .map(|f| scanned.iter().position(|g| g == f).map(|i| found.per_folder[i]).unwrap_or(0))
            .collect();
        let said = |agent: bool| {
            if scanned.is_empty() {
                "No library folders. Add one under Folders.".to_string()
            } else {
                library::summary(&found, scanned.len(), |f| shown_text(f, agent))
            }
        };
        m.status = said(false);
        m.agent_status = Some(said(true)).filter(|a| *a != m.status);
        let answer = |agent: bool| {
            json!({
                "library_size": found.tracks.len(),
                "folders": scanned.iter().zip(&found.per_folder)
                    .map(|(f, n)| json!({"path": shown_path(Some(f), agent), "tracks": n})).collect::<Vec<_>>(),
                "unreadable": found.unreadable.iter()
                    .map(|(f, why)| json!({"path": shown_path(Some(f), agent), "why": why})).collect::<Vec<_>>(),
                "summary": said(agent),
            })
        };
        let (person, agent) = (answer(false), answer(true));
        m.library = found.tracks;
        m.refilter();
        let again = std::mem::take(&mut m.rescan_after);
        let waiters = if again { Vec::new() } else { std::mem::take(&mut m.scan_waiters) };
        (again, waiters, (person, agent))
    };
    for (w, is_agent) in waiters {
        let _ = w.send(if is_agent { agent.clone() } else { person.clone() });
    }
    draw(s);
    if again {
        let _ = start_scan(s, None);
    }
}

fn expanded(raw: &str) -> PathBuf {
    match raw.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if raw == "~" => home(),
        None => PathBuf::from(raw),
    }
}

fn add_folder(s: &State, raw: &str) -> Result<(), String> {
    let raw = raw.trim();
    let path = expanded(raw);
    if !path.is_absolute() {
        return Err(refuse(s, format!("Give the folder's full path, starting with / or ~/ (not \u{201c}{raw}\u{201d}).")));
    }
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    let text = path.to_string_lossy().into_owned();
    if s.borrow().folders.contains(&text) {
        return Err(refuse(s, format!("{text} is already in the list.")));
    }
    if !path.is_dir() {
        return Err(refuse(s, format!("Not a folder: {text}")));
    }
    {
        let mut m = s.borrow_mut();
        m.folders.push(text);
        m.per_folder.push(0);
        m.notice.clear();
    }
    if let Err(why) = save(&s.borrow()) {
        refuse(s, why);
    }
    start_scan(s, None)
}

fn remove_folder(s: &State, idx: usize) -> Result<(), String> {
    {
        let mut m = s.borrow_mut();
        if idx >= m.folders.len() {
            drop(m);
            return Err(refuse(s, "That folder is not in the list."));
        }
        m.folders.remove(idx);
        if idx < m.per_folder.len() {
            m.per_folder.remove(idx);
        }
    }
    if let Err(why) = save(&s.borrow()) {
        refuse(s, why);
    }
    start_scan(s, None)
}

// ── The window's controls ────────────────────────────────────────────

fn wire(app: &MusicPlayerApp, music: Music) -> State {
    let state: State = Rc::new(RefCell::new(music));
    STATE.with(|s| *s.borrow_mut() = Some(state.clone()));
    {
        let mut m = state.borrow_mut();
        m.refilter();
    }
    app.set_browse_filter(SharedString::new());
    app.set_show_scan_folders_dialog(false);
    draw(&state);

    // Every handler below does what its control says or puts the reason in `notice`.
    let with = |f: fn(&State)| {
        let s = state.clone();
        move || f(&s)
    };
    app.on_play_pause(with(|s| {
        let result = if s.borrow().observed().playing() { pause(s).map(|_| ()) } else { play(s).map(|_| ()) };
        if result.is_ok() {
            draw_transport(s);
        }
    }));
    app.on_stop(with(|s| {
        if let Ok(e) = playing_engine(s, "stop") {
            match e.client.command(&[json!("stop")]) {
                Ok(_) => say(s, "Stopped."),
                Err(why) => {
                    refuse(s, format!("Could not stop: {why}."));
                }
            }
        }
    }));
    app.on_next_track(with(|s| {
        let _ = next(s);
    }));
    app.on_prev_track(with(|s| {
        let _ = previous(s);
    }));
    {
        let s = state.clone();
        app.on_seek(move |fraction| {
            let duration = s.borrow().observed().duration;
            match duration {
                Some(d) => {
                    let _ = seek_to(&s, d * fraction as f64);
                }
                None => {
                    refuse(&s, "Nothing is playing, so there is nothing to seek in.");
                }
            }
        });
    }
    {
        let s = state.clone();
        app.on_volume_changed(move |v| {
            let _ = set_volume(&s, v as f64 * 100.0);
        });
    }
    app.on_toggle_mute(with(|s| {
        let e = s.borrow().live_engine();
        let muted = !s.borrow().muted;
        match e {
            Some(e) => {
                if let Err(why) = e.client.set("mute", json!(muted)) {
                    refuse(s, format!("Could not mute: {why}."));
                }
            }
            None => {
                s.borrow_mut().muted = muted;
                draw_transport(s);
            }
        }
    }));
    {
        let s = state.clone();
        app.on_play_track_index(move |i| {
            if i >= 0 {
                let _ = play_shown(&s, i as usize);
            }
        });
    }
    {
        let s = state.clone();
        app.on_queue_play_index(move |i| {
            let t = if i >= 0 { s.borrow_mut().queue.jump(i as usize).cloned() } else { None };
            match t {
                Some(t) => {
                    let _ = load(&s, &t);
                }
                None => {
                    refuse(&s, "That track is no longer in the queue.");
                }
            }
        });
    }
    {
        let s = state.clone();
        app.on_browse_filter_changed(move |text| {
            {
                let mut m = s.borrow_mut();
                m.filter = text.to_string();
                m.refilter();
            }
            draw_lists(&s);
        });
    }
    {
        let s = state.clone();
        app.on_scan_add_folder(move |path| {
            let _ = add_folder(&s, &path);
        });
    }
    {
        let s = state.clone();
        app.on_scan_remove_folder(move |i| {
            let _ = remove_folder(&s, i.max(0) as usize);
        });
    }
    app.on_scan_rescan(with(|s| {
        let _ = start_scan(s, None);
    }));
    app.on_dismiss_notice(with(|s| {
        s.borrow_mut().notice.clear();
        draw_status(s);
    }));
    state
}

// ── The control surface ──────────────────────────────────────────────

/// What mpv reports now, asked fresh over the socket rather than read from the last event.
/// `agent`: whether the caller is an agent, asked on the UI thread (see [`shown_path`]).
fn report(e: &mpv::Engine, agent: bool) -> Value {
    let get = |p: &str| e.client.get(p).unwrap_or(Value::Null);
    let paused = get("pause").as_bool().unwrap_or(false);
    let idle = get("idle-active").as_bool().unwrap_or(true);
    let path = get("path");
    json!({
        "playing": !idle && !path.is_null() && !paused,
        "paused": paused,
        "path": shown_path(path.as_str(), agent),
        "position": get("time-pos"),
        "duration": get("duration"),
        "volume": get("volume"),
        "muted": get("mute").as_bool().unwrap_or(false),
        "reported_by": "mpv",
    })
}

/// A path as this caller may see it: an agent is not told about a file outside the person's
/// home, or in its protected places (#443), even one the person is playing.
///
/// `agent` is [`agent_is_calling`], asked by the handler on the UI thread. It cannot be asked
/// here: an answer left to `answer_later` is built on the RPC thread, where neither the caller
/// nor the agent token is in scope, so every caller would look like the person.
fn shown_path(path: Option<&str>, agent: bool) -> Value {
    match path {
        None => Value::Null,
        Some(p) => json!(shown_text(p, agent)),
    }
}

fn shown_text(path: &str, agent: bool) -> String {
    if agent && yantrik_ipc_contracts::home_paths::may_read_file(path, &home()).is_err() {
        "(hidden)".to_string()
    } else {
        path.to_string()
    }
}

/// Answer once mpv has loaded `track`, with what it reported; or say what it said instead.
///
/// Waited for off the UI thread (`answer_later`), so the window keeps drawing and mpv's events
/// keep arriving while the caller waits. Outside a dispatch (the tests) it waits inline.
fn answer_loaded(e: Arc<mpv::Engine>, track: Track, before: Mark) -> Result<Value, String> {
    // Counted from the engine that is playing it: a player started for this load starts at 0.
    let before = match &before.engine {
        Some(old) if Arc::ptr_eq(old, &e) => (before.loads, before.errors),
        _ => (0, 0),
    };
    // Asked here, on the UI thread: `work` may run where the caller is no longer known.
    let agent = agent_is_calling();
    let work = move || {
        let loaded = |o: &mpv::Observed| o.loads > before.0 && o.path.as_deref() == Some(track.path.as_str());
        let (_, o) = e.observed.wait_for(LOAD_WAIT, |o| o.gone || o.errors > before.1 || loaded(o));
        if !o.gone && o.errors == before.1 && loaded(&o) {
            let mut answer = report(&e, agent);
            answer["track"] = json!(track.title);
            return Ok(answer);
        }
        let why = match (&o.last_error, &o.path) {
            (Some(error), _) if o.errors > before.1 => error.clone(),
            _ if o.gone => "the player (mpv) stopped".into(),
            // mpv loaded something, but not this file: say what it is playing instead.
            (_, Some(other)) if o.loads > before.0 => {
                format!("mpv is playing {} instead", shown_text(other, agent))
            }
            _ => "mpv did not report the file as loaded".into(),
        };
        Err(format!("mpv did not play \u{201c}{}\u{201d}: {why}.", track.title))
    };
    answer_later(work).map(|()| Value::Null).or_else(|work| work())
}

/// Where the running player's counts stood before an action loaded something.
struct Mark {
    engine: Option<Arc<mpv::Engine>>,
    loads: u64,
    errors: u64,
}

fn mark(s: &State) -> Mark {
    let engine = s.borrow().live_engine();
    let o = engine.as_ref().map(|e| e.observed.snapshot()).unwrap_or_default();
    Mark { engine, loads: o.loads, errors: o.errors }
}

/// One published action and the code that does it.
type Handler = Box<dyn Fn(&Value) -> Result<Value, String>>;

fn act(name: &str, sentence: &str) -> Action {
    Action::new(name, sentence).risk("standard")
}

/// Everything this window offers a mind. A list, so the tests can run a handler without a socket.
fn surface(s: &State) -> Vec<(Action, Handler)> {
    let mut out: Vec<(Action, Handler)> = Vec::new();
    let mut add = |spec: Action, run: fn(&State, &Value) -> Result<Value, String>| {
        let s = s.clone();
        out.push((spec, Box::new(move |args: &Value| run(&s, args)) as Handler));
    };

    add(
        act(
            "play",
            "Start or resume playback. With `index`, play that track of the library list (1 is \
             the first) and make the list the queue; without it, resume what is paused, or play \
             the queue's current track, or the library from the top. Answers with what mpv \
             reports once it is playing.",
        )
        .arg(Param::integer("index").describe("A track number in the library list, from 1, as `describe.library` numbers them.").optional()),
        |s, args| {
            if let Some(i) = args.get("index").and_then(Value::as_i64) {
                if i < 1 {
                    return Err(refuse(s, "`index` counts from 1."));
                }
                let before = mark(s);
                let (e, t) = play_shown(s, i as usize - 1)?;
                return answer_loaded(e, t, before);
            }
            let before = mark(s);
            match play(s)? {
                (e, Some(t)) => answer_loaded(e, t, before),
                (e, None) => {
                    let (ok, _) = e.observed.wait_for(Duration::from_secs(1), |o| !o.paused);
                    let answer = report(&e, agent_is_calling());
                    if !ok || answer["paused"] == true {
                        return Err(refuse(s, "mpv was asked to resume and still reports paused."));
                    }
                    Ok(answer)
                }
            }
        },
    );
    add(act("pause", "Pause what is playing. Answers with mpv's own report of pause and position."), |s, _| {
        let e = pause(s)?;
        let answer = report(&e, agent_is_calling());
        if answer["paused"] != true {
            return Err(refuse(s, "mpv was asked to pause and does not report paused."));
        }
        Ok(answer)
    });
    add(
        act("next", "Play the next track in the queue. Refused at the end of the queue."),
        |s, _| {
            let before = mark(s);
            let (e, t) = next(s)?;
            answer_loaded(e, t, before)
        },
    );
    add(
        act(
            "previous",
            "Go back: to the start of the track if it has played more than three seconds, \
             otherwise to the track before it in the queue.",
        ),
        |s, _| {
            let before = mark(s);
            match previous(s)? {
                (e, Some(t)) => answer_loaded(e, t, before),
                (e, None) => Ok(report(&e, agent_is_calling())),
            }
        },
    );
    add(
        act("seek", "Move to a point in the playing track. Answers with the position mpv reports after the move.")
            .arg(Param::number("seconds").describe("Seconds from the start of the track.")),
        |s, args| {
            let seconds = args.get("seconds").and_then(Value::as_f64).ok_or_else(|| refuse(s, "`seek` needs `seconds`."))?;
            if !seconds.is_finite() || seconds < 0.0 {
                return Err(refuse(s, "`seconds` is a point in the track, 0 or more."));
            }
            let duration = s.borrow().observed().duration;
            if let Some(d) = duration.filter(|d| seconds > *d) {
                return Err(refuse(s, format!("The track is {} long; {seconds} seconds is past its end.", library::clock(Some(d)))));
            }
            let e = seek_to(s, seconds)?;
            Ok(report(&e, agent_is_calling()))
        },
    );
    add(
        act("set_volume", "Set the volume, 0 to 100. Answers with the volume mpv reports.")
            .arg(Param::number("percent").describe("0 is silent, 100 is full volume.")),
        |s, args| {
            let p = args.get("percent").and_then(Value::as_f64).ok_or_else(|| refuse(s, "`set_volume` needs `percent`."))?;
            if !(0.0..=100.0).contains(&p) {
                return Err(refuse(s, format!("`percent` is from 0 to 100, not {p}.")));
            }
            match set_volume(s, p)? {
                Some(e) => Ok(report(&e, agent_is_calling())),
                // Nothing to ask: the volume is kept and mpv starts with it.
                None => Ok(json!({ "volume": p, "player": "not running; mpv starts at this volume", "reported_by": "music" })),
            }
        },
    );
    add(
        act(
            "play_file",
            "Play one sound file now, by its path, as the whole queue. An agent may name only \
             files in the person's home outside its protected places.",
        )
        .arg(Param::text("path").describe("The file's path: absolute, or starting with ~/. mp3, flac, ogg, opus, wav or m4a.")),
        |s, args| {
            let raw = args.get("path").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            if raw.is_empty() {
                return Err(refuse(s, "`play_file` needs `path`."));
            }
            let path = expanded(&raw);
            if agent_is_calling() {
                yantrik_ipc_contracts::home_paths::may_read_file(&path.to_string_lossy(), &home())
                    .map_err(|why| refuse(s, why))?;
            }
            let before = mark(s);
            let (e, t) = play_file(s, &path)?;
            answer_loaded(e, t, before)
        },
    );
    add(
        act(
            "rescan",
            "Scan the library folders again and answer with what was found: how many tracks in \
             each folder, and any folder that could not be read.",
        )
        .expected_seconds(30),
        |s, _| {
            let (tx, rx) = mpsc::channel();
            start_scan(s, Some((tx, agent_is_calling())))?;
            // The scan's result reaches the window through the event loop, which this handler is
            // holding; the wait therefore happens after it returns.
            let work = move || {
                rx.recv_timeout(Duration::from_secs(120))
                    .map_err(|_| "The scan did not finish within two minutes; read `describe` for its progress.".to_string())
            };
            answer_later(work).map(|()| Value::Null).or_else(|work| work())
        },
    );
    out
}

/// The state a mind reads first.
fn view(s: &State) -> View {
    let agent = agent_is_calling();
    let m = s.borrow();
    let o = m.observed();
    let loaded = o.path.is_some() && !o.idle && !o.gone;
    let current = m.queue.current().filter(|t| o.path.as_deref() == Some(t.path.as_str()));
    let title = current.map(|t| t.title.clone()).or_else(|| o.path.as_deref().map(title_of)).filter(|_| loaded);
    let library = library::count(m.library.len(), "track", "tracks");
    let mut summary = match &title {
        Some(t) if o.playing() => format!(
            "Music — playing \u{201c}{t}\u{201d} ({} of {})",
            library::clock(o.time_pos),
            library::clock(o.duration)
        ),
        Some(t) => format!("Music — paused on \u{201c}{t}\u{201d} at {}", library::clock(o.time_pos)),
        None => "Music — nothing playing".to_string(),
    };
    summary.push_str(&format!(", library {library}"));
    if m.scanning {
        summary.push_str(" · scanning");
    }
    if !m.notice.is_empty() {
        summary.push_str(&format!(" · {}", m.notice));
    }
    View::new(summary)
        .with("track", title)
        .with("path", if loaded { shown_path(o.path.as_deref(), agent) } else { Value::Null })
        .with("position", if loaded { json!(o.time_pos) } else { Value::Null })
        .with("duration", if loaded { json!(o.duration) } else { Value::Null })
        .with("playing", o.playing())
        .with("paused", loaded && o.paused)
        .with("volume", o.volume.unwrap_or(m.volume))
        .with("muted", if m.live_engine().is_some() { o.muted } else { m.muted })
        .with("queue_length", m.queue.len() as i64)
        .with("queue_position", m.queue.current_index().map(|i| i as i64 + 1))
        .with("library_size", m.library.len() as i64)
        .with("library", m.shown.iter().take(50).enumerate().filter_map(|(n, &i)| {
            m.library.get(i).map(|t| json!({"index": n + 1, "title": t.title, "folder": t.album}))
        }).collect::<Vec<_>>())
        .with("library_listed", m.shown.len() as i64)
        .with("folders", m.folders.iter().enumerate().map(|(i, f)| {
            json!({"path": shown_path(Some(f), agent), "tracks": m.per_folder.get(i).copied().unwrap_or(0)})
        }).collect::<Vec<_>>())
        .with("scanning", m.scanning)
        .with("player", m.player.word())
        .with("player_problem", match &m.player {
            Player::Unavailable(why) => Value::String(why.clone()),
            _ => Value::Null,
        })
        .with("status", m.agent_status.as_ref().filter(|_| agent).unwrap_or(&m.status).clone())
        .with("notice", m.notice.clone())
}

fn publish(_app: &MusicPlayerApp, s: &State) {
    let described = s.clone();
    let mut app = App::new(SURFACE).describe(move || view(&described));
    for (spec, run) in surface(s) {
        app = app.action(spec, run);
    }
    app.serve();
}
