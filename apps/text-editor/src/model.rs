//! What the Text Editor is: the model every command changes and every render reads.
//!
//! Each tab is a view, in the one window, on a document in the framework's `Documents` table: the
//! document owns the text and what is known of its file (its baseline, its save, an outside
//! change, its draft); the [`Tab`] beside it owns what is the tab's (its undo, who opened it, the
//! text its file held). The strip's order is `tabs`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use yantrik_app::documents::disk::{Disk, RealDisk};
use yantrik_app::documents::{Document, DocumentId, Documents, OutsideState};

use crate::claim::Claim;
use crate::notice::Notice;
use crate::owner::TabFacts;
use crate::prompt::Closing;
use crate::tab::{Tab, TabId};
use crate::text::budget::Weighed;
use crate::text::viewport::{view_of, Viewport};

/// How a tab with no file is named.
pub const UNTITLED: &str = "Untitled";
/// How a tab an agent is not shown is named to it, in place of its file's name and path.
pub const HIDDEN_TAB: &str = "(hidden)";

/// The one file operation in flight. The editor reads or writes one file at a time, as it always
/// has, and refuses what would change a tab meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Open,
    Save(TabId),
}

/// The dialog the window shows, apart from the close prompt (prompt.rs), which is drawn from the
/// close flow itself so it can never show with nothing behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    None,
    Open,
    SaveAs,
    Goto,
}

/// The find bar: what it looks for, what replaces it, and what it found in the tab in front.
#[derive(Debug, Clone, Default)]
pub struct Find {
    pub query: String,
    pub replacement: String,
    pub match_case: bool,
    pub matches: Vec<(usize, usize)>,
    /// The match the selection is on, counting from 0.
    pub at: usize,
}

/// Something the window does once, in the render of the command that asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    FocusEditor,
    /// Scroll to the top-left and put the caret at the start.
    ResetPosition,
    Select(usize, usize),
    ShowFind,
    /// The text on screen is put back to the tab's, whatever the editor field holds.
    ResetContent,
    /// A dialog opened: its field is given what the model put there.
    Dialog,
    /// The replace field is given the model's replacement (`replace_text`).
    Replacement,
}

/// How the editor ends once every owed answer has settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// The last window closes.
    Close,
    /// The editor ends, after a linger.
    Exit,
}

/// The font size and render limit the view is windowed for (#328).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub font_px: i32,
    pub limit_px: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { font_px: 14, limit_px: 15000.0 }
    }
}

/// The old build's recovery file, until its tabs' drafts are all the lane's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy {
    /// Nothing to do: none, already set aside, or left as it was (unreadable).
    Done,
    /// Its tabs' drafts not yet on disk, by draft name; set aside once this is empty.
    Waiting(BTreeSet<String>),
    /// Being set aside.
    Archiving,
}

pub struct Model {
    pub documents: Documents<String>,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub busy: Option<Work>,
    /// What could not be done, or what happened that the person must know, in one line; also
    /// `describe.notice`, in its agent-safe form for an agent (notice.rs).
    pub notice: Notice,
    pub dialog: Dialog,
    /// What the dialog's field was given when it opened.
    pub dialog_path: String,
    pub dialog_error: String,
    /// A close or a quit asking about a tab with unsaved changes, or waiting on its save.
    pub closing: Option<Closing>,
    pub find: Find,
    /// The caret's line and column, counting from 1.
    pub cursor: (i32, i32),
    pub layout: Layout,
    /// The footer's word on the drafts: `describe.recovery`.
    pub recovery: String,
    /// Drafts asked for and not yet on disk.
    pub drafts_pending: BTreeSet<String>,
    /// Drafts that could not be written, by name, with the edit they were of: tried again at the
    /// tab's next edit.
    pub draft_failed_at: BTreeMap<String, u64>,
    /// Where the editor keeps its drafts and the old build kept its recovery file.
    pub state_dir: PathBuf,
    pub legacy: Legacy,
    /// Files a launch asked to open, waiting their turn.
    pub queue: VecDeque<PathBuf>,
    /// The window closed with a save in flight: the editor ends when it lands (`fx.linger`).
    pub lingering: bool,
    /// Answers waiting on their read-back job: a close or an exit waits for them (`finish`).
    pub answers_pending: usize,
    /// The window's close, or the editor's end, decided and waiting for those answers.
    pub finish: Option<Finish>,
    /// The window was asked to close while a file was read or written, with other tabs unsaved:
    /// the close goes on once that lands (update/closing.rs).
    pub close_when_done: bool,
    /// Tabs whose re-read, after another program changed their file, was set aside for the save
    /// in flight: the save's landing takes the change up again (update/outside.rs).
    pub reread_owed: BTreeSet<TabId>,
    /// What the last call from the surface is answered with.
    pub answer: Value,
    /// The last tab closed and the file it had, for the answer of the save that closed it.
    pub last_closed: Option<(TabId, PathBuf)>,
    pub cues: Vec<Cue>,
    /// The disk saves write through.
    pub disk: Arc<dyn Disk>,
    next_tab: u64,
    _claim: Claim,
}

