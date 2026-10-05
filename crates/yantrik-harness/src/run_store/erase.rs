//! Erasing a conversation's words from the run store, at the person's request (`redact`).
//!
//! # The acceptance rule
//!
//! A harness could otherwise scrub its own trail whenever it liked. So the store erases only when
//! all of these hold, and otherwise refuses with the reason and changes nothing:
//!
//! - **(a)** the request id names a question *this* run asked (a row in `requests`);
//! - **(b)** the person's stored answer to it is exactly [`ERASE_ANSWER`], that was one of the
//!   question's offered options, and the person pressed it (`requests.by_option`) — a typed
//!   "erase", a typed "Erase" or a free answer does not count;
//! - **(c)** the run is still in flight, or ended no more than [`ERASE_WINDOW_MS`] ago;
//! - **(d)** the redaction comes from the harness and the session that hold the run;
//! - **(e)** every needle is exactly one quoted span of the question, as the person was shown it
//!   when they answered (`redact::quoted_needles`, kept with the answer): they saw exactly the
//!   words that go, set apart in quotes.
//!
//! And once per question: a second `redact` for the same request is refused, and so is a third
//! one refused for being too much to search ([`TOO_MUCH_TIMES`]).
//!
//! # What it touches
//!
//! The words the person and the agent said, in the question's own conversation: every run of the
//! same agent (harness and conversation) held by the same session. The reply text (each run's
//! `text` chunks, joined before matching), its thinking (`thinking` deltas, joined the same way),
//! its `status` lines, why a run failed, the prompt a resumed run carried over, and the prompt of
//! each question the agent asked — the answered question's own included, in its `requests` row and
//! its `request` event, since it quotes the words. Never the record of what happened: tool calls
//! and their output, usage, states, owners, answers and options, sequence numbers and times all
//! stay as they are, and so does the order of the log. What was erased is recorded in `redactions`
//! as the run, the request and how many places, with no words and no digest.
//!
//! # Searched off the lock
//!
//! The rule is checked and the texts are copied out under the store's lock
//! ([`RunStore::prepare_redact`]); the copies are measured against `redact::MAX_WORK` and
//! searched with no lock held ([`RunPlan`]); and only then does [`RunStore::apply_redact`] take an
//! `IMMEDIATE` transaction, check the rule again, claim the question, and replace each match after
//! checking that it still hashes to its needle in the text as it is now.
//!
//! # How it reaches the disk
//!
//! `secure_delete` is on for the update, so the space the old text occupied is zeroed rather than
//! left free; the updates commit in one transaction; and then `wal_checkpoint(TRUNCATE)` copies
//! the new pages into the database and empties the write-ahead log, so the old page images leave
//! `runs.db-wal` too. What this cannot reach is below SQLite: the filesystem's own journal, blocks
//! a truncated file gave back, snapshots and backups.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::{state_in, RunStore};
use crate::redact::{Found, Needle, Prepared, Search, MAX_WORK, TOO_MUCH};

/// The answer that lets a question's words be erased. Exact: the case matters.
pub const ERASE_ANSWER: &str = "Erase";

/// How long after a run ended its question may still be acted on: five minutes.
pub const ERASE_WINDOW_MS: i64 = 5 * 60 * 1000;

/// How many times a question may be refused for [`Refusal::TooMuch`] before it is used up.
pub const TOO_MUCH_TIMES: i64 = 3;

/// The refusal that uses a question up: the third time it was too much to search.
pub const TOO_MUCH_USED_UP: &str =
    "too much to search, three times; this question can no longer be used to erase anything";

/// The refusal when a needle is not quoted in the question the person answered.
pub const NOT_IN_QUESTION: &str = "a needle is not in the question the person answered";

/// What an accepted erasure says when it committed but `secure_delete` could not be put back as
/// it was on the store's connection afterwards.
pub const SECURE_DELETE_WARNING: &str = "secure_delete could not be restored on this connection";

/// What a harness asked to erase, and who is asking.
#[derive(Debug, Clone)]
pub struct Erasure<'a> {
    /// The run that asked the Keep/Erase question.
    pub run_id: u64,
    pub request_id: &'a str,
    /// The harness and session the `redact` came from.
    pub harness: &'a str,
    pub owner: &'a str,
    pub needles: &'a [Needle],
}

/// What the store erased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erased {
    /// The agent whose conversation it was: its harness and conversation.
    pub harness: String,
    pub conversation: String,
    /// How many places in the run store.
    pub places: usize,
    /// Whether the write-ahead log was emptied. `false` only when another connection held it.
    pub checkpointed: bool,
    /// Something that went wrong after the erasure committed, so it still happened:
    /// [`SECURE_DELETE_WARNING`].
    pub warning: Option<String>,
}

/// One erasure, as it is kept: no words and no digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redaction {
    pub run_id: u64,
    pub request_id: String,
    pub places: u64,
    /// Unix milliseconds.
    pub at: i64,
    /// The question was used up by being too much to search ([`TOO_MUCH_TIMES`]): nothing was
    /// erased.
    pub used_up: bool,
}

impl Redaction {
    /// The line it is shown as.
    pub fn said(&self) -> String {
        if self.used_up {
            "Not erased (too much to search).".to_string()
        } else {
            format!("Erased {} place{} at your request.", self.places, if self.places == 1 { "" } else { "s" })
        }
    }
}

/// Why nothing was erased.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    NoSuchRun(u64),
    /// (d): the run belongs to another harness or another session.
    NotYours(u64),
    /// (a): this run never asked it.
    NoSuchRequest { run_id: u64, request_id: String },
    /// (b): the question has not been answered.
    NotAnswered(String),
    /// (b): it was answered with something other than the offered `Erase`.
    NotErase(String),
    /// (b): `Erase` was typed, not pressed.
    Typed(String),
    /// (e): a needle is not quoted in what the person was shown of the question.
    NotInQuestion,
    /// (c): the run ended too long ago.
    Expired { run_id: u64, ended_ms_ago: i64 },
    /// Once per question.
    AlreadyErased(String),
    /// More to search than [`MAX_WORK`].
    TooMuch,
    /// The third [`Refusal::TooMuch`] for one question: it is used up.
    TooMuchUsedUp,
    Storage(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NoSuchRun(id) => write!(f, "run {id} does not exist"),
            Refusal::NotYours(id) => write!(f, "run {id} is not held by this harness and session"),
            Refusal::NoSuchRequest { run_id, request_id } => write!(f, "run {run_id} never asked {request_id:?}"),
            Refusal::NotAnswered(r) => write!(f, "the person has not answered {r:?}"),
            Refusal::NotErase(r) => write!(
                f,
                "the person's answer to {r:?} was not the offered {ERASE_ANSWER:?}, so nothing may be erased"
            ),
            Refusal::Expired { run_id, ended_ms_ago } => write!(
                f,
                "run {run_id} ended {}s ago; a redaction must come within {}s of its end",
                ended_ms_ago / 1000,
                ERASE_WINDOW_MS / 1000
            ),
            Refusal::Typed(r) => write!(
                f,
                "the person typed the answer to {r:?} rather than pressing the offered {ERASE_ANSWER:?}, so nothing may be erased"
            ),
            Refusal::NotInQuestion => f.write_str(NOT_IN_QUESTION),
            Refusal::AlreadyErased(r) => write!(f, "{r:?} has already been acted on; one redaction per question"),
            Refusal::TooMuch => f.write_str(TOO_MUCH),
            Refusal::TooMuchUsedUp => f.write_str(TOO_MUCH_USED_UP),
            Refusal::Storage(e) => write!(f, "run store: {e}"),
        }
    }
}

