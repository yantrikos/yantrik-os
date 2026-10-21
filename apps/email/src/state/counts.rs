//! What the header says about the open folder: the server's two numbers, or that it has none.

use yantrik_ipc_contracts::email::{EmailFolder, FolderCounts as WireCounts};

/// The two numbers in "INBOX, 12 unread of 35", as the mail server counts them.
///
/// They come from the folder list — the same list `describe` prints two lines under the header
/// — and from nowhere else. They used to be counted over the page of messages in hand: "9 unread
/// of 21" for a folder the list beside it said held 35, where 21 was one page and 9 the unread
/// among those, so one reply gave two answers to one question (#74, #123). The comment on that
/// code already said the counts were "of the folder, not of the tab"; now they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FolderCounts {
    pub unread: i32,
    pub total: i32,
}

impl From<WireCounts> for FolderCounts {
    fn from(c: WireCounts) -> Self {
        FolderCounts { unread: c.unread, total: c.total }
    }
}

/// What the header can say about the open folder: its two numbers, or that it has none.
///
/// A folder whose counts the mail server would not give — a STATUS it refused, a reply that did
/// not come — used to reach the header as `0 unread of 0`, which is what an empty folder says
/// too, so a failure to read the mailbox was reported as a fact about it (#131). The list now
/// carries "not counted" as its own value, and the header carries it through rather than
/// inventing a zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Counted {
    /// The server's numbers — or, for a folder the list does not carry, what is in hand.
    Known(FolderCounts),
    /// The folder is listed and the server did not count it this time; `reason` is why, in the
    /// list's own words.
    Unavailable { reason: String },
}

impl Counted {
    /// The counts for `folder`, off the list the server gave.
    ///
    /// A folder the list does not have — a mailbox opened by a name LIST did not return — has no
    /// server count, and then the page in hand is all there is: `loaded` messages, `loaded_unread`
    /// of them unread. A lower bound, and the only honest number left; there is no row in the
    /// list for it to contradict. A folder the list *does* have, uncounted, is a different thing:
    /// the server was asked and did not say, and the page in hand is not offered in place of an
    /// answer it refused.
    pub fn of(folders: &[EmailFolder], folder: &str, loaded_unread: usize, loaded: usize) -> Self {
        match folders.iter().find(|f| f.name.eq_ignore_ascii_case(folder)) {
            Some(EmailFolder { counts: Some(counts), .. }) => Counted::Known((*counts).into()),
            Some(EmailFolder { reason, .. }) => Counted::Unavailable {
                reason: reason
                    .clone()
                    .unwrap_or_else(|| "the mail server did not count this folder".to_string()),
            },
            None => Counted::Known(FolderCounts { unread: loaded_unread as i32, total: loaded as i32 }),
        }
    }

    /// The counts, if there are any.
    pub fn known(&self) -> Option<FolderCounts> {
        match self {
            Counted::Known(counts) => Some(*counts),
            Counted::Unavailable { .. } => None,
        }
    }

    /// The same folder after a change the server agreed to, when there are numbers to change.
    /// Counts that were never read stay unread: one message fewer than "not known" is still not
    /// known.
    pub fn after(self, change: impl FnOnce(FolderCounts) -> FolderCounts) -> Self {
        match self {
            Counted::Known(counts) => Counted::Known(change(counts)),
            unavailable => unavailable,
        }
    }
}

impl FolderCounts {
    /// The same folder after one of its messages was marked read or unread, and the mail server
    /// agreed. Reading a message marks it read; a Refresh asks the server again and replaces
    /// this, but between the two the header must not say twelve unread over a list with eleven
    /// unread dots in it.
    pub fn after_read_change(self, was_read: bool, now_read: bool) -> Self {
        let unread = match (was_read, now_read) {
            (false, true) => self.unread - 1,
            (true, false) => self.unread + 1,
            _ => self.unread,
        };
        FolderCounts { unread: unread.max(0), total: self.total }
    }

    /// The same folder after a message left it — deleted, or moved elsewhere — and the server
    /// confirmed it is gone.
    pub fn after_removal(self, was_read: bool) -> Self {
        FolderCounts {
            unread: if was_read { self.unread } else { (self.unread - 1).max(0) },
            total: (self.total - 1).max(0),
        }
    }

    /// The folder a message was moved into, after it arrived.
    pub fn after_arrival(self, is_read: bool) -> Self {
        FolderCounts {
            unread: if is_read { self.unread } else { self.unread + 1 },
            total: self.total + 1,
        }
    }
}

/// The one line over a folder that is open, when nothing is being read or written.
///
/// With a search on, the list under the header is the results and not the folder, and the
/// header says so rather than putting the folder's counts over a list they do not describe.
/// With no counts, it says that — and why — rather than "0 unread of 0", which is a different
/// claim and, for this folder, an unverified one.
pub fn folder_summary(folder: &str, counts: &Counted, search: Option<(&str, usize)>) -> String {
    match (search, counts) {
        (Some((query, hits)), _) => format!(
            "Email — {folder}, {hits} {} for \u{201c}{query}\u{201d}",
            if hits == 1 { "result" } else { "results" }
        ),
        (None, Counted::Known(counts)) => {
            format!("Email — {folder}, {} unread of {}", counts.unread, counts.total)
        }
        (None, Counted::Unavailable { reason }) => {
            format!("Email — {folder}, counts unavailable ({reason})")
        }
    }
}
