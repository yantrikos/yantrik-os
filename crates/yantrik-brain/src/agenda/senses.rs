//! Two senses for the agenda: what is on the calendar this week, and which recently edited notes
//! still have things to do. Plain code, no model.
//!
//! Same rules as `gather`: read-only on the sources (the caller hands in what it read), an upsert
//! by `(source, source_id)`, and an entry whose source item is gone is closed. Nothing here
//! copies a note's text: a [`NoteInfo`] keeps its title, a count of unticked items and the *set
//! of words* of its title and items, used only to find a link to an event and never stored.
//!
//! The link: a note whose title or open items share a meaningful word with a calendar event in
//! the next three days is more urgent than a bare to-do list, and says so in its reason ("it
//! mentions "demo video", which matches "Record the 60 s demo video" starting in 1 day").

use std::collections::BTreeSet;

use rusqlite::Connection;

use super::gather::SourceReport;
use super::salience::{UrgencyFn, PERSON_CONFIDENCE};
use super::store::{self, By, NewEntry};
use super::{Kind, NextStep, State};

pub const CALENDAR: &str = "calendar";
pub const NOTES: &str = "notes";

const DAY: f64 = 86_400.0;
/// Events this far ahead are mirrored.
pub const EVENT_WINDOW_DAYS: f64 = 7.0;
/// An event this near may be linked to a note.
pub const LINK_WINDOW_DAYS: f64 = 3.0;
/// A note edited within this many days is mirrored (if it has unticked items).
pub const NOTE_RECENT_DAYS: f64 = 3.0;
/// A note's confidence: the person wrote it, but the list may be stale.
const NOTE_CONFIDENCE: f64 = 0.9;
/// Urgency of a note with nothing linked: `NOTE_BASE + NOTE_PER_ITEM * open`, at most `NOTE_CAP`.
const NOTE_BASE: f64 = 0.15;
const NOTE_PER_ITEM: f64 = 0.01;
const NOTE_CAP: f64 = 0.3;
const TITLE_MAX: usize = 100;

/// Words too common to say two things are about the same thing.
const STOP: &[&str] = &[
    "about", "also", "been", "call", "could", "done", "from", "have", "into", "just", "meeting", "more",
    "most", "need", "needs", "next", "note", "notes", "over", "should", "some", "start", "started", "task",
    "tasks", "than", "that", "their", "them", "then", "there", "this", "todo", "today", "tomorrow", "week",
    "were", "what", "when", "where", "will", "with", "would", "your",
];

/// One calendar event, as the caller read it. `start` is Unix seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct CalEvent {
    pub id: String,
    pub title: String,
    pub start: f64,
}

/// One note, as the caller read it: no text, only what the agenda may keep.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteInfo {
    pub id: String,
    pub title: String,
    /// Unix seconds of the last edit.
    pub edited: f64,
    /// Unticked `- [ ]` items.
    pub open: usize,
    words: BTreeSet<String>,
}

impl NoteInfo {
    /// `items` are the texts of the unticked items; only their words are kept.
    pub fn new(id: &str, title: &str, edited: f64, items: &[String]) -> NoteInfo {
        let mut words: BTreeSet<String> = meaningful(title).into_iter().collect();
        for i in items {
            words.extend(meaningful(i));
        }
        NoteInfo { id: id.to_string(), title: title.to_string(), edited, open: items.len(), words }
    }
}

/// The meaningful words of a text, in order, lower-case: letters only, four or more of them,
/// not in the stop list.
fn meaningful(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphabetic())
        .filter(|w| w.chars().count() >= 4)
        .map(str::to_lowercase)
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

