//! Every change to the model, wherever it came from: a button, the Mind, a launch, a job's result.
//!
//! Plain data, `Clone` and `Send`. Nothing here derives `Debug`: a command may carry a snippet's
//! code, and the code is never printed.

use std::path::PathBuf;

use serde_json::Value;
use yantrik_app::app::WindowId;
use yantrik_app::rail::RailCommand;

use crate::lane::{Landed, Ticket};
use crate::model::{Editor, Restore};
use crate::store::file::Incoming;
use crate::store::{Library, Patch};

#[derive(Clone)]
pub enum Command {
    // ── The window ──
    Search(String),
    TagFilter(String),
    Select(i32),
    New,
    /// The Save button, with the four fields as it sent them.
    Save(Editor),
    Delete(i32),
    Copy(i32),
    ToggleFavorite(i32),
    SetLanguage(String),
    /// The person typed into the editor.
    Edited(Editor),
    CollectionSelect(i32),
    CollectionCreate(String),
    CollectionRename(i32, String),
    CollectionDelete(i32),
    ExportAll,
    /// The header's AI and the pane's AI Explain: the rail's one question.
    AiExplain,
    /// A row of the rail's context pressed: the snippet it names.
    ContextActivated(String),
    Rail(RailCommand),

    // ── The Mind and `yos`, by name ──
    Add {
        title: String,
        language: String,
        code: String,
        tags: String,
    },
    Change {
        id: String,
        patch: Patch,
        collection: Option<String>,
    },
    Open(String),
    Find(String),
    CopyNamed(String),
    DeleteNamed(String),
    FavoriteNamed(String),
    Import { path: PathBuf, from: From },
    Show,

    // ── A launch's files ──
    Launch(Vec<PathBuf>),

    // ── What jobs brought back ──
    Written(Box<Written>),
    Copied(Box<Copied>),
    Read(Box<Read>),
    Exported(Result<(PathBuf, usize), String>),
    /// A job ended without its result: it panicked, or the pool could not run it.
    WorkFailed(Box<Work>),
    /// The library read again after a write that did not finish.
    Reloaded(Result<Library, String>),
    CopyFeedbackOver,

    // ── The app's life ──
    /// The last window closed with work in flight or typing unsaved.
    Closing(WindowId),
    /// A logout, a signal, the display gone, or a person's quit.
    Ending,
    /// A second launch opened the window again while a close waited.
    Reopened,
    /// The draft lane could not write a draft.
    DraftNotKept { name: String, why: String },
    /// The recovery bar's choices about the text it offers.
    ShowOffer,
    RestoreOffer,
    DiscardOffer,
}

/// Who asked for an import: the Mind or `yos` by name, or a file on the command line.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum From {
    Surface,
    Launch,
}

/// What a change does once its write has landed.
#[derive(Clone)]
pub enum After {
    /// Clear the notice, and answer the call this change runs under with this.
    Answer(Value),
    /// Say this on the notice line.
    Say(String),
    /// A copy's use count: the code is on the clipboard already, so the call is answered either
    /// way, with `saved` or with `unsaved`. `from_window`: the button said it on the notice line.
    Counted { saved: Value, unsaved: Value, from_window: bool },
}

/// A write, as its job carries it there and back.
#[derive(Clone)]
pub struct Write {
    pub ticket: Ticket,
    pub after: After,
    pub restore: Restore,
}

#[derive(Clone)]
pub struct Written {
    pub write: Write,
    pub landed: Landed,
}

/// What putting a snippet on the clipboard came to.
#[derive(Clone)]
pub struct Copied {
    /// The snippet as it was when the copy began. Its code went to the clipboard and is not
    /// carried back, only how long it was.
    pub id: i32,
    pub title: String,
    pub bytes: usize,
    pub use_count: i32,
    pub result: Result<(), String>,
    pub from_window: bool,
}

/// A file read for an import.
#[derive(Clone)]
pub struct Read {
    pub path: PathBuf,
    pub from: From,
    pub result: Result<Incoming, String>,
    /// The Mind named a file that is not there: refused, and not said on the notice line.
    pub missing: bool,
    /// The collection selected when the import was asked for: what comes in is filed there.
    pub collection: i32,
}

/// The work a job was doing when it ended without a result.
#[derive(Clone)]
pub enum Work {
    Write(Box<Write>),
    Copy { from_window: bool },
    Import,
    Export,
    Reload,
}
