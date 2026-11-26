//! The first "anticipate" rule: an event about to start, and a note about it with things still to
//! do. Plain code over the agenda table; no model, no clock (the caller passes `now`), no network.
//!
//! The rule: an `Event` starting within [`WINDOW_HOURS`], linked by `senses` (a word the event and
//! the note share) to a `NoteTodo` with open items, makes one *moment candidate*. Its words are
//! the entries' titles, counts and times and nothing else. A candidate is made once per event,
//! ever: the ledger (`agenda_moment`) remembers that it was offered and what the person did, so
//! a restart, a second unlock or an edited note never shows it again.
//!
//! This file decides *whether* and *what to say*. Delivery (the notice, the day's budget, quiet
//! hours, the person's switches' last word) belongs to the shell and the notifications service,
//! which already do it for the meeting brief. More rules can follow by adding a rule name and a
//! function beside [`candidates`].

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use super::senses::{in_words, CALENDAR, NOTES};
use super::tick::Switches;

const HOUR: f64 = 3600.0;
/// An event this near, and no nearer than now, may make a candidate.
pub const WINDOW_HOURS: f64 = 36.0;
/// The rule's name, in the ledger and in `describe`.
pub const RULE_EVENT_NOTE: &str = "event_with_open_note";
/// The moment's key in Settings → Initiative (`MOMENT_PREPARE`).
pub const KEY: &str = "prepare";
/// How long a title may be, as the notice carries it.
const TITLE_MAX: usize = 100;

/// One moment the OS could offer: counts, titles and times, copied from the agenda.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub rule: &'static str,
    /// The calendar's own id for the event.
    pub event_id: String,
    pub event_title: String,
    /// Unix seconds.
    pub start: f64,
    pub note_title: String,
    pub open_items: usize,
    /// The words the event and the note share, as `senses` found them.
    pub words: String,
    /// "Why you got this", in plain words.
    pub reason: String,
}

impl Candidate {
    /// What the "Ask" button puts in the Lens. The person presses send.
    pub fn ask_text(&self) -> String {
        self.ask_text_at("")
    }

    /// The same, naming when the event is ("Fri 27 Nov, 15:00"), as the person's clock has it.
    pub fn ask_text_at(&self, when: &str) -> String {
        let when = if when.is_empty() { String::new() } else { format!(" ({when})") };
        format!("Make a shot list for \u{2018}{}\u{2019}{when} from my note \u{2018}{}\u{2019}.", self.event_title, self.note_title)
    }
}

/// What the person did with a moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Opened,
    Dismissed,
    Ignored,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Opened => "opened",
            Outcome::Dismissed => "dismissed",
            Outcome::Ignored => "ignored",
        }
    }
}

/// Creates the ledger. Idempotent.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agenda_moment (
            rule        TEXT NOT NULL,
            event_id    TEXT NOT NULL,
            start       REAL NOT NULL,
            event_title TEXT NOT NULL,
            note_title  TEXT NOT NULL,
            reason      TEXT NOT NULL,
            offered_at  REAL NOT NULL,
            outcome     TEXT,            -- opened | dismissed | ignored; NULL while it waits
            outcome_at  REAL,
            PRIMARY KEY (rule, event_id)
        );",
    )
}

fn clip(s: &str) -> String {
    s.chars().take(TITLE_MAX).collect()
}

/// The reason, in words: only what the two entries say.
fn reason(c: &Candidate, now: f64) -> String {
    let items = format!("{} open item{}", c.open_items, if c.open_items == 1 { "" } else { "s" });
    format!(
        "Your note \u{201c}{}\u{201d} has {items} and mentions \u{201c}{}\u{201d}, which matches this event, starting in {}.",
        c.note_title,
        c.words,
        in_words(c.start - now)
    )
}

