//! What yDoc is: the model every command changes and every render reads.
//!
//! The document is a view on one document in the framework's `Documents` table: that document
//! owns the text and what is known of its file (its baseline, its save, an outside change, its
//! draft); [`Doc`] beside it owns what is yDoc's (the text its file held, whether it came back
//! from a draft, whether an agent is shown it). There is always exactly one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use yantrik_app::documents::disk::{Disk, RealDisk};
use yantrik_app::documents::{Document, DocumentId, Documents, Fingerprint, OutsideState, Source, ViewId};
use yantrik_app::rail::{ContextItem, Rail};

use crate::claim::Claim;
use crate::files::read::Inode;
use crate::markdown;
use crate::notice::Notice;
use crate::prompt::Closing;

/// How a document with no file and no heading is named.
pub const UNTITLED: &str = "Untitled";
/// The extension a Save As offers a document with no file yet.
pub const EXTENSION: &str = "md";

/// The one file operation in flight. yDoc reads or writes one file at a time, and refuses what
/// would change the document meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Open,
    Save,
    Export,
}

/// The file dialog the window shows, apart from the unsaved-changes prompt (prompt.rs), which is
/// drawn from the close flow itself so it can never show with nothing behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    None,
    Open,
    SaveAs,
    ExportMarkdown,
    ExportHtml,
}

/// The find bar: what it looks for, what replaces it, and what it found in the document.
#[derive(Debug, Clone, Default)]
pub struct Find {
    pub query: String,
    pub replacement: String,
    pub matches: Vec<(usize, usize)>,
    /// The match the selection is on, counting from 0.
    pub at: usize,
}

/// Something the window does once, in the render of the command that asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    FocusBody,
    Select(usize, usize),
    /// The find bar opens on the model's query.
    ShowFind,
    /// The find and replace fields are given the model's query and replacement.
    FindFields,
    /// The text on screen is put back to the document's, whatever the field holds.
    ResetContent,
    /// A dialog opened: its field is given what the model put there.
    Dialog,
}

/// The old build's recovery file, until its draft is the lane's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy {
    /// Nothing to do: none, already set aside, or left as it was (unreadable).
    Done,
    /// Its document is in the lane already; the file is set aside at the first command.
    Owed,
    /// Its document's draft not yet on disk, by draft name; set aside once it is.
    Waiting(String),
    /// Being set aside.
    Archiving,
}

/// What yDoc knows of the document beside what the framework's document holds.
#[derive(Debug, Clone)]
pub struct Doc {
    pub id: DocumentId,
    pub view: ViewId,
    /// The text as yDoc last read it from its file or wrote it there; `None` for a document never
    /// written anywhere. What "unsaved" is measured against, as it always was: a document typed
    /// back to its file's text is saved again.
    pub on_disk: Option<String>,
    /// Restored from a draft after yDoc ended with it unsaved: unsaved until it is saved,
    /// whatever its text.
    pub recovered: bool,
    /// Its file is one no agent is shown (agent_rule.rs), decided when it took the file.
    pub hidden: bool,
    /// The name its draft is kept under. The document's own, so a save as or a move keeps one.
    pub draft: String,
    /// Where its file was when last read or written.
    pub inode: Option<Inode>,
    /// Bytes of ours a failed save may have left in its file: seeing them there is our own write.
    pub unsure: Option<Fingerprint>,
}

