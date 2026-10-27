//! Every way the model changes: what the window, the surface, the framework and the jobs ask.

use std::path::PathBuf;

use yantrik_app::documents::Observed;
use yantrik_app::rail::{ContextItem, RailCommand};

use crate::caller::Caller;
use crate::files::open::Opened;
use crate::files::save::{SaveFailed, Wrote};
use crate::markdown::Format;

#[derive(Clone)]
pub enum Command {
    /// One of the window's buttons or keys, or its dialog's answers.
    Window(WindowAction),
    /// The person typed in the page; the page already shows this text.
    Edit(String),
    /// The find field changed.
    FindChanged(String),
    /// A formatting button, with the selection the window held when it was pressed.
    Format { what: Format, anchor: usize, cursor: usize },
    /// The link button, with the address typed and the selection.
    Link { url: String, anchor: usize, cursor: usize },
    /// An outline row pressed: the caret goes to this byte offset.
    Goto(usize),
    /// A call from the surface, with who made it.
    Surface { act: Act, caller: Caller },
    /// The AI panel or the rail asked about the document: a suggestion's id, or `ask:` and the
    /// person's own words.
    Ask(String),
    /// The rail's own commands.
    Rail(RailCommand),
    /// The document's title has stood still for a beat: the Mind is asked what it remembers of it.
    MemoryDue,
    /// What the Mind remembered about `query`.
    Remembered { query: String, rows: Vec<ContextItem> },
    /// The first command, once the window is drawn: the recovered document's file watched, the
    /// old build's draft handed to the lane.
    Started,
    /// A file a launch asked to open, the first launch's included.
    Launch(PathBuf),
    /// The person closed the window.
    CloseRequested,
    /// A person asked yDoc to quit (a Quit menu): the same as closing its window.
    Quit,
    /// The session or the system is ending yDoc, with no one to ask.
    LoggingOut,
    /// A second launch brought the window back, during a linger or not.
    Presented,
    DraftSaved(String),
    DraftNotSaved { name: String, why: String },
    /// The watch saw the open document's file change.
    FileChanged { key: String, seen: Observed },
    // What the jobs found.
    Opened { then: AfterOpen, result: Result<Opened, String> },
    SaveLanded { seq: u64, then: AfterSave, result: Result<Box<Wrote>, Box<SaveFailed>> },
    Exported { then: AfterExport, html: bool, result: Result<Box<Wrote>, Box<SaveFailed>> },
    /// The document, with nothing unsaved, read again after another program changed its file.
    /// `from` is the file that was read; a result for a file no longer open is dropped.
    Reread { from: PathBuf, result: Result<Opened, String> },
    /// Where the document's file went after it was found gone; `None` when nothing was found.
    Located { from: PathBuf, found: Option<Opened> },
    /// The old build's recovery file was set aside once its draft was the lane's.
    LegacyArchived(Result<(), String>),
    /// The disk saves write through, for a test to make a step fail.
    #[cfg(test)]
    UseDisk(std::sync::Arc<dyn yantrik_app::documents::disk::Disk>),
}

/// The window's actions: its header, its keys, its buttons and its dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowAction {
    New,
    Open,
    Save,
    SaveAs,
    ExportMarkdown,
    ExportHtml,
    /// The dialog's button, with what its field holds. Also the unsaved prompt's *Save*.
    Confirm { path: String },
    Cancel,
    /// The unsaved prompt's *Discard*.
    Discard,
    FindNext,
    FindPrev,
    ReplaceOne { with: String },
    ReplaceAll { with: String },
}

impl WindowAction {
    /// Whether the window takes this while a dialog is up: the answers a dialog asks for.
    pub fn answers_a_dialog(&self) -> bool {
        matches!(self, WindowAction::Cancel | WindowAction::Confirm { .. } | WindowAction::Discard)
    }
}

/// A call from the surface, its arguments read.
#[derive(Debug, Clone)]
pub enum Act {
    Open { path: String, discard: bool },
    New,
    Save,
    SaveAs { path: String, overwrite: bool },
    SetContent { text: String },
    Append { text: String },
    Find { query: String },
    ReplaceAll { find: String, with: String },
    ExportMarkdown { path: String },
    Show,
}

impl Act {
    /// The action's published name, for the sentences that name it.
    pub fn name(&self) -> &'static str {
        match self {
            Act::Open { .. } => "open",
            Act::New => "new",
            Act::Save => "save",
            Act::SaveAs { .. } => "save_as",
            Act::SetContent { .. } => "set_content",
            Act::Append { .. } => "append",
            Act::Find { .. } => "find",
            Act::ReplaceAll { .. } => "replace_all",
            Act::ExportMarkdown { .. } => "export_markdown",
            Act::Show => "show",
        }
    }
}

/// What an open does once it lands.
#[derive(Debug, Clone)]
pub enum AfterOpen {
    /// The Open dialog's: a failure stays in the dialog.
    Dialog,
    /// A launch's file.
    Launch,
    /// The unsaved prompt's answer: the file the person asked for before it was shown.
    Prompted,
    /// The surface's `open`, answered from what it opened.
    Surface(Caller),
}

/// What a save does once it lands.
#[derive(Debug, Clone)]
pub enum AfterSave {
    /// The window's Save or Save As, or the prompt's *Save*.
    Window,
    /// The surface's `save`.
    Save(Caller),
    /// The surface's `save_as`.
    SaveAs(Caller),
}

/// What an export does once it lands.
#[derive(Debug, Clone)]
pub enum AfterExport {
    /// The window's Export dialog: a failure stays in the dialog.
    Window,
    /// The surface's `export_markdown`.
    Surface(Caller),
}
