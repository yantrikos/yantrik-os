//! What Email holds: the mailbox as the service last described it, the message open, the
//! composer, the setup form, and what the window says about each. Plain Rust: the window draws
//! it (`view`), the Mind reads it (`describe`), and only `update` changes it.

use std::sync::Arc;

use serde_json::Value;
use yantrik_app::mind::Mind;
use yantrik_ipc_contracts::email::EmailFolder;

use crate::backend::MailService;
use crate::files::Files;
use crate::state::{Counted, Draft, MailState, MessageRow, Triage};

/// The parts of the model `render` draws separately. The composer's text, the search box and
/// the setup fields are the window's own while the person types, so they are written only when
/// a command put something there.
pub mod part {
    /// What the service said: reachable, an account, Google sign-in, the storage note.
    pub const STATE: &str = "state";
    pub const FOLDERS: &str = "folders";
    /// The message list, the triage tab, and the header's counts.
    pub const LIST: &str = "list";
    /// The reading pane: the message, its attachments, its thread.
    pub const OPEN: &str = "open";
    /// The notice, the sync line, the busy flags, the composer's state and the search's.
    pub const STATUS: &str = "status";
    /// The composer's five fields, when a command filled them.
    pub const COMPOSE_TEXT: &str = "compose-text";
    /// The search box, when a command set it.
    pub const SEARCH_TEXT: &str = "search-text";
    /// The setup form's state and the Google sign-in's.
    pub const SETUP: &str = "setup";
    /// The setup form's fields, cleared as `Setup::clear` says.
    pub const SETUP_FIELDS: &str = "setup-fields";
    /// Only what `describe` reads changed; nothing on screen does.
    pub const MODEL: &str = "model";
}

/// The id the reading pane's buttons send: "the message that is open". The list's rows send
/// their own place in the listing, from 0, so the two never meet; and a row id is never used for
/// the open message, which may have left the list since it was opened.
pub const OPEN_MESSAGE: i32 = i32::MAX;

/// A colour, as three bytes. The view makes it a Slint colour.
pub type Rgb = (u8, u8, u8);

/// One folder of the sidebar.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderRow {
    /// The server's name: the folder's id for selection, moves and counts.
    pub name: String,
    pub label: String,
    pub icon: String,
    pub kind: String,
    /// The server's numbers, or `None` when it did not count this folder.
    pub counts: Option<crate::state::FolderCounts>,
    /// Why there are no numbers, as the server's list said it; read only when there are none.
    pub reason: String,
    /// Highlighted in the sidebar. Set when the mailbox is read, as it always was.
    pub selected: bool,
}

impl FolderRow {
    /// The row as the wire had it: counted, or uncounted with its reason.
    pub fn record(&self) -> EmailFolder {
        match self.counts {
            Some(c) => EmailFolder::counted(self.name.clone(), c.unread, c.total),
            None => EmailFolder::uncounted(self.name.clone(), self.reason.clone()),
        }
    }
}

/// One message of the list, as the list draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub preview: String,
    pub date_text: String,
    pub is_read: bool,
    pub is_flagged: bool,
    pub is_selected: bool,
    pub has_attachment: bool,
    pub thread_count: i32,
    pub thread_id: String,
    pub avatar_initial: String,
    pub avatar: Rgb,
}

/// A listed message: its id, the folder it was listed from, and its row. The folder travels
/// with the row because IMAP UIDs are per-mailbox, and the same id in another mailbox is another
/// message — everything done to a row has to ask the mailbox that listed it (#275, #288).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub folder: String,
    pub item: ListItem,
}

/// The message in the reading pane, as the pane draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Detail {
    pub from_name: String,
    pub from_addr: String,
    pub from_initial: String,
    pub avatar: Rgb,
    pub to_addr: String,
    pub cc_addr: String,
    pub subject: String,
    pub date_text: String,
    pub body: String,
    pub ai_summary: String,
    pub is_flagged: bool,
    pub is_read: bool,
    pub has_attachment: bool,
    pub attachment_names: String,
    pub thread_count: i32,
    /// Whether "Open original" has an original to open: the sender's HTML.
    pub has_html: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub size_text: String,
    pub mime_type: String,
    pub is_downloaded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadEntry {
    pub from_name: String,
    pub from_addr: String,
    pub date_text: String,
    pub body: String,
    pub is_collapsed: bool,
}

