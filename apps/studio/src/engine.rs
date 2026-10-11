//! The state Studio holds: which backend is configured, what is being made, and what has been made.
//!
//! Generation runs on worker threads, because a render takes seconds to minutes and the control
//! surface has a three-second budget on the thread that owns the window. The actions therefore
//! declare that they settle later, and the queue in `describe` is how a caller watches one.
//!
//! The gallery is a cache of files on disk. That is deliberate: the files are the real thing, and
//! a cache can be wrong in a way that a second copy of the truth cannot. Anything that changes the
//! folder — including a person copying a picture into it — is picked up by `poll`, and `refresh`
//! reads it again on demand.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use chrono::Local;
use serde_json::json;
use sha2::Digest;
use yantrik_app_runtime::control;

use crate::backend::{self, Backend, Cancel, Request, Shot};
use crate::config::{Backend as BackendConfig, Config, Facts, Kind, KINDS};
use crate::gallery::{self, Record, Sidecar};
use crate::trash;

/// How many pictures one `generate` will make. A hosted backend charges per picture, so an
/// unbounded `count` is a way to spend somebody's money from one sentence; four is enough to choose
/// from and small enough that asking for more is a decision rather than a typo.
pub const MAX_COUNT: u32 = 4;
pub const MIN_EDGE: u32 = 64;
pub const MAX_EDGE: u32 = 4096;
pub const MAX_STEPS: u32 = 150;
pub const MAX_UPSCALE: u32 = 8;

pub const DEFAULT_NEGATIVE: &str =
    "blurry, low quality, watermark, text, signature, deformed hands, extra limbs";
pub const DEFAULT_WIDTH: u32 = 1024;
pub const DEFAULT_HEIGHT: u32 = 1024;
pub const DEFAULT_STEPS: u32 = 30;
pub const DEFAULT_CFG: f64 = 7.0;

/// The places this app writes to, looked up once and held.
///
/// Held rather than asked for each time, for two reasons. A test can point all three at a temporary
/// directory and be certain nothing it does reaches the person's real Trash or their real settings
/// file — the environment variables those come from are process-wide, and tests in one binary run at
/// the same time. And `set_backend` has to know whether it was able to write the change down,
/// because a backend that cannot be persisted is a backend that will not survive a restart, which is
/// worth saying out loud.
#[derive(Clone, Debug)]
pub struct Places {
    pub gallery: PathBuf,
    pub trash: PathBuf,
    /// `None` when there is no home directory to put a config file in.
    pub settings: Option<PathBuf>,
}

impl Places {
    pub fn real(config: &Config) -> Places {
        Places {
            gallery: gallery::output_root(&config.output_folder),
            trash: trash::root(),
            settings: crate::config::config_path(),
        }
    }
}

/// What a `generate` asked for, before it becomes one or more requests to a backend.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub prompt: String,
    pub negative: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f64,
    /// `None` means "choose one", and each picture of a multi-picture ask then gets its own.
    pub seed: Option<u64>,
    pub count: u32,
}

impl Plan {
    /// A one-picture ask with the defaults filled in.
    pub fn new(prompt: impl Into<String>) -> Plan {
        Plan {
            prompt: prompt.into(),
            negative: DEFAULT_NEGATIVE.to_string(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            steps: DEFAULT_STEPS,
            cfg: DEFAULT_CFG,
            seed: None,
            count: 1,
        }
    }

    /// Check one ask and bring its numbers onto the grid, or say what is wrong with it in words a
    /// caller can act on.
    ///
    /// Clamping rather than refusing, for the numbers: a mind that asks for 1025 pixels wide wants a
    /// picture, not a lecture about the latent grid, and the sidecar records what was actually sent.
    /// Refusing outright is kept for the things that cannot be repaired by picking a nearby value —
    /// an empty prompt, or a count that would spend real money.
    pub fn checked(self) -> Result<Plan, String> {
        let prompt = self.prompt.trim();
        if prompt.is_empty() {
            return Err("a prompt is needed: `generate prompt=\"a lighthouse in fog\"`".to_string());
        }
        if prompt.chars().count() > 4000 {
            return Err(format!(
                "that prompt is {} characters, which is longer than any model here will read; the first few sentences are enough",
                prompt.chars().count()
            ));
        }
        if self.count == 0 {
            return Err("count=0 would make nothing; ask for at least one picture".to_string());
        }
        if self.count > MAX_COUNT {
            return Err(format!(
                "count={} is more than the {MAX_COUNT} this app will make in one ask, because a hosted backend charges for each one",
                self.count
            ));
        }
        Ok(Plan {
            prompt: prompt.to_string(),
            // An empty negative is left empty. Some people want no negative prompt, and
            // substituting this app's opinion of one would change their picture without saying so.
            negative: self.negative.trim().to_string(),
            width: clamp_edge(self.width, DEFAULT_WIDTH),
            height: clamp_edge(self.height, DEFAULT_HEIGHT),
            steps: self.steps.clamp(1, MAX_STEPS),
            cfg: self.cfg.clamp(0.0, 30.0),
            seed: self.seed,
            count: self.count,
        })
    }

    /// The request for the `index`th picture of this ask. Seeds run on from the one given, so asking
    /// for four with `seed=7` produces 7, 8, 9 and 10 rather than four of the same thing.
    fn request(&self, index: u32) -> Request {
        Request {
            prompt: self.prompt.clone(),
            negative: self.negative.clone(),
            width: self.width,
            height: self.height,
            steps: self.steps,
            cfg: self.cfg,
            seed: self
                .seed
                .map(|seed| seed.wrapping_add(index as u64))
                .unwrap_or_else(random_seed),
        }
    }
}

fn clamp_edge(value: u32, fallback: u32) -> u32 {
    if value == 0 {
        fallback
    } else {
        value.clamp(MIN_EDGE, MAX_EDGE)
    }
}

/// One entry in the queue.
#[derive(Clone, Debug)]
pub struct Job {
    pub id: i32,
    /// "generate", "variations" or "upscale".
    pub kind: String,
    /// What it is doing, in words a person would read.
    pub label: String,
    /// "pending", "running" or "cancelling".
    pub status: String,
    pub wanted: u32,
    pub made: u32,
    pub seconds: f64,
}

impl Job {
    fn json(&self) -> serde_json::Value {
        let mut row = serde_json::Map::new();
        row.insert("id".into(), json!(self.id));
        row.insert("kind".into(), json!(self.kind));
        row.insert("label".into(), json!(self.label));
        row.insert("status".into(), json!(self.status));
        if self.wanted > 1 {
            row.insert("progress".into(), json!(format!("{} of {}", self.made, self.wanted)));
        }
        if self.seconds >= 1.0 {
            row.insert("seconds".into(), json!(self.seconds.round()));
        }
        json!(row)
    }
}

struct State {
    config: Config,
    backend: Arc<dyn Backend>,
    places: Places,
    jobs: Vec<Job>,
    cancels: HashMap<i32, Cancel>,
    gallery: Vec<Record>,
    /// Two directory timestamps, so `poll` can tell whether anything moved without walking the folder.
    stamp: String,
    /// A fingerprint of the listing that `stamp` produced, so a re-read that found the same pictures
    /// does not announce itself as a change and no window redraws for nothing.
    content: String,
    revision: u64,
    /// The last thing worth saying that is not in the queue or the gallery: a failure, a
    /// cancellation, a deletion, a backend that cannot send anything yet.
    notice: String,
}

/// Everything `describe` needs, taken in one lock so the parts cannot disagree with each other.
#[derive(Clone)]
pub struct Snapshot {
    pub config: Config,
    pub facts: Facts,
    pub places: Places,
    pub jobs: Vec<Job>,
    pub gallery: Vec<Record>,
    pub notice: String,
}

impl Snapshot {
    /// One line, trouble first: a caller surveying every open window pays one line per app, and the
    /// reason to look is what belongs at the front.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.notice.is_empty() {
            parts.push(self.notice.clone());
        }
        let running = self.jobs.iter().filter(|job| job.status == "running").count();
        let waiting = self.jobs.len() - running;
        match (running, waiting) {
            (0, 0) => {}
            (1, 0) => parts.push("1 picture being made".to_string()),
            (n, 0) => parts.push(format!("{n} pictures being made")),
            (0, n) => parts.push(format!("{n} waiting")),
            (r, p) => parts.push(format!("{r} being made, {p} waiting")),
        }
        parts.push(self.backend_in_short());
        parts.push(match self.gallery.len() {
            0 => "nothing in the gallery yet".to_string(),
            1 => format!("1 picture in {}", display(&self.places.gallery)),
            n => format!("{n} pictures in {}", display(&self.places.gallery)),
        });
        format!("Studio — {}", parts.join("; "))
    }

    fn backend_in_short(&self) -> String {
        if !self.facts.configured {
            return "no backend configured, so pictures are placeholders".to_string();
        }
        match self.facts.kind {
            "fake" => "placeholder images, because no image model is configured".to_string(),
            "comfyui" if !self.facts.prompt_leaves => {
                format!("ComfyUI at {}", self.config.backend.base_url)
            }
            "comfyui" => format!("ComfyUI away from your network at {}", self.config.backend.base_url),
            _ => format!("a hosted service at {}", self.config.backend.base_url),
        }
    }

    /// The state object. Deliberately a glance: the newest few pictures with what made them, the
    /// queue, and where the files are. A caller that wants the rest can read the folder, which is
    /// the point of writing real files into it.
    pub fn state(&self) -> serde_json::Value {
        let running: Vec<_> =
            self.jobs.iter().filter(|job| job.status == "running").map(Job::json).collect();
        let waiting: Vec<_> =
            self.jobs.iter().filter(|job| job.status != "running").map(Job::json).collect();
        let mut state = serde_json::Map::new();
        state.insert("backend".into(), self.config.state(self.key_is_set()));
        state.insert("output_folder".into(), json!(display(&self.places.gallery)));
        state.insert(
            "gallery".into(),
            json!({
                "count": self.gallery.len(),
                "newest": self.gallery.iter().take(6).map(Record::json).collect::<Vec<_>>(),
            }),
        );
        // An empty queue is still shown. A caller has to be able to tell "nothing is running" from
        // "I could not read it".
        state.insert("queue".into(), json!({ "running": running, "pending": waiting }));
        if !self.notice.is_empty() {
            state.insert("notice".into(), json!(self.notice));
        }
        json!(state)
    }

    fn key_is_set(&self) -> bool {
        self.config.backend.api_key().is_ok()
    }
}