pub struct Model {
    pub documents: Documents<String>,
    pub doc: Doc,
    pub busy: Option<Work>,
    /// What could not be done, or what happened that the person must know, in one line; also
    /// `describe.notice`, in its agent-safe form for an agent (notice.rs).
    pub notice: Notice,
    pub dialog: Dialog,
    /// What the dialog's field was given when it opened.
    pub dialog_path: String,
    pub dialog_error: String,
    /// The unsaved-changes prompt, and what it stands in front of.
    pub closing: Option<Closing>,
    pub find: Find,
    pub rail: Rail,
    /// A card the AI controls put up themselves, when the rail cannot ask (an empty document, no
    /// model): drawn on the rail's own card when the rail has none up.
    pub card: Option<yantrik_app::rail::Proposal>,
    /// Memory rows for the rail: what the Mind recalled about `memory_for`.
    pub memory: Vec<ContextItem>,
    pub memory_for: String,
    /// The title the memory rows are wanted for: asked for once it has stood still.
    pub memory_wanted: String,
    pub mind: Arc<dyn yantrik_app::mind::Mind>,
    /// The window is drawn for a Mind (Mind View).
    pub for_mind: bool,
    /// Drafts that could not be written, by name, with the edit they were of: tried again at the
    /// document's next edit.
    pub draft_failed_at: std::collections::BTreeMap<String, u64>,
    /// Where yDoc keeps its drafts and the old build kept its recovery file's neighbour.
    pub state_dir: PathBuf,
    pub legacy: Legacy,
    /// What was said of a recovery file or draft this start left alone because a file was named
    /// on the command line: the file wins the window, and the draft is offered again at the next
    /// start with no file. Said again once that file has opened, since opening clears the notice.
    pub held_aside: Option<String>,
    /// A file a launch asked to open, waiting for yDoc to be free.
    pub queue: Option<PathBuf>,
    /// The window closed with a save in flight: yDoc ends when it lands (`fx.linger`).
    pub lingering: bool,
    /// The window was asked to close while a file was read or exported: the close goes on once
    /// that lands (update/closing.rs).
    pub close_when_done: bool,
    /// A re-read, after another program changed the file, was set aside for the save in flight:
    /// the save's landing takes the change up again (update/outside.rs).
    pub reread_owed: bool,
    /// What the last call from the surface is answered with.
    pub answer: Value,
    pub cues: Vec<Cue>,
    /// The disk saves write through.
    pub disk: Arc<dyn Disk>,
    next_draft: u64,
    _claim: Claim,
}

impl Model {
    pub fn new(claim: Claim, state_dir: PathBuf, mind: Arc<dyn yantrik_app::mind::Mind>, for_mind: bool) -> Model {
        let mut documents = Documents::new();
        let draft = fresh_draft(&mut 1);
        let doc = blank(&mut documents, draft);
        Model {
            documents,
            doc,
            busy: None,
            notice: Notice::default(),
            dialog: Dialog::None,
            dialog_path: String::new(),
            dialog_error: String::new(),
            closing: None,
            find: Find::default(),
            rail: Rail::new(mind.clone()),
            card: None,
            memory: Vec::new(),
            memory_for: String::new(),
            memory_wanted: String::new(),
            mind,
            for_mind,
            draft_failed_at: Default::default(),
            state_dir,
            legacy: Legacy::Done,
            held_aside: None,
            queue: None,
            lingering: false,
            close_when_done: false,
            reread_owed: false,
            answer: Value::Null,
            cues: Vec::new(),
            disk: Arc::new(RealDisk),
            next_draft: 2,
            _claim: claim,
        }
    }

    /// A fresh draft name, the next document's for its life.
    pub fn next_draft(&mut self) -> String {
        fresh_draft(&mut self.next_draft)
    }

    /// The framework's document.
    pub fn d(&self) -> &Document<String> {
        self.documents.get(self.doc.id).expect("yDoc always has its document")
    }

    pub fn d_mut(&mut self) -> &mut Document<String> {
        self.documents.get_mut(self.doc.id).expect("yDoc always has its document")
    }

    pub fn text(&self) -> &str {
        &self.d().content
    }

    /// The document's file, if it has one.
    pub fn path(&self) -> Option<&Path> {
        self.d().source.path()
    }