impl From<rusqlite::Error> for Refusal {
    fn from(e: rusqlite::Error) -> Self {
        Refusal::Storage(e.to_string())
    }
}

impl From<super::RunError> for Refusal {
    fn from(e: super::RunError) -> Self {
        match e {
            super::RunError::NoSuchRun(id) => Refusal::NoSuchRun(id),
            other => Refusal::Storage(other.to_string()),
        }
    }
}

/// How much text an erasure would search in the run store ([`RunStore::redact_size`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSize {
    /// The agent whose conversation it is.
    pub harness: String,
    pub conversation: String,
    /// The raw bytes of every text it reaches.
    pub bytes: u64,
}

/// The run store's half of one erasure: its texts, copied out under the store's lock and searched
/// without it. See [`RunStore::prepare_redact`].
pub struct RunPlan {
    harness: String,
    conversation: String,
    texts: Vec<(Place, Prepared)>,
    found: HashMap<Place, Found>,
}

impl RunPlan {
    /// The agent whose conversation it is.
    pub fn harness(&self) -> &str {
        &self.harness
    }

    pub fn conversation(&self) -> &str {
        &self.conversation
    }

    /// What searching it will cost (see `redact::MAX_WORK`).
    pub fn work(&self, search: &Search) -> u64 {
        self.texts.iter().fold(0u64, |sum, (_, text)| sum.saturating_add(search.work(text)))
    }

    /// Search the copies. No lock is held.
    pub fn search(&mut self, search: &Search) {
        for (place, text) in &self.texts {
            let found = search.find(text);
            if !found.is_empty() {
                self.found.insert(place.clone(), found);
            }
        }
    }
}

impl RunStore {
    /// Erase `e.needles` from the conversation of the agent that ran `e.run_id`, if the acceptance
    /// rule holds (see the module docs) at `now` (Unix milliseconds) and the search is within
    /// `redact::MAX_WORK`: [`RunStore::prepare_redact`], then the search, then
    /// [`RunStore::apply_redact`]. The host does the same steps itself, so the shell's own search
    /// counts towards the same limit.
    pub fn redact(&self, e: &Erasure<'_>, now: i64) -> Result<Erased, Refusal> {
        let search = Search::new(e.needles);
        let size = self.redact_size(e, now)?;
        if search.estimate(size.bytes) > MAX_WORK {
            return Err(self.too_much(e)?);
        }
        let mut plan = self.prepare_redact(e, now)?;
        if plan.work(&search) > MAX_WORK {
            return Err(self.too_much(e)?);
        }
        plan.search(&search);
        self.apply_redact(e, now, &plan)
    }

    /// Check the acceptance rule, and say how much text the erasure would search — the raw bytes
    /// of every text it reaches — without copying any of it.
    pub fn redact_size(&self, e: &Erasure<'_>, now: i64) -> Result<RunSize, Refusal> {
        self.with_refusal(|db| {
            let tx = db.transaction()?;
            let (harness, conversation) = accept(&tx, e, now)?;
            let bytes = size_of(&tx, &harness, &conversation, e.owner)?;
            Ok(RunSize { harness, conversation, bytes })
        })
    }