/// The open message: which one it is, and what the pane shows of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Open {
    pub id: String,
    pub folder: String,
    pub detail: Detail,
    pub attachments: Vec<Attachment>,
    pub thread: Vec<ThreadEntry>,
    /// The HTML original, held for the toolbar's "Open original" so the button does not go back
    /// to the mail server for what the reading pane was just given. `None` for a plain-text
    /// message, whose button is not drawn.
    pub html: Option<String>,
}

/// The composer.
#[derive(Debug, Clone)]
pub struct Composer {
    pub open: bool,
    /// The five fields, as the window last reported them or a command filled them.
    pub text: Draft,
    /// "Draft saved", "Draft restored", or nothing.
    pub status: String,
    /// The draft file as it was last read or written: what Compose reopens. An error is a file
    /// that is there and could not be read, said when Compose is pressed.
    pub kept: Result<Option<Draft>, String>,
    /// What is being sent, while a send is in flight: the window says so, a second press cannot
    /// send the message twice, and an edit made meanwhile is told apart from what went.
    pub sending: Option<Draft>,
    /// Text the lanes held at start, written as the draft file before any press: what the
    /// composer held at a logout, or else a message that was being sent when the app last closed
    /// (whether that went is not known, so it is offered with the warning).
    pub unsent_at_start: Option<Draft>,
    /// The kept draft's message may already have gone: a send of it never came back. Kept beside
    /// the draft file, and said whenever Compose opens it, until it is sent or discarded.
    pub kept_unsure: bool,
    /// The same, of the text in the composer now.
    pub unsure: bool,
    /// Cancel was pressed while a send was out: the send's own sentence says where the edit went.
    pub closed_during_send: bool,
    /// The sentence about the text offered at start is owed, once the first look has landed.
    /// Dropped when the person keeps or discards the draft first: they have chosen.
    pub announce_unsent: Option<&'static str>,
    /// True while the draft file is being written or removed.
    pub writing: bool,
    /// What to do to the draft file once the write in flight lands: only the latest.
    pub queued: Option<FileOp>,
}

/// A write or a removal of the draft file.
#[derive(Debug, Clone, PartialEq)]
pub enum FileOp {
    /// `unsure`: written with the mark that says whether its message went is not known.
    /// `release_lane`: the send's lane copy goes once this has landed.
    Save { draft: Draft, keep: Keep, unsure: bool, release_lane: bool },
    Clear { after_send: bool, release_lane: bool },
}

impl FileOp {
    pub fn releases_lane(&self) -> bool {
        match self {
            FileOp::Save { release_lane, .. } | FileOp::Clear { release_lane, .. } => *release_lane,
        }
    }

    pub fn releasing_lane(mut self, release: bool) -> FileOp {
        match &mut self {
            FileOp::Save { release_lane, .. } | FileOp::Clear { release_lane, .. } => *release_lane = release,
        }
        self
    }
}

/// Why the draft file is written, which decides what is said when it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// Save Draft: the composer's status says so.
    Saved,
    /// Cancel: the notice says Compose reopens it.
    Closing,
    /// Around a send, so the text survives whatever becomes of it: said only when it fails.
    Quietly,
}

/// The framework draft that holds a message while it is being sent. It goes through the
/// persistence lane, which is written before the process ends, so a send still waiting for a
/// worker when the window closes is not lost with it.
pub const SENDING: &str = "sending";

/// The framework draft that holds the open composer's text when the session ends the app (a
/// logout, SIGTERM, the display going): handed to the lane in the command that answers it.
pub const COMPOSING: &str = "composing";

/// What the setup form's fields are cleared to when `part::SETUP_FIELDS` is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Clear {
    #[default]
    Nothing,
    /// The password leaves the window the moment it is not needed.
    Password,
    /// A form opened fresh for another account.
    Fresh,
}

/// The account setup form. The fields themselves are the window's: a password is never kept
/// in the model, and leaves the window when the form is done with it.
#[derive(Debug, Clone, Default)]
pub struct Setup {
    pub open: bool,
    pub status: String,
    pub ok: bool,
    pub testing: bool,
    /// True while a connection test, a save or a sign-in start is in flight, so the buttons
    /// cannot be pressed on top of each other.
    pub busy: bool,
    pub clear: Clear,
}

/// The Google sign-in this window is in the middle of.
#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    pub id: String,
    /// Polls left before this window stops waiting: a little past the service's own deadline, so
    /// the service's sentence about giving up is the one that is shown.
    pub polls_left: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Google {
    /// Held so that a status answer arriving from a flow that was cancelled — or from one the
    /// person abandoned and started again — is dropped rather than applied.
    pub flow: Option<Flow>,
    pub waiting: bool,
    /// The consent page's address, shown only when no browser could be opened for it.
    pub url: String,
}

