//! Runs that outlive the chat: a turn, persisted.
//!
//! A forty-minute task used to be a chat turn whose only identity was a live channel. Closing the
//! panel closed the channel and the work with it; an approval could reach the wrong run; and "what
//! is it doing now" could only be answered by reading another process's database (#25).
//!
//! This store gives the turn id the host already mints a durable record: the run's state, and an
//! append-only, sequenced log of what happened in it. It is deliberately not a task system — no
//! second id namespace, no scheduler, no resumable execution. A run that was unfinished when the
//! host stopped is *orphaned*: readable, never resumed, and replay never re-executes anything.
//!
//! # Guarantees
//!
//! - Every event gets the next sequence number in its run, `(run_id, seq)` unique, and is written
//!   before the call that recorded it returns — so the host can acknowledge only what is stored.
//!   The database runs in WAL mode with `synchronous = NORMAL`: a committed event survives the
//!   process being killed; the last instant before a power cut may not.
//! - A request for input is consumed at most once. The answer, the event that records it and the
//!   run's return to `running` commit together, and a second answer to the same request, an answer
//!   to a request the run never made, and an answer after the run ended are each refused with a
//!   reason. (Run R asks A; the person answers twice; the first releases A; R reaches B; the
//!   duplicate must not release B. That is the bug `request_id` exists for.)
//! - States move only along [`RunState::can_become`]; a finished run never changes again, and
//!   ending it expires whatever it was still waiting for.
//! - Run ids keep counting across restarts: [`RunStore::next_run_id`] is where the host's counter
//!   starts, so a run id never names two runs.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};

/// Where a run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Running,
    /// It asked the person something and is waiting for the answer.
    WaitingOnPerson,
    Done,
    Failed,
    Cancelled,
    /// It was unfinished when its connection or the host went away. Readable, not resumable.
    Orphaned,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Running => "running",
            RunState::WaitingOnPerson => "waiting-on-person",
            RunState::Done => "done",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
            RunState::Orphaned => "orphaned",
        }
    }

    pub fn parse(s: &str) -> Option<RunState> {
        [RunState::Running, RunState::WaitingOnPerson, RunState::Done, RunState::Failed, RunState::Cancelled, RunState::Orphaned]
            .into_iter()
            .find(|st| st.as_str() == s)
    }

    /// Done, failed, cancelled and orphaned are final.
    pub fn is_final(self) -> bool {
        !matches!(self, RunState::Running | RunState::WaitingOnPerson)
    }

    /// The transition table: an unfinished run may wait, resume from waiting, or end in any
    /// final state; a final state is final.
    pub fn can_become(self, to: RunState) -> bool {
        match (self, to) {
            (from, _) if from.is_final() => false,
            (RunState::Running, RunState::Running) => false,
            (RunState::WaitingOnPerson, RunState::WaitingOnPerson) => false,
            _ => true,
        }
    }
}

/// A run's record, without its events.
#[derive(Debug, Clone, PartialEq)]
pub struct RunInfo {
    pub run_id: u64,
    pub harness: String,
    pub conversation: String,
    /// The connection that started it; only that connection may act on it.
    pub owner: String,
    pub state: RunState,
    /// Unix milliseconds.
    pub started_at: i64,
    pub updated_at: i64,
    /// The sequence number of its latest event (0 before the first).
    pub last_seq: u64,
}

/// One entry in a run's log.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub seq: u64,
    pub kind: String,
    pub payload: Value,
    /// Unix milliseconds.
    pub at: i64,
}

/// A page of a run's log: the events after the cursor asked for, and where to continue.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub events: Vec<StoredEvent>,
    /// Pass this as `after_seq` for the next page.
    pub next_after: u64,
    /// Whether events remain after this page.
    pub more: bool,
}

