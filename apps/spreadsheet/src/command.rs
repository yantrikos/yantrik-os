//! Every way the model changes: what the window, the surface, the framework and the jobs ask.

use std::path::PathBuf;

use yantrik_app::documents::Observed;

use crate::address::{Cell, Range};
use crate::caller::Caller;
use crate::files::open::OpenedSheet;
use crate::files::picker::Choice;
use crate::files::save::{Refusal, Saved};
use crate::history::Mark;
use crate::sheet::Solved;

#[derive(Clone)]
pub enum Command {
    /// A cell was clicked, with what the formula bar held: the model is told what the window
    /// shows, and never reads the window itself.
    Cell { row: i32, col: i32, typed: String },
    /// Enter, Tab or an arrow in the formula bar: the typed text, and which way to go.
    Formula { typed: String, direction: String },
    /// Escape in the formula bar: it shows the cell again.
    Cancelled,
    /// One of the header's buttons or keys, with the formula bar as it was pressed.
    Header { id: String, typed: String },
    /// The dialog's main button, with what its field holds. Also the question's *Save*.
    Confirm { path: String },
    /// The dialog's or the question's *Cancel*.
    Cancel,
    /// The question's *Discard*.
    Discard,
    /// A Mind View window was typed into: the notice, and the mask put back in the formula bar,
    /// before the typing is refused.
    Masked,
    /// A call from the surface, with who made it.
    Surface { act: Act, caller: Caller },
    /// The first command, once the window is drawn: the recovered sheet's file watched.
    Started,
    /// A file a launch asked to open, the first launch's included.
    Launch(PathBuf),
    /// The person closed the window.
    CloseRequested,
    /// A person asked ySheets to quit (a Quit menu): the same as closing its window.
    Quit,
    /// The session or the system is ending ySheets, with no one to ask.
    LoggingOut,
    /// A second launch brought the window back, during a linger or not.
    Presented,
    DraftSaved(String),
    DraftNotSaved { name: String, why: String },
    /// The sheet has stood still for a beat: its draft is encoded now.
    DraftDue,
    /// A draft was encoded off the window's thread, for the document's `seq` it was of.
    DraftEncoded { name: String, seq: u64, bytes: Option<Vec<u8>> },
    /// The watch saw the open sheet's file change.
    FileChanged { key: String, seen: Observed },
    // What the jobs found.
    Opened { then: Cause, result: Result<Box<OpenedSheet>, String> },
    /// The desktop's picker answered, and the place to suggest if it could not be asked.
    Picked { saving: bool, choice: Choice, suggested: PathBuf },
    /// A save landed or did not.
    SaveLanded { write: Box<Write>, then: AfterSave, result: Result<Box<Saved>, Box<Refusal>> },
    /// The sheet was worked out for one revision; `changed` is the cells that show something
    /// else from what they showed when the job began, `None` for all of them.
    Solved { solved: Box<Solved>, changed: Option<Vec<usize>>, then: Option<Reply> },
    /// A job working the sheet out ended without an answer.
    SolveLost { rev: u64, then: Option<Reply> },
    /// The sheet, with nothing unsaved, read again after another program changed its file.
    /// `from` is the file that was read; a result for a file no longer open is dropped.
    Reread { from: PathBuf, result: Result<Box<OpenedSheet>, String> },
    /// Where the sheet's file went after it was found gone; `None` when nothing was found.
    Located { from: PathBuf, found: Option<Box<OpenedSheet>> },
    /// The disk saves write through, for a test to make a step fail.
    #[cfg(test)]
    UseDisk(std::sync::Arc<dyn yantrik_app::documents::disk::Disk>),
    /// Who is asked for a file, for a test to stand in for the desktop's picker.
    #[cfg(test)]
    UseAsk(crate::files::picker::Ask),
}

/// What a save was of, kept with it until it lands: the document's `seq` it was for, the state of
/// the sheet it wrote, where, and how many rows and columns of text that was.
#[derive(Debug, Clone)]
pub struct Write {
    pub seq: u64,
    pub mark: Mark,
    pub target: PathBuf,
    pub shape: (usize, usize),
}

/// A call from the surface, its arguments read.
#[derive(Debug, Clone)]
pub enum Act {
    SetCell { cell: String, text: String },
    GetRange { range: String },
    Select { cell: String },
    Undo,
    Redo,
    Open { path: String },
    Save { path: Option<String> },
    SaveOver { path: Option<String> },
}

impl Act {
    /// The action's published name, for the sentences that name it.
    pub fn name(&self) -> &'static str {
        match self {
            Act::SetCell { .. } => "set_cell",
            Act::GetRange { .. } => "get_range",
            Act::Select { .. } => "select",
            Act::Undo => "undo",
            Act::Redo => "redo",
            Act::Open { .. } => "open",
            Act::Save { .. } => "save",
            Act::SaveOver { .. } => "save_over",
        }
    }
}

/// Who asked for a sheet to be put in front, and so what is answered when it is.
#[derive(Debug, Clone)]
pub enum Cause {
    /// The window: its Open, the picker's answer or the typed path.
    Window,
    /// A launch's file.
    Launch,
    /// The surface's `open`, answered from what it opened.
    Surface(Caller),
}

/// What a save does once it lands.
#[derive(Debug, Clone)]
pub enum AfterSave {
    /// The window's Save or Save As, and whether the question's *Save* asked for it: its landing
    /// carries out what the question stood in front of.
    Window { prompted: bool },
    /// The surface's `save` or `save_over`.
    Surface(Caller),
}

/// What the sheet looked like when a call's own edit was made: the facts its answer carries,
/// taken at that moment and not read again when the answer is built.
#[derive(Debug, Clone, Copy)]
pub struct Facts {
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub rows: usize,
    pub cols: usize,
}

/// What a call that needed the sheet worked out is answered with, built from the result of the
/// job made for it.
#[derive(Debug, Clone)]
pub struct Reply {
    pub kind: ReplyKind,
    pub facts: Facts,
}

#[derive(Debug, Clone)]
pub enum ReplyKind {
    SetCell { cell: Cell, before: String, typed: String },
    Range(Range),
    Select(Cell),
    Undo(Cell),
    Redo(Cell),
}
