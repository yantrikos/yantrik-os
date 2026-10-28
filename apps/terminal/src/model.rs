//! What the Terminal is: its tabs, the sessions behind them, and the little the window holds that
//! is not the screen. The sessions own the PTYs and the screens (`session.rs`); the model holds
//! what the last probe found about each shell, so `describe` and the window never read `/proc`.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use yantrik_app::app::WindowId;
use yantrik_app::mind::Mind;

use crate::hub::Hub;
use crate::session::Session;

/// The parts of the window `render` draws separately.
pub const PART_TABS: &str = "tabs";
pub const PART_SCREEN: &str = "screen";
pub const PART_FIND: &str = "find";
pub const PART_EXPLAIN: &str = "explain";
pub const PART_NOTICE: &str = "notice";
pub const PART_DIALOG: &str = "dialog";
pub const PART_FONT: &str = "font";
pub const PART_FOCUS: &str = "focus";
pub const PART_RAISE: &str = "raise";

/// The most tabs open at once. Each tab is a shell process and a PTY, so the number stays
/// bounded; 32 is enough for an agent that opens tabs as it works (#683) without exhausting the
/// machine. A tab being started counts: the cap is on what is open and what is on its way.
pub const MAX_TABS: usize = 32;

/// The text size the zoom keys move between, and the size they reset to.
pub const FONT_MIN: i32 = 11;
pub const FONT_MAX: i32 = 24;
pub const FONT_DEFAULT: i32 = 14;

/// The refusal to open one more tab, or `None` when there is room.
pub fn tabs_full(open: usize) -> Option<String> {
    (open >= MAX_TABS).then(|| format!("Terminal has {MAX_TABS} tabs. Close a tab before opening another."))
}

/// What a probe found out about one shell by reading `/proc`: where it is, and what is running
/// in it. Read off the UI thread; the model keeps the last one.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Facts {
    pub cwd: PathBuf,
    /// The name of the shell's foreground job, if it has one.
    pub running: Option<String>,
    /// Whether the shell is alive and has a child: what closing the tab would end.
    pub busy: bool,
}

pub struct Tab {
    pub id: u64,
    pub session: Arc<Session>,
    pub facts: Facts,
    /// A probe of this shell is on a worker.
    pub probing: bool,
    /// Something happened in this shell since it was probed.
    pub due: bool,
    /// The session's revision when the last frame looked.
    pub revision: u64,
}

/// What a prompt is about to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Tab(u64),
    Window(WindowId),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Reading what is running, on a worker; no dialog yet.
    Checking,
    /// The dialog is up and waits for the person.
    Asking,
}

/// The one question Terminal may have open: close this tab, this window, or quit.
#[derive(Debug, Clone, Copy)]
pub struct Ask {
    pub subject: Subject,
    pub stage: Stage,
    /// The check this question is waiting on; an answer from an older one is not this one's.
    pub ticket: u64,
    /// How many shells have something running.
    pub busy: usize,
}

/// The assistant panel: one explanation at a time, and the generation that tells a late answer
/// from a stale one.
#[derive(Debug, Clone, Default)]
pub struct Explaining {
    pub generation: u64,
    pub working: bool,
    pub text: String,
}

pub struct Model {
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// The id the next tab gets.
    pub serial: u64,
    /// Tabs being started on a worker.
    pub spawning: usize,
    /// Shells being ended after a close or a logout, so the end waits for them.
    pub dying: Vec<Arc<Session>>,
    pub rows: u16,
    pub cols: u16,
    pub font_size: i32,
    pub notice: String,

    // ── Find ──
    pub query: String,
    pub matches: Vec<usize>,
    pub match_index: usize,
    pub match_row: i32,

    pub explain: Explaining,
    pub ask: Option<Ask>,
    pub next_ticket: u64,

    // ── What the window last showed, so a frame draws only what moved ──
    pub shown: Option<(u64, u64)>,
    pub signature: Vec<(String, bool, bool)>,
    pub frame_armed: bool,
    pub probe_armed: bool,
    /// A command asked the window to come forward, and how many times.
    pub raises: u64,
    /// The terminal should take the keyboard.
    pub focus: u64,

    // ── The app's life ──
    pub closing: bool,
    pub ending: bool,
    /// The answer the intent being settled is owed, written in the command that settles it.
    pub answer: Value,

    pub hub: Hub,
    /// This copy is drawn for a Mind: terminal contents are hidden and nothing is typed.
    pub for_mind: bool,
    pub mind: Arc<dyn Mind>,
}

impl Model {
    pub fn new(hub: Hub, mind: Arc<dyn Mind>, for_mind: bool) -> Model {
        Model {
            tabs: Vec::new(),
            active: 0,
            serial: 0,
            spawning: 0,
            dying: Vec::new(),
            rows: 24,
            cols: 100,
            font_size: FONT_DEFAULT,
            notice: String::new(),
            query: String::new(),
            matches: Vec::new(),
            match_index: 0,
            match_row: -1,
            explain: Explaining::default(),
            ask: None,
            next_ticket: 0,
            shown: None,
            signature: Vec::new(),
            frame_armed: false,
            probe_armed: false,
            raises: 0,
            focus: 0,
            closing: false,
            ending: false,
            answer: Value::Null,
            hub,
            for_mind,
            mind,
        }
    }

    pub fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn session(&self) -> Option<&Arc<Session>> {
        self.tab().map(|t| &t.session)
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Whether a prompt is open or being prepared.
    pub fn asking(&self) -> bool {
        self.ask.is_some_and(|a| a.stage == Stage::Asking)
    }

    /// Whether the app is on its way out, by a close or by a logout.
    pub fn leaving(&self) -> bool {
        self.closing || self.ending
    }

    /// Whether every shell this app started is gone and none is being started.
    pub fn all_ended(&self) -> bool {
        self.spawning == 0
            && self.tabs.iter().all(|t| !t.session.alive())
            && self.dying.iter().all(|s| !s.alive())
    }

    /// The sentence the window's prompt says, its heading and the confirming button's label.
    pub fn dialog(&self) -> (&'static str, String, &'static str) {
        match self.ask {
            Some(Ask { subject: Subject::Window(_), busy, .. }) => {
                ("Close Terminal?", running_in(busy, " Closing it will end them."), "Close Terminal")
            }
            Some(Ask { subject: Subject::Quit, busy, .. }) => {
                ("Quit Terminal?", running_in(busy, " Quitting will end them."), "Quit Terminal")
            }
            _ => (
                "Close this session?",
                "A process is still running in this tab. Closing it will end that session.".into(),
                "Close session",
            ),
        }
    }
}

fn running_in(busy: usize, then: &str) -> String {
    if busy == 1 {
        format!("A process is still running in one tab.{then}")
    } else {
        format!("Processes are still running in {busy} tabs.{then}")
    }
}

/// The name a tab wears: the folder its shell is in.
pub fn title_of(cwd: &std::path::Path) -> String {
    cwd.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into())
}