/// Why a call on the store was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum RunError {
    NoSuchRun(u64),
    /// The run already exists (its id was minted twice).
    Exists(u64),
    /// The run has ended; nothing more is recorded against it.
    Ended { run_id: u64, state: RunState },
    IllegalTransition { run_id: u64, from: RunState, to: RunState },
    /// The run never asked this.
    NoSuchRequest { run_id: u64, request_id: String },
    /// This request was already asked (a request id names one question).
    RequestExists { run_id: u64, request_id: String },
    /// This request was already answered; the answer was not applied twice.
    AlreadyAnswered { run_id: u64, request_id: String },
    Storage(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::NoSuchRun(id) => write!(f, "run {id} does not exist"),
            RunError::Exists(id) => write!(f, "run {id} already exists"),
            RunError::Ended { run_id, state } => write!(f, "run {run_id} has ended ({})", state.as_str()),
            RunError::IllegalTransition { run_id, from, to } => {
                write!(f, "run {run_id} cannot go from {} to {}", from.as_str(), to.as_str())
            }
            RunError::NoSuchRequest { run_id, request_id } => write!(f, "run {run_id} never asked {request_id:?}"),
            RunError::RequestExists { run_id, request_id } => write!(f, "run {run_id} already asked {request_id:?}"),
            RunError::AlreadyAnswered { run_id, request_id } => {
                write!(f, "request {request_id:?} of run {run_id} was already answered")
            }
            RunError::Storage(e) => write!(f, "run store: {e}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<rusqlite::Error> for RunError {
    fn from(e: rusqlite::Error) -> Self {
        RunError::Storage(e.to_string())
    }
}

/// The most events one page returns.
pub const PAGE_MAX: usize = 500;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS runs (
    run_id       INTEGER PRIMARY KEY,
    harness      TEXT NOT NULL,
    conversation TEXT NOT NULL,
    owner        TEXT NOT NULL,
    state        TEXT NOT NULL,
    started_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
    run_id  INTEGER NOT NULL REFERENCES runs(run_id),
    seq     INTEGER NOT NULL,
    kind    TEXT NOT NULL,
    payload TEXT NOT NULL,
    at      INTEGER NOT NULL,
    PRIMARY KEY (run_id, seq)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS requests (
    run_id      INTEGER NOT NULL REFERENCES runs(run_id),
    request_id  TEXT NOT NULL,
    prompt      TEXT NOT NULL,
    state       TEXT NOT NULL,
    answer      TEXT,
    asked_at    INTEGER NOT NULL,
    answered_at INTEGER,
    PRIMARY KEY (run_id, request_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS runs_by_state ON runs(state);
";

/// The run log. One per host; safe to share between threads.
pub struct RunStore {
    db: Mutex<Connection>,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl RunStore {
    /// Open (or create) the store at `path`. The parent directory must exist.
    pub fn open(path: &Path) -> Result<RunStore, RunError> {
        let db = Connection::open(path)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(db)
    }

    /// A store that lives only as long as this value; for tests and hosts with nowhere to write.
    pub fn in_memory() -> Result<RunStore, RunError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<RunStore, RunError> {
        db.pragma_update(None, "foreign_keys", "ON")?;
        db.execute_batch(SCHEMA)?;
        Ok(RunStore { db: Mutex::new(db) })
    }

    fn with<R>(&self, f: impl FnOnce(&mut Connection) -> Result<R, RunError>) -> Result<R, RunError> {
        let mut db = self.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut db)
    }

    /// The first run id not yet used: where the host's turn counter starts.
    pub fn next_run_id(&self) -> Result<u64, RunError> {
        self.with(|db| {
            let max: Option<i64> = db.query_row("SELECT MAX(run_id) FROM runs", [], |r| r.get(0))?;
            Ok(max.map(|m| m as u64 + 1).unwrap_or(1))
        })
    }

    /// Record that `run_id` started, on `harness`, in `conversation`, for the connection `owner`.
    pub fn start(&self, run_id: u64, harness: &str, conversation: &str, owner: &str) -> Result<(), RunError> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let at = now_ms();
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO runs (run_id, harness, conversation, owner, state, started_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![run_id as i64, harness, conversation, owner, RunState::Running.as_str(), at],
            )?;
            if inserted == 0 {
                return Err(RunError::Exists(run_id));
            }
            append_in(&tx, run_id, "state", &json!({"to": RunState::Running.as_str()}), at)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Append an event to an unfinished run; returns its sequence number.
    pub fn append(&self, run_id: u64, kind: &str, payload: &Value) -> Result<u64, RunError> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let state = state_in(&tx, run_id)?;
            if state.is_final() {
                return Err(RunError::Ended { run_id, state });
            }
            let seq = append_in(&tx, run_id, kind, payload, now_ms())?;
            tx.commit()?;
            Ok(seq)
        })
    }

    /// The events of `run_id` after `after_seq`, at most `limit` (capped at [`PAGE_MAX`]).
    pub fn events(&self, run_id: u64, after_seq: u64, limit: usize) -> Result<Page, RunError> {
        let limit = limit.clamp(1, PAGE_MAX);
        self.with(|db| {
            state_in(db, run_id)?;
            let mut stmt = db.prepare(
                "SELECT seq, kind, payload, at FROM events WHERE run_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
            )?;
            let mut events: Vec<StoredEvent> = stmt
                .query_map(params![run_id as i64, after_seq as i64, limit as i64 + 1], |r| {
                    let payload: String = r.get(2)?;
                    Ok(StoredEvent {
                        seq: r.get::<_, i64>(0)? as u64,
                        kind: r.get(1)?,
                        payload: serde_json::from_str(&payload).unwrap_or(Value::String(payload)),
                        at: r.get(3)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            let more = events.len() > limit;
            events.truncate(limit);
            let next_after = events.last().map(|e| e.seq).unwrap_or(after_seq);
            Ok(Page { events, next_after, more })
        })
    }

    /// Move `run_id` to `to`, recording the change as an event. Ending a run expires any request
    /// it was still waiting on.
    pub fn transition(&self, run_id: u64, to: RunState) -> Result<u64, RunError> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let seq = transition_in(&tx, run_id, to, now_ms())?;
            tx.commit()?;
            Ok(seq)
        })
    }

    /// The run asks the person something. The run waits until every question it asked is
    /// answered. Returns the sequence number of the event that records the question.
    pub fn ask(&self, run_id: u64, request_id: &str, prompt: &Value) -> Result<u64, RunError> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let at = now_ms();
            let state = state_in(&tx, run_id)?;
            if state.is_final() {
                return Err(RunError::Ended { run_id, state });
            }
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO requests (run_id, request_id, prompt, state, asked_at) VALUES (?1, ?2, ?3, 'pending', ?4)",
                params![run_id as i64, request_id, prompt.to_string(), at],
            )?;
            if inserted == 0 {
                return Err(RunError::RequestExists { run_id, request_id: request_id.to_string() });
            }
            let seq = append_in(&tx, run_id, "request", &json!({"request_id": request_id, "prompt": prompt}), at)?;
            if state == RunState::Running {
                transition_in(&tx, run_id, RunState::WaitingOnPerson, at)?;
            }
            tx.commit()?;
            Ok(seq)
        })
    }

    /// Apply the person's answer to one request, exactly once. When it was the last question the
    /// run was waiting on, the run is running again. Returns the answer event's sequence number.
    pub fn answer(&self, run_id: u64, request_id: &str, answer: &Value) -> Result<u64, RunError> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let at = now_ms();
            let state = state_in(&tx, run_id)?;
            let request: Option<String> = tx
                .query_row(
                    "SELECT state FROM requests WHERE run_id = ?1 AND request_id = ?2",
                    params![run_id as i64, request_id],
                    |r| r.get(0),
                )
                .optional()?;
            match request.as_deref() {
                None => return Err(RunError::NoSuchRequest { run_id, request_id: request_id.to_string() }),
                Some("answered") => return Err(RunError::AlreadyAnswered { run_id, request_id: request_id.to_string() }),
                Some(_) if state.is_final() => return Err(RunError::Ended { run_id, state }),
                Some(_) => {}
            }
            tx.execute(
                "UPDATE requests SET state = 'answered', answer = ?3, answered_at = ?4
                 WHERE run_id = ?1 AND request_id = ?2 AND state = 'pending'",
                params![run_id as i64, request_id, answer.to_string(), at],
            )?;
            let seq = append_in(&tx, run_id, "answer", &json!({"request_id": request_id, "answer": answer}), at)?;
            let pending: i64 = tx.query_row(
                "SELECT COUNT(*) FROM requests WHERE run_id = ?1 AND state = 'pending'",
                params![run_id as i64],
                |r| r.get(0),
            )?;
            if pending == 0 && state == RunState::WaitingOnPerson {
                transition_in(&tx, run_id, RunState::Running, at)?;
            }
            tx.commit()?;
            Ok(seq)
        })
    }

    /// Hand an unfinished run to a new connection: a harness that lost its connection and came
    /// back still answering it (#246). From now on only `owner` may act on it; the log says when
    /// it changed hands.
    pub fn set_owner(&self, run_id: u64, owner: &str) -> Result<u64, RunError> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let state = state_in(&tx, run_id)?;
            if state.is_final() {
                return Err(RunError::Ended { run_id, state });
            }
            let at = now_ms();
            tx.execute("UPDATE runs SET owner = ?2 WHERE run_id = ?1", params![run_id as i64, owner])?;
            let seq = append_in(&tx, run_id, "owner", &json!({"owner": owner}), at)?;
            tx.commit()?;
            Ok(seq)
        })
    }

    /// The questions `run_id` is still waiting on, oldest first: `(request_id, prompt)`.
    pub fn pending_requests(&self, run_id: u64) -> Result<Vec<(String, Value)>, RunError> {
        self.with(|db| {
            state_in(db, run_id)?;
            let mut stmt = db.prepare(
                "SELECT request_id, prompt FROM requests WHERE run_id = ?1 AND state = 'pending' ORDER BY asked_at, request_id",
            )?;
            let rows = stmt
                .query_map(params![run_id as i64], |r| {
                    let prompt: String = r.get(1)?;
                    Ok((r.get(0)?, serde_json::from_str(&prompt).unwrap_or(Value::String(prompt))))
                })?
                .collect::<Result<_, _>>()?;
            Ok(rows)
        })
    }

    /// At host start: every run left unfinished by the last one becomes orphaned (readable,
    /// never resumed). Returns how many.
    pub fn orphan_unfinished(&self) -> Result<usize, RunError> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let at = now_ms();
            let ids: Vec<u64> = {
                let mut stmt = tx.prepare("SELECT run_id FROM runs WHERE state IN ('running', 'waiting-on-person')")?;
                let ids = stmt.query_map([], |r| r.get::<_, i64>(0).map(|v| v as u64))?.collect::<Result<_, _>>()?;
                ids
            };
            for &id in &ids {
                transition_in(&tx, id, RunState::Orphaned, at)?;
            }
            tx.commit()?;
            Ok(ids.len())
        })
    }

    /// One run's record.
    pub fn run(&self, run_id: u64) -> Result<Option<RunInfo>, RunError> {
        self.with(|db| {
            db.query_row(&format!("{RUN_ROW} WHERE r.run_id = ?1"), params![run_id as i64], run_info)
                .optional()
                .map_err(Into::into)
        })
    }

    /// The latest runs, newest first.
    pub fn recent(&self, limit: usize) -> Result<Vec<RunInfo>, RunError> {
        self.with(|db| {
            let mut stmt = db.prepare(&format!("{RUN_ROW} ORDER BY r.run_id DESC LIMIT ?1"))?;
            let rows = stmt.query_map(params![limit.clamp(1, PAGE_MAX) as i64], run_info)?.collect::<Result<_, _>>()?;
            Ok(rows)
        })
    }
}

