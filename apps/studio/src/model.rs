//! What Studio is: which backend is configured, what is being made, what has been made, and the
//! little the window holds that is not one of those.
//!
//! The gallery is a cache of files on disk. That is deliberate: the files are the real thing, and
//! a cache can be wrong in a way that a second copy of the truth cannot. Anything that changes the
//! folder, a person copying a picture into it included, is picked up by the poll, and `refresh`
//! reads it again on demand. The cache is only ever replaced from a listing a worker made; nothing
//! on the UI thread reads the folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use yantrik_app::for_mind::HIDDEN;

use crate::backend::{self, Backend};
use crate::config::{Config, Facts};
use crate::gallery::{self, Listed, Record};
use crate::plan::{self, Plan};
use crate::trash;

/// The parts of the window `render` draws separately.
pub const PART_BACKEND: &str = "backend";
pub const PART_STATUS: &str = "status";
pub const PART_JOBS: &str = "jobs";
pub const PART_GALLERY: &str = "gallery";
pub const PART_DETAIL: &str = "detail";
pub const PART_FORM: &str = "form";
pub const PART_NOTICE: &str = "notice";
pub const PART_WIDE: &str = "wide";
pub const PART_FOCUS: &str = "focus";

/// How wide the window has to be to carry the record pane on the right. The number lives here
/// rather than in the .slint because the .slint may not read the window's own width inside the
/// layout the window's size is derived from: Slint reports that as a binding loop on
/// `layoutinfo-h` and says a loop there may panic at runtime. The window reports its width as a
/// plain number (`resized`), and this decides what it means, so the .slint tree has no edge back
/// to the window size.
pub const WIDE_WINDOW: i32 = 900;

/// How many pictures are being made at once. The rest of the queue waits its turn: a job is a
/// thread of the app's small pool for as long as a render takes, and the pool also serves the
/// window's reads of the folder and of a picture's record, which must never wait behind four
/// renders.
pub const RENDERING_AT_ONCE: usize = 2;

/// The places this app writes to, looked up once and held.
///
/// Held rather than asked for each time, for two reasons. A test can point all three at a temporary
/// directory and be certain nothing it does reaches the person's real Trash or their real settings
/// file: the environment variables those come from are process-wide, and tests in one binary run at
/// the same time. And `set_backend` has to know whether it was able to write the change down,
/// because a backend that cannot be persisted is a backend that will not survive a restart, which
/// is worth saying out loud.
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

/// One entry in the queue, as `describe` and the window show it.
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
    /// The label as a window or a Mind may read it: a label holds the person's sentence.
    pub fn shown_label(&self, for_mind: bool) -> String {
        if for_mind {
            HIDDEN.to_string()
        } else {
            self.label.clone()
        }
    }

    pub fn json(&self, for_mind: bool) -> Value {
        let mut row = serde_json::Map::new();
        row.insert("id".into(), json!(self.id));
        row.insert("kind".into(), json!(self.kind));
        row.insert("label".into(), json!(self.shown_label(for_mind)));
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

/// What a queue entry will do when its turn comes.
#[derive(Clone, Debug)]
pub enum Work {
    /// Pictures from a sentence; `made_from` says what they vary, if they do.
    Plan { plan: Plan, made_from: String },
    /// A resample of one picture.
    Upscale { source: PathBuf, factor: u32 },
}

/// A job and what it needs to run. The backend is the one that was configured when the job was
/// asked for, so a `set_backend` that lands meanwhile cannot send an approved sentence somewhere
/// the person did not approve.
#[derive(Clone)]
pub struct Entry {
    pub job: Job,
    pub work: Work,
    pub backend: Arc<dyn Backend>,
    /// Set to stop the job; the picture in flight polls it, and so does the framework's own flag.
    pub stop: backend::Stop,
    /// A picture of this job is on a worker.
    pub flying: bool,
}

/// The ask as the window has it: text, because that is what the fields hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub prompt: String,
    pub negative: String,
    pub width: String,
    pub height: String,
    pub steps: String,
    pub count: String,
}

impl Default for Form {
    /// The engine's defaults rather than blank: a field that looks empty and means "1024" is a
    /// field a person cannot trust, and one that shows 1024 can be edited.
    fn default() -> Form {
        Form {
            prompt: String::new(),
            negative: plan::DEFAULT_NEGATIVE.to_string(),
            width: plan::DEFAULT_WIDTH.to_string(),
            height: plan::DEFAULT_HEIGHT.to_string(),
            steps: plan::DEFAULT_STEPS.to_string(),
            count: "1".to_string(),
        }
    }
}

/// The fields the window reports being typed into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Prompt,
    Negative,
    Width,
    Height,
    Steps,
    Count,
}

impl Field {
    pub fn of(name: &str) -> Option<Field> {
        Some(match name {
            "prompt" => Field::Prompt,
            "negative" => Field::Negative,
            "width" => Field::Width,
            "height" => Field::Height,
            "steps" => Field::Steps,
            "count" => Field::Count,
            _ => return None,
        })
    }
}

impl Form {
    pub fn set(&mut self, field: Field, text: String) {
        match field {
            Field::Prompt => self.prompt = text,
            Field::Negative => self.negative = text,
            Field::Width => self.width = text,
            Field::Height => self.height = text,
            Field::Steps => self.steps = text,
            Field::Count => self.count = text,
        }
    }

