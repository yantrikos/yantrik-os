//! What ySheets is: the model every command changes and every render reads.
//!
//! The sheet is a view on one document in the framework's `Documents` table: that document owns
//! the book (the sheet and its history) and what is known of its file (its baseline, its save, an
//! outside change, its draft); [`Doc`] beside it owns what is ySheets' (the text its file held,
//! whether it came back from a draft, whether an agent is shown it). There is always exactly one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use yantrik_app::documents::disk::{Disk, RealDisk};
use yantrik_app::documents::notice::Notice;
use yantrik_app::documents::{DraftState, Document, DocumentId, Documents, Fingerprint, OutsideState, Source, ViewId};

use crate::address::Cell;
use crate::book::Book;
use crate::files::picker;
use crate::files::read::Inode;
use crate::sheet::Sheet;

/// The title of a sheet with no file.
pub const UNTITLED: &str = "Untitled";

/// The one file operation in flight. ySheets reads or writes one file at a time, and refuses what
/// would start another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Open,
    Save,
}

/// The dialog over the grid that asks for a typed path, when the desktop's file picker could not
/// be asked. The unsaved-changes question is not one of these: it is shown while [`Model::pending`]
/// holds it, and never otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    None,
    OpenPath,
    SavePath,
}

/// What waits on the unsaved-changes question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Open,
    Close,
}

/// The last thing that happened, in words. It replaces the status line's summary until the
/// selection moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub text: String,
    pub problem: bool,
    /// The sentence quotes what is in a cell, so a Mind View window does not show it.
    pub quotes_cells: bool,
}

/// Which cells of the grid the window draws again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redraw {
    Nothing,
    /// These cells, by their place in the sheet row by row.
    Cells(Vec<usize>),
    All,
}

impl Redraw {
    /// Also these cells.
    pub fn add(&mut self, more: impl IntoIterator<Item = usize>) {
        match self {
            Redraw::All => {}
            Redraw::Nothing => *self = Redraw::Cells(more.into_iter().collect()),
            Redraw::Cells(cells) => cells.extend(more),
        }
    }
}

/// Something the window does once, in the render of the command that asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// The formula bar takes the keyboard, so typing goes into the selected cell from the first key.
    EditCell,
    /// The formula bar is given the selected cell's text, whatever it holds.
    LoadField,
    /// The formula bar's text is sent to the model as a command of its own (`Header` "commit"),
    /// so the window is not closed over text nobody has committed.
    CommitField,
}

/// The cell a person just typed into, kept until the sheet is worked out: if it shows an error the
/// selection comes back to it, as it always did, unless the person has moved on.
#[derive(Debug, Clone, Copy)]
pub struct Watch {
    pub cell: Cell,
    pub rev: u64,
    /// How many selection moves the person had made when the cell was typed into.
    pub moves: u64,
}

/// The cells of a sheet just read, already made into what the window shows, for the one render
/// that draws it whole.
#[derive(Debug, Clone)]
pub struct Prebuilt {
    pub rev: u64,
    pub rows: Vec<Vec<crate::SpreadsheetCell>>,
}

/// What ySheets knows of the sheet beside what the framework's document holds.
#[derive(Debug, Clone)]
pub struct Doc {
    pub id: DocumentId,
    pub view: ViewId,
    /// The text as ySheets last read it from its file or wrote it there; `None` for a sheet never
    /// written anywhere. What the framework's save compares the file with.
    pub on_disk: Option<String>,
    /// Restored from a draft after ySheets ended with it unsaved: unsaved until it is saved,
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
    /// The book's revision the framework's document has been told about.
    pub seen_revision: u64,
}