/// The folder's own answer to "did anything change", from two directory timestamps. A stat costs
/// microseconds, which is what makes it cheap enough to ask several times a second from the thread
/// that owns the window; walking a gallery and decoding thumbnails is not.
///
/// What this does not notice is a file being overwritten under the same name, which moves no
/// directory timestamp. Nothing this app writes does that — every shot gets a fresh name — and
/// `refresh` is there for the rest.
fn cheap_stamp(root: &Path) -> String {
    let today = gallery::day_folder(root, Local::now());
    format!("{}|{}", modified(root), modified(&today))
}

fn modified(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

fn content_stamp(rows: &[Record]) -> String {
    rows.iter()
        // Whether the record is there belongs in the stamp. A picture whose sidecar was deleted
        // has not moved and its timestamps have not changed, so a stamp of path and date alone
        // says "nothing changed" and the gallery goes on quoting a record that is gone.
        .map(|row| {
            let record = if row.has_sidecar { "" } else { "#no-record" };
            format!("{}@{}{record}", row.path.display(), row.created)
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Clone)]
pub struct Engine {
    state: Arc<Mutex<State>>,
    next_id: Arc<AtomicI32>,
    alive: Arc<AtomicBool>,
    /// One re-read at a time. A folder being written to steadily would otherwise have `poll`
    /// starting a thread every tick, each decoding the same twelve thumbnails.
    reading: Arc<AtomicBool>,
}

fn lock<T>(cell: &Mutex<T>) -> MutexGuard<'_, T> {
    // A worker that panicked while holding this lock would poison it, and every later `describe`
    // would then fail with somebody else's mistake. What is inside is a queue and a cache of files
    // that are still on disk, so carrying on is better than an app whose surface has stopped
    // answering.
    cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Engine {
    /// Read the configuration and the gallery, and be ready.
    pub fn new() -> Engine {
        let config = Config::load();
        let places = Places::real(&config);
        Engine::open(config, places)
    }

    /// The same, with the configuration and the places given. This is the seam the tests use.
    pub fn open(config: Config, places: Places) -> Engine {
        let engine = Engine {
            state: Arc::new(Mutex::new(State {
                backend: backend::build(&config),
                config,
                places,
                jobs: Vec::new(),
                cancels: HashMap::new(),
                gallery: Vec::new(),
                stamp: String::new(),
                content: String::new(),
                revision: 1,
                notice: String::new(),
            })),
            next_id: Arc::new(AtomicI32::new(1)),
            alive: Arc::new(AtomicBool::new(true)),
            reading: Arc::new(AtomicBool::new(false)),
        };
        // Read the gallery now rather than on the first `describe`, so a caller never catches the app
        // half-built and the first paint has something to show.
        engine.relist();
        engine
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.state);
        let key_is_set = state.config.backend.api_key().is_ok();
        Snapshot {
            facts: state.config.facts(key_is_set),
            config: state.config.clone(),
            places: state.places.clone(),
            jobs: state.jobs.clone(),
            gallery: state.gallery.clone(),
            notice: state.notice.clone(),
        }
    }

    /// How many times the state has moved.
    ///
    /// Its own call rather than a field of `Snapshot`, because the only reader is the redraw timer,
    /// which asks for this number four times a second and wants nothing else: a snapshot is a
    /// listing of the gallery with a decoded thumbnail on every row.
    pub fn revision(&self) -> u64 {
        lock(&self.state).revision
    }

    pub fn places(&self) -> Places {
        lock(&self.state).places.clone()
    }

    pub fn config(&self) -> Config {
        lock(&self.state).config.clone()
    }

    /// The grade `generate` and `variations` should carry with the backend that is configured now.
    pub fn grade(&self) -> &'static str {
        let state = lock(&self.state);
        // The grade does not depend on whether the key is exported — a prompt that cannot be sent
        // today can be sent tomorrow — so `facts` is asked with the cheapest answer here.
        state.config.facts(false).grade
    }

    pub fn shutdown(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let state = lock(&self.state);
        for cancel in state.cancels.values() {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Say one thing, in the window and in `describe` at the same time.
    ///
    /// Public because the window has its own things to say that are not the engine's: a person who
    /// presses "Where pictures are made" and is told the file to edit has been answered, and the
    /// same line reaches a mind that reads `describe` a moment later.
    pub fn say(&self, notice: impl Into<String>) {
        let mut state = lock(&self.state);
        state.notice = notice.into();
        state.revision += 1;
    }

    /// Re-read the gallery, on this thread. Called from workers and from `open`; the thread that owns
    /// the window only ever reaches it through `spawn_relist`.
    fn relist(&self) {
        let root = { lock(&self.state).places.gallery.clone() };
        // Stamped before the listing rather than after. A file that arrives while this is reading
        // then leaves the stamp older than the folder, and the next `poll` reads again; stamped
        // afterwards, that file would be missed until something else moved.
        let stamp = cheap_stamp(&root);
        let rows = gallery::listing(&root, gallery::LISTING_CAP, true);
        let content = content_stamp(&rows);
        let mut state = lock(&self.state);
        state.stamp = stamp;
        if content != state.content {
            state.content = content;
            state.gallery = rows;
            state.revision += 1;
        }
    }

    /// Re-read the gallery on a worker, at most one at a time.
    pub fn spawn_relist(&self) {
        if !self.alive.load(Ordering::SeqCst) || self.reading.swap(true, Ordering::SeqCst) {
            return;
        }
        let engine = self.clone();
        std::thread::spawn(move || {
            engine.relist();
            engine.reading.store(false, Ordering::SeqCst);
        });
    }

    /// Ask whether the output folder moved, and read it again if it did. Two directory stats when
    /// nothing happened, which is cheap enough to do from a timer on the thread that owns the window.
    pub fn poll(&self) {
        if !self.alive.load(Ordering::SeqCst) {
            return;
        }
        let (root, known) = {
            let state = lock(&self.state);
            (state.places.gallery.clone(), state.stamp.clone())
        };
        if known == cheap_stamp(&root) {
            return;
        }
        self.spawn_relist();
    }

    /// Drop the notice. Called when a person has seen the window, so a failure is reported once
    /// rather than leading every summary forever.
    pub fn clear_notice(&self) {
        let mut state = lock(&self.state);
        if !state.notice.is_empty() {
            state.notice = String::new();
            state.revision += 1;
        }
    }

    // ── the three things that make a picture ──

    /// Queue a generation and start it. Returns the job's identifier, which is what `cancel` takes.
    pub fn generate(&self, plan: Plan) -> Result<i32, String> {
        let plan = plan.checked()?;
        let label = match plan.count {
            1 => short_prompt(&plan.prompt),
            n => format!("{} pictures of {}", n, short_prompt(&plan.prompt)),
        };
        let id = self.enqueue("generate", &label, plan.count);
        self.spawn(move |engine| engine.run_plan(id, plan, String::new()));
        Ok(id)
    }

    /// Queue more pictures of one that already exists, from its own sidecar: the same sentence, the
    /// same size, the same step count, new seeds.
    pub fn variations(&self, of: &str, count: u32) -> Result<i32, String> {
        let path = self.resolve(of, false)?;
        let Some(sidecar) = Sidecar::read(&path) else {
            return Err(format!(
                "there is no record beside {}, so there is nothing to say what prompt made it. Generate from a sentence instead: `generate prompt=\"…\"`.",
                path.display()
            ));
        };
        if sidecar.prompt.trim().is_empty() {
            return Err(format!(
                "{} has a record but no prompt in it, so there is nothing to vary",
                path.display()
            ));
        }
        let count = count.clamp(1, MAX_COUNT);
        let plan = Plan {
            prompt: sidecar.prompt.clone(),
            negative: sidecar.negative,
            width: sidecar.width,
            height: sidecar.height,
            steps: sidecar.steps.unwrap_or(DEFAULT_STEPS),
            cfg: sidecar.cfg.unwrap_or(DEFAULT_CFG),
            seed: None,
            count,
        }
        .checked()?;
        let name = file_name(&path);
        let label = match count {
            1 => format!("a variation of {name}"),
            n => format!("{n} variations of {name}"),
        };
        let id = self.enqueue("variations", &label, count);
        self.spawn(move |engine| engine.run_plan(id, plan, format!("variation of {name}")));
        Ok(id)
    }

    /// Queue a resample of one picture, written into the gallery beside the rest with a record that
    /// says where it came from.
    pub fn upscale(&self, path: &str, factor: u32) -> Result<i32, String> {
        let source = self.resolve(path, false)?;
        let factor = if factor == 0 { 2 } else { factor.clamp(2, MAX_UPSCALE) };
        let name = file_name(&source);
        let id = self.enqueue("upscale", &format!("a {factor}x resample of {name}"), 1);
        self.spawn(move |engine| engine.run_upscale(id, source, factor));
        Ok(id)
    }

    fn enqueue(&self, kind: &str, label: &str, wanted: u32) -> i32 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut state = lock(&self.state);
        state.jobs.push(Job {
            id,
            kind: kind.to_string(),
            label: label.to_string(),
            status: "pending".to_string(),
            wanted,
            made: 0,
            seconds: 0.0,
        });
        state.cancels.insert(id, backend::new_cancel());
        // A fresh ask supersedes the last thing that went wrong, which is otherwise still leading
        // the summary while a new picture is being made.
        state.notice = String::new();
        state.revision += 1;
        id
    }

    fn cancel_flag(&self, id: i32) -> Option<Cancel> {
        lock(&self.state).cancels.get(&id).cloned()
    }

    fn spawn(&self, work: impl FnOnce(Engine) + Send + 'static) {
        if !self.alive.load(Ordering::SeqCst) {
            return;
        }
        let engine = self.clone();
        std::thread::spawn(move || work(engine));
    }

    fn mark(&self, id: i32, status: &str) {
        let mut state = lock(&self.state);
        if let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) {
            job.status = status.to_string();
        }
        state.revision += 1;
    }

    fn progress(&self, id: i32, made: u32, seconds: f64) {
        let mut state = lock(&self.state);
        if let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) {
            job.made = made;
            job.seconds = seconds;
        }
        state.revision += 1;
    }

    fn finish(&self, id: i32) {
        let mut state = lock(&self.state);
        state.jobs.retain(|job| job.id != id);
        state.cancels.remove(&id);
        state.revision += 1;
    }

    fn run_plan(self, id: i32, plan: Plan, made_from: String) {
        let Some(cancel) = self.cancel_flag(id) else { return self.finish(id) };
        let backend = { lock(&self.state).backend.clone() };
        self.mark(id, "running");
        let mut made = 0;
        let started = Instant::now();
        for index in 0..plan.count {
            if !self.alive.load(Ordering::SeqCst) || cancel.load(Ordering::SeqCst) {
                return self.abandon(id, made);
            }
            let request = plan.request(index);
            let attempt = Instant::now();
            match backend.generate(&request, &cancel) {
                Ok(shot) => {
                    let seconds = attempt.elapsed().as_secs_f64();
                    match self.save_as(&shot, &request, seconds, &made_from, backend.kind()) {
                        Ok(path) => {
                            tracing::info!("{} made in {seconds:.1}s by {}", path.display(), backend.kind())
                        }
                        Err(problem) => return self.fail(id, problem),
                    }
                }
                Err(problem) if problem == "cancelled" => return self.abandon(id, made),
                Err(problem) => {
                    // Which backend was asked is part of the answer: "could not be reached" means
                    // something different when it is a server on your own LAN and when it is a
                    // service that would have kept your prompt. Named by the backend that was
                    // actually asked rather than by the configuration beside it, so the record and
                    // the refusal cannot disagree about who made what.
                    return self.fail(
                        id,
                        format!("{} could not make the picture: {problem}", backend.kind()),
                    );
                }
            }
            made += 1;
            self.progress(id, made, started.elapsed().as_secs_f64());
        }
        self.finish(id);
    }

    fn run_upscale(self, id: i32, source: PathBuf, factor: u32) {
        let Some(cancel) = self.cancel_flag(id) else { return self.finish(id) };
        self.mark(id, "running");
        if cancel.load(Ordering::SeqCst) {
            return self.abandon(id, 0);
        }
        let started = Instant::now();
        let bytes = match gallery::resample(&source, factor, gallery::LONGEST_EDGE) {
            Ok(bytes) => bytes,
            Err(problem) => return self.fail(id, problem),
        };
        let (width, height) = gallery::png_size(&bytes);
        let shot = Shot {
            bytes,
            width: width.unwrap_or(0),
            height: height.unwrap_or(0),
            model: String::new(),
            sent: format!("{}x{}", width.unwrap_or(0), height.unwrap_or(0)),
        };
        // The prompt travels with the picture. An upscaled lighthouse is still a picture of a
        // lighthouse made from that sentence, and losing the sentence would be losing the part of the
        // record that matters.
        let original = Sidecar::read(&source);
        let request = Request {
            prompt: original.as_ref().map(|each| each.prompt.clone()).unwrap_or_default(),
            negative: String::new(),
            width: shot.width,
            height: shot.height,
            steps: 0,
            cfg: 0.0,
            seed: original.as_ref().map(|each| each.seed).unwrap_or(0),
        };
        let made_from = format!("upscale of {}", file_name(&source));
        // Nothing was sent to a model, and the record does not claim otherwise: `studio-resample`
        // goes in the field that names a backend everywhere else.
        if let Err(problem) =
            self.save_as(&shot, &request, started.elapsed().as_secs_f64(), &made_from, "studio-resample")
        {
            return self.fail(id, problem);
        }
        self.finish(id);
    }

    fn abandon(&self, id: i32, made: u32) {
        // The notice is set before the job leaves the queue, and that order is the point: a reader
        // who sees an empty queue has to see the reason in the same snapshot. The other order has a
        // window — small, and exactly where a watcher polling the queue lands — in which the job is
        // gone and the notice is not there yet, so a failure reads as a success.
        self.say(match made {
            0 => "Cancelled before anything was made.".to_string(),
            n => format!(
                "Cancelled. {n} had already been written to the gallery and have been left there; a generation cannot be resumed, so ask again if you want the rest."
            ),
        });
        self.finish(id);
    }

    fn fail(&self, id: i32, problem: String) {
        // The notice is the only place a failure can go that a caller will read: the job is out of the
        // queue by then, and the gallery never held it. Set first, for the reason `abandon` gives.
        self.say(problem);
        self.finish(id);
    }

    /// Write one shot into today's folder with its record, and put it at the front of the gallery.
    ///
    /// `backend_name` arrives from whoever made the pixels: a generation names the backend that was
    /// asked, and a resample says `studio-resample`, because nothing was asked and the record must
    /// not claim otherwise.
    fn save_as(
        &self,
        shot: &Shot,
        request: &Request,
        seconds: f64,
        made_from: &str,
        backend_name: &str,
    ) -> Result<PathBuf, String> {
        let root = { lock(&self.state).places.gallery.clone() };
        let now = Local::now();
        let folder = gallery::day_folder(&root, now);
        std::fs::create_dir_all(&folder).map_err(|e| {
            format!(
                "{} could not be created ({e}), so the picture has nowhere to go",
                folder.display()
            )
        })?;
        let path = gallery::unique_file(&folder, now, request.seed);

        // The picture is written before its record, and if the record cannot be written the picture
        // is removed again. A picture with no record beside it is indistinguishable in the gallery
        // from one somebody copied in, which loses the thing that makes this app worth having; a
        // record with no picture is worse, and cannot happen this way round.
        std::fs::write(&path, &shot.bytes)
            .map_err(|e| format!("{} could not be written ({e})", path.display()))?;
        let sidecar = Sidecar {
            prompt: request.prompt.clone(),
            negative: request.negative.clone(),
            seed: request.seed,
            backend: backend_name.to_string(),
            model: shot.model.clone(),
            seconds: (seconds * 100.0).round() / 100.0,
            width: shot.width,
            height: shot.height,
            sent: shot.sent.clone(),
            created: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
            made_from: made_from.to_string(),
            steps: (request.steps > 0).then_some(request.steps),
            cfg: (request.cfg > 0.0).then_some(request.cfg),
            made_by: "yantrik-studio".to_string(),
        };
        if let Err(problem) = sidecar.write(&path) {
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "{problem}; the picture was removed rather than left without its record"
            ));
        }

        // One row added rather than a full re-read: this is the newest picture in the folder, so it
        // belongs at the front, and decoding twelve thumbnails for every shot of a batch is waste.
        let row = gallery::record(&path, true);
        let mut state = lock(&self.state);
        state.gallery.retain(|each| each.path != path);
        state.gallery.insert(0, row);
        state.gallery.truncate(gallery::LISTING_CAP);
        state.stamp = cheap_stamp(&root);
        state.content = content_stamp(&state.gallery);
        state.revision += 1;
        Ok(path)
    }

    // ── the things that are not generation ──

    /// Turn what a caller named into a file.
    ///
    /// A bare filename is looked up in the gallery, because "the lighthouse one" is how a person
    /// refers to a picture and a path is how a machine does; both have to work or the action is only
    /// half usable.
    pub fn resolve(&self, named: &str, must_be_in_the_gallery: bool) -> Result<PathBuf, String> {
        let named = named.trim();
        if named.is_empty() {
            return Err("no file was named".to_string());
        }
        let candidate = if named.contains('/') || named.starts_with('~') {
            gallery::expand_home(named)
        } else {
            // A name with no folder in it is matched against the gallery, newest first, so the
            // picture a person is looking at is the one that is meant.
            let found = lock(&self.state)
                .gallery
                .iter()
                .find(|row| row.name == named)
                .map(|row| row.path.clone());
            match found {
                Some(path) => path,
                None => {
                    let guess = gallery::day_folder(&self.gallery_root(), Local::now()).join(named);
                    if guess.exists() {
                        guess
                    } else {
                        return Err(format!(
                            "there is no {named} in the gallery. `describe` lists what is there, with the full path of each."
                        ));
                    }
                }
            }
        };
        let path = match std::fs::canonicalize(&candidate) {
            Ok(path) => path,
            Err(_) => {
                return Err(format!(
                    "{} is not there. Nothing was changed.",
                    candidate.display()
                ))
            }
        };
        if !path.is_file() {
            return Err(format!("{} is a folder, not a picture", path.display()));
        }
        let name = file_name(&path);
        if !yantrik_image_core::is_image(&name) {
            // The same answer the file browser gives, so this app cannot be talked into treating as a
            // picture a file that Files would not show as one.
            return Err(format!(
                "{name} is not a picture this OS recognises (it looks for {})",
                yantrik_image_core::IMAGE_EXTENSIONS.join(", ")
            ));
        }
        if must_be_in_the_gallery {
            let root = self.gallery_root();
            if !gallery::is_inside(&path, &root) {
                return Err(format!(
                    "{} is not in Studio's gallery at {}, and this app only deletes its own pictures. Files can delete anything.",
                    path.display(),
                    display(&root)
                ));
            }
        }
        Ok(path)
    }

    fn gallery_root(&self) -> PathBuf {
        lock(&self.state).places.gallery.clone()
    }

    /// Move a picture and the record beside it to the Trash.
    pub fn delete(&self, named: &str) -> Result<Vec<trash::Moved>, String> {
        let path = self.resolve(named, true)?;
        let places = { lock(&self.state).places.clone() };
        let moved = trash::picture(&path, &places.trash)?;
        let names: Vec<String> = moved.iter().map(|each| each.name.clone()).collect();
        let mut state = lock(&self.state);
        state.gallery.retain(|row| row.path != path);
        state.stamp = cheap_stamp(&places.gallery);
        state.content = content_stamp(&state.gallery);
        state.notice = format!(
            "Moved {} to the Trash. Files can put {} back until the Trash is emptied.",
            names.join(" and "),
            if names.len() == 1 { "it" } else { "them" }
        );
        state.revision += 1;
        Ok(moved)
    }

    /// Change where pictures are made, and re-declare what that costs.
    ///
    /// The regrade is why this is one action rather than a file a person edits. Without it, a caller
    /// could point Studio at a hosted service and generate in the same breath, and the prompt would
    /// leave the machine under the grade that applied when it was still local — which is the one
    /// direction a grade must never be wrong in.
    pub fn set_backend(
        &self,
        kind: &str,
        base_url: &str,
        model: &str,
        api_key_env: &str,
        workflow: &str,
    ) -> Result<Snapshot, String> {
        let Some(parsed) = Kind::parse(kind) else {
            return Err(format!(
                "`{kind}` is not a backend Studio has. It can make pictures with {}, or with `fake`, which draws a placeholder here.",
                KINDS.join(", ")
            ));
        };
        let mut backend = BackendConfig::defaults_for(parsed);
        if !base_url.trim().is_empty() {
            backend.base_url = base_url.trim().trim_end_matches('/').to_string();
        }
        if !model.trim().is_empty() {
            backend.model = model.trim().to_string();
        }
        if !api_key_env.trim().is_empty() {
            backend.api_key_env = api_key_env.trim().to_string();
        }
        if !workflow.trim().is_empty() {
            backend.workflow = gallery::expand_home(workflow.trim()).display().to_string();
        }

        // Checked by the same parser the config file goes through, before anything is written, so a
        // mistake does not leave behind a configuration that the next start would read and fail on.
        let candidate = Config {
            backend: backend.clone(),
            configured: true,
            output_folder: String::new(),
        };
        let text = serde_json::to_string(&candidate.to_json()).map_err(|e| e.to_string())?;
        let mut config = Config::parse(&text)?;
        if parsed == Kind::ComfyUi && !backend.workflow.is_empty() {
            // Reading the graph now is the difference between "your workflow file is not in the API
            // format" and a 400 from a server three minutes into a queue.
            crate::workflow::graph(&backend.workflow)?;
        }
        // The folder the person already had stays theirs; `parse` starts from empty.
        config.output_folder = lock(&self.state).config.output_folder.clone();
        config.configured = true;

        let places = { lock(&self.state).places.clone() };
        let written = match &places.settings {
            Some(path) => {
                config.save_to(path)?;
                true
            }
            None => false,
        };

        // The notice is only set for the thing that stops a generation. `facts.note` also carries the
        // everyday caveats, and those belong in `describe`, where a caller reads them, rather than at
        // the front of every one-line summary.
        let notice = if parsed == Kind::OpenAiImages {
            config.backend.api_key().err().unwrap_or_default()
        } else {
            String::new()
        };

        {
            let mut state = lock(&self.state);
            state.config = config.clone();
            state.backend = backend::build(&config);
            state.notice = notice;
            state.revision += 1;
        }
        self.relist();

        // The published grade follows the backend. This runs on the thread that owns the window,
        // which is where the registry lives.
        let grade = self.grade();
        for action in ["generate", "variations"] {
            if let Err(problem) = control::regrade(action, grade) {
                // Not fatal, and not silent: the app keeps working, and the log says the grade on the
                // approval card may be the one from before the change.
                tracing::error!("the grade of {action} could not follow the new backend: {problem}");
            }
        }

        if !written {
            self.say(
                "The backend is changed for this session, but there is no home directory to write it down in, so it will not survive a restart.",
            );
        }
        Ok(self.snapshot())
    }

    /// Stop one job, or every job. `None` means every job, which is what a person pressing Stop in the
    /// window means and what a mind asking for `cancel_all` means.
    pub fn cancel(&self, job: Option<i32>) -> Result<String, String> {
        let mut state = lock(&self.state);
        let targets: Vec<i32> = match job {
            Some(id) => {
                if state.jobs.iter().any(|each| each.id == id) {
                    vec![id]
                } else {
                    return Err(format!(
                        "there is no job {id} in the queue. `describe` lists the ones there are."
                    ));
                }
            }
            None => state.jobs.iter().map(|each| each.id).collect(),
        };
        if targets.is_empty() {
            return Err("nothing was queued or running".to_string());
        }
        for id in &targets {
            if let Some(cancel) = state.cancels.get(id) {
                cancel.store(true, Ordering::SeqCst);
            }
            // "cancelling" rather than gone, because the worker is still in the middle of something;
            // a queue that said nothing while a server was still rendering would be describing a
            // busy machine as idle.
            if let Some(each) = state.jobs.iter_mut().find(|each| each.id == *id) {
                each.status = "cancelling".to_string();
            }
        }
        state.revision += 1;
        Ok(match targets.len() {
            1 => format!("asked job {} to stop", targets[0]),
            n => format!("asked {n} jobs to stop"),
        })
    }
}