    /// The ask, as the window has it.
    ///
    /// An unparsable number arrives as 0, which `Plan::checked` reads as "the default" for a size
    /// and as "not given" for the rest. Steps and count are filled in here instead, because
    /// `checked` clamps 0 steps to 1 and refuses a count of 0 outright, and a person who cleared the
    /// field wants a picture, not a refusal about a text box.
    pub fn plan(&self) -> Plan {
        let mut plan = Plan::new(self.prompt.clone());
        plan.negative = self.negative.clone();
        plan.width = typed(&self.width);
        plan.height = typed(&self.height);
        let steps = typed(&self.steps);
        plan.steps = if steps == 0 { plan::DEFAULT_STEPS } else { steps };
        plan.count = typed(&self.count).clamp(1, plan::MAX_COUNT);
        plan
    }
}

/// A number somebody typed into a text field.
fn typed(value: &str) -> u32 {
    value.trim().parse::<u32>().unwrap_or(0)
}

/// Which picture the right-hand pane is reading back. The record is read by a worker when a row is
/// chosen; `ticket` tells its answer from an older one for a row the person has since left.
#[derive(Clone, Default)]
pub struct Chosen {
    pub path: String,
    pub detail: String,
    pub ticket: u64,
}

pub struct Model {
    pub config: Config,
    pub backend: Arc<dyn Backend>,
    pub places: Places,
    pub queue: Vec<Entry>,
    pub next_job: i32,
    pub gallery: Vec<Record>,
    /// Two directory timestamps, so the poll can tell whether anything moved without walking the
    /// folder.
    pub stamp: String,
    /// A fingerprint of the listing that `stamp` produced, so a re-read that found the same
    /// pictures does not announce itself as a change and no window redraws for nothing.
    pub content: String,
    /// The last thing worth saying that is not in the queue or the gallery: a failure, a
    /// cancellation, a deletion, a backend that cannot send anything yet.
    pub notice: String,
    pub form: Form,
    pub chosen: Chosen,
    pub wide: bool,
    /// The grade `generate` and `variations` were last published at, and whether that has been
    /// done since the surface was served. Until it has, those two are refused: a prompt must not
    /// be offered at the local grade to a machine already pointed at a hosted service.
    pub graded: bool,
    pub published_grade: &'static str,
    /// A `set_backend` is on a worker, writing the choice down.
    pub changing_backend: bool,
    /// How many times the backend has been changed. A call that was approved against one backend
    /// and lands against another is asked for again, not sent to the new one.
    pub backend_generation: u64,
    /// The folder is being read on a worker, so a change made meanwhile can drop the result.
    pub reading: Option<yantrik_app::JobId>,
    /// The answer the intent being settled is owed, written in the command that settles it.
    pub answer: Value,
    /// The sentence that was last sent to a backend from the prompt box. A prompt still in the
    /// box that is not this one is unsent, and is kept when the app closes.
    pub sent_prompt: String,
    /// The prompt box should take the keyboard, and how many times it was asked to.
    pub focus: u64,
    /// This copy is drawn for a Mind: prompts and pictures are hidden and nothing is made.
    pub for_mind: bool,
}

impl Model {
    pub fn new(config: Config, places: Places, listed: Listed, for_mind: bool) -> Model {
        Model {
            backend: backend::build(&config),
            config,
            places,
            queue: Vec::new(),
            next_job: 1,
            gallery: listed.rows,
            stamp: listed.stamp,
            content: listed.content,
            notice: String::new(),
            form: Form::default(),
            chosen: Chosen::default(),
            wide: true,
            graded: false,
            published_grade: "standard",
            changing_backend: false,
            backend_generation: 0,
            reading: None,
            answer: Value::Null,
            sent_prompt: String::new(),
            focus: 0,
            for_mind,
        }
    }

    /// Whether the key a hosted backend would use is exported. Only the question is asked of it,
    /// never its value.
    pub fn key_is_set(&self) -> bool {
        self.config.backend.api_key().is_ok()
    }

    pub fn facts(&self) -> Facts {
        self.config.facts(self.key_is_set())
    }

    /// The grade `generate` and `variations` should carry with the backend that is configured now.
    ///
    /// The grade does not depend on whether the key is exported (a prompt that cannot be sent today
    /// can be sent tomorrow), so `facts` is asked with the cheapest answer here.
    pub fn grade(&self) -> &'static str {
        self.config.facts(false).grade
    }

    pub fn entry(&self, id: i32) -> Option<&Entry> {
        self.queue.iter().find(|e| e.job.id == id)
    }

    pub fn entry_mut(&mut self, id: i32) -> Option<&mut Entry> {
        self.queue.iter_mut().find(|e| e.job.id == id)
    }

    pub fn running(&self) -> usize {
        self.queue.iter().filter(|e| e.job.status == "running").count()
    }

    pub fn jobs(&self) -> impl Iterator<Item = &Job> {
        self.queue.iter().map(|e| &e.job)
    }

    pub fn gallery_root(&self) -> &Path {
        &self.places.gallery
    }
}

/// A path with the home folder written as `~`, which is how a person would have typed it and how
/// much less of a line it takes up in a summary.
pub fn display(path: &Path) -> String {
    let text = path.display().to_string();
    match gallery::home().to_str() {
        Some(home) if !home.is_empty() && text.starts_with(home) => format!("~{}", &text[home.len()..]),
        _ => text,
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default()
}

/// The queue's name for a sentence: readable and short.
pub fn short_prompt(prompt: &str) -> String {
    let flat = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let quoted = format!("\u{201c}{flat}\u{201d}");
    if quoted.chars().count() <= 42 {
        quoted
    } else {
        format!("{}\u{2026}\u{201d}", quoted.chars().take(41).collect::<String>())
    }
}