#[derive(Debug, Clone, Default)]
pub struct Search {
    /// The search box, as the window last reported it or a command set it.
    pub query: String,
    pub active: bool,
    pub count: i32,
}

pub struct Model {
    pub mail: Arc<dyn MailService>,
    pub mind: Arc<dyn Mind>,
    pub files: Files,
    /// What the app has been told about the mail service. Not inferred from the window, because a
    /// `bool` cannot say "not known".
    pub state: MailState,
    /// The first look at the service has landed.
    pub looked: bool,
    pub account_name: String,
    /// The folder on screen.
    pub folder: String,
    pub folders: Vec<FolderRow>,
    pub selected_folder: usize,
    /// Every message the folder returned, before the triage tabs filter it. Held so that
    /// switching tabs is a filter over what is in hand rather than another question to the mail
    /// server. A row's place here is the id the list draws it with.
    pub rows: Vec<Row>,
    pub triage: Triage,
    pub open: Option<Open>,
    pub search: Search,
    pub composer: Composer,
    pub setup: Setup,
    pub google: Google,
    pub ai_working: bool,
    /// The open message's category, one of the four the screen draws, or nothing.
    pub ai_intent: String,
    /// What went wrong, for the person at the window; `describe.notice` carries the same.
    pub notice: String,
    pub sync_status: String,
    pub loading: bool,
    /// True while a sync is running, so Refresh cannot stack.
    pub syncing: bool,
    /// The answer the surface call being settled gets.
    pub answer: Value,
}

impl Model {
    pub fn new(mail: Arc<dyn MailService>, mind: Arc<dyn Mind>, files: Files, kept: Result<Option<Draft>, String>) -> Model {
        Model {
            mail,
            mind,
            files,
            state: MailState::Unreachable { reason: "the mail service has not been asked yet".to_string() },
            looked: false,
            account_name: String::new(),
            folder: "INBOX".to_string(),
            folders: Vec::new(),
            selected_folder: 0,
            rows: Vec::new(),
            triage: Triage::All,
            open: None,
            search: Search::default(),
            composer: Composer {
                open: false,
                text: Draft::default(),
                status: String::new(),
                kept,
                sending: None,
                unsent_at_start: None,
                kept_unsure: false,
                unsure: false,
                closed_during_send: false,
                announce_unsent: None,
                writing: false,
                queued: None,
            },
            setup: Setup::default(),
            google: Google::default(),
            ai_working: false,
            ai_intent: String::new(),
            notice: String::new(),
            sync_status: "Syncing\u{2026}".to_string(),
            loading: true,
            syncing: false,
            answer: Value::Null,
        }
    }

    pub fn account(&self) -> String {
        self.state.account_id()
    }

    /// The reason the service could not be used, if that is the state.
    pub fn unreachable_reason(&self) -> Option<String> {
        match &self.state {
            MailState::Unreachable { reason } => Some(reason.clone()),
            _ => None,
        }
    }

    /// The rows on screen: what the triage tab keeps, with each one's id in the list.
    pub fn shown(&self) -> Vec<(i32, &Row)> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| self.triage.keeps(r.item.is_read, r.item.is_flagged))
            .map(|(i, r)| (i as i32, r))
            .collect()
    }

    /// The rows on screen as a caller names them.
    pub fn message_rows(&self) -> Vec<MessageRow> {
        self.shown()
            .iter()
            .map(|(_, r)| MessageRow::new(&r.item.subject, &r.item.from_name, &r.item.from_addr))
            .collect()
    }

    /// The folder list as the wire had it.
    pub fn folder_records(&self) -> Vec<EmailFolder> {
        self.folders.iter().map(FolderRow::record).collect()
    }

    /// The open folder's counts, off the folder list: the same two numbers as its entry there.
    pub fn header(&self) -> Counted {
        let unread = self.rows.iter().filter(|r| !r.item.is_read).count();
        Counted::of(&self.folder_records(), &self.folder, unread, self.rows.len())
    }

    /// The folder this account archives into.
    ///
    /// Read off the folder list the mail server gave us, because "[Gmail]/All Mail" — which was
    /// hardcoded — exists on exactly one provider. `Archive` is the IMAP convention and what
    /// everyone else uses.
    pub fn archive_folder(&self) -> String {
        self.folders
            .iter()
            .find(|f| f.kind == "archive")
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "Archive".to_string())
    }

    /// Whether a message is open in the reading pane.
    pub fn reading(&self) -> Option<&Open> {
        self.open.as_ref().filter(|o| !o.detail.subject.is_empty())
    }
}