    /// What to call the document on screen: the first `# ` heading, because in a Markdown
    /// document that IS the title and it is inside the file rather than beside it; failing that
    /// the file name, failing that "Untitled". A title read out of the text cannot drift from it.
    pub fn title(&self) -> String {
        if let Some(h) = markdown::outline(self.text()).into_iter().find(|h| h.level == 1) {
            if !h.title.is_empty() {
                return h.title;
            }
        }
        self.path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| UNTITLED.into())
    }

    /// Whether the document holds anything its file does not: its text is not what the file held,
    /// it was restored from a draft, its file was moved or deleted and not found again, or another
    /// program changed the file under it: in the last two its text is on no disk at all, whatever
    /// it is, until it is saved somewhere.
    pub fn modified(&self) -> bool {
        self.text_unsaved() || matches!(self.d().outside, OutsideState::Deleted | OutsideState::Conflict { .. })
    }

    /// Whether the text in front differs from the file's (or was recovered): what a re-read of the
    /// file would replace. A conflict alone does not count; it says the file moved on, not that
    /// there is anything here to lose.
    pub fn text_unsaved(&self) -> bool {
        self.doc.recovered || self.d().content != self.doc.on_disk.as_deref().unwrap_or_default()
    }

    /// Where a Save As is offered when the document has no file yet: a name out of its title, in
    /// the person's Documents folder; or the file it already has.
    pub fn home(&self) -> PathBuf {
        if let Some(p) = self.path() {
            return p.to_path_buf();
        }
        let documents = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Documents");
        documents.join(format!("{}.{EXTENSION}", slug(&self.title())))
    }

    /// What is in the document and in no file, as a clause a refusal can carry, or `None` when
    /// nothing is at stake. A refusal that says only "there are unsaved changes" leaves a person
    /// working out how much they are about to lose and where the rest of it is.
    ///
    /// `hide` names a document an agent is not shown `(hidden)`, with no title and no path: the
    /// sentence lands in the notice, which `describe` shows to whoever reads next.
    pub fn unsaved(&self, hide: bool) -> Option<String> {
        if !self.modified() {
            return None;
        }
        let title = if hide { crate::agent_rule::HIDDEN.to_string() } else { self.title() };
        let baseline = self.doc.on_disk.as_deref().unwrap_or_default();
        // A recovered draft that has not been typed into since has no "edit" to measure: the
        // whole of it is the thing nobody has agreed to.
        let untouched_draft = self.doc.recovered && self.text() == baseline;
        let count = if untouched_draft {
            markdown::char_count(self.text())
        } else {
            markdown::unsaved_chars(self.text(), baseline)
        };
        let home = match self.path() {
            Some(_) if hide => format!("are not in {}", crate::agent_rule::HIDDEN),
            Some(p) => format!("are not in {}", p.display()),
            None => "are in no file at all".to_string(),
        };
        Some(if untouched_draft {
            format!(
                "a recovered draft of \u{201c}{title}\u{201d} \u{2014} {count} characters that {home} and that \
                 nobody has agreed to yet"
            )
        } else {
            format!("{count} characters of \u{201c}{title}\u{201d} that {home}")
        })
    }
}

fn fresh_draft(next: &mut u64) -> String {
    let n = *next;
    *next += 1;
    format!("doc-{:x}-{:x}", crate::files::drafts::now_ms(), n)
}

/// A new, empty document with no file, shown in the primary window.
pub fn blank(documents: &mut Documents<String>, draft: String) -> Doc {
    let id = documents.open(Source::New(draft.clone()), String::new(), None);
    place(documents, id, None, draft, false, None)
}

/// A document in the table, shown in the primary window.
pub fn place(
    documents: &mut Documents<String>,
    id: DocumentId,
    on_disk: Option<String>,
    draft: String,
    hidden: bool,
    inode: Option<Inode>,
) -> Doc {
    let view = documents.show(id, yantrik_app::app::WindowId::PRIMARY);
    Doc { id, view, on_disk, recovered: false, hidden, draft, inode, unsure: None }
}

/// A file name out of a document title. ASCII, lower case, no separators of its own.
fn slug(title: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.extend(c.to_lowercase());
        } else {
            gap = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}