pub struct Model {
    pub documents: Documents<Book>,
    pub doc: Doc,
    pub busy: Option<Work>,
    /// What could not be done, or what happened that the person must know, in one line; also
    /// `describe.notice`, in its agent-safe form for an agent.
    pub notice: Notice,
    pub said: Option<Said>,
    pub active: Cell,
    pub dialog: Dialog,
    pub dialog_message: String,
    pub dialog_path: String,
    pub pending: Option<Pending>,
    /// A typed save path the person has been told already exists, and has not yet confirmed.
    pub confirm_replace: Option<PathBuf>,
    /// The desktop's file picker is open; a second one is not asked for.
    pub picking: bool,
    /// Who is asked for a file: `saving`, and the sheet's current file to suggest.
    pub ask: picker::Ask,
    /// The window is drawn for a Mind (Mind View): the cells' contents are masked (mind_view.rs).
    pub for_mind: bool,
    /// Drafts that could not be written, by name, with the edit they were of: tried again at the
    /// sheet's next edit.
    pub draft_failed_at: std::collections::BTreeMap<String, u64>,
    /// Where ySheets keeps its drafts.
    pub state_dir: PathBuf,
    /// What was said of a draft this start left alone because a file was named on the command
    /// line: the file wins the window, and the draft is offered again at the next start with none.
    pub held_aside: Option<String>,
    /// A file a launch asked to open, waiting for ySheets to be free.
    pub queue: Option<PathBuf>,
    /// The question's *Discard* is on its way to a file picker: the file chosen replaces the sheet
    /// whatever it holds.
    pub discarding: bool,
    /// The sheet was given up on purpose (a close with *Discard*): its draft is not made again.
    pub given_up: bool,
    /// The session is ending: whatever draft the logout wrote is the last word, and no later step of
    /// the same command takes it away.
    pub ending: bool,
    /// The window was asked to close and is waiting for the formula bar's text to be committed.
    pub closing_after_commit: bool,
    /// The window closed with a save in flight: ySheets ends when it lands (`fx.linger`).
    pub lingering: bool,
    /// The window was asked to close while a file was read: the close goes on once that lands.
    pub close_when_done: bool,
    /// A re-read, after another program changed the file, was set aside for the save in flight:
    /// the save's landing takes the change up again.
    pub reread_owed: bool,
    /// What the last call from the surface is answered with.
    pub answer: Value,
    /// The newest revision of the sheet a job was started to work out.
    pub solving: u64,
    /// The document `seq` whose draft is being encoded in a job.
    pub encoding: Option<u64>,
    /// A revision whose job ended without an answer, so none is started for it again.
    pub solve_lost: Option<u64>,
    pub watch: Option<Watch>,
    /// How many times the person moved the selection: a changed count means they moved on.
    pub moves: u64,
    pub redraw: Redraw,
    /// Taken by the render that draws the sheet whole (it is moved into the window, not copied).
    pub prebuilt: std::cell::RefCell<Option<Prebuilt>>,
    /// The window, for `bar`. Set when the callbacks are bound.
    pub window: std::cell::RefCell<Option<slint::Weak<crate::SpreadsheetApp>>>,
    pub cues: Vec<Cue>,
    /// The disk saves write through.
    pub disk: Arc<dyn Disk>,
    next_draft: u64,
}

impl Model {
    pub fn new(state_dir: PathBuf, for_mind: bool) -> Model {
        let mut documents = Documents::new();
        let doc = blank(&mut documents, fresh_draft(&mut 1), Book::new());
        Model {
            documents,
            doc,
            busy: None,
            notice: Notice::default(),
            said: None,
            active: Cell::new(0, 0),
            dialog: Dialog::None,
            dialog_message: String::new(),
            dialog_path: String::new(),
            pending: None,
            confirm_replace: None,
            picking: false,
            ask: picker::desktop(),
            for_mind,
            draft_failed_at: Default::default(),
            state_dir,
            held_aside: None,
            queue: None,
            discarding: false,
            given_up: false,
            ending: false,
            closing_after_commit: false,
            lingering: false,
            close_when_done: false,
            reread_owed: false,
            answer: Value::Null,
            solving: 0,
            encoding: None,
            solve_lost: None,
            watch: None,
            moves: 0,
            redraw: Redraw::All,
            prebuilt: Default::default(),
            window: Default::default(),
            // Typing goes into A1 from the first key, without a click first.
            cues: vec![Cue::LoadField, Cue::EditCell],
            disk: Arc::new(RealDisk),
            next_draft: 2,
        }
    }

    /// A fresh draft name, the next sheet's for its life.
    pub fn next_draft(&mut self) -> String {
        fresh_draft(&mut self.next_draft)
    }

    /// The framework's document.
    pub fn d(&self) -> &Document<Book> {
        self.documents.get(self.doc.id).expect("ySheets always has its document")
    }

    pub fn d_mut(&mut self) -> &mut Document<Book> {
        self.documents.get_mut(self.doc.id).expect("ySheets always has its document")
    }

    pub fn book(&self) -> &Book {
        &self.d().content
    }

    pub fn book_mut(&mut self) -> &mut Book {
        &mut self.d_mut().content
    }

    pub fn sheet(&self) -> &Sheet {
        &self.book().sheet
    }

    /// The sheet's file, if it has one.
    pub fn path(&self) -> Option<&Path> {
        self.d().source.path()
    }

