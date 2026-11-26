//! "What's going on": the short, true picture of the person's week that a mind starts every turn
//! with. Plain code over the agenda's top entries: no model, no network, no clock (the caller
//! passes `now` and says how a time reads).
//!
//! At most [`MAX_LINES`] lines and [`MAX_CHARS`] characters, each line a title, when, and the
//! short form of the reason. Titles and counts only: nothing of a note's text, and an entry whose
//! title is not the person's own words to show (`title_public`) is left out. An empty agenda, a
//! private desktop or the Brain switch off give no note at all.

use rusqlite::Connection;

use super::salience::ranked;
use super::store::{reason_of, Entry};
use super::tick::Switches;
use super::Kind;

/// The most lines the note carries.
pub const MAX_LINES: usize = 5;
/// The most characters the whole note carries, header included.
pub const MAX_CHARS: usize = 600;
const DAY: f64 = 86_400.0;
/// A promise with a deadline inside this many seconds is this week's.
const PROMISE_AHEAD: f64 = 7.0 * DAY;
/// A promise made inside this many seconds is this week's.
const PROMISE_FRESH: f64 = 3.0 * DAY;
const HEADER: &str = "What's going on for the person (from their calendar and notes, read by the desktop):";

/// The note and what it was made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Told {
    /// The exact text the mind receives.
    pub text: String,
    /// How many entries it names.
    pub items: usize,
}

impl Told {
    /// The quiet line Mind View shows under the turn.
    pub fn shown(&self) -> String {
        format!("Told: what's going on ({} item{})", self.items, if self.items == 1 { "" } else { "s" })
    }
}

/// The note for the person's top entries. `when` says how a time (Unix seconds) reads to them,
/// e.g. "Fri 27 Nov, 15:00 (tomorrow)". `None` when a switch says no, or there is nothing to say.
pub fn told(conn: &Connection, now: f64, switches: Switches, when: impl Fn(f64) -> String) -> Option<Told> {
    if switches.private || !switches.enabled {
        return None;
    }
    let ranked = ranked(conn, now, 50).ok()?;
    let mut text = HEADER.to_string();
    let mut items = 0;
    for (e, _) in ranked {
        if items == MAX_LINES {
            break;
        }
        if !e.title_public || e.title.trim().is_empty() || !current(&e, now) {
            continue;
        }
        let line = match e.kind {
            Kind::NoteTodo => {
                let extra = reason_of(conn, e.id).map(|r| short_note_reason(&r)).unwrap_or_default();
                format!("- {}: {}", e.title.trim(), extra)
            }
            _ => match e.due_at {
                Some(due) => format!("- {}, {}", e.title.trim(), when(due)),
                None => format!("- {}", e.title.trim()),
            },
        };
        let line = line.trim_end_matches(|c| c == ':' || c == ' ').to_string();
        if text.chars().count() + 1 + line.chars().count() > MAX_CHARS {
            break;
        }
        text.push('\n');
        text.push_str(&line);
        items += 1;
    }
    (items > 0).then_some(Told { text, items })
}

/// Whether an entry is plausibly this week's. An open promise counts only with a deadline in the
/// next [`PROMISE_AHEAD`] or when it was made in the last [`PROMISE_FRESH`]; an older one with no
/// deadline stays in the agenda but is not told. Everything else the agenda ranks is current.
fn current(e: &Entry, now: f64) -> bool {
    if e.kind != Kind::Promise {
        return true;
    }
    let due_soon = e.due_at.is_some_and(|d| d >= now && d <= now + PROMISE_AHEAD);
    due_soon || now - e.created_at <= PROMISE_FRESH
}

/// A note's reason in short form: "4 open items, one about the demo video". The reason reads
/// "4 unticked items. It mentions “demo video”, which matches …" or "3 unticked items, last
/// edited 2 days ago."; only the count and the shared words are kept.
fn short_note_reason(reason: &str) -> String {
    let count = reason.split_whitespace().next().and_then(|n| n.parse::<usize>().ok());
    let mut out = match count {
        Some(n) => format!("{n} open item{}", if n == 1 { "" } else { "s" }),
        None => return String::new(),
    };
    if let Some(words) = reason.split('\u{201c}').nth(1).and_then(|s| s.split('\u{201d}').next()) {
        out.push_str(&format!(", one about the {words}"));
    }
    out
}
