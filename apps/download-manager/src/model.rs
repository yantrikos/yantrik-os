//! What the app is. The downloads themselves are the engine's (`engine/`): a handle onto shared
//! state that its worker threads write as they go. The model holds that handle, the line of
//! commands to it (a `Lane`), and what the window is set to show.

use serde_json::Value;
use yantrik_app::rail::Rail;

use yantrik_app::lane::Lane;
use crate::engine::{Download, Engine, Status};

/// The parts of the window `render` draws separately.
pub const PART_LIST: &str = "list";
pub const PART_BANNER: &str = "banner";
pub const PART_FORM: &str = "form";
pub const PART_AI: &str = "ai";

pub struct Model {
    pub engine: Engine,
    pub desk: Lane<Engine>,
    /// Commands to the desk (and the exit's settling) that have not landed yet.
    pub work: usize,
    /// The engine's revision as last read: the list is drawn when it moves.
    pub seen: u64,
    /// Whether the 250 ms look at the engine is running. It runs only while a download is.
    pub ticking: bool,

    // ── What the window is set to show ──
    pub filter: i32,
    pub sort: i32,
    pub search: String,
    pub all_selected: bool,
    /// The banner under the header: a command's own refusal, or what the engine has to say.
    pub banner: String,
    pub banner_info: bool,
    /// Set by the landing of the person's own add, so the dialog closes; cleared by the next command.
    pub form_done: bool,
    /// Why the AI button has nothing to ask, in place of an answer.
    pub ai_hint: Option<String>,
    pub rail: Rail,

    // ── The app's life ──
    pub closing: bool,
    pub ending: bool,
    /// The answer the intent being settled is owed, written in the command that settles it.
    pub answer: Value,
}

impl Model {
    pub fn new(engine: Engine, rail: Rail) -> Model {
        let mut m = Model {
            desk: Lane::start("download-desk", engine.clone()),
            engine,
            work: 0,
            // Not the engine's own: the first look must draw the empty list's counters.
            seen: u64::MAX,
            ticking: false,
            filter: 0,
            sort: 0,
            search: String::new(),
            all_selected: false,
            banner: String::new(),
            banner_info: false,
            form_done: false,
            ai_hint: None,
            rail,
            closing: false,
            ending: false,
            answer: Value::Null,
        };
        m.refresh_banner();
        m
    }

    /// Whether a download is being written, or a command to the engine is on its way: what a
    /// close has to wait for.
    pub fn busy(&self) -> bool {
        self.work > 0 || self.engine.writing()
    }

    /// The banner, when nothing a command said holds it: the engine's notice first (a list that
    /// could not be read or written, or came back changed), then the newest failed download. Once
    /// shown it stays until a command's outcome replaces it. Whether it changed.
    pub fn refresh_banner(&mut self) -> bool {
        if !self.banner.is_empty() {
            return false;
        }
        if let Some(notice) = self.engine.unseen_notice() {
            self.banner = notice;
            self.banner_info = self.engine.notice_is_info();
            return true;
        }
        let items = self.engine.snapshot();
        match items.iter().rev().find(|d| d.status == Status::Failed && !d.error.is_empty()) {
            Some(failed) => {
                self.banner = format!("{} \u{2014} {}", failed.filename, failed.error);
                self.banner_info = false;
                true
            }
            None => false,
        }
    }

    /// The downloads as they stand, oldest first.
    pub fn items(&self) -> Vec<Download> {
        self.engine.snapshot()
    }
}
