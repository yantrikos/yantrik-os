//! What Snippets is: the model every command changes and every render reads.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use yantrik_app::rail::Rail;

use crate::clipboard::Clipboard;
use crate::lane::Lane;
use crate::store::{Library, Matched, Patch, Snippet};

/// The parts of the model `render` redraws by name. Every command names what it changed.
pub const NOTHING: &str = "nothing";
/// The list, the tag chips, the sidebar, the counts and the search box.
pub const PART_LIST: &str = "list";
/// The editor pane.
pub const PART_DETAIL: &str = "detail";
pub const PART_NOTICE: &str = "notice";
/// "Copied!" on the button.
pub const PART_COPY: &str = "copy";
/// The window brought forward. A part of its own so only the commands that ask for it raise the
/// window.
pub const PART_RAISE: &str = "raise";
/// The recovery bar: unsaved text kept and offered back.
pub const PART_RECOVERY: &str = "recovery";

/// What a snippet's unsaved text is kept under in the framework's draft lane: one draft per
/// snippet, so the text of one is never written over by typing in another.
pub fn draft_name(id: i32) -> String {
    format!("{DRAFT_PREFIX}{id}")
}

pub const DRAFT_PREFIX: &str = "snippet-";

/// Why a text is offered back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// A run of Snippets ended before it was kept.
    Closed,
    /// Its save was refused, and the editor has since moved on.
    NotSaved,
}

/// A snippet's text as the draft lane holds it, and how new it is: `typed` is `Model::typed` when
/// it was handed over, so of two texts for one snippet the newer typing always wins (#938 review,
/// M1).
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub text: Editor,
    pub typed: u64,
    /// The snippet whose kept text this draft's text becomes: its own, or the new snippet a
    /// *Restore as new* made of it.
    pub kept_as: i32,
    /// The write that holds this text, once one is issued: when it has landed the draft may go
    /// (`update::drafts::release`). `None` while no write is known to hold it.
    pub pending: Option<u64>,
}

impl Draft {
    pub fn of(text: Editor, typed: u64) -> Draft {
        Draft { kept_as: text.id, text, typed, pending: None }
    }
}

/// Unsaved text kept in the draft lane and offered back on the recovery bar, until the person
/// chooses: Restore (Restore as new, for a snippet no longer kept) or Discard. Nothing is put back
/// until then. The text is `Model::drafts[id]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub id: i32,
    pub why: Why,
}

/// What the editor holds: the snippet it shows, and its four fields as the person has them. It
/// is the truth about the typing; the library is the truth about what is kept, and the two differ
/// exactly while there is typing the store has not taken.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Editor {
    /// The snippet's id, or -1 for none.
    pub id: i32,
    pub title: String,
    pub language: String,
    pub code: String,
    pub tags: String,
}

/// The code may be a secret: never printed, only its length.
impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("language", &self.language)
            .field("code", &format_args!("<{} bytes>", self.code.len()))
            .field("tags", &self.tags)
            .finish()
    }
}

impl Editor {
    pub fn none() -> Editor {
        Editor { id: -1, ..Editor::default() }
    }

    pub fn of(snippet: &Snippet) -> Editor {
        Editor {
            id: snippet.id,
            title: snippet.title.clone(),
            language: snippet.language.clone(),
            code: snippet.code.clone(),
            tags: snippet.tags.clone(),
        }
    }

    pub fn patch(&self) -> Patch {
        Patch::edits(&self.title, &self.language, &self.code, &self.tags)
    }
}

/// Where the window was before a change, put back if the change's write fails and the window
/// would otherwise be left on something the file does not hold; and the typing the change was
/// keeping, handed to the draft lane if the file did not take it.
#[derive(Clone, Debug)]
pub struct Restore {
    pub selected: i32,
    pub editor: Editor,
    pub collection: i32,
    /// The text the change was keeping: the editor's, or the offered text a *Restore as new* made
    /// a snippet of.
    pub typing: Editor,
    /// How new that text is (`Model::typed` when the change was made).
    pub typed: u64,
}

pub struct Model {
    /// What is kept: the file's library, with the changes whose writes are still in flight.
    pub library: Library,
    /// The search box, the tag chip and the sidebar's collection: what the person is looking at.
    pub query: String,
    /// What the query is matched against: the person's own search matches the code too, an
    /// agent's only what snippets are called (`Matched`).
    pub query_by: Matched,
    pub tag: String,
    pub collection: i32,
    /// The snippet open in the editor, or -1. The editor's `id` is always this.
    pub selected: i32,
    pub editor: Editor,
    /// The last thing that went wrong, in words. Shown on screen and published in `describe`.
    pub notice: String,
    /// "Copied!" is on the button.
    pub copied: bool,
    /// The writes of the library's file.
    pub lane: Lane,
    /// Jobs whose outcome is the person's and must land before the app ends: writes, copies,
    /// imports and exports.
    pub work: usize,
    /// The last window closed with work in flight: the app runs on without it until the work
    /// lands (`fx.linger`).
    pub closing: bool,
    /// The app was asked to end (a logout, a quit): it ends once the work lands.
    pub ending: bool,
    /// The answer of the invocation settling now: written and read in one command
    /// (`answers::settle`).
    pub answer: Value,
    pub rail: Rail,
    pub clipboard: Arc<dyn Clipboard>,
    /// What the draft lane holds, by snippet: the text last handed to it, until it is removed.
    pub drafts: BTreeMap<i32, Draft>,
    /// Counts every edit the person makes in the editor: how new a text is.
    pub typed: u64,
    /// The drafts offered back, the bar's first.
    pub offers: Vec<Offer>,
    /// Drawn for a Mind (`Boot::for_mind`): the window masks every snippet's code
    /// (`crate::mind_view`).
    pub for_mind: bool,
}

impl Model {
    /// The snippet open in the editor, as it is kept.
    pub fn open(&self) -> Option<&Snippet> {
        self.library.get(self.selected)
    }

    /// The rows the list shows.
    pub fn rows(&self) -> Vec<Snippet> {
        self.library.matching_by(&self.query, &self.tag, self.collection, self.query_by)
    }

    /// The editor holds typing the store has not taken.
    pub fn editor_dirty(&self) -> bool {
        self.editor.id >= 0 && self.library.differs(self.editor.id, &self.editor.patch())
    }

    /// The text offered back for `id`, if one is.
    pub fn offered(&self, id: i32) -> Option<&Editor> {
        self.offers.iter().any(|o| o.id == id).then(|| self.drafted(id)).flatten()
    }

    /// The text the draft lane holds for `id`, if any.
    pub fn drafted(&self, id: i32) -> Option<&Editor> {
        self.drafts.get(&id).map(|d| &d.text)
    }

    /// Open `id` in the editor, from what is kept, or from the text offered back for it so the
    /// person sees what Restore keeps; -1, or an id not kept, opens nothing.
    pub fn select(&mut self, id: i32) {
        match self.library.get(id) {
            Some(snippet) => {
                self.selected = id;
                self.editor = self.offered(id).cloned().unwrap_or_else(|| Editor::of(snippet));
            }
            None => {
                self.selected = -1;
                self.editor = Editor::none();
            }
        }
    }

    /// Where the window is now, for a change that may have to put it back.
    pub fn restore_point(&self) -> Restore {
        Restore {
            selected: self.selected,
            editor: self.editor.clone(),
            collection: self.collection,
            typing: self.editor.clone(),
            typed: self.typed,
        }
    }
}
