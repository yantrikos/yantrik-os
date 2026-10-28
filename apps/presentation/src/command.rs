//! Every way the model changes: what the window, the surface, the framework and the jobs ask.

use std::path::PathBuf;

use yantrik_app::documents::Observed;

use crate::caller::Caller;
use crate::deck::{Content, Edits};
use crate::files::open::Opened;
use crate::files::save::{SaveFailed, Wrote};
use crate::model::NotesAsk;

#[derive(Clone)]
pub enum Command {
    /// One of the window's buttons or keys, with what the canvas held when it was pressed: the
    /// model is told what the window shows, and never reads the window itself.
    Window { action: WindowAction, canvas: Edits },
    /// The person typed in the canvas; it already shows this text.
    Canvas(Edits),
    /// A call from the surface, with who made it.
    Surface { act: Act, caller: Caller },
    /// The first command, once the window is drawn: the recovered deck's file watched, the old
    /// build's draft handed to the lane.
    Started,
    /// A file a launch asked to open, the first launch's included.
    Launch(PathBuf),
    /// The person closed the window.
    CloseRequested,
    /// A person asked yPresent to quit (a Quit menu): the same as closing its window.
    Quit,
    /// The session or the system is ending yPresent, with no one to ask.
    LoggingOut,
    /// A second launch brought the window back, during a linger or not.
    Presented,
    /// The presenter's clock ticked.
    Tick,
    DraftSaved(String),
    DraftNotSaved { name: String, why: String },
    /// The watch saw the open deck's file change.
    FileChanged { key: String, seen: Observed },
    // What the jobs found.
    Opened { then: Cause, result: Result<Box<Opened>, String> },
    SaveLanded { seq: u64, then: AfterSave, written: Box<Content>, result: Result<Box<Wrote>, Box<SaveFailed>> },
    Exported { then: Cause, slides: usize, remember: bool, result: Result<Box<Wrote>, Box<SaveFailed>> },
    /// The deck, with nothing unsaved, read again after another program changed its file. `from`
    /// is the file that was read; a result for a file no longer open is dropped.
    Reread { from: PathBuf, result: Result<Box<Opened>, String> },
    /// Where the deck's file went after it was found gone; `None` when nothing was found.
    Located { from: PathBuf, found: Option<Box<Opened>> },
    /// The old build's recovery file was set aside once its draft was the lane's.
    LegacyArchived(Result<(), String>),
    /// The Mind's answer to a request for speaker notes, with the slide it was asked about.
    NotesWritten { ask: NotesAsk, result: Result<String, String> },
    /// The disk saves write through, for a test to make a step fail.
    #[cfg(test)]
    UseDisk(std::sync::Arc<dyn yantrik_app::documents::disk::Disk>),
    /// The folder a deck with no name is saved in, for a test to keep it out of the home.
    #[cfg(test)]
    UseDeckDir(PathBuf),
}

/// The window's actions: its header, its keys and its buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowAction {
    AddSlide,
    DeleteSlide,
    DuplicateSlide,
    MoveUp,
    MoveDown,
    Select(i64),
    Next,
    Previous,
    First,
    Last,
    Escape,
    SetLayout(i32),
    SetTheme(i32),
    OpenGallery,
    TogglePresent,
    Save,
    /// The ribbon's Open: the deck most recently worked on.
    OpenRecent,
    ExportMarkdown,
    ExportOutline,
    Search(String),
    SearchNext,
    Undo,
    Redo,
    GenerateNotes,
    ToggleTimer,
    ResetTimer,
}

/// A slide number out of a call's arguments, or why it is not one.
pub type Index = Result<usize, String>;

/// A call from the surface, its arguments read.
#[derive(Debug, Clone)]
pub enum Act {
    Open { path: String },
    Show,
    Save,
    SaveAs { path: String, overwrite: bool },
    NewDeck { title: Option<String> },
    AddSlide { title: Option<String>, body: Option<String>, notes: Option<String>, after: Option<Index> },
    SetSlide { index: Index, title: Option<String>, body: Option<String>, notes: Option<String> },
    DeleteSlide { index: Index },
    MoveSlide { from: Index, to: Index },
    GoTo { index: Index },
    Next,
    Previous,
    PresenterView { mode: String },
    ExportMarkdown { path: Option<String> },
}

impl Act {
    /// The action's published name, for the sentences that name it.
    pub fn name(&self) -> &'static str {
        match self {
            Act::Open { .. } => "open",
            Act::Show => "show",
            Act::Save => "save",
            Act::SaveAs { .. } => "save_as",
            Act::NewDeck { .. } => "new_deck",
            Act::AddSlide { .. } => "add_slide",
            Act::SetSlide { .. } => "set_slide",
            Act::DeleteSlide { .. } => "delete_slide",
            Act::MoveSlide { .. } => "move_slide",
            Act::GoTo { .. } => "go_to",
            Act::Next => "next",
            Act::Previous => "previous",
            Act::PresenterView { .. } => "presenter_view",
            Act::ExportMarkdown { .. } => "export_markdown",
        }
    }
}

/// Who asked for a deck to be put in front, and so what is answered when it is.
#[derive(Debug, Clone)]
pub enum Cause {
    /// The Open button.
    Window,
    /// A launch's file.
    Launch,
    /// The surface's `open` or `new_deck`, answered from what it did.
    Surface(Caller),
}

/// What a deck put in front is: a file just read, or a new empty one.
#[derive(Debug, Clone)]
pub enum Next {
    File(Box<Opened>),
    Blank(String),
}

/// Why the window's Save wrote a new file instead of the deck's own: the window has no Save As, so
/// a deck whose file is gone, or was changed by another program, is kept beside it instead of
/// being written over it or left unsaved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Away {
    /// The file was moved or deleted.
    Gone,
    /// Another program changed the file.
    Changed,
    /// The deck came back from an older build's recovery file, which never kept what its file
    /// held, so nothing says the file is still what the deck was typed against.
    Unchecked,
}

/// What a save does once it lands.
#[derive(Debug, Clone)]
pub enum AfterSave {
    /// The window's Save, and why it went to a file of its own if it did.
    Window(Option<Away>),
    /// The surface's `save`.
    Save(Caller),
    /// The surface's `save_as`.
    SaveAs(Caller),
    /// The deck being replaced, written to a file of its own first so replacing it loses nothing.
    Rescue { next: Next, cause: Cause },
}