    /// The file's name, or "Untitled".
    pub fn title(&self) -> String {
        self.path()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| UNTITLED.into())
    }

    /// Whether the sheet holds anything its file does not: it was changed since it was read or
    /// written, it was restored from a draft, its file was moved or deleted and not found again,
    /// or another program changed the file under it. In the last two its content is on no disk at
    /// all, whatever it is, until it is saved somewhere.
    pub fn modified(&self) -> bool {
        self.content_unsaved() || matches!(self.d().outside, OutsideState::Deleted | OutsideState::Conflict { .. })
    }

    /// Whether the sheet in front differs from the file's (or was recovered): what a re-read of
    /// the file would replace. A conflict alone does not count; it says the file moved on, not
    /// that there is anything here to lose.
    pub fn content_unsaved(&self) -> bool {
        self.doc.recovered || self.book().dirty()
    }

    /// The draft of the sheet as it is now is asked for (or is on disk). The framework's own
    /// version of these three wants the content to be serialisable; the sheet's draft is the
    /// app's own schema (files/drafts.rs), so the state is set here.
    pub fn draft_asked(&mut self) {
        let d = self.d_mut();
        d.draft = DraftState::Kept { seq: d.seq };
    }

    pub fn draft_failed(&mut self, sentence: &str) {
        self.d_mut().draft = DraftState::Failed { sentence: sentence.into() };
    }

    pub fn draft_discarded(&mut self) {
        self.d_mut().draft = DraftState::None;
    }

    /// Where the sheet's draft file is.
    pub fn draft_path(&self) -> PathBuf {
        yantrik_app::drafts::path_in(&self.state_dir, &self.doc.draft)
    }

    /// What the formula bar holds now, if the window is there. Only a logout asks (update/closing.rs):
    /// every other command is told the bar's text with it.
    pub fn bar(&self) -> Option<String> {
        self.window.borrow().as_ref().and_then(slint::Weak::upgrade).map(|ui| ui.get_cell_data().to_string())
    }

    /// Whether the unsaved-changes question is up.
    pub fn asking(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether a dialog or the question is over the grid: only its own answers are taken.
    pub fn dialog_up(&self) -> bool {
        self.asking() || self.dialog != Dialog::None
    }

    /// What the formula bar holds for the selected cell: its text, or in a Mind View window the
    /// framework's one sentence in place of it (an empty cell stays empty, since a mask over
    /// nothing would say there is something to hide).
    pub fn field_text(&self) -> String {
        if self.mask() == crate::view::render::Mask::All {
            return yantrik_app::for_mind::HIDDEN.into();
        }
        yantrik_app::for_mind::shown_if_any(self.for_mind, self.sheet().raw(self.active)).into_owned()
    }

    /// What this window hides of the sheet in front.
    pub fn mask(&self) -> crate::view::render::Mask {
        crate::view::render::Mask::of(self.for_mind, self.doc.hidden)
    }

    /// What the status line says now, and whether it is a problem.
    pub fn status(&self) -> Said {
        if let Some(said) = &self.said {
            return said.clone();
        }
        let s = self.sheet();
        if let Some(problem) = s.problem(self.active) {
            return Said { text: problem, problem: true, quotes_cells: true };
        }
        let (filled, errors) = s.counts();
        let mut line = format!(
            "{} · {} rows × {} columns · {} {} filled",
            self.active.name(),
            s.rows(),
            s.cols(),
            filled,
            if filled == 1 { "cell" } else { "cells" }
        );
        if errors > 0 {
            let first = s.first_error().map(|c| c.name()).unwrap_or_default();
            line.push_str(&format!(
                " · {errors} {} an error, the first in {first}",
                if errors == 1 { "shows" } else { "show" }
            ));
        }
        Said { text: line, problem: false, quotes_cells: false }
    }
}

fn fresh_draft(next: &mut u64) -> String {
    let n = *next;
    *next += 1;
    format!("sheet-{:x}-{:x}", crate::files::drafts::now_ms(), n)
}

/// A sheet with no file, shown in the primary window.
pub fn blank(documents: &mut Documents<Book>, draft: String, book: Book) -> Doc {
    let id = documents.open(Source::New(draft.clone()), book, None);
    place(documents, id, None, draft, false, None)
}

/// A document in the table, shown in the primary window.
pub fn place(
    documents: &mut Documents<Book>,
    id: DocumentId,
    on_disk: Option<String>,
    draft: String,
    hidden: bool,
    inode: Option<Inode>,
) -> Doc {
    let view = documents.show(id, yantrik_app::app::WindowId::PRIMARY);
    let seen_revision = documents.get(id).map_or(0, |d| d.content.revision());
    Doc { id, view, on_disk, recovered: false, hidden, draft, inode, unsure: None, seen_revision }
}