/// Candidates not yet offered, soonest event first, at most one per event. Nothing when Private
/// mode is on or the Brain switch is off (the same two switches as the rest of the agenda), or
/// when the person turned this moment off (`enabled`).
pub fn candidates(conn: &Connection, now: f64, switches: Switches, enabled: bool) -> rusqlite::Result<Vec<Candidate>> {
    if switches.private || !switches.enabled || !enabled {
        return Ok(Vec::new());
    }
    super::store::migrate(conn)?;
    migrate(conn)?;
    let mut stmt = conn.prepare(
        "SELECT ev.source_id, ev.title, ev.due_at, n.title, n.open_items, COALESCE(n.link_words, '') \
         FROM agenda ev JOIN agenda n ON n.link_event = ev.source_id AND n.source = ?4 AND n.is_open = 1 \
              AND n.open_items > 0 \
         WHERE ev.source = ?3 AND ev.kind = 'event' AND ev.is_open = 1 AND ev.snoozed_until <= ?1 \
           AND ev.due_at > ?1 AND ev.due_at <= ?2 \
           AND NOT EXISTS (SELECT 1 FROM agenda_moment m WHERE m.rule = ?5 AND m.event_id = ev.source_id) \
         ORDER BY ev.due_at, n.open_items DESC, n.source_id",
    )?;
    let rows = stmt.query_map(params![now, now + WINDOW_HOURS * HOUR, CALENDAR, NOTES, RULE_EVENT_NOTE], |r| {
        Ok(Candidate {
            rule: RULE_EVENT_NOTE,
            event_id: r.get(0)?,
            event_title: clip(&r.get::<_, String>(1)?),
            start: r.get(2)?,
            note_title: clip(&r.get::<_, String>(3)?),
            open_items: r.get::<_, i64>(4)?.max(0) as usize,
            words: r.get(5)?,
            reason: String::new(),
        })
    })?;
    let mut out: Vec<Candidate> = Vec::new();
    for c in rows.flatten() {
        if out.iter().all(|o| o.event_id != c.event_id) {
            out.push(Candidate { reason: reason(&c, now), ..c });
        }
    }
    Ok(out)
}

/// Writes the ledger row: this moment was offered. `false` when it already had been.
pub fn offered(conn: &Connection, c: &Candidate, now: f64) -> rusqlite::Result<bool> {
    migrate(conn)?;
    let n = conn.execute(
        "INSERT OR IGNORE INTO agenda_moment (rule, event_id, start, event_title, note_title, reason, offered_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![c.rule, c.event_id, c.start, c.event_title, c.note_title, c.reason, now],
    )?;
    Ok(n > 0)
}

/// Records what the person did. The first outcome stays.
pub fn record(conn: &Connection, rule: &str, event_id: &str, outcome: Outcome, now: f64) -> rusqlite::Result<()> {
    migrate(conn)?;
    conn.execute(
        "UPDATE agenda_moment SET outcome = ?3, outcome_at = ?4 WHERE rule = ?1 AND event_id = ?2 AND outcome IS NULL",
        params![rule, event_id, outcome.as_str(), now],
    )?;
    Ok(())
}

/// Moments whose event has begun with no answer are closed as ignored. Returns how many.
pub fn settle_ignored(conn: &Connection, now: f64) -> rusqlite::Result<usize> {
    migrate(conn)?;
    conn.execute("UPDATE agenda_moment SET outcome = 'ignored', outcome_at = ?1 WHERE outcome IS NULL AND start <= ?1", params![now])
}

/// The offered moments that still wait for an answer: `(rule, event_id)`.
pub fn waiting(conn: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    migrate(conn)?;
    let mut stmt = conn.prepare("SELECT rule, event_id FROM agenda_moment WHERE outcome IS NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.flatten().collect())
}

/// What `describe shell` says: the newest moments with key, title, reason and outcome. Titles and
/// counts only (a note's text is never in the agenda).
pub fn describe(conn: &Connection) -> rusqlite::Result<Value> {
    // A reader: it creates nothing, and an agenda that never offered a moment has no ledger yet.
    let Ok(mut stmt) = conn.prepare(
        "SELECT rule, event_title, note_title, reason, outcome FROM agenda_moment ORDER BY offered_at DESC LIMIT 5",
    ) else {
        return Ok(Value::Array(Vec::new()));
    };
    let rows = stmt.query_map([], |r| {
        Ok(json!({
            "key": KEY,
            "rule": r.get::<_, String>(0)?,
            "title": r.get::<_, String>(1)?,
            "note": r.get::<_, String>(2)?,
            "reason": r.get::<_, String>(3)?,
            "outcome": r.get::<_, Option<String>>(4)?.unwrap_or_else(|| "waiting".to_string()),
        }))
    })?;
    Ok(Value::Array(rows.flatten().collect()))
}