const RUN_ROW: &str = "SELECT r.run_id, r.harness, r.conversation, r.owner, r.state, r.started_at, r.updated_at,
    (SELECT COALESCE(MAX(seq), 0) FROM events e WHERE e.run_id = r.run_id) FROM runs r";

fn run_info(r: &rusqlite::Row<'_>) -> rusqlite::Result<RunInfo> {
    let state: String = r.get(4)?;
    Ok(RunInfo {
        run_id: r.get::<_, i64>(0)? as u64,
        harness: r.get(1)?,
        conversation: r.get(2)?,
        owner: r.get(3)?,
        // A state this build does not know is shown as orphaned rather than failing the read.
        state: RunState::parse(&state).unwrap_or(RunState::Orphaned),
        started_at: r.get(5)?,
        updated_at: r.get(6)?,
        last_seq: r.get::<_, i64>(7)? as u64,
    })
}

fn state_in(db: &Connection, run_id: u64) -> Result<RunState, RunError> {
    let state: Option<String> = db
        .query_row("SELECT state FROM runs WHERE run_id = ?1", params![run_id as i64], |r| r.get(0))
        .optional()?;
    let state = state.ok_or(RunError::NoSuchRun(run_id))?;
    Ok(RunState::parse(&state).unwrap_or(RunState::Orphaned))
}