impl Model {
    pub fn new(claim: Claim, state_dir: PathBuf) -> Model {
        Model {
            documents: Documents::new(),
            tabs: Vec::new(),
            active: 0,
            busy: None,
            notice: Notice::default(),
            dialog: Dialog::None,
            dialog_path: String::new(),
            dialog_error: String::new(),
            closing: None,
            find: Find::default(),
            cursor: (1, 1),
            layout: Layout::default(),
            recovery: crate::update::drafts::READY.into(),
            drafts_pending: BTreeSet::new(),
            draft_failed_at: BTreeMap::new(),
            state_dir,
            legacy: Legacy::Done,
            queue: VecDeque::new(),
            lingering: false,
            answers_pending: 0,
            finish: None,
            close_when_done: false,
            reread_owed: BTreeSet::new(),
            answer: Value::Null,
            last_closed: None,
            cues: Vec::new(),
            disk: Arc::new(RealDisk),
            next_tab: 1,
            _claim: claim,
        }
    }

    /// A fresh tab id and draft name. The name is the tab's for its life and survives a restart.
    pub fn next_ids(&mut self) -> (TabId, String) {
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        (id, format!("tab-{:x}-{:x}", crate::files::drafts::now_ms(), id.0))
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// The tab in front. There is always one.
    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active.min(self.tabs.len().saturating_sub(1))]
    }

    pub fn doc(&self, tab: &Tab) -> &Document<String> {
        self.documents.get(tab.doc).expect("every tab has its document")
    }

    pub fn doc_mut(&mut self, id: DocumentId) -> Option<&mut Document<String>> {
        self.documents.get_mut(id)
    }

    pub fn text(&self, tab: &Tab) -> &str {
        &self.doc(tab).content
    }

    pub fn active_text(&self) -> &str {
        self.text(self.active_tab())
    }

    /// The tab's file, if it has one.
    pub fn path(&self, tab: &Tab) -> Option<&Path> {
        self.doc(tab).source.path()
    }

    /// The tab's name: its file's name, or Untitled.
    pub fn title(&self, tab: &Tab) -> String {
        self.path(tab)
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| UNTITLED.into())
    }

    /// Whether the tab holds anything its file does not: its text is not what the file held,
    /// it was restored from a draft, its file was moved or deleted and not found again, or another
    /// program changed the file under it: in the last two its text is on no disk at all, whatever
    /// it is, until it is saved somewhere.
    pub fn modified(&self, tab: &Tab) -> bool {
        let doc = self.doc(tab);
        tab.recovered
            || matches!(doc.outside, OutsideState::Deleted | OutsideState::Conflict { .. })
            || doc.content != tab.on_disk.as_deref().unwrap_or_default()
    }

    pub fn tab_by_doc(&self, doc: DocumentId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.doc == doc)
    }

    /// What the budget counts of every tab.
    pub fn weighed(&self) -> Vec<Weighed> {
        self.tabs
            .iter()
            .map(|t| Weighed { text: self.text(t).len(), saved: t.on_disk.as_ref().map_or(0, String::len) })
            .collect()
    }

    /// What the room refusals need of every tab.
    pub fn facts(&self) -> Vec<TabFacts> {
        self.tabs
            .iter()
            .map(|t| TabFacts {
                modified: self.modified(t),
                has_file: self.path(t).is_some(),
                opened_by: t.opened_by.clone(),
                name: if t.hidden { HIDDEN_TAB.into() } else { self.title(t) },
            })
            .collect()
    }

    /// What the renderer may be handed of the tab in front.
    pub fn viewport(&self) -> Viewport {
        view_of(self.active_text(), self.layout.limit_px, self.layout.font_px)
    }

    /// The text on screen: the tab in front, windowed.
    pub fn view_text(&self) -> &str {
        let text = self.active_text();
        &text[..self.viewport().end]
    }

    /// The one tab, blank and the person's, that a file opened replaces.
    pub fn is_blank(&self, tab: &Tab) -> bool {
        self.path(tab).is_none() && !self.modified(tab)
    }
}
