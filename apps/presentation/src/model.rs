//! What yPresent is: the model every command changes and every render reads.
//!
//! The deck is a view on one document in the framework's `Documents` table: that document owns
//! the deck and what is known of its file (its baseline, its save, an outside change, its
//! draft); [`Doc`] beside it owns what is yPresent's (the text its file held, whether it came
//! back from a draft, whether an agent is shown it). There is always exactly one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use yantrik_app::documents::disk::{Disk, RealDisk};
use yantrik_app::documents::notice::Notice;
use yantrik_app::documents::{Document, DocumentId, Documents, Fingerprint, OutsideState, Source, ViewId};

use crate::deck::Deck;
use crate::files::names::first_choice;
use crate::files::read::Inode;

/// The title of a deck nobody has named.
pub const UNTITLED: &str = "Untitled";

/// How many slide titles `describe` will list before it starts counting instead.
pub const OUTLINE_CAP: usize = 100;

/// The one file operation in flight. yPresent reads or writes one file at a time, and refuses
/// what would change the deck meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Open,
    Save,
    Export,
}

/// The presenter's clock: counts only while it runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Clock {
    pub running: bool,
    pub seconds: i64,
}

/// Something the window does once, in the render of the command that asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// The search bar and the theme gallery are put away.
    CloseOverlays,
    /// The theme gallery opens.
    OpenGallery,
    /// The search field is given the model's query.
    Query,
}

/// The old build's recovery file, until its draft is the lane's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy {
    /// Nothing to do: none, already set aside, or left as it was (unreadable).
    Done,
    /// Its deck is in the lane already; the file is set aside at the first command.
    Owed,
    /// Its deck's draft not yet on disk, by draft name; set aside once it is.
    Waiting(String),
    /// Being set aside.
    Archiving,
}

/// What yPresent knows of the deck beside what the framework's document holds.
#[derive(Debug, Clone)]
pub struct Doc {
    pub id: DocumentId,
    pub view: ViewId,
    /// The text as yPresent last read it from its file or wrote it there; `None` for a deck
    /// never written anywhere. What the framework's save compares the file with.
    pub on_disk: Option<String>,
    /// Restored from a draft after yPresent ended with it unsaved: unsaved until it is saved,
    /// whatever its content.
    pub recovered: bool,
    /// Its file is one no agent is shown (agent_rule.rs), decided when it took the file.
    pub hidden: bool,
    /// The name its draft is kept under. The document's own, so a save as or a move keeps one.
    pub draft: String,
    /// Where its file was when last read or written.
    pub inode: Option<Inode>,
    /// Bytes of ours a failed save may have left in its file: seeing them there is our own write.
    pub unsure: Option<Fingerprint>,
    /// The deck's revision the framework's document has been told about.
    pub seen_revision: u64,
    /// The path and fingerprint of each default export this deck has made: a default export
    /// replaces a file only when it is still exactly one of these.
    pub exports: Vec<(PathBuf, Fingerprint)>,
}

/// A request to the Mind for one slide's speaker notes, kept until its answer lands so the
/// answer goes to the slide it was asked about and to no other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotesAsk {
    /// The deck it was asked about: an answer for another deck is not written.
    pub doc: DocumentId,
    pub index: usize,
    pub title: String,
    pub body: String,
}

pub struct Model {
    pub documents: Documents<Deck>,
    pub doc: Doc,
    pub busy: Option<Work>,
    /// What could not be done, or what happened that the person must know, in one line; also
    /// `describe.notice`, in its agent-safe form for an agent.
    pub notice: Notice,
    /// The slides the current search matched, and which of them is selected.
    pub hits: Vec<usize>,
    pub hit_at: usize,
    pub query: String,
    pub clock: Clock,
    /// The full-screen presenter view is up.
    pub presenting: bool,
    /// Speaker notes asked of the Mind and not yet answered.
    pub asking: Option<NotesAsk>,
    pub mind: Arc<dyn yantrik_app::mind::Mind>,
    /// The window is drawn for a Mind (Mind View): the deck's text is masked (mind_view.rs).
    pub for_mind: bool,
    /// Drafts that could not be written, by name, with the edit they were of: tried again at the
    /// deck's next edit.
    pub draft_failed_at: std::collections::BTreeMap<String, u64>,
    /// Where yPresent keeps its drafts, and where the old build kept its recovery file.
    pub state_dir: PathBuf,
    pub legacy: Legacy,
    /// What was said of a recovery file or draft this start left alone because a file was named
    /// on the command line: the file wins the window, and the draft is offered again at the next
    /// start with no file.
    pub held_aside: Option<String>,
    /// A file a launch asked to open, waiting for yPresent to be free.
    pub queue: Option<PathBuf>,
    /// The window closed with a save in flight: yPresent ends when it lands (`fx.linger`).
    pub lingering: bool,
    /// The window was asked to close while a file was read or exported: the close goes on once
    /// that lands.
    pub close_when_done: bool,
    /// A re-read, after another program changed the file, was set aside for the save in flight:
    /// the save's landing takes the change up again.
    pub reread_owed: bool,
    /// What the last call from the surface is answered with.
    pub answer: Value,
    /// The slide's text is put back on the canvas in this render.
    pub reload: bool,
    pub cues: Vec<Cue>,
    /// The disk saves write through.
    pub disk: Arc<dyn Disk>,
    /// The folder a deck with no name is saved in, and the Open button looks in.
    pub deck_dir: PathBuf,
    next_draft: u64,
}