    /// Count one refusal of `e` for being too much to search. The third uses the question up,
    /// as an accepted erasure would: its claim is taken with nothing erased. The refusal to give.
    pub fn too_much(&self, e: &Erasure<'_>) -> Result<Refusal, Refusal> {
        self.with_refusal(|db| {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO redact_too_much (run_id, request_id, times) VALUES (?1, ?2, 1)
                 ON CONFLICT (run_id, request_id) DO UPDATE SET times = times + 1",
                params![e.run_id as i64, e.request_id],
            )?;
            let times: i64 = tx.query_row(
                "SELECT times FROM redact_too_much WHERE run_id = ?1 AND request_id = ?2",
                params![e.run_id as i64, e.request_id],
                |r| r.get(0),
            )?;
            let refusal = if times >= TOO_MUCH_TIMES {
                tx.execute(
                    "INSERT OR IGNORE INTO redactions (run_id, request_id, places, at, used_up) VALUES (?1, ?2, 0, ?3, 1)",
                    params![e.run_id as i64, e.request_id, super::now_ms()],
                )?;
                Refusal::TooMuchUsedUp
            } else {
                Refusal::TooMuch
            };
            tx.commit()?;
            Ok(refusal)
        })
    }

    /// Check the acceptance rule and copy out the texts of the conversation it would erase, under
    /// the store's lock only for as long as reading takes. Nothing is changed and nothing claimed:
    /// [`RunStore::apply_redact`] checks the rule again, in its own transaction, before it does
    /// anything.
    pub fn prepare_redact(&self, e: &Erasure<'_>, now: i64) -> Result<RunPlan, Refusal> {
        let (harness, conversation, texts) = self.with_refusal(|db| {
            let tx = db.transaction()?;
            let (harness, conversation) = accept(&tx, e, now)?;
            let texts = texts_of(&tx, &harness, &conversation, e.owner)?;
            Ok((harness, conversation, texts))
        })?;
        let texts = texts
            .iter()
            .map(|text| (text.place.clone(), Prepared::new(text.pieces().as_slice())))
            .collect();
        Ok(RunPlan { harness, conversation, texts, found: HashMap::new() })
    }

    /// Apply what `plan` found, if the acceptance rule still holds at `now`: inside one
    /// `IMMEDIATE` transaction the rule is checked again, the question is claimed, and each match
    /// is checked against the text as it is now before it is replaced. Then `secure_delete` is put
    /// back and the write-ahead log emptied.
    pub fn apply_redact(&self, e: &Erasure<'_>, now: i64, plan: &RunPlan) -> Result<Erased, Refusal> {
        self.with_refusal(|db| {
            let previous: i64 = db.query_row("PRAGMA secure_delete", [], |r| r.get(0))?;
            db.pragma_update(None, "secure_delete", "ON")?;
            let outcome = erase_in(db, e, now, plan);
            let checkpointed = match &outcome {
                Ok(_) => checkpoint(db),
                Err(_) => true,
            };
            let restored = restore_secure_delete(db, previous);
            match (outcome, restored) {
                (Ok(erased), Ok(())) => Ok(Erased { checkpointed, ..erased }),
                // It committed: the erasure happened, and the reply must say so.
                (Ok(erased), Err(err)) => {
                    tracing::error!(error = %err, "erased, but secure_delete could not be put back on the run store's connection");
                    Ok(Erased { checkpointed, warning: Some(SECURE_DELETE_WARNING.to_string()), ..erased })
                }
                (Err(refusal), restored) => {
                    if let Err(err) = restored {
                        tracing::error!(error = %err, "secure_delete could not be put back on the run store's connection");
                    }
                    Err(refusal)
                }
            }
        })
    }

    /// Add the places the shell erased from its own copies to the record of `request_id`.
    pub fn set_redaction_places(&self, run_id: u64, request_id: &str, places: u64) -> Result<(), super::RunError> {
        self.with(|db| {
            db.execute(
                "UPDATE redactions SET places = ?3 WHERE run_id = ?1 AND request_id = ?2",
                params![run_id as i64, request_id, places as i64],
            )?;
            Ok(())
        })
    }

    /// The erasures recorded against the runs of one agent, oldest first.
    pub fn redactions(&self, harness: &str, conversation: &str) -> Result<Vec<Redaction>, super::RunError> {
        self.with(|db| {
            let mut stmt = db.prepare(
                "SELECT d.run_id, d.request_id, d.places, d.at, d.used_up FROM redactions d JOIN runs r ON r.run_id = d.run_id
                 WHERE r.harness = ?1 AND r.conversation = ?2 ORDER BY d.at, d.run_id",
            )?;
            let rows = stmt
                .query_map(params![harness, conversation], |r| {
                    Ok(Redaction {
                        run_id: r.get::<_, i64>(0)? as u64,
                        request_id: r.get(1)?,
                        places: r.get::<_, i64>(2)? as u64,
                        at: r.get(3)?,
                        used_up: r.get(4)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            Ok(rows)
        })
    }

    fn with_refusal<R>(&self, f: impl FnOnce(&mut rusqlite::Connection) -> Result<R, Refusal>) -> Result<R, Refusal> {
        let mut db = self.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut db)
    }

    /// Move a finished run's end `ms` into the past, for tests of the window.
    #[cfg(test)]
    pub(crate) fn backdate_end(&self, run_id: u64, ms: i64) {
        self.with(|db| {
            db.execute("UPDATE events SET at = at - ?2 WHERE run_id = ?1", params![run_id as i64, ms])?;
            Ok(())
        })
        .unwrap();
    }
}

/// The acceptance rule (the module docs), read in `db`: the agent whose conversation would be
/// erased, or why not. Changes nothing.
fn accept(db: &Connection, e: &Erasure<'_>, now: i64) -> Result<(String, String), Refusal> {
    let run_id = e.run_id;
    let state = state_in(db, run_id)?;
    let (harness, conversation, owner): (String, String, String) = db.query_row(
        "SELECT harness, conversation, owner FROM runs WHERE run_id = ?1",
        params![run_id as i64],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    // (d) the harness and session that hold the run.
    if harness != e.harness || owner != e.owner {
        return Err(Refusal::NotYours(run_id));
    }
    // (a) a question this run asked.
    let request: Option<(String, String, Option<String>, bool, Option<String>)> = db
        .query_row(
            "SELECT prompt, state, answer, by_option, quoted FROM requests WHERE run_id = ?1 AND request_id = ?2",
            params![run_id as i64, e.request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((prompt, asked_state, answer, by_option, quoted)) = request else {
        return Err(Refusal::NoSuchRequest { run_id, request_id: e.request_id.to_string() });
    };
    // Once per question: checked first, since an erasure took the question's own words too.
    let erased: Option<i64> = db
        .query_row(
            "SELECT 1 FROM redactions WHERE run_id = ?1 AND request_id = ?2",
            params![run_id as i64, e.request_id],
            |r| r.get(0),
        )
        .optional()?;
    if erased.is_some() {
        return Err(Refusal::AlreadyErased(e.request_id.to_string()));
    }
    // (b) answered exactly the offered `Erase`.
    if asked_state != "answered" {
        return Err(Refusal::NotAnswered(e.request_id.to_string()));
    }
    let answer: Value = answer.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or(Value::Null);
    let prompt: Value = serde_json::from_str(&prompt).unwrap_or(Value::Null);
    let offered = prompt["options"].as_array().is_some_and(|o| o.iter().any(|v| v.as_str() == Some(ERASE_ANSWER)));
    if answer.as_str() != Some(ERASE_ANSWER) || !offered {
        return Err(Refusal::NotErase(e.request_id.to_string()));
    }
    // ... pressed, not typed.
    if !by_option {
        return Err(Refusal::Typed(e.request_id.to_string()));
    }
    // (e) every needle is exactly one quoted span of the question as the person was shown it and
    // answered it (`redact::quoted_needles`, kept when the answer was recorded): they saw exactly
    // the words that go, set apart in quotes.
    let quoted: Vec<Needle> = quoted.and_then(|q| serde_json::from_str(&q).ok()).unwrap_or_default();
    if !e.needles.iter().all(|needle| quoted.contains(needle)) {
        return Err(Refusal::NotInQuestion);
    }
    // (c) in flight, or ended within the window.
    if state.is_final() {
        let ended: i64 = db.query_row(
            "SELECT at FROM events WHERE run_id = ?1 AND kind = 'state' ORDER BY seq DESC LIMIT 1",
            params![run_id as i64],
            |r| r.get(0),
        )?;
        if now - ended > ERASE_WINDOW_MS {
            return Err(Refusal::Expired { run_id, ended_ms_ago: now - ended });
        }
    }
    Ok((harness, conversation))
}

fn erase_in(db: &mut rusqlite::Connection, e: &Erasure<'_>, now: i64, plan: &RunPlan) -> Result<Erased, Refusal> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let run_id = e.run_id;
    let (harness, conversation) = accept(&tx, e, now)?;
    // The record is the claim, in the same transaction as the rule.
    let claimed = tx.execute(
        "INSERT OR IGNORE INTO redactions (run_id, request_id, places, at) VALUES (?1, ?2, 0, ?3)",
        params![run_id as i64, e.request_id, now],
    )?;
    if claimed == 0 {
        return Err(Refusal::AlreadyErased(e.request_id.to_string()));
    }

    let mut places = 0;
    if harness == plan.harness && conversation == plan.conversation {
        for text in texts_of(&tx, &harness, &conversation, e.owner)? {
            // A text that was not there when the plan was searched has nothing found in it.
            let Some(found) = plan.found.get(&text.place) else { continue };
            let pieces = text.pieces();
            let Some((erased, n)) = found.apply(&pieces) else { continue };
            // A question's words are counted once: its `request` event holds the same words.
            if !matches!(text.place, Place::Question(..)) {
                places += n;
            }
            for (row, (before, after)) in text.rows.iter().zip(pieces.iter().zip(erased)) {
                if *before != after {
                    row.write(&tx, after)?;
                }
            }
        }
    }
    tx.execute(
        "UPDATE redactions SET places = ?3 WHERE run_id = ?1 AND request_id = ?2",
        params![run_id as i64, e.request_id, places as i64],
    )?;
    tx.commit()?;
    Ok(Erased { harness, conversation, places, checkpointed: false, warning: None })
}

/// Where one text of a conversation is, so it can be found again inside the transaction.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Place {
    /// A run's reply: its `text` chunks, joined.
    Reply(i64),
    /// A run's `thinking` deltas, joined.
    Thinking(i64),
    /// Why a run failed: the `failure` event at this sequence number.
    Failure(i64, i64),
    /// The prompt of the `request` event at this sequence number.
    Asked(i64, i64),
    /// The prompt in a question's `requests` row.
    Question(i64, String),
    /// A `status` line the agent sent, at this sequence number.
    Status(i64, i64),
    /// The person's prompt a resumed run carried over, at this sequence number.
    Resumed(i64, i64),
}

/// One text: the rows it is in, a piece in each.
struct Text {
    place: Place,
    rows: Vec<Row>,
}

impl Text {
    fn pieces(&self) -> Vec<&str> {
        self.rows.iter().map(Row::piece).collect()
    }
}

/// A row holding one piece of a text, as a string at `path` inside its JSON.
struct Row {
    at: RowAt,
    value: Value,
    path: &'static [&'static str],
}

enum RowAt {
    Event { run: i64, seq: i64 },
    Request { run: i64, request_id: String },
}

impl Row {
    fn new(at: RowAt, value: Value, path: &'static [&'static str]) -> Option<Row> {
        let row = Row { at, value, path };
        if row.string().is_some() {
            Some(row)
        } else {
            None
        }
    }

    fn string(&self) -> Option<&str> {
        let mut at = &self.value;
        for key in self.path {
            at = at.get(*key)?;
        }
        at.as_str()
    }

    fn piece(&self) -> &str {
        self.string().unwrap_or_default()
    }

    /// Write `text` back as this row's piece.
    fn write(&self, tx: &Connection, text: String) -> Result<(), Refusal> {
        let mut value = self.value.clone();
        let mut at = &mut value;
        for key in self.path {
            match at.get_mut(*key) {
                Some(next) => at = next,
                None => return Ok(()),
            }
        }
        *at = Value::String(text);
        match &self.at {
            RowAt::Event { run, seq } => tx.execute(
                "UPDATE events SET payload = ?3 WHERE run_id = ?1 AND seq = ?2",
                params![run, seq, value.to_string()],
            )?,
            RowAt::Request { run, request_id } => tx.execute(
                "UPDATE requests SET prompt = ?3 WHERE run_id = ?1 AND request_id = ?2",
                params![run, request_id, value.to_string()],
            )?,
        };
        Ok(())
    }
}

/// Every text of one agent's conversation the erasure reaches, in every run of it: the reply and
/// the thinking (each joined across its chunks), why a run failed, and the prompt of each
/// question (its `request` event and its `requests` row). Never a tool call, an answer, a state.
fn texts_of(db: &Connection, harness: &str, conversation: &str, owner: &str) -> Result<Vec<Text>, Refusal> {
    let runs: Vec<i64> = {
        let mut stmt = db.prepare(
            "SELECT run_id FROM runs WHERE harness = ?1 AND conversation = ?2 AND owner = ?3 ORDER BY run_id",
        )?;
        let ids = stmt.query_map(params![harness, conversation, owner], |r| r.get(0))?.collect::<Result<_, _>>()?;
        ids
    };
    let mut texts = Vec::new();
    for run in runs {
        let rows: Vec<(i64, String, Value)> = {
            let mut stmt = db.prepare(
                "SELECT seq, kind, payload FROM events WHERE run_id = ?1
                 AND kind IN ('text', 'event', 'failure', 'request', 'resumed') ORDER BY seq",
            )?;
            let rows = stmt
                .query_map(params![run], |r| {
                    let payload: String = r.get(2)?;
                    Ok((r.get(0)?, r.get(1)?, serde_json::from_str(&payload).unwrap_or(Value::Null)))
                })?
                .collect::<Result<_, _>>()?;
            rows
        };
        let mut reply = Vec::new();
        let mut thinking = Vec::new();
        for (seq, kind, payload) in rows {
            let at = RowAt::Event { run, seq };
            match kind.as_str() {
                "text" => reply.extend(Row::new(at, payload, &["delta"])),
                "event" if payload["kind"] == "thinking" => thinking.extend(Row::new(at, payload, &["delta"])),
                "event" if payload["kind"] == "status" => {
                    texts.extend(Row::new(at, payload, &["text"]).map(|row| Text { place: Place::Status(run, seq), rows: vec![row] }))
                }
                "resumed" => texts.extend(
                    Row::new(at, payload, &["prompt"]).map(|row| Text { place: Place::Resumed(run, seq), rows: vec![row] }),
                ),
                "failure" => texts.extend(Row::new(at, payload, &["why"]).map(|row| Text { place: Place::Failure(run, seq), rows: vec![row] })),
                "request" => texts.extend(
                    Row::new(at, payload, &["prompt", "prompt"]).map(|row| Text { place: Place::Asked(run, seq), rows: vec![row] }),
                ),
                _ => {}
            }
        }
        if !reply.is_empty() {
            texts.push(Text { place: Place::Reply(run), rows: reply });
        }
        if !thinking.is_empty() {
            texts.push(Text { place: Place::Thinking(run), rows: thinking });
        }
        let questions: Vec<(String, String)> = {
            let mut stmt = db.prepare("SELECT request_id, prompt FROM requests WHERE run_id = ?1")?;
            let rows = stmt.query_map(params![run], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            rows
        };
        for (request_id, prompt) in questions {
            let prompt: Value = serde_json::from_str(&prompt).unwrap_or(Value::Null);
            let place = Place::Question(run, request_id.clone());
            texts.extend(Row::new(RowAt::Request { run, request_id }, prompt, &["prompt"]).map(|row| Text { place, rows: vec![row] }));
        }
    }
    Ok(texts)
}

#[cfg(test)]
thread_local! {
    /// Makes putting `secure_delete` back fail on this thread, as a broken connection would.
    pub(crate) static FAIL_RESTORE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The raw bytes of every text [`texts_of`] would read, summed by SQLite without reading them out:
/// an upper bound on the text itself (each is held inside its JSON).
fn size_of(db: &Connection, harness: &str, conversation: &str, owner: &str) -> Result<u64, Refusal> {
    let events: i64 = db.query_row(
        "SELECT COALESCE(SUM(LENGTH(CAST(e.payload AS BLOB))), 0) FROM events e JOIN runs r ON r.run_id = e.run_id
         WHERE r.harness = ?1 AND r.conversation = ?2 AND r.owner = ?3
         AND (e.kind IN ('text', 'failure', 'request', 'resumed')
              OR (e.kind = 'event' AND json_extract(e.payload, '$.kind') IN ('thinking', 'status')))",
        params![harness, conversation, owner],
        |r| r.get(0),
    )?;
    let questions: i64 = db.query_row(
        "SELECT COALESCE(SUM(LENGTH(CAST(q.prompt AS BLOB))), 0) FROM requests q JOIN runs r ON r.run_id = q.run_id
         WHERE r.harness = ?1 AND r.conversation = ?2 AND r.owner = ?3",
        params![harness, conversation, owner],
        |r| r.get(0),
    )?;
    Ok((events.max(0) as u64).saturating_add(questions.max(0) as u64))
}

/// Put `secure_delete` back as it was before the erasure.
fn restore_secure_delete(db: &Connection, previous: i64) -> rusqlite::Result<()> {
    #[cfg(test)]
    if FAIL_RESTORE.with(|fail| fail.get()) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    db.pragma_update(None, "secure_delete", previous)
}

/// Copy the new pages into the database and empty the write-ahead log. `false` when another
/// connection kept it from finishing.
fn checkpoint(db: &rusqlite::Connection) -> bool {
    match db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get::<_, i64>(0)) {
        Ok(busy) => busy == 0,
        Err(e) => {
            tracing::error!(error = %e, "the run store could not empty its write-ahead log after an erasure");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::MARKER;
    use crate::run_store::{now_ms, RunState};
    use serde_json::json;

    const SECRET: &str = "Priya lives at 12 Elm Street";

    fn needles() -> Vec<Needle> {
        vec![Needle::of("Priya"), Needle::of("12 Elm Street")]
    }

    /// A store where run 1 (`mind`, conversation `main`, session `s1`) said the secret across two
    /// chunks, ran a tool that named it, asked Keep/Erase as `forget-1`, and was answered `answer`.
    fn store_with(store: RunStore, answer: &str) -> RunStore {
        store.start(1, "mind", "main", "s1").unwrap();
        store.append(1, "text", &json!({"delta": "Noted: Pri"})).unwrap();
        store.append(1, "text", &json!({"delta": "ya lives at 12 Elm Street."})).unwrap();
        store.append(1, "event", &json!({"kind": "thinking", "delta": "remember Priya"})).unwrap();
        store
            .append(1, "event", &json!({"kind": "tool_start", "call": "t1", "name": "notes", "args": {"text": SECRET}}))
            .unwrap();
        store.append(1, "event", &json!({"kind": "tool_output", "call": "t1", "delta": SECRET})).unwrap();
        store
            .ask(1, "forget-1", &json!({"prompt": "Forget that \"Priya\" lives at \"12 Elm Street\"?", "options": ["Keep", "Erase"]}))
            .unwrap();
        store.answer(1, "forget-1", &json!(answer), true).unwrap();
        store
    }

    fn erasure<'a>(run_id: u64, request_id: &'a str, needles: &'a [Needle]) -> Erasure<'a> {
        Erasure { run_id, request_id, harness: "mind", owner: "s1", needles }
    }

    fn payloads(store: &RunStore, run: u64) -> Vec<(String, Value)> {
        store.events(run, 0, super::super::PAGE_MAX).unwrap().events.into_iter().map(|e| (e.kind, e.payload)).collect()
    }

    fn text_of(store: &RunStore, run: u64) -> String {
        payloads(store, run)
            .into_iter()
            .filter(|(k, _)| k == "text")
            .map(|(_, p)| p["delta"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn after_erase_the_words_go_and_the_record_of_what_happened_stays() {
        let store = store_with(RunStore::in_memory().unwrap(), "Erase");
        let before = payloads(&store, 1);
        let needles = needles();
        let erased = store.redact(&erasure(1, "forget-1", &needles), now_ms()).unwrap();
        assert_eq!((erased.harness.as_str(), erased.conversation.as_str()), ("mind", "main"));

        // The reply, joined across its chunks: "Pri" + "ya" was one name.
        assert_eq!(text_of(&store, 1), format!("Noted: {MARKER} lives at {MARKER}."));
        let after = payloads(&store, 1);
        assert_eq!(after.len(), before.len(), "nothing is added to or taken from the log");
        let thinking = after.iter().find(|(_, p)| p["kind"] == "thinking").unwrap();
        assert_eq!(thinking.1["delta"], format!("remember {MARKER}"));
        // The question's words go; its id, options and answer stay.
        let asked = after.iter().find(|(k, _)| k == "request").unwrap();
        assert_eq!(asked.1["prompt"]["prompt"], format!("Forget that \"{MARKER}\" lives at \"{MARKER}\"?"));
        assert_eq!(asked.1["prompt"]["options"], json!(["Keep", "Erase"]));
        assert_eq!(after.iter().find(|(k, _)| k == "answer").unwrap().1["answer"], "Erase");
        // Tool calls are the record of what happened: untouched, byte for byte.
        for (kind, payload) in &before {
            if kind == "event" && payload["kind"] != "thinking" {
                assert!(after.contains(&(kind.clone(), payload.clone())), "{payload} was changed");
            }
        }
        // 2 in the reply, 1 in the thinking, 2 in the question.
        assert_eq!(erased.places, 5);
        let kept = store.redactions("mind", "main").unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].run_id, kept[0].request_id.as_str(), kept[0].places), (1, "forget-1", 5));
    }

    #[test]
    fn every_run_of_the_same_agent_is_erased_and_no_other_agents() {
        let store = store_with(RunStore::in_memory().unwrap(), "Erase");
        store.start(2, "mind", "main", "s1").unwrap();
        store.append(2, "text", &json!({"delta": "Priya again"})).unwrap();
        store.start(3, "mind", "c-other", "s1").unwrap();
        store.append(3, "text", &json!({"delta": "Priya elsewhere"})).unwrap();
        store.redact(&erasure(1, "forget-1", &needles()), now_ms()).unwrap();
        assert_eq!(text_of(&store, 2), format!("{MARKER} again"));
        assert_eq!(text_of(&store, 3), "Priya elsewhere", "another agent's conversation is not this one");
    }

    #[test]
    fn refused_unless_the_person_answered_the_offered_erase() {
        let n = needles();
        for answer in ["Keep", "erase", "ERASE", "Erase please"] {
            let store = store_with(RunStore::in_memory().unwrap(), answer);
            assert_eq!(
                store.redact(&erasure(1, "forget-1", &n), now_ms()),
                Err(Refusal::NotErase("forget-1".into())),
                "answered {answer:?}"
            );
            assert!(text_of(&store, 1).contains("Elm Street"), "nothing changed after {answer:?}");
            assert!(store.redactions("mind", "main").unwrap().is_empty());
        }
        // `Erase` typed as a free answer to a question that never offered it.
        let store = RunStore::in_memory().unwrap();
        store.start(1, "mind", "main", "s1").unwrap();
        store.ask(1, "free", &json!({"prompt": "What should I do?", "options": []})).unwrap();
        store.answer(1, "free", &json!("Erase"), true).unwrap();
        assert_eq!(store.redact(&erasure(1, "free", &n), now_ms()), Err(Refusal::NotErase("free".into())));
        // Not answered yet.
        store.ask(1, "open", &json!({"prompt": "Forget?", "options": ["Keep", "Erase"]})).unwrap();
        assert_eq!(store.redact(&erasure(1, "open", &n), now_ms()), Err(Refusal::NotAnswered("open".into())));
    }

    #[test]
    fn refused_for_a_question_never_asked_another_runs_another_session_and_a_second_use() {
        let store = store_with(RunStore::in_memory().unwrap(), "Erase");
        store.start(2, "mind", "main", "s1").unwrap();
        let n = needles();
        assert_eq!(
            store.redact(&erasure(1, "nope", &n), now_ms()),
            Err(Refusal::NoSuchRequest { run_id: 1, request_id: "nope".into() })
        );
        // Run 2 is the same agent's, but it is not the run that asked.
        assert_eq!(
            store.redact(&erasure(2, "forget-1", &n), now_ms()),
            Err(Refusal::NoSuchRequest { run_id: 2, request_id: "forget-1".into() })
        );
        assert_eq!(store.redact(&erasure(9, "forget-1", &n), now_ms()), Err(Refusal::NoSuchRun(9)));
        let other_session = Erasure { owner: "s2", ..erasure(1, "forget-1", &n) };
        assert_eq!(store.redact(&other_session, now_ms()), Err(Refusal::NotYours(1)));
        let other_harness = Erasure { harness: "pi", ..erasure(1, "forget-1", &n) };
        assert_eq!(store.redact(&other_harness, now_ms()), Err(Refusal::NotYours(1)));
        assert!(text_of(&store, 1).contains("Elm Street"));

        store.redact(&erasure(1, "forget-1", &n), now_ms()).unwrap();
        assert_eq!(
            store.redact(&erasure(1, "forget-1", &[Needle::of("Noted")]), now_ms()),
            Err(Refusal::AlreadyErased("forget-1".into()))
        );
        assert!(text_of(&store, 1).starts_with("Noted"), "the second one changed nothing");
    }

    #[test]
    fn a_run_that_ended_is_erased_within_five_minutes_and_refused_after() {
        let n = needles();
        let store = store_with(RunStore::in_memory().unwrap(), "Erase");
        store.transition(1, RunState::Done).unwrap();
        store.backdate_end(1, ERASE_WINDOW_MS - 5_000);
        assert!(store.redact(&erasure(1, "forget-1", &n), now_ms()).is_ok(), "within the window");

        let store = store_with(RunStore::in_memory().unwrap(), "Erase");
        store.transition(1, RunState::Done).unwrap();
        store.backdate_end(1, ERASE_WINDOW_MS + 5_000);
        assert!(matches!(store.redact(&erasure(1, "forget-1", &n), now_ms()), Err(Refusal::Expired { run_id: 1, .. })));
        assert!(text_of(&store, 1).contains("Elm Street"));
    }

    #[test]
    fn composed_and_decomposed_words_are_erased_alike() {
        let store = RunStore::in_memory().unwrap();
        store.start(1, "mind", "main", "s1").unwrap();
        store.append(1, "text", &json!({"delta": "We met at Cafe\u{301} Lune"})).unwrap();
        // Curly quotes delimit as straight ones do.
        store.ask(1, "f", &json!({"prompt": "Forget \u{201c}Café Lune\u{201d}?", "options": ["Keep", "Erase"]})).unwrap();
        store.answer(1, "f", &json!("Erase"), true).unwrap();
        let composed = [Needle::of("Caf\u{e9} Lune")];
        let erased = store.redact(&erasure(1, "f", &composed), now_ms()).unwrap();
        // The reply's, and the question's own.
        assert_eq!(erased.places, 2);
        assert_eq!(text_of(&store, 1), format!("We met at {MARKER}"));
    }

    #[test]
    fn one_needle_erases_every_case_the_words_were_written_in_and_the_rest_keeps_its_case() {
        let store = RunStore::in_memory().unwrap();
        store.start(1, "mind", "main", "s1").unwrap();
        store.append(1, "text", &json!({"delta": "Code THROWAWAY-ERASE2, then Throw"})).unwrap();
        store.append(1, "text", &json!({"delta": "away-Erase2 and throwaway-erase2. OK?"})).unwrap();
        store.append(1, "event", &json!({"kind": "thinking", "delta": "Keep Throwaway-Erase2 SAFE"})).unwrap();
        store.ask(1, "f", &json!({"prompt": "Forget \"THROWAWAY-ERASE2\"?", "options": ["Keep", "Erase"]})).unwrap();
        store.answer(1, "f", &json!("Erase"), true).unwrap();
        let needle = [Needle::of("throwaway-erase2")];
        let erased = store.redact(&erasure(1, "f", &needle), now_ms()).unwrap();
        assert_eq!(text_of(&store, 1), format!("Code {MARKER}, then {MARKER} and {MARKER}. OK?"));
        let after = payloads(&store, 1);
        let thinking = after.iter().find(|(_, p)| p["kind"] == "thinking").unwrap();
        assert_eq!(thinking.1["delta"], format!("Keep {MARKER} SAFE"));
        let asked = after.iter().find(|(k, _)| k == "request").unwrap();
        assert_eq!(asked.1["prompt"]["prompt"], format!("Forget \"{MARKER}\"?"));
        // 3 in the reply, 1 in the thinking, 1 in the question.
        assert_eq!(erased.places, 5);
        let all = serde_json::to_string(&after).unwrap().to_lowercase();
        assert!(!all.contains("throwaway-erase2"), "no case of the words is left: {all}");
    }

    #[test]
    fn the_old_text_leaves_the_write_ahead_log_and_the_database_file() {
        let dir = std::env::temp_dir().join(format!("yantrik-erase-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runs.db");
        let wal = dir.join("runs.db-wal");
        // Stored in mixed case; the needle is derived from the lowercase form.
        let stored = "Zanzibar-QUOKKA-7741";
        let lower = stored.to_lowercase();
        let store = RunStore::open(&path).unwrap();
        store.start(1, "mind", "main", "s1").unwrap();
        store.append(1, "text", &json!({"delta": format!("the code is {stored}, keep it")})).unwrap();
        // The question quotes the words, as it must; and it is erased with them.
        store.ask(1, "f", &json!({"prompt": format!("Forget the code \"{stored}\"?"), "options": ["Keep", "Erase"]})).unwrap();
        store.answer(1, "f", &json!("Erase"), true).unwrap();
        // Any case of the words, in the file's bytes.
        let holds = |file: &std::path::Path| {
            let bytes = std::fs::read(file).unwrap_or_default().to_ascii_lowercase();
            bytes.windows(lower.len()).any(|w| w == lower.as_bytes())
        };
        assert!(holds(&wal), "before: the write-ahead log holds the words, so the check below means something");

        let erased = store.redact(&erasure(1, "f", &[Needle::of(&lower)]), now_ms()).unwrap();
        assert!(erased.checkpointed);
        assert!(!holds(&wal), "after: runs.db-wal holds no copy of the words, in any case");
        assert!(!holds(&path), "after: runs.db holds no copy of the words, in any case");
        assert_eq!(text_of(&store, 1), format!("the code is {MARKER}, keep it"));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_erasure_that_committed_says_so_even_when_secure_delete_cannot_be_put_back() {
        let store = store_with(RunStore::in_memory().unwrap(), "Erase");
        FAIL_RESTORE.with(|fail| fail.set(true));
        let erased = store.redact(&erasure(1, "forget-1", &needles()), now_ms());
        FAIL_RESTORE.with(|fail| fail.set(false));
        let erased = erased.expect("it committed, so it is not a refusal");
        assert_eq!(erased.warning.as_deref(), Some(SECURE_DELETE_WARNING));
        assert_eq!(erased.places, 5);
        assert_eq!(text_of(&store, 1), format!("Noted: {MARKER} lives at {MARKER}."));
        assert_eq!(store.redactions("mind", "main").unwrap().len(), 1);
    }

    /// A run whose reply is half a megabyte, asked a question quoting "Priya" and sixteen runs of
    /// 100 to 115 x's, answered Erase.
    fn big_store() -> RunStore {
        let store = RunStore::in_memory().unwrap();
        store.start(1, "mind", "main", "s1").unwrap();
        let chunk = "the quick brown fox jumps over Priya's lazy dog. ".repeat(1_000);
        for _ in 0..10 {
            store.append(1, "text", &json!({"delta": chunk})).unwrap();
        }
        let runs: Vec<String> = (0..16).map(|i| format!("\"{}\"", "x".repeat(100 + i))).collect();
        let prompt = format!("Forget \"Priya\"? And {}", runs.join(" "));
        assert!(prompt.chars().count() < 1_999, "all of it shown");
        store.ask(1, "f", &json!({"prompt": prompt, "options": ["Keep", "Erase"]})).unwrap();
        store.answer(1, "f", &json!("Erase"), true).unwrap();
        store
    }

    /// 16 needles of 100 to 115 characters, each a span the question quoted: over half a megabyte,
    /// every window hashed in full would be about 1.4 GB of SHA-256.
    fn long_needles() -> Vec<Needle> {
        (0..16).map(|i| Needle::of(&"x".repeat(100 + i))).collect()
    }

    #[test]
    fn too_much_to_search_is_refused_before_anything_is_searched_or_touched() {
        let store = big_store();
        let long = long_needles();
        let started = std::time::Instant::now();
        assert_eq!(store.redact(&erasure(1, "f", &long), now_ms()), Err(Refusal::TooMuch));
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "refused without searching: {:?}", started.elapsed());
        assert_eq!(Refusal::TooMuch.to_string(), "too much to search; ask again with fewer or shorter needles");
        // Nothing touched, and the question not used up: a smaller one is still taken.
        assert!(store.redactions("mind", "main").unwrap().is_empty());
        assert!(text_of(&store, 1).contains("Priya"));
        let erased = store.redact(&erasure(1, "f", &[Needle::of("Priya")]), now_ms()).unwrap();
        // The reply's, and the question's own.
        assert_eq!(erased.places, 10_000 + 1);
        assert!(!text_of(&store, 1).contains("Priya"));
    }

    #[test]
    fn the_third_refusal_for_too_much_uses_the_question_up() {
        let store = big_store();
        let long = long_needles();
        assert_eq!(store.redact(&erasure(1, "f", &long), now_ms()), Err(Refusal::TooMuch));
        assert_eq!(store.redact(&erasure(1, "f", &long), now_ms()), Err(Refusal::TooMuch));
        assert_eq!(store.redact(&erasure(1, "f", &long), now_ms()), Err(Refusal::TooMuchUsedUp));
        assert_eq!(
            store.redact(&erasure(1, "f", &[Needle::of("Priya")]), now_ms()),
            Err(Refusal::AlreadyErased("f".into())),
            "used up: no more tries"
        );
        assert!(text_of(&store, 1).contains("Priya"), "and nothing was erased");
    }

    #[test]
    fn a_needle_the_question_did_not_quote_is_refused_and_changes_nothing() {
        let store = store_with(RunStore::in_memory().unwrap(), "Erase");
        store.append(1, "text", &json!({"delta": " Don't touch ~/Photos."})).unwrap();
        let before = payloads(&store, 1);
        // "Priya" is quoted; "~/Photos" never was.
        let needles = [Needle::of("Priya"), Needle::of("~/Photos")];
        assert_eq!(store.redact(&erasure(1, "forget-1", &needles), now_ms()), Err(Refusal::NotInQuestion));
        assert_eq!(Refusal::NotInQuestion.to_string(), "a needle is not in the question the person answered");
        assert_eq!(payloads(&store, 1), before);
        assert!(store.redactions("mind", "main").unwrap().is_empty(), "the question is not used up");
        // Only what the person was shown counts: past the card's 1999 characters is not quoted.
        let store = RunStore::in_memory().unwrap();
        store.start(1, "mind", "main", "s1").unwrap();
        let prompt = format!("{} \"Priya\"", "y".repeat(2_000));
        store.ask(1, "f", &json!({"prompt": prompt, "options": ["Keep", "Erase"]})).unwrap();
        store.answer(1, "f", &json!("Erase"), true).unwrap();
        assert_eq!(store.redact(&erasure(1, "f", &[Needle::of("Priya")]), now_ms()), Err(Refusal::NotInQuestion));
    }

    #[test]
    fn a_typed_erase_is_not_a_pressed_one() {
        let store = RunStore::in_memory().unwrap();
        store.start(1, "mind", "main", "s1").unwrap();
        store.append(1, "text", &json!({"delta": "Priya"})).unwrap();
        store.ask(1, "f", &json!({"prompt": "Forget \"Priya\"?", "options": ["Keep", "Erase"]})).unwrap();
        store.answer(1, "f", &json!("Erase"), false).unwrap();
        assert_eq!(store.redact(&erasure(1, "f", &[Needle::of("Priya")]), now_ms()), Err(Refusal::Typed("f".into())));
        assert_eq!(text_of(&store, 1), "Priya");
    }

    #[test]
    fn status_lines_and_resumed_prompts_are_erased_and_other_sessions_runs_are_not_searched() {
        let store = store_with(RunStore::in_memory().unwrap(), "Erase");
        store.append(1, "event", &json!({"kind": "status", "text": "saving Priya"})).unwrap();
        store.start(2, "mind", "main", "s1").unwrap();
        store.append(2, "resumed", &json!({"was": 1, "prompt": "my sister is Priya"})).unwrap();
        // The same agent's run under an earlier session is outside this question's conversation.
        store.start(3, "mind", "main", "s0").unwrap();
        store.append(3, "text", &json!({"delta": "Priya, from before"})).unwrap();
        store.redact(&erasure(1, "forget-1", &needles()), now_ms()).unwrap();
        let status = payloads(&store, 1).into_iter().find(|(_, p)| p["kind"] == "status").unwrap();
        assert_eq!(status.1["text"], format!("saving {MARKER}"));
        let resumed = payloads(&store, 2).into_iter().find(|(k, _)| k == "resumed").unwrap();
        assert_eq!(resumed.1["prompt"], format!("my sister is {MARKER}"));
        assert_eq!(text_of(&store, 3), "Priya, from before");
    }

    #[test]
    fn a_plan_searched_before_more_was_said_erases_what_it_found_and_checks_it_again() {
        let store = store_with(RunStore::in_memory().unwrap(), "Erase");
        let needles = needles();
        let search = Search::new(&needles);
        let e = erasure(1, "forget-1", &needles);
        let mut plan = store.prepare_redact(&e, now_ms()).unwrap();
        plan.search(&search);
        // More of the reply arrives between the search and the transaction.
        store.append(1, "text", &json!({"delta": " Priya again."})).unwrap();
        let erased = store.apply_redact(&e, now_ms(), &plan).unwrap();
        // What was searched is erased; what came after the search was not searched.
        assert_eq!(text_of(&store, 1), format!("Noted: {MARKER} lives at {MARKER}. Priya again."));
        assert_eq!(erased.places, 5);
        // The plan cannot be applied twice: the question is claimed.
        assert_eq!(store.apply_redact(&e, now_ms(), &plan), Err(Refusal::AlreadyErased("forget-1".into())));
    }

    /// A store whose run 1 said `said`, asked `question` as `f`, and was answered Erase.
    fn asked(said: &str, question: &str) -> RunStore {
        let store = RunStore::in_memory().unwrap();
        store.start(1, "mind", "main", "s1").unwrap();
        store.append(1, "text", &json!({"delta": said})).unwrap();
        store.ask(1, "f", &json!({"prompt": question, "options": ["Keep", "Erase"]})).unwrap();
        store.answer(1, "f", &json!("Erase"), true).unwrap();
        store
    }

    #[test]
    fn a_needle_must_equal_a_whole_quoted_span_of_the_question() {
        let said = "I will not delete ~/Photos. Noted.";
        let question = "Forget that you said \u{201c}you will not delete ~/Photos\u{201d}?";
        // "not", "e": too short to be a span the question can offer.
        for short in ["not", "e"] {
            let store = asked(said, question);
            assert_eq!(store.redact(&erasure(1, "f", &[Needle::of(short)]), now_ms()), Err(Refusal::NotInQuestion), "{short:?}");
            assert_eq!(text_of(&store, 1), said);
        }
        // A piece of the quoted span, or words outside it: refused, nothing changed.
        for piece in ["not delete", "~/Photos", "Forget that you said"] {
            let store = asked(said, question);
            assert_eq!(store.redact(&erasure(1, "f", &[Needle::of(piece)]), now_ms()), Err(Refusal::NotInQuestion), "{piece:?}");
            assert_eq!(text_of(&store, 1), said);
        }
        // The whole span, in curly quotes, in another case: accepted.
        let store = asked(said, question);
        store.redact(&erasure(1, "f", &[Needle::of("You will NOT delete ~/Photos")]), now_ms()).unwrap();
        assert_eq!(text_of(&store, 1), said, "the reply says \"I will\", not the span");
        let store = asked("you will not delete ~/Photos, I promise", question);
        store.redact(&erasure(1, "f", &[Needle::of("You will NOT delete ~/Photos")]), now_ms()).unwrap();
        assert_eq!(text_of(&store, 1), format!("{MARKER}, I promise"));
    }

    #[test]
    fn the_question_is_checked_as_it_was_answered_not_as_a_later_erasure_left_it() {
        // Two questions in one run, both answered: "f" quotes "secret code", "g" quotes "code".
        let store = RunStore::in_memory().unwrap();
        store.start(1, "mind", "main", "s1").unwrap();
        store.append(1, "text", &json!({"delta": "the secret code is here"})).unwrap();
        store.ask(1, "f", &json!({"prompt": "Forget \"the secret code\"?", "options": ["Keep", "Erase"]})).unwrap();
        store.answer(1, "f", &json!("Erase"), true).unwrap();
        store.ask(1, "g", &json!({"prompt": "Forget \"code\" too?", "options": ["Keep", "Erase"]})).unwrap();
        store.answer(1, "g", &json!("Erase"), true).unwrap();
        // Erasing "code" (through "g") rewrites the prompt of "f": "the secret [erased at your request]".
        store.redact(&erasure(1, "g", &[Needle::of("code")]), now_ms()).unwrap();
        let rewritten = format!("the secret {MARKER}");
        assert!(payloads(&store, 1).iter().any(|(k, p)| k == "request" && p["prompt"]["prompt"] == format!("Forget \"{rewritten}\"?")));
        // That rewritten span was never shown to the person: refused.
        assert_eq!(store.redact(&erasure(1, "f", &[Needle::of(&rewritten)]), now_ms()), Err(Refusal::NotInQuestion));
        // What they were shown, and answered, still is the question's span.
        assert!(store.redact(&erasure(1, "f", &[Needle::of("the secret code")]), now_ms()).is_ok());
    }

    #[test]
    fn a_question_used_up_by_too_much_is_recorded_as_not_erased() {
        let store = big_store();
        let long = long_needles();
        for _ in 0..3 {
            let _ = store.redact(&erasure(1, "f", &long), now_ms());
        }
        let kept = store.redactions("mind", "main").unwrap();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].used_up);
        assert_eq!(kept[0].said(), "Not erased (too much to search).");
        let erased = store_with(RunStore::in_memory().unwrap(), "Erase");
        erased.redact(&erasure(1, "forget-1", &needles()), now_ms()).unwrap();
        let kept = erased.redactions("mind", "main").unwrap();
        assert!(!kept[0].used_up);
        assert_eq!(kept[0].said(), "Erased 5 places at your request.");
    }
}
