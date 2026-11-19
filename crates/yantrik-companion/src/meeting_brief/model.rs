//! The brief as it is stored and as the shell reads it.

use serde::{Deserialize, Serialize};

/// How long before the start a brief is made.
pub const LEAD_SECS: i64 = 10 * 60;
/// How far back mail and files count.
pub const LOOKBACK_DAYS: i64 = 30;
pub const MAX_EMAILS: usize = 3;
pub const MAX_NOTES: usize = 3;
pub const MAX_PROMISES: usize = 3;
pub const MAX_FILES: usize = 2;
pub const DESCRIPTION_LINES: usize = 2;
/// The longest a row's words may be. A row is one line on a card.
pub const MAX_ROW_CHARS: usize = 140;

/// What a row's Open button opens. The shell turns each into the opener the notice inbox already
/// has; nothing here opens anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Open {
    /// Email, at exactly this message: the account's address, the folder it is in and its id
    /// there. Never found by its words, which can name another message.
    Message { account: String, folder: String, id: String },
    /// Notes, at the note with this id (its file name), never by title.
    Note { id: String },
    /// A file, by absolute path; the shell checks it again at the press.
    File { path: String },
    /// Calendar, at the event with this id.
    Event { id: String },
}

/// What the person pressed on a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// Any Open.
    Opened,
    Dismissed,
}

/// One line in a section, and what its button opens, when there is something to open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub text: String,
    pub open: Option<Open>,
}

/// The five sections of one meeting. An empty one is left out of the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Brief {
    pub event_id: String,
    pub title: String,
    /// Unix seconds.
    pub start: i64,
    pub end: i64,
    /// "10:00–10:30", local time.
    pub when: String,
    pub place: Option<String>,
    /// The other people's names; the person's own addresses are never here.
    pub people: Vec<String>,
    /// What the brief was made from, so a change to the title, the start or the people is seen.
    pub fingerprint: String,
    /// Whether every source answered. A brief shown at the last call with a section missing is
    /// not final: it is filled in when the source answers, without a second toast.
    #[serde(default = "yes")]
    pub complete: bool,
    /// Since when the calendar has not listed the event, while it is only maybe gone: a calendar
    /// that answers an empty list while its files are rewritten must not end a brief.
    #[serde(default)]
    pub missing_since: Option<i64>,
    pub emails: Vec<Row>,
    pub notes: Vec<Row>,
    pub promises: Vec<Row>,
    pub files: Vec<Row>,
    /// The first lines of the event's own description.
    pub description: Vec<String>,
}

fn yes() -> bool {
    true
}

/// How the briefs of the last 30 days ended: all `describe` says about them, as counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcomes {
    /// An Open was pressed.
    pub opened: u32,
    pub dismissed: u32,
    /// Shown, and over with no press.
    pub ignored: u32,
}

/// How many lines each section has. All the shell's `describe` says about a brief.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub emails: usize,
    pub notes: usize,
    pub promises: usize,
    pub files: usize,
    pub description: usize,
}

impl Brief {
    pub fn counts(&self) -> Counts {
        Counts {
            emails: self.emails.len(),
            notes: self.notes.len(),
            promises: self.promises.len(),
            files: self.files.len(),
            description: self.description.len(),
        }
    }

    /// A brief that says nothing is noise: it gets no card and no toast.
    pub fn is_empty(&self) -> bool {
        let c = self.counts();
        c.emails + c.notes + c.promises + c.files + c.description == 0
    }
}

/// A brief ready to be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// `meeting:<event_id>:<start_unix>`.
    pub id: String,
    pub brief: Brief,
}

/// The id of the item a meeting's brief is stored under: one per event and start, so a moved
/// event is a different item.
pub fn item_id(event_id: &str, start: i64) -> String {
    format!("meeting:{event_id}:{start}")
}

/// `text` as one line, cut to `max` characters on a character boundary, with an ellipsis when it
/// was cut.
pub fn clamp(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    let cut: String = one_line.chars().take(max.saturating_sub(1)).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// "5 min", "3 h", "2 d": how long ago `then` was, at `now`, both unix seconds.
pub fn age(now: i64, then: i64) -> String {
    let secs = (now - then).max(0);
    match secs {
        s if s < 3600 => format!("{} min", (s / 60).max(1)),
        s if s < 86_400 => format!("{} h", s / 3600),
        s => format!("{} d", s / 86_400),
    }
}