fn random_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = sha2::Sha256::new();
    hasher.update(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    // The process id and a counter, so two asks in the same millisecond — which is what a batch does
    // — still get different seeds. This has to be different every time and reproducible from the
    // sidecar afterwards; it does not have to be unpredictable to an attacker, which is not what a
    // seed is for.
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    u64::from_le_bytes(hasher.finalize()[..8].try_into().unwrap())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// A path with the home folder written as `~`, which is how a person would have typed it and how much
/// less of a line it takes up in a summary.
pub fn display(path: &Path) -> String {
    let text = path.display().to_string();
    match gallery::home().to_str() {
        Some(home) if !home.is_empty() && text.starts_with(home) => format!("~{}", &text[home.len()..]),
        _ => text,
    }
}

fn short_prompt(prompt: &str) -> String {
    let flat = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let quoted = format!("“{flat}”");
    if quoted.chars().count() <= 42 {
        quoted
    } else {
        format!("{}…”", quoted.chars().take(41).collect::<String>())
    }
}

/// Open a picture, or a folder, with whatever the desktop uses for it. Not on the surface's critical
/// path: it spawns a program and returns, which is what the window's own double-click does.
pub fn open(path: &Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{} could not be opened ({e}); is xdg-open there?", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary set of places, so nothing here touches a real gallery, a real Trash or a real
    /// settings file. The environment variables those come from are process-wide and tests run at the
    /// same time, which is why `Places` exists rather than `set_var`.
    fn places(name: &str) -> (PathBuf, Places) {
        let dir = std::env::temp_dir().join(format!(
            "studio-engine-{name}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let found = Places {
            gallery: dir.join("gallery"),
            trash: dir.join("trash-v2"),
            settings: Some(dir.join("studio.json")),
        };
        (dir, found)
    }

    fn engine_with(config_text: &str, found: &Places) -> Engine {
        Engine::open(Config::parse(config_text).unwrap(), found.clone())
    }

    /// Wait for the queue to empty, the way a caller watching `describe` would, and return the notice
    /// it left behind.
    fn drain(engine: &Engine) -> String {
        for _ in 0..200 {
            let snapshot = engine.snapshot();
            if snapshot.jobs.is_empty() {
                return snapshot.notice;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("the job never finished: {:?}", engine.snapshot().jobs);
    }

    #[test]
    fn an_ask_that_cannot_work_is_refused_before_anything_is_queued() {
        let (dir, found) = places("plan");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        for plan in [
            Plan::new(""),
            Plan::new("   "),
            Plan { count: 0, ..Plan::new("a lighthouse") },
            Plan { count: 9, ..Plan::new("a lighthouse") },
        ] {
            let problem = engine.generate(plan.clone()).unwrap_err();
            assert!(!problem.is_empty());
            assert!(engine.snapshot().jobs.is_empty(), "{problem} left a job in the queue");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_refusals_say_what_would_work() {
        let (dir, found) = places("words");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let count = engine.generate(Plan { count: 12, ..Plan::new("x") }).unwrap_err();
        assert!(count.contains("12") && count.contains(&MAX_COUNT.to_string()), "{count}");
        assert!(count.contains("charges"), "{count}");
        let prompt = engine.generate(Plan::new("  ")).unwrap_err();
        assert!(prompt.contains("prompt="), "{prompt}");
        let zero = engine.generate(Plan { count: 0, ..Plan::new("x") }).unwrap_err();
        assert!(zero.contains("count=0"), "{zero}");
        let huge = Plan { prompt: "a".repeat(5000), ..Plan::new("x") }.checked().unwrap_err();
        assert!(huge.contains("5000"), "{huge}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_numbers_are_brought_onto_the_grid_rather_than_refused() {
        let plan = Plan { width: 10_000, height: 0, steps: 900, cfg: 99.0, ..Plan::new("x") }
            .checked()
            .unwrap();
        assert_eq!(plan.width, MAX_EDGE);
        assert_eq!(plan.height, DEFAULT_HEIGHT, "a height of 0 means the default, not an error");
        assert_eq!(plan.steps, MAX_STEPS);
        assert_eq!(plan.cfg, 30.0);
        let plan = Plan { negative: "   ".into(), ..Plan::new("x") }.checked().unwrap();
        assert_eq!(plan.negative, "", "an empty negative had this app's opinion substituted into it");
        let plan = Plan { prompt: "  a lighthouse  ".into(), ..Plan::new("x") }.checked().unwrap();
        assert_eq!(plan.prompt, "a lighthouse");
    }

    #[test]
    fn generating_writes_a_picture_and_the_record_of_how_it_was_made() {
        let (dir, found) = places("generate");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let id = engine.generate(Plan { seed: Some(4242), ..Plan::new("a lighthouse in fog") }).unwrap();
        assert!(id > 0);
        // The action settles later, so the queue is where a caller looks meanwhile.
        let notice = drain(&engine);
        assert_eq!(notice, "", "a generation that worked left a notice: {notice}");

        let rows = engine.snapshot().gallery;
        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = &rows[0];
        assert!(row.path.starts_with(&found.gallery));
        assert!(row.path.exists());
        // Pictures go into a folder named after the day, so a gallery of thousands stays navigable
        // and a person can find last Tuesday's without this app.
        assert_eq!(
            row.path.parent().unwrap().file_name().unwrap().to_string_lossy(),
            Local::now().format("%Y-%m-%d").to_string()
        );
        assert_eq!(row.prompt, "a lighthouse in fog");
        assert_eq!(row.seed, 4242);
        assert_eq!(row.backend, "fake");
        assert!(row.has_sidecar);

        // The record is the point of the exercise: a reader who was not here when the picture was made
        // can still tell exactly how it was.
        let sidecar = Sidecar::read(&row.path).unwrap();
        assert_eq!(sidecar.prompt, "a lighthouse in fog");
        assert_eq!(sidecar.negative, DEFAULT_NEGATIVE);
        assert_eq!(sidecar.seed, 4242);
        assert_eq!(sidecar.backend, "fake");
        assert_eq!(sidecar.steps, Some(DEFAULT_STEPS));
        assert_eq!(sidecar.made_by, "yantrik-studio");
        assert!(sidecar.seconds >= 0.0);
        assert!(!sidecar.created.is_empty());
        assert!(Sidecar::path_for(&row.path).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn asking_for_several_pictures_gives_each_its_own_seed_and_its_own_record() {
        let (dir, found) = places("count");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        engine
            .generate(Plan { seed: Some(7), count: 3, ..Plan::new("four attempts at a harbour") })
            .unwrap();
        drain(&engine);
        let rows = engine.snapshot().gallery;
        assert_eq!(rows.len(), 3, "{rows:?}");
        let mut seeds: Vec<u64> = rows.iter().map(|row| row.seed).collect();
        seeds.sort();
        assert_eq!(seeds, [7, 8, 9]);
        let paths: Vec<PathBuf> = rows.iter().map(|row| row.path.clone()).collect();
        assert_eq!(paths.iter().filter(|path| path.exists()).count(), 3);
        assert_eq!(paths.iter().filter(|path| Sidecar::path_for(path).exists()).count(), 3);
        let first = std::fs::read(&paths[0]).unwrap();
        assert!(paths[1..].iter().all(|path| std::fs::read(path).unwrap() != first));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_picture_with_no_seed_chosen_gets_one_that_is_written_down() {
        let (dir, found) = places("seed");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        engine.generate(Plan::new("a harbour at dusk")).unwrap();
        engine.generate(Plan::new("a harbour at dusk")).unwrap();
        drain(&engine);
        let rows = engine.snapshot().gallery;
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].seed, rows[1].seed, "two asks in a row got the same seed");
        assert_ne!(rows[0].seed, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn variations_come_from_the_record_of_the_picture_they_vary() {
        let (dir, found) = places("variations");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        engine
            .generate(Plan {
                seed: Some(11),
                steps: 21,
                width: 512,
                height: 768,
                ..Plan::new("a lighthouse in fog")
            })
            .unwrap();
        drain(&engine);
        let original = engine.snapshot().gallery[0].path.clone();
        let name = file_name(&original);

        // By full path, and by the bare filename a person would say.
        engine.variations(original.to_str().unwrap(), 2).unwrap();
        drain(&engine);
        engine.variations(&name, 1).unwrap();
        drain(&engine);

        let rows = engine.snapshot().gallery;
        assert_eq!(rows.len(), 4, "{rows:?}");
        let varied: Vec<&Record> = rows.iter().filter(|row| !row.made_from.is_empty()).collect();
        assert_eq!(varied.len(), 3, "{rows:?}");
        assert_eq!(varied[0].made_from, format!("variation of {name}"));
        // The sentence, the size and the step count came from the record; the seeds did not.
        let sidecar = Sidecar::read(&varied[0].path).unwrap();
        assert_eq!(sidecar.prompt, "a lighthouse in fog");
        assert_eq!(sidecar.steps, Some(21));
        assert_eq!((sidecar.width, sidecar.height), (512, 768), "{sidecar:?}");
        assert_ne!(sidecar.seed, 11);
        // And the original is still there, unchanged.
        assert!(rows.iter().any(|row| row.path == original));
        assert!(original.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn varying_a_picture_with_no_record_says_there_is_nothing_to_vary() {
        let (dir, found) = places("no-sidecar");
        let day = gallery::day_folder(&found.gallery, Local::now());
        std::fs::create_dir_all(&day).unwrap();
        let copied = day.join("copied-in.png");
        std::fs::write(&copied, backend::draw("elsewhere", 1, 32, 32)).unwrap();
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let problem = engine.variations(copied.to_str().unwrap(), 1).unwrap_err();
        assert!(problem.contains("no record beside"), "{problem}");
        assert!(problem.contains("generate prompt="), "{problem}");
        assert!(problem.contains("copied-in.png"), "{problem}");
        // Nor was anything queued by the refusal.
        assert!(engine.snapshot().jobs.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upscaling_makes_a_bigger_file_and_keeps_the_sentence_that_made_the_original() {
        let (dir, found) = places("upscale");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        engine.generate(Plan { width: 256, height: 192, ..Plan::new("a cliff path") }).unwrap();
        drain(&engine);
        let original = engine.snapshot().gallery[0].path.clone();

        engine.upscale(original.to_str().unwrap(), 2).unwrap();
        drain(&engine);

        let rows = engine.snapshot().gallery;
        assert_eq!(rows.len(), 2, "{rows:?}");
        let bigger = rows.iter().find(|row| row.path != original).unwrap();
        assert_eq!((bigger.width, bigger.height), (512, 384), "{bigger:?}");
        assert_eq!(bigger.prompt, "a cliff path", "the sentence was lost");
        assert!(bigger.made_from.starts_with("upscale of "), "{bigger:?}");
        let sidecar = Sidecar::read(&bigger.path).unwrap();
        // Nothing was sent to a model, and the record does not claim otherwise.
        assert_eq!(sidecar.backend, "studio-resample");
        assert_eq!(sidecar.model, "");
        assert!(sidecar.steps.is_none());
        assert!(original.exists(), "the original was consumed");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_that_is_not_there_is_not_reported_as_deleted_or_upscaled() {
        let (dir, found) = places("missing");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let problem = engine.delete("/no/such/picture.png").unwrap_err();
        assert!(problem.contains("is not there"), "{problem}");
        assert!(problem.contains("Nothing was changed"), "{problem}");
        assert!(engine.upscale("nope.png", 2).unwrap_err().contains("no nope.png in the gallery"));
        assert!(engine.resolve("", false).unwrap_err().contains("no file was named"));
        assert!(engine.snapshot().jobs.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_that_is_not_a_picture_is_not_treated_as_one() {
        let (dir, found) = places("not-a-picture");
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let problem = engine.delete(dir.join("notes.txt").to_str().unwrap()).unwrap_err();
        assert!(problem.contains("not a picture"), "{problem}");
        assert!(dir.join("notes.txt").exists(), "it was moved anyway");
        let problem = engine.resolve(dir.to_str().unwrap(), false).unwrap_err();
        assert!(problem.contains("a folder"), "{problem}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn deleting_moves_the_picture_and_its_record_to_the_trash_and_files_can_put_them_back() {
        let (dir, found) = places("delete");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        engine.generate(Plan::new("a picture to regret")).unwrap();
        drain(&engine);
        let path = engine.snapshot().gallery[0].path.clone();
        assert_eq!(engine.snapshot().gallery.len(), 1);

        let moved = engine.delete(path.to_str().unwrap()).unwrap();
        assert!(!path.exists());
        assert!(!Sidecar::path_for(&path).exists(), "the record was left beside a picture that is gone");
        assert_eq!(moved.len(), 2, "{moved:?}");
        assert!(engine.snapshot().gallery.is_empty(), "the gallery still shows it");
        assert!(engine.snapshot().notice.contains("Trash"), "{:?}", engine.snapshot().notice);

        // The promise `delete` is graded `standard` on: Files can undo it, and it undoes it by reading
        // the same folder this app wrote to.
        let listed = trash::items(&found.trash).unwrap();
        assert_eq!(listed.len(), 2, "{listed:?}");
        for item in &listed {
            trash::restore(item, &found.trash).unwrap();
        }
        assert!(path.exists(), "the picture did not come back");
        assert_eq!(Sidecar::read(&path).unwrap().prompt, "a picture to regret");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn this_app_will_not_delete_a_file_outside_its_own_gallery() {
        let (dir, found) = places("outside");
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let picture = elsewhere.join("important.png");
        std::fs::write(&picture, backend::draw("x", 1, 16, 16)).unwrap();

        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let problem = engine.delete(picture.to_str().unwrap()).unwrap_err();
        assert!(problem.contains("not in Studio's gallery"), "{problem}");
        assert!(problem.contains("Files can delete anything"), "{problem}");
        assert!(picture.exists(), "it was deleted anyway");
        // Reading it for an upscale is a different question and the answer is yes: what is written
        // lands in the gallery, and nothing outside it is touched.
        assert!(engine.resolve(picture.to_str().unwrap(), false).is_ok());
        assert!(engine.resolve(picture.to_str().unwrap(), true).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bare_filename_means_the_gallery_and_a_path_means_itself() {
        let (dir, found) = places("resolve");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        engine.generate(Plan::new("a lighthouse")).unwrap();
        drain(&engine);
        let path = engine.snapshot().gallery[0].path.clone();
        let name = file_name(&path);
        let canonical = std::fs::canonicalize(&path).unwrap();
        assert_eq!(engine.resolve(&name, true).unwrap(), canonical);
        assert_eq!(engine.resolve(path.to_str().unwrap(), true).unwrap(), canonical);
        let problem = engine.resolve("something-else.png", false).unwrap_err();
        assert!(problem.contains("something-else.png"), "{problem}");
        assert!(problem.contains("describe"), "{problem}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cancelling_stops_a_job_and_keeps_what_it_had_already_written() {
        let (dir, found) = places("cancel");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let id = engine.generate(Plan { count: 4, ..Plan::new("four harbours") }).unwrap();
        engine.cancel(Some(id)).unwrap();
        let notice = drain(&engine);
        assert!(notice.contains("Cancelled"), "{notice}");
        assert!(engine.snapshot().jobs.is_empty());
        // Whatever landed before the flag was seen stays: a cancellation is not a deletion, and the
        // notice says how many there were and that it cannot be resumed.
        assert!(
            notice.contains("already been written") || notice.contains("before anything"),
            "{notice}"
        );
        assert!(engine.cancel(Some(id)).unwrap_err().contains("no job"), "an unknown id was accepted");
        assert!(engine.cancel(None).unwrap_err().contains("nothing was queued"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn setting_the_backend_rewrites_the_config_and_regrades_the_action_that_sends_a_prompt() {
        let (dir, found) = places("set-backend");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        assert_eq!(engine.grade(), "standard");

        // A ComfyUI on the person's own LAN: nothing leaves, and the grade does not move.
        engine
            .set_backend("comfyui", "http://192.168.4.35:8188/", "dreamshaperXL_v21.safetensors", "", "")
            .unwrap();
        assert_eq!(engine.grade(), "standard");
        let snapshot = engine.snapshot();
        assert!(!snapshot.facts.prompt_leaves);
        assert!(snapshot.summary().contains("192.168.4.35"), "{}", snapshot.summary());

        // The same app pointed at a hosted service: the prompt now leaves the machine, and the grade
        // has to follow it or the approval card is promising something untrue.
        let snapshot = engine
            .set_backend(
                "openai-images",
                "https://api.openai.com/v1",
                "gpt-image-1",
                "STUDIO_TEST_UNSET_KEY",
                "",
            )
            .unwrap();
        assert_eq!(engine.grade(), "sensitive");
        assert!(snapshot.facts.prompt_leaves);

        // Written down, and readable by the next start.
        let settings = found.settings.as_ref().unwrap();
        let on_disk = Config::read(settings);
        assert_eq!(on_disk.backend.kind, Kind::OpenAiImages);
        assert_eq!(on_disk.backend.model, "gpt-image-1");
        assert_eq!(on_disk.backend.api_key_env, "STUDIO_TEST_UNSET_KEY");
        // The name of the variable is written down, because a person has to know which to export.
        assert!(std::fs::read_to_string(settings).unwrap().contains("STUDIO_TEST_UNSET_KEY"));

        // Back to local, and the grade comes back with it.
        engine.set_backend("comfyui", "http://127.0.0.1:8188", "", "", "").unwrap();
        assert_eq!(engine.grade(), "standard");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_backend_that_does_not_exist_is_refused_with_the_ones_that_do() {
        let (dir, found) = places("bad-kind");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let problem = engine
            .set_backend("midjourney", "", "", "", "")
            .err()
            .expect("`midjourney` should have been refused");
        for kind in KINDS {
            assert!(problem.contains(kind), "{problem} does not name {kind}");
        }
        assert!(problem.contains("midjourney"), "{problem}");
        // Nothing was written and nothing changed.
        assert_eq!(engine.config().backend.kind, Kind::Fake);
        assert!(!found.settings.as_ref().unwrap().exists());
        // Nor was a hosted backend with no model saved: the file would have been left holding a
        // configuration that cannot work, and the next start would have found it.
        let problem = engine
            .set_backend("openai-images", "https://x.example/v1", "", "", "")
            .err()
            .expect("a hosted backend with no model should have been refused");
        assert!(problem.contains("`model`"), "{problem}");
        assert_eq!(engine.config().backend.kind, Kind::Fake);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_hosted_backend_whose_key_is_not_exported_is_saved_and_says_what_is_missing() {
        // Saving it is right: the person may export the key a moment later, and refusing would have
        // them configure the same thing twice. But the summary has to lead with the problem, or the
        // window says "a hosted service" while every generation fails.
        let (dir, found) = places("no-key");
        std::env::remove_var("STUDIO_TEST_UNSET_KEY");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let snapshot = engine
            .set_backend("openai-images", "", "gpt-image-1", "STUDIO_TEST_UNSET_KEY", "")
            .unwrap();
        assert!(snapshot.notice.contains("STUDIO_TEST_UNSET_KEY"), "{:?}", snapshot.notice);
        assert!(
            snapshot.summary().starts_with("Studio — the environment variable"),
            "{}",
            snapshot.summary()
        );
        assert_eq!(snapshot.state()["backend"]["api_key_is_set"], json!(false));
        // The configuration is still on disk, so exporting the key is the only thing left to do.
        assert!(found.settings.as_ref().unwrap().exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_backend_with_nowhere_to_write_it_down_says_it_will_not_survive_a_restart() {
        let (dir, mut found) = places("no-home");
        found.settings = None;
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let snapshot = engine.set_backend("comfyui", "http://127.0.0.1:8188", "", "", "").unwrap();
        assert!(snapshot.notice.contains("will not survive a restart"), "{:?}", snapshot.notice);
        // The change still took effect for this session, which is what was asked for.
        assert_eq!(snapshot.config.backend.kind, Kind::ComfyUi);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_workflow_file_that_is_not_the_api_format_is_refused_before_it_is_saved() {
        let (dir, found) = places("workflow");
        let editor_format = dir.join("from-the-editor.json");
        std::fs::write(
            &editor_format,
            r#"{"last_node_id": 9, "nodes": [{"id": 3, "type": "KSampler"}], "links": []}"#,
        )
        .unwrap();
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let problem = engine
            .set_backend("comfyui", "http://127.0.0.1:8188", "", "", editor_format.to_str().unwrap())
            .err()
            .expect("a workflow that is not in the API format should have been refused");
        assert!(problem.contains("API Format"), "{problem}");
        assert_eq!(engine.config().backend.kind, Kind::Fake, "the broken workflow was saved anyway");
        assert!(!found.settings.as_ref().unwrap().exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_summary_leads_with_trouble_and_says_where_the_pictures_are() {
        let (dir, found) = places("summary");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        let idle = engine.snapshot().summary();
        assert!(idle.starts_with("Studio — "), "{idle}");
        assert!(idle.contains("placeholder images"), "{idle}");
        assert!(!idle.contains("fake"), "the summary is read by a person: {idle}");
        assert!(idle.contains("nothing in the gallery yet"), "{idle}");
        assert_eq!(idle.lines().count(), 1, "{idle}");

        engine.generate(Plan::new("a lighthouse")).unwrap();
        drain(&engine);
        assert!(engine.snapshot().summary().contains("1 picture in"), "{}", engine.snapshot().summary());

        // Trouble goes first, because it is the reason to look.
        engine.say("the server could not be reached");
        let troubled = engine.snapshot().summary();
        assert!(troubled.starts_with("Studio — the server could not be reached"), "{troubled}");
        engine.clear_notice();
        assert!(!engine.snapshot().summary().contains("reached"));

        let unconfigured = Engine::open(
            Config::unconfigured(),
            Places { gallery: dir.join("gallery2"), trash: found.trash.clone(), settings: None },
        )
        .snapshot()
        .summary();
        assert!(unconfigured.contains("no backend configured"), "{unconfigured}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_state_carries_the_backend_the_queue_the_gallery_and_the_folder() {
        let (dir, found) = places("state");
        // The in-process server, so a ComfyUI generation really finishes and the gallery really
        // holds what came back. The LAN box this was written against was not reachable from
        // where the tests run, and a test that needs one machine on one network is a test that
        // fails somewhere else.
        let server = crate::tests::Server::start(crate::tests::Manners::default());
        let engine = engine_with(
            &format!(
                r#"{{"backend":{{"kind":"comfyui","base_url":"{}","model":"dreamshaperXL_v21.safetensors"}}}}"#,
                server.base_url
            ),
            &found,
        );
        engine.generate(Plan::new("a lighthouse")).unwrap();
        drain(&engine);
        let state = engine.snapshot().state();
        assert_eq!(state["backend"]["kind"], json!("comfyui"));
        assert_eq!(state["backend"]["model"], json!("dreamshaperXL_v21.safetensors"));
        assert_eq!(state["backend"]["generate_is_graded"], json!("standard"));
        assert_eq!(state["backend"]["prompt_leaves_this_machine"], json!(false));
        let place = state["backend"]["where"].as_str().unwrap().to_string();
        assert!(place.contains(server.base_url.as_str()), "{place}");
        assert_eq!(state["gallery"]["count"], json!(1));
        assert_eq!(state["gallery"]["newest"][0]["prompt"], json!("a lighthouse"));
        assert_eq!(state["gallery"]["newest"][0]["backend"], json!("comfyui"));
        assert!(state["gallery"]["newest"][0]["seconds"].is_number());
        assert!(state["gallery"]["newest"][0]["path"].as_str().unwrap().ends_with(".png"));
        assert!(state["output_folder"].as_str().unwrap().ends_with("gallery"));
        assert!(state.get("notice").is_none(), "an empty notice is not a fact");
        // The queue is empty and is still shown, so "nothing running" is not confused with "unreadable".
        assert_eq!(state["queue"]["running"], json!([]));
        assert_eq!(state["queue"]["pending"], json!([]));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failure_is_reported_where_a_caller_will_read_it() {
        let (dir, found) = places("failure");
        // A ComfyUI that is not there. The point is not the transport but that the answer names the
        // backend and the server, says what to check, and lands in the notice rather than a log
        // nobody is tailing.
        let engine =
            engine_with(r#"{"backend":{"kind":"comfyui","base_url":"http://127.0.0.1:1"}}"#, &found);
        engine.generate(Plan::new("a lighthouse")).unwrap();
        let notice = drain(&engine);
        assert!(notice.contains("comfyui"), "{notice}");
        assert!(notice.contains("127.0.0.1:1"), "{notice}");
        assert!(notice.contains("reached"), "{notice}");
        assert!(engine.snapshot().gallery.is_empty(), "a failed generation left a row behind");
        assert!(engine.snapshot().summary().contains("127.0.0.1:1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_picture_somebody_else_puts_in_the_folder_turns_up_in_the_gallery() {
        let (dir, found) = places("poll");
        let engine = engine_with(r#"{"backend":{"kind":"fake"}}"#, &found);
        assert!(engine.snapshot().gallery.is_empty());
        let day = gallery::day_folder(&found.gallery, Local::now());
        std::fs::create_dir_all(&day).unwrap();
        let copied = day.join("copied-in.png");
        std::fs::write(&copied, backend::draw("a harbour", 5, 32, 32)).unwrap();

        engine.poll();
        for _ in 0..100 {
            if !engine.snapshot().gallery.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let rows = engine.snapshot().gallery;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].name, "copied-in.png");
        assert!(!rows[0].has_sidecar, "a record was invented for a file that has none");
        assert_eq!(rows[0].json()["sidecar"], json!("missing"));

        // And polling an unchanged folder does not bump the revision, which is what keeps an idle
        // window from redrawing and an idle app from costing a share of a core.
        let before = engine.revision();
        for _ in 0..5 {
            engine.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(engine.revision(), before, "an unchanged folder was announced as a change");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_path_is_written_with_the_home_folder_as_a_tilde() {
        let home = gallery::home();
        assert_eq!(display(&home.join("Pictures/Studio")), "~/Pictures/Studio");
        assert_eq!(display(Path::new("/srv/pictures")), "/srv/pictures");
    }

    #[test]
    fn a_label_in_the_queue_is_readable_and_short() {
        assert_eq!(short_prompt("a lighthouse in fog"), "“a lighthouse in fog”");
        assert_eq!(short_prompt("  a   lighthouse  "), "“a lighthouse”");
        let long = "a very long prompt that goes on and on about a lighthouse in the fog";
        let label = short_prompt(long);
        assert!(label.chars().count() <= 43, "{label}");
        assert!(label.ends_with("…”"), "{label}");
        assert!(label.starts_with("“a very long prompt"), "{label}");
    }

    #[test]
    fn an_unconfigured_machine_gets_the_fake_backend_and_a_working_surface() {
        let (dir, found) = places("unconfigured");
        let engine = Engine::open(Config::unconfigured(), found);
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.facts.kind, "fake");
        assert!(!snapshot.facts.configured);
        assert_eq!(engine.grade(), "standard");
        assert!(snapshot.facts.note.contains("No backend is configured"), "{:?}", snapshot.facts.note);
        // It still makes pictures, so the whole surface is drivable on a machine with no GPU and no
        // key — which is also what lets every test here run anywhere.
        engine.generate(Plan::new("a lighthouse")).unwrap();
        assert_eq!(drain(&engine), "");
        assert_eq!(engine.snapshot().gallery.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
