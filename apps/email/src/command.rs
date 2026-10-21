//! Every change to Email's model, wherever it came from: a button, an intent, a timer or a job's
//! result. A button and the action of the same name raise the same command.

use yantrik_ipc_contracts::email::{EmailAccountSummary, EmailDetail, EmailSummary, OAuthBeginResult, TestAccountResult};

use yantrik_app::app::WindowId;

use crate::backend::look::Loaded;
use crate::model::FileOp;
use crate::state::{Draft, GoogleOutcome};

/// Why the mailbox is being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// The first look, at start: the sync line says only what it found.
    Start,
    /// Sync, pressed or raised after a sign-in: the sync line counts what came.
    Sync,
}

/// Why a folder is being listed, which decides what its landing answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    /// A folder chosen in the sidebar or by `select_folder`.
    Folder,
    /// A search emptied: the folder comes back, and `search` answers with it.
    SearchCleared,
    /// After a send, so the Sent mail shows; nothing waits on it.
    AfterSend,
}

/// Which message a command is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// The id a list row or the reading pane's toolbar sent: the row's place in the listing, or
    /// [`crate::model::OPEN_MESSAGE`].
    Key(i32),
    /// What a caller named: a row number from `describe.messages`, or part of a subject or
    /// sender. Empty means the message that is open.
    Named(String),
}

/// One message, as a command acts on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: String,
    /// The folder it was listed from.
    pub folder: String,
    pub subject: String,
    pub was_read: bool,
    pub flagged: bool,
    /// Its place among the rows on screen, when a caller named it by one.
    pub shown: Option<usize>,
}

/// Which flag a change sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Read,
    Starred,
}

/// What takes a message out of the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    Delete,
    /// Moved to this folder: the archive.
    Move(String),
}

/// The four things the companion is asked about mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Summarize,
    SuggestReply,
    /// A message drafted from the person's instruction.
    Draft(String),
    /// The composer's body rewritten in this tone.
    Enhance(String),
    Classify,
}

/// What is ending the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// The session: a logout, SIGTERM or SIGHUP, the display going.
    Logout,
    /// A person's quit.
    Quit,
    /// The window was closed.
    Window(WindowId),
}

/// A job that ended without its result: it panicked, or the pool was full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Sync,
    Listing,
    Open,
    Change,
    Send,
    Draft,
    Ai,
    Setup,
    Google,
    Original,
}

/// What the setup form holds when a button is pressed: the eight fields, password included.
///
/// It lives only as long as the command: the settings are built from it and handed to a job,
/// and it is never kept in the model. Its `Debug` is written by hand so the password cannot reach
/// a log line or a panic message.
#[derive(Clone, Default)]
pub struct SetupForm {
    pub email: String,
    pub password: String,
    pub display_name: String,
    pub provider: String,
    pub imap_server: String,
    pub imap_port: String,
    pub smtp_server: String,
    pub smtp_port: String,
}

impl std::fmt::Debug for SetupForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupForm")
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .field("display_name", &self.display_name)
            .field("provider", &self.provider)
            .field("imap_server", &self.imap_server)
            .field("imap_port", &self.imap_port)
            .field("smtp_server", &self.smtp_server)
            .field("smtp_port", &self.smtp_port)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    // ── The mailbox ──
    Look(Look),
    Looked { why: Look, folder: String, loaded: Loaded },
    /// A folder of the sidebar, by its place there.
    FolderClicked(i32),
    /// A folder by name or kind (`select_folder`).
    SelectFolder(String),
    Listed { why: Listing, folder: String, outcome: Result<Vec<EmailSummary>, String> },
    /// The search box submitted.
    Search(String),
    /// `search`: the box is set to the query, then searched.
    SearchFor(String),
    Searched { query: String, outcome: Result<Vec<EmailSummary>, String> },
    Triage(i32),
    // ── A message ──
    Open(Pick),
    Fetched { target: Target, outcome: Result<EmailDetail, String> },
    MarkedOnOpen { id: String, folder: String, outcome: Result<(), String> },
    /// Mark read (`true`), or flag: the toolbar's star turns the flag round, `flag` sets it.
    Mark { pick: Pick, flag: Flag, toggle: bool },
    /// `read` is the folder listed after the change: the message's own.
    Marked {
        target: Target,
        flag: Flag,
        wanted: bool,
        call: Result<(), String>,
        read: String,
        listed: Option<Result<Vec<EmailSummary>, String>>,
    },
    Delete(i32),
    Archive(i32),
    Removed {
        target: Target,
        how: Removal,
        call: Result<(), String>,
        read: String,
        listed: Option<Result<Vec<EmailSummary>, String>>,
    },
    ToggleThread(i32),
    OpenOriginal,
    /// The sentence the notice says about it, whichever way it went.
    OriginalOpened(String),
    // ── Writing ──
    ComposeNew,
    /// `compose`: the composer opened as Compose opens it, then these three filled in.
    Compose { to: String, subject: String, body: String },
    Reply { all: bool },
    Forward,
    /// The window's own edits: the composer and the search box.
    Edited { text: Draft, query: String },
    Send(Draft),
    Sent(Result<(), String>),
    CancelCompose(Draft),
    SaveDraft(Draft),
    /// At start, before any press: text the lanes held at the last close becomes the draft file.
    OfferUnsent,
    /// The app is ending: by the session, by a person's quit, or by its window closing. The
    /// composer's text is kept, and the app ends.
    Ending(Ending),
    /// A write or a removal of the draft file landed, or did not.
    DraftDone { op: FileOp, outcome: Result<(), String> },
    // ── The companion ──
    Ask(Ask),
    /// What the companion said, about the message that was open when it was asked.
    Answered { ask: Ask, about: Option<(String, String)>, outcome: Result<String, String> },
    // ── Account setup ──
    AddAccount,
    CancelSetup,
    TestConnection(SetupForm),
    SaveAccount(SetupForm),
    Tested(Result<TestAccountResult, String>),
    Saved(Result<EmailAccountSummary, String>),
    SignInWithGoogle,
    CancelGoogle,
    GoogleBegun { begun: Result<OAuthBeginResult, String>, opened: Option<Result<(), String>> },
    GooglePoll,
    GoogleStatus { flow: String, outcome: GoogleOutcome },
    /// The mail service told to give a sign-in up, or not.
    GoogleCancelled(Result<(), String>),
    // ── A job that did not come back ──
    Unfinished(Work),
}
