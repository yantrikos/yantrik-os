//! One open tab: a view on one document, and what the tab knows beside it.

use yantrik_app::documents::{DocumentId, Fingerprint, ViewId};

use crate::owner::Opener;
use crate::text::history::History;

/// A tab, for the life of this run. Commands name tabs by this, never by their place in the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabId(pub u64);

/// Where a file is, as the disk names it (device and inode): how a file moved while it was open
/// is told from another file with the same bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inode {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Debug, Clone)]
pub struct Tab {
    pub id: TabId,
    pub doc: DocumentId,
    pub view: ViewId,
    /// The text as this tab last read it from its file or wrote it there; `None` for a tab never
    /// written anywhere. What "unsaved" is measured against, as it always was: a tab typed back
    /// to its file's text is saved again.
    pub on_disk: Option<String>,
    /// Restored from a draft after the editor ended with it unsaved: unsaved until it is saved,
    /// whatever its text.
    pub recovered: bool,
    pub opened_by: Opener,
    pub history: History,
    /// Its file is one no agent is shown (agent_rule.rs), decided when the tab took the file.
    pub hidden: bool,
    /// The name its draft is kept under. The tab's own, so a save as or a move keeps one draft.
    pub draft: String,
    /// When the tab was first opened, in milliseconds: restored tabs come back in this order.
    pub opened_at: u64,
    /// Where its file was when last read or written.
    pub inode: Option<Inode>,
    /// Bytes of ours a failed save may have left in its file: seeing them there is our own write.
    pub unsure: Option<Fingerprint>,
}