/// How far off, in words a person says: "under an hour", "3 hours", "2 days".
pub fn in_words(secs: f64) -> String {
    let s = secs.max(0.0);
    let (n, unit) = if s < 3600.0 {
        return "under an hour".to_string();
    } else if s < DAY {
        ((s / 3600.0).floor() as i64, "hour")
    } else {
        ((s / DAY).floor() as i64, "day")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

fn clip(s: &str) -> String {
    let t: String = s.trim().chars().take(TITLE_MAX).collect();
    if t.is_empty() { "(untitled)".to_string() } else { t }
}

/// The best event to link a note to: the one sharing the most words, the nearer on a tie.
/// Returns the event and the shared words in the order the event's title says them.
fn link<'a>(note: &NoteInfo, events: &'a [CalEvent], now: f64) -> Option<(&'a CalEvent, Vec<String>)> {
    let mut best: Option<(&CalEvent, Vec<String>)> = None;
    for e in events.iter().filter(|e| e.start >= now && e.start <= now + LINK_WINDOW_DAYS * DAY) {
        let mut seen = BTreeSet::new();
        let shared: Vec<String> =
            meaningful(&e.title).into_iter().filter(|w| note.words.contains(w) && seen.insert(w.clone())).collect();
        if shared.is_empty() {
            continue;
        }
        let better = match &best {
            None => true,
            Some((b, bs)) => shared.len() > bs.len() || (shared.len() == bs.len() && e.start < b.start),
        };
        if better {
            best = Some((e, shared));
        }
    }
    best
}

/// An entry and its reason, ready to write.
struct Mirrored {
    entry: NewEntry,
    reason: String,
}

fn event_entry(e: &CalEvent, now: f64) -> Mirrored {
    Mirrored {
        reason: format!("Starts in {}.", in_words(e.start - now)),
        entry: NewEntry {
            source: CALENDAR,
            source_id: e.id.clone(),
            kind: Kind::Event,
            title: clip(&e.title),
            title_public: true,
            state: State::Open,
            next_step: NextStep::Person,
            urgency: UrgencyFn::dated(e.start),
            due_at: Some(e.start),
            confidence: PERSON_CONFIDENCE,
            hold_until: 0.0,
            created_at: None,
        },
    }
}

fn note_entry(n: &NoteInfo, events: &[CalEvent], now: f64) -> Mirrored {
    let items = format!("{} unticked item{}", n.open, if n.open == 1 { "" } else { "s" });
    let (urgency, reason) = match link(n, events, now) {
        Some((e, shared)) => (
            UrgencyFn::dated(e.start),
            format!(
                "{items}. It mentions \u{201c}{}\u{201d}, which matches \u{201c}{}\u{201d} starting in {}.",
                shared.join(" "),
                clip(&e.title),
                in_words(e.start - now)
            ),
        ),
        None => (
            UrgencyFn::Constant { value: (NOTE_BASE + NOTE_PER_ITEM * n.open as f64).min(NOTE_CAP) },
            format!("{items}, last edited {} ago.", in_words(now - n.edited)),
        ),
    };
    Mirrored {
        reason,
        entry: NewEntry {
            source: NOTES,
            source_id: n.id.clone(),
            kind: Kind::NoteTodo,
            title: clip(&n.title),
            title_public: true,
            state: State::Open,
            next_step: NextStep::Person,
            urgency,
            due_at: None,
            confidence: NOTE_CONFIDENCE,
            hold_until: 0.0,
            created_at: Some(n.edited),
        },
    }
}

/// Writes one source's entries and closes the ones whose item is no longer there.
fn apply(conn: &Connection, source: &'static str, items: Vec<Mirrored>, now: f64) -> rusqlite::Result<SourceReport> {
    let mut rep = SourceReport { source, present: true, read: items.len(), ..Default::default() };
    let mut seen = BTreeSet::new();
    for m in items {
        seen.insert(m.entry.source_id.clone());
        if store::upsert(conn, &m.entry, now).is_ok() && store::set_reason(conn, source, &m.entry.source_id, &m.reason).is_ok() {
            rep.upserted += 1;
        } else {
            rep.skipped += 1;
        }
    }
    let open: Vec<(String, i64)> = {
        let mut stmt = conn.prepare("SELECT source_id, id FROM agenda WHERE source = ?1 AND is_open = 1")?;
        let rows = stmt.query_map([source], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        rows.flatten().collect()
    };
    for (sid, id) in open {
        if !seen.contains(&sid) && store::move_to(conn, id, State::Dropped, By::Source, now)? {
            rep.closed += 1;
        }
    }
    Ok(rep)
}

/// Mirrors the calendar and the notes. `None` for a source means "could not read it this pass":
/// its entries are left as they are (a calendar service that is down closes nothing). Idempotent.
pub fn gather_senses(
    conn: &Connection,
    now: f64,
    events: Option<&[CalEvent]>,
    notes: Option<&[NoteInfo]>,
) -> rusqlite::Result<Vec<SourceReport>> {
    store::migrate(conn)?;
    let tx = conn.unchecked_transaction()?;
    let mut out = Vec::new();
    if let Some(events) = events {
        let items = events
            .iter()
            .filter(|e| e.start >= now && e.start <= now + EVENT_WINDOW_DAYS * DAY)
            .map(|e| event_entry(e, now))
            .collect();
        out.push(apply(&tx, CALENDAR, items, now)?);
    }
    if let Some(notes) = notes {
        let evs = events.unwrap_or(&[]);
        let items = notes
            .iter()
            .filter(|n| n.open > 0 && n.edited >= now - NOTE_RECENT_DAYS * DAY)
            .map(|n| note_entry(n, evs, now))
            .collect();
        out.push(apply(&tx, NOTES, items, now)?);
    }
    tx.commit()?;
    Ok(out)
}
