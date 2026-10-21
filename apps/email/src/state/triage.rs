//! The triage tabs: filters over the rows already in hand.

/// Which of the listed messages the tabs above the list keep.
///
/// There were four tabs and the fourth was Priority, which nothing in this app or its wire could
/// have computed: no message carries an importance, and `email-priority-count` was an `in`
/// property nothing ever set, so the badge beside it was always absent and the tab always empty.
/// It is gone. These three are filters over the rows already in hand, which is why they can be
/// applied without asking the mail server anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Triage {
    All,
    Unread,
    Flagged,
}

impl Triage {
    pub fn from_index(index: i32) -> Option<Triage> {
        match index {
            0 => Some(Triage::All),
            1 => Some(Triage::Unread),
            2 => Some(Triage::Flagged),
            _ => None,
        }
    }

    pub fn index(&self) -> i32 {
        match self {
            Triage::All => 0,
            Triage::Unread => 1,
            Triage::Flagged => 2,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Triage::All => "All",
            Triage::Unread => "Unread",
            Triage::Flagged => "Flagged",
        }
    }

    pub fn keeps(&self, is_read: bool, is_flagged: bool) -> bool {
        match self {
            Triage::All => true,
            Triage::Unread => !is_read,
            Triage::Flagged => is_flagged,
        }
    }
}