fn append_in(tx: &Transaction<'_>, run_id: u64, kind: &str, payload: &Value, at: i64) -> Result<u64, RunError> {
    let seq: i64 = tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE run_id = ?1",
        params![run_id as i64],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO events (run_id, seq, kind, payload, at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![run_id as i64, seq, kind, payload.to_string(), at],
    )?;
    tx.execute("UPDATE runs SET updated_at = ?2 WHERE run_id = ?1", params![run_id as i64, at])?;
    Ok(seq as u64)
}

fn transition_in(tx: &Transaction<'_>, run_id: u64, to: RunState, at: i64) -> Result<u64, RunError> {
    let from = state_in(tx, run_id)?;
    if !from.can_become(to) {
        return Err(if from.is_final() {
            RunError::Ended { run_id, state: from }
        } else {
            RunError::IllegalTransition { run_id, from, to }
        });
    }
    tx.execute("UPDATE runs SET state = ?2 WHERE run_id = ?1", params![run_id as i64, to.as_str()])?;
    if to.is_final() {
        tx.execute(
            "UPDATE requests SET state = 'expired' WHERE run_id = ?1 AND state = 'pending'",
            params![run_id as i64],
        )?;
    }
    append_in(tx, run_id, "state", &json!({"from": from.as_str(), "to": to.as_str()}), at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_runs(ids: &[u64]) -> RunStore {
        let store = RunStore::in_memory().unwrap();
        for &id in ids {
            store.start(id, "scripted", &format!("c{id}"), "conn-1").unwrap();
        }
        store
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-runs-{}-{name}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("runs.db")
    }

    #[test]
    fn events_are_sequenced_and_page_back_in_order_however_long() {
        let store = store_with_runs(&[1]);
        let long = "x".repeat(2_000);
        for i in 0..7 {
            store.append(1, "text", &json!({"delta": format!("{i}{long}")})).unwrap();
        }
        // seq 1 is the start; the seven texts are 2..=8.
        let first = store.events(1, 0, 3).unwrap();
        assert_eq!(first.events.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(first.more);
        let mut all = first.events.clone();
        let mut cursor = first.next_after;
        loop {
            let page = store.events(1, cursor, 3).unwrap();
            all.extend(page.events.clone());
            cursor = page.next_after;
            if !page.more {
                break;
            }
        }
        assert_eq!(all.iter().map(|e| e.seq).collect::<Vec<_>>(), (1..=8).collect::<Vec<_>>());
        assert_eq!(all[7].payload["delta"].as_str().unwrap().len(), 2_001, "a long message comes back whole");
        assert_eq!(store.events(1, 8, 3).unwrap(), Page { events: vec![], next_after: 8, more: false });
    }

    #[test]
    fn answering_the_first_request_twice_lands_exactly_once_and_the_other_run_still_waits() {
        let store = store_with_runs(&[1, 2]);
        store.ask(1, "approve-a", &json!({"text": "Delete 3 files?"})).unwrap();
        store.ask(2, "approve-a", &json!({"text": "Send the mail?"})).unwrap();

        store.answer(1, "approve-a", &json!("yes")).unwrap();
        assert_eq!(
            store.answer(1, "approve-a", &json!("yes")),
            Err(RunError::AlreadyAnswered { run_id: 1, request_id: "approve-a".into() })
        );
        let answers = store.events(1, 0, PAGE_MAX).unwrap().events.into_iter().filter(|e| e.kind == "answer").count();
        assert_eq!(answers, 1);
        assert_eq!(store.run(1).unwrap().unwrap().state, RunState::Running);
        assert_eq!(store.run(2).unwrap().unwrap().state, RunState::WaitingOnPerson);
        assert_eq!(store.pending_requests(2).unwrap().len(), 1);
    }

    #[test]
    fn a_duplicate_answer_cannot_release_the_next_request() {
        // R asks A, the person answers twice, R reaches B: the duplicate must not release B.
        let store = store_with_runs(&[1]);
        store.ask(1, "a", &json!("first?")).unwrap();
        store.answer(1, "a", &json!("yes")).unwrap();
        store.ask(1, "b", &json!("second?")).unwrap();
        assert!(matches!(store.answer(1, "a", &json!("yes")), Err(RunError::AlreadyAnswered { .. })));
        assert_eq!(store.run(1).unwrap().unwrap().state, RunState::WaitingOnPerson);
        assert_eq!(store.pending_requests(1).unwrap(), vec![("b".to_string(), json!("second?"))]);
    }

    #[test]
    fn a_run_waits_until_every_question_is_answered() {
        let store = store_with_runs(&[1]);
        store.ask(1, "a", &json!("?")).unwrap();
        store.ask(1, "b", &json!("?")).unwrap();
        store.answer(1, "b", &json!(1)).unwrap();
        assert_eq!(store.run(1).unwrap().unwrap().state, RunState::WaitingOnPerson);
        store.answer(1, "a", &json!(2)).unwrap();
        assert_eq!(store.run(1).unwrap().unwrap().state, RunState::Running);
    }

    #[test]
    fn stale_unknown_and_repeated_questions_are_refused_with_their_reason() {
        let store = store_with_runs(&[1]);
        store.ask(1, "a", &json!("?")).unwrap();
        assert_eq!(store.ask(1, "a", &json!("?")), Err(RunError::RequestExists { run_id: 1, request_id: "a".into() }));
        assert_eq!(store.answer(1, "zz", &json!(1)), Err(RunError::NoSuchRequest { run_id: 1, request_id: "zz".into() }));
        assert_eq!(store.answer(9, "a", &json!(1)), Err(RunError::NoSuchRun(9)));
        store.transition(1, RunState::Cancelled).unwrap();
        assert_eq!(store.answer(1, "a", &json!(1)), Err(RunError::Ended { run_id: 1, state: RunState::Cancelled }));
        assert!(store.pending_requests(1).unwrap().is_empty(), "ending a run expires what it waited on");
    }

    #[test]
    fn a_finished_run_never_changes_again() {
        let store = store_with_runs(&[1]);
        store.transition(1, RunState::Done).unwrap();
        for to in [RunState::Running, RunState::Failed, RunState::Orphaned] {
            assert_eq!(store.transition(1, to), Err(RunError::Ended { run_id: 1, state: RunState::Done }));
        }
        assert_eq!(store.append(1, "text", &json!("late")), Err(RunError::Ended { run_id: 1, state: RunState::Done }));
        assert_eq!(store.ask(1, "a", &json!("?")), Err(RunError::Ended { run_id: 1, state: RunState::Done }));
        assert_eq!(store.transition(2, RunState::Done), Err(RunError::NoSuchRun(2)));
    }

    #[test]
    fn running_does_not_become_running() {
        let store = store_with_runs(&[1]);
        assert_eq!(
            store.transition(1, RunState::Running),
            Err(RunError::IllegalTransition { run_id: 1, from: RunState::Running, to: RunState::Running })
        );
    }

    #[test]
    fn a_restart_orphans_unfinished_runs_keeps_their_log_and_ids_keep_counting() {
        let path = scratch("restart");
        {
            let store = RunStore::open(&path).unwrap();
            assert_eq!(store.next_run_id().unwrap(), 1);
            for id in 1..=3 {
                store.start(id, "scripted", "c", "conn-1").unwrap();
            }
            store.append(1, "text", &json!({"delta": "half an answer"})).unwrap();
            store.ask(2, "approve", &json!("go?")).unwrap();
            store.transition(3, RunState::Done).unwrap();
        }
        let store = RunStore::open(&path).unwrap();
        assert_eq!(store.orphan_unfinished().unwrap(), 2);
        assert_eq!(store.run(1).unwrap().unwrap().state, RunState::Orphaned);
        assert_eq!(store.run(2).unwrap().unwrap().state, RunState::Orphaned);
        assert_eq!(store.run(3).unwrap().unwrap().state, RunState::Done);
        let log = store.events(1, 0, PAGE_MAX).unwrap().events;
        assert_eq!(log[1].payload["delta"], "half an answer", "an orphaned run stays readable");
        assert!(matches!(store.answer(2, "approve", &json!("yes")), Err(RunError::Ended { state: RunState::Orphaned, .. })));
        assert_eq!(store.next_run_id().unwrap(), 4, "a run id never names two runs");
        assert_eq!(store.start(3, "scripted", "c", "conn-2"), Err(RunError::Exists(3)));
        assert_eq!(store.orphan_unfinished().unwrap(), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_unfinished_run_can_change_hands_and_a_finished_one_cannot() {
        let store = store_with_runs(&[1]);
        store.set_owner(1, "conn-2").unwrap();
        assert_eq!(store.run(1).unwrap().unwrap().owner, "conn-2");
        assert_eq!(store.events(1, 0, PAGE_MAX).unwrap().events.last().unwrap().payload["owner"], "conn-2");
        store.transition(1, RunState::Done).unwrap();
        assert_eq!(store.set_owner(1, "conn-3"), Err(RunError::Ended { run_id: 1, state: RunState::Done }));
    }

    #[test]
    fn recent_runs_come_newest_first_with_their_last_sequence() {
        let store = store_with_runs(&[1, 2]);
        store.append(2, "text", &json!("hi")).unwrap();
        let recent = store.recent(10).unwrap();
        assert_eq!(recent.iter().map(|r| r.run_id).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(recent[0].last_seq, 2);
        assert_eq!(recent[0].owner, "conn-1");
    }
}
