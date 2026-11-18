//! Every way the model changes: what the window, the surface, the framework and the jobs ask.

use std::path::PathBuf;

use serde_json::Value;
use yantrik_app::documents::Observed;

use crate::caller::Caller;
use crate::files::open::Opened;
use crate::files::save::{SaveFailed, Wrote};
use crate::tab::TabId;

#[derive(Clone)]
pub enum Command {
    /// One of the window's actions, by the id its button or key sends.
    Window(WindowAction),
    /// The person typed in the editor; the editor already shows this text.
    Edit(String),
    /// The caret moved to this byte offset of the text on screen.
    Cursor(i32),
    /// The find field, or its match-case box, changed.
    Find { query: String, match_case: bool },
    SelectTab(TabId),
    CloseTab(TabId),
    /// The font size or the render limit changed: the document is windowed again (#328).
    Reflow { font_px: i32, limit_px: f32 },
    /// A call from the surface, with who made it.
    Surface { act: Act, caller: Caller },
    /// The first command, once the window is drawn: the recovered tabs' files watched, the old
    /// build's drafts handed to the lane.
    Started,
    /// Files a launch asked to open, the first launch's included.
    Launch(Vec<PathBuf>),
    /// The person closed the window.
    CloseRequested,
    /// A person asked the editor to quit (a Quit menu): the same as closing its window.
    Quit,
    /// The session or the system is ending the editor, with no one to ask.
    LoggingOut,
    /// A second launch brought the window back, during a linger or not.
    Presented,
    DraftSaved(String),
    DraftNotSaved { name: String, why: String },
    /// The watch saw an open tab's file change.
    FileChanged { key: String, seen: Observed },
    // What the jobs found.
    Opened { then: AfterOpen, result: Result<Opened, String> },
    SaveLanded { tab: TabId, seq: u64, then: AfterSave, result: Result<Wrote, Box<SaveFailed>> },
    /// A tab with nothing unsaved, read again after another program changed its file.
    Reread { tab: TabId, seq: u64, result: Result<Opened, String> },
    /// Where a tab's file went after it was found gone; `None` when nothing was found.
    Located { tab: TabId, from: PathBuf, found: Option<Opened> },
    /// An answer to a call, read back off the disk: refused with `failed` (or the notice) when
    /// the disk did not hold what the call was owed.
    Answered { value: Value, failed: Option<String>, agent: bool },
    /// The old build's recovery file was set aside once its drafts were the lane's.
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
    SaveAs,
    Save,
    /// The close prompt's Save: write the tab, then close it.
    SaveClose,
    Goto,
    /// The dialog's Open, Save or Go, with what its field holds.
    Confirm { path: String },
    Cancel,
    /// The close prompt's Discard.
    Discard,
    Close,
    NextTab,
    PreviousTab,
    FindNext,
    FindPrev,
    Replace { with: String },
    ReplaceAll { with: String },
    Undo,
    Redo,
}

impl WindowAction {
    /// The action an id names. `path` and `replacement` read the dialog's and the replace
    /// field's text, for the actions that need them. `None` for an id the window does not send.
    pub fn parse(id: &str, path: impl FnOnce() -> String, replacement: impl FnOnce() -> String) -> Option<WindowAction> {
        Some(match id {
            "new" => WindowAction::New,
            "open" => WindowAction::Open,
            "save-as" => WindowAction::SaveAs,
            "save" => WindowAction::Save,
            "save-close" => WindowAction::SaveClose,
            "goto" => WindowAction::Goto,
            "confirm" => WindowAction::Confirm { path: path() },
            "cancel" => WindowAction::Cancel,
            "discard" => WindowAction::Discard,
            "close" => WindowAction::Close,
            "next-tab" => WindowAction::NextTab,
            "previous-tab" => WindowAction::PreviousTab,
            "find-next" => WindowAction::FindNext,
            "find-prev" => WindowAction::FindPrev,
            "replace" => WindowAction::Replace { with: replacement() },
            "replace-all" => WindowAction::ReplaceAll { with: replacement() },
            "undo" => WindowAction::Undo,
            "redo" => WindowAction::Redo,
            _ => return None,
        })
    }

    /// Whether the window takes this while a dialog is up: the answers a dialog asks for.
    pub fn answers_a_dialog(&self) -> bool {
        matches!(
            self,
            WindowAction::Cancel
                | WindowAction::Confirm { .. }
                | WindowAction::Discard
                | WindowAction::SaveClose
                | WindowAction::SaveAs
        )
    }
}

/// A call from the surface, its arguments read.
#[derive(Debug, Clone)]
pub enum Act {
    New { text: String },
    Open { path: String },
    Read { from_line: usize, tab: Option<i64> },
    Save,
    SaveAs { path: String, overwrite: bool },
    Show,
    Close,
    Cancel,
    Discard,
    Find { text: String },
    FindNext,
    FindPrev,
    ReplaceText { text: String },
    Replace,
    ReplaceAll,
    SetContent { text: String },
    Append { text: String },
    Undo,
    Redo,
    SelectTab { index: i64 },
    /// An argument given empty: refused by name, in the window's notice too.
    Refuse(String),
}

impl Act {
    /// The action's published name, for the sentences that name it.
    pub fn name(&self) -> &'static str {
        match self {
            Act::New { .. } => "new",
            Act::Open { .. } => "open",
            Act::Read { .. } => "read",
            Act::Save => "save",
            Act::SaveAs { .. } => "save_as",
            Act::Show => "show",
            Act::Close => "close",
            Act::Cancel => "cancel",
            Act::Discard => "discard",
            Act::Find { .. } => "find",
            Act::FindNext => "find-next",
            Act::FindPrev => "find-prev",
            Act::ReplaceText { .. } => "replace_text",
            Act::Replace => "replace",
            Act::ReplaceAll => "replace-all",
            Act::SetContent { .. } => "set_content",
            Act::Append { .. } => "append",
            Act::Undo => "undo",
            Act::Redo => "redo",
            Act::SelectTab { .. } => "select_tab",
            Act::Refuse(_) => "refused",
        }
    }
}

/// What an open does once it lands.
#[derive(Debug, Clone)]
pub enum AfterOpen {
    /// The Open dialog's: a failure stays in the dialog.
    Dialog,
    /// A launch's file: the next one waiting opens after it.
    Launch,
    /// The surface's `open`, answered from what it opened.
    Surface(Caller),
}

/// What a save does once it lands.
#[derive(Debug, Clone)]
pub enum AfterSave {
    /// The window's Save or Save As.
    Window,
    /// The surface's `save`; `closing` when it answered the close prompt.
    Save { caller: Caller, closing: bool },
    /// The surface's `save_as` to this path.
    SaveAs { caller: Caller, path: PathBuf },
}