impl Model {
    pub fn new(state_dir: PathBuf, mind: Arc<dyn yantrik_app::mind::Mind>, for_mind: bool) -> Model {
        let mut documents = Documents::new();
        let doc = blank(&mut documents, fresh_draft(&mut 1), Deck::blank(UNTITLED));
        Model {
            documents,
            doc,
            busy: None,
            notice: Notice::default(),
            hits: Vec::new(),
            hit_at: 0,
            query: String::new(),
            clock: Clock::default(),
            presenting: false,
            asking: None,
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
            reload: false,
            cues: Vec::new(),
            disk: Arc::new(RealDisk),
            deck_dir: crate::files::names::default_deck_dir(),
            next_draft: 2,
        }
    }

    /// A fresh draft name, the next deck's for its life.
    pub fn next_draft(&mut self) -> String {
        fresh_draft(&mut self.next_draft)
    }

    /// The framework's document.
    pub fn d(&self) -> &Document<Deck> {
        self.documents.get(self.doc.id).expect("yPresent always has its document")
    }

    pub fn d_mut(&mut self) -> &mut Document<Deck> {
        self.documents.get_mut(self.doc.id).expect("yPresent always has its document")
    }

    /// The deck in front.
    pub fn deck(&self) -> &Deck {
        &self.d().content
    }

    pub fn deck_mut(&mut self) -> &mut Deck {
        &mut self.d_mut().content
    }

    /// The deck's file, if it has one.
    pub fn path(&self) -> Option<&Path> {
        self.d().source.path()
    }

    pub fn title(&self) -> &str {
        self.deck().title()
    }

    /// Whether the deck holds anything its file does not: its content is not what the file
    /// held, it was restored from a draft, its file was moved or deleted and not found again, or
    /// another program changed the file under it. In the last two its content is on no disk at
    /// all, whatever it is, until it is saved somewhere.
    pub fn modified(&self) -> bool {
        self.content_unsaved() || matches!(self.d().outside, OutsideState::Deleted | OutsideState::Conflict { .. })
    }

    /// Whether the content in front differs from the file's (or was recovered): what a re-read of
    /// the file would replace. A conflict alone does not count; it says the file moved on, not
    /// that there is anything here to lose.
    pub fn content_unsaved(&self) -> bool {
        self.doc.recovered || self.deck().dirty()
    }

    /// Where Save would put a deck that has never been given a home. It reads no disk: the real
    /// name is chosen in the job that writes, which counts up past a name that is taken.
    pub fn saves_to(&self) -> PathBuf {
        self.path().map_or_else(|| first_choice(&self.deck_dir, self.title()), Path::to_path_buf)
    }

    /// Where the deck's draft file is.
    pub fn draft_path(&self) -> PathBuf {
        yantrik_app::drafts::path_in(&self.state_dir, &self.doc.draft)
    }
}

fn fresh_draft(next: &mut u64) -> String {
    let n = *next;
    *next += 1;
    format!("deck-{:x}-{:x}", crate::files::drafts::now_ms(), n)
}

/// A deck with no file, shown in the primary window.
pub fn blank(documents: &mut Documents<Deck>, draft: String, deck: Deck) -> Doc {
    let id = documents.open(Source::New(draft.clone()), deck, None);
    place(documents, id, None, draft, false, None)
}

/// A document in the table, shown in the primary window.
pub fn place(
    documents: &mut Documents<Deck>,
    id: DocumentId,
    on_disk: Option<String>,
    draft: String,
    hidden: bool,
    inode: Option<Inode>,
) -> Doc {
    let view = documents.show(id, yantrik_app::app::WindowId::PRIMARY);
    let seen_revision = documents.get(id).map_or(0, |d| d.content.revision());
    Doc { id, view, on_disk, recovered: false, hidden, draft, inode, unsure: None, seen_revision, exports: Vec::new() }
}
