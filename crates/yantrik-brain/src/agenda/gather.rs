//! `gather`: mirrors five other stores into the agenda.
//!
//! It reads them and writes only the `agenda` table. Each mirror names the table and columns it
//! reads in its own `Mirror` constant (the `sql` field), so a change to one of those schemas has one place to land.
//! A source is read with a filter: its active rows, plus the rows of entries the agenda holds
//! open (so a finish is noticed). The query returns only those rows, but the database still scans a source
//! table that has no usable index on that filter, so a very long history costs a little per row
//! (about 0.25 µs on a 500,000-row table); `SourceReport::read` counts rows returned, not scanned.
//! Each is tolerant of its table not existing (a machine that never made one) and of rows it
//! cannot read (skipped and counted, never a panic). One SQL read per source, one write
//! transaction for the whole pass.
//!
//! Private things are never gathered: conflicts are read only when both memories are in the
//! shared `default` namespace, so nothing from a Mind's own namespace reaches the agenda. The
//! caller must not call `gather` at all while Private mode is on.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, Row};

use super::salience::{
    task_urgency, UrgencyFn, CONFLICT_CONFIDENCE, CONFLICT_URGENCY, MODEL_CONFIDENCE, PERSON_CONFIDENCE,
    SCHEDULE_CONFIDENCE, SCHEDULE_DEFAULT_URGENCY, THREAD_CONFIDENCE, THREAD_URGENCY_SCALE,
};
use super::store::{self, NewEntry};
use super::{Kind, NextStep, State};

/// What one source did in one pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceReport {
    pub source: &'static str,
    /// False when the source table does not exist (nothing to do, not an error).
    pub present: bool,
    /// True when the read itself failed (for example a column the mirror expects is missing).
    pub failed: bool,
    pub upserted: usize,
    pub closed: usize,
    /// Rows that could not be read or understood.
    pub skipped: usize,
    /// Source rows returned by the read this pass (active ones, plus those of entries held open).
    pub read: usize,
    /// True when an unreadable row stopped this pass from dropping entries whose source row
    /// vanished: the same row will keep doing it until it is fixed.
    pub drop_check_skipped: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GatherReport {
    pub sources: Vec<SourceReport>,
}

impl GatherReport {
    pub fn skipped(&self) -> usize {
        self.sources.iter().map(|s| s.skipped).sum()
    }
}

/// What a mirror says about one source row.
enum Mirrored {
    /// Still worth doing: keep the entry current.
    Active(NewEntry),
    /// The source says it is finished: close the entry if there is one.
    Closed { source_id: String, state: State },
}

struct Mirror {
    source: &'static str,
    /// The tables that must exist for the read to make sense.
    tables: &'static [&'static str],
    sql: &'static str,
    map: fn(&Row, f64) -> rusqlite::Result<Mirrored>,
}

// Each SQL reads the rows not yet finished, plus the rows of entries still open in the agenda
// (`... IN (SELECT source_id FROM agenda WHERE source = ... AND is_open = 1)`), so a finish is
// seen. The finished statuses listed here are the ones the `map_*` functions treat as closed.

// wm_commitments: id, action, deadline (0 = none), status, confidence, source_type, created_at
const PROMISES: Mirror = Mirror {
    source: "wm_commitments",
    tables: &["wm_commitments"],
    sql: "SELECT id, action, deadline, status, confidence, source_type, created_at FROM wm_commitments \
          WHERE status NOT IN ('completed','cancelled','superseded') \
          OR id IN (SELECT CAST(source_id AS INTEGER) FROM agenda \
                    WHERE source = 'wm_commitments' AND is_open = 1)",
    map: map_promise,
};
// life_threads: id, label (never read), deadline_ts (0 = none), status, importance, snooze_until
const THREADS: Mirror = Mirror {
    source: "life_threads",
    tables: &["life_threads"],
    sql: "SELECT id, deadline_ts, status, importance, snooze_until FROM life_threads \
          WHERE status NOT IN ('resolved','archived') \
          OR id IN (SELECT CAST(source_id AS INTEGER) FROM agenda \
                    WHERE source = 'life_threads' AND is_open = 1)",
    map: map_thread,
};
// conflicts joined to memories only to prove both are in the shared namespace; no memory text
// is selected.
const CONFLICTS: Mirror = Mirror {
    source: "conflicts",
    tables: &["conflicts", "memories"],
    sql: "SELECT c.conflict_id, c.priority, c.status FROM conflicts c \
          JOIN memories a ON a.rid = c.memory_a JOIN memories b ON b.rid = c.memory_b \
          WHERE a.namespace = 'default' AND b.namespace = 'default' \
          AND (c.status NOT IN ('resolved','dismissed') \
               OR c.conflict_id IN (SELECT source_id FROM agenda \
                                    WHERE source = 'conflicts' AND is_open = 1))",
    map: map_conflict,
};
// scheduled_tasks: task_id, next_invoke, status, action, urgency (the label is never read: the
// table does not say who wrote it)
const SCHEDULES: Mirror = Mirror {
    source: "scheduled_tasks",
    tables: &["scheduled_tasks"],
    sql: "SELECT task_id, next_invoke, status, action, urgency FROM scheduled_tasks \
          WHERE COALESCE(status, 'active') NOT IN ('completed','cancelled','paused') \
          OR task_id IN (SELECT source_id FROM agenda \
                         WHERE source = 'scheduled_tasks' AND is_open = 1)",
    map: map_schedule,
};
// task_queue: task_id, title, priority (1-4), status, created_by
const TASKS: Mirror = Mirror {
    source: "task_queue",
    tables: &["task_queue"],
    sql: "SELECT task_id, title, priority, status, created_by FROM task_queue \
          WHERE status NOT IN ('completed','failed','cancelled') \
          OR task_id IN (SELECT source_id FROM agenda WHERE source = 'task_queue' AND is_open = 1)",
    map: map_task,
};

const MIRRORS: [&Mirror; 5] = [&PROMISES, &THREADS, &CONFLICTS, &SCHEDULES, &TASKS];

fn map_promise(r: &Row, _now: f64) -> rusqlite::Result<Mirrored> {
    let id: i64 = r.get(0)?;
    let action: String = r.get(1)?;
    let deadline: f64 = r.get(2)?;
    let status: String = r.get(3)?;
    let confidence: f64 = r.get(4)?;
    let source_type: String = r.get(5)?;
    let created_at: f64 = r.get(6)?;
    let source_id = id.to_string();
    match status.as_str() {
        "completed" => return Ok(Mirrored::Closed { source_id, state: State::Done }),
        "cancelled" | "superseded" => return Ok(Mirrored::Closed { source_id, state: State::Dropped }),
        _ => {}
    }
    let has_deadline = deadline > 0.0;
    Ok(Mirrored::Active(NewEntry {
        source: PROMISES.source,
        source_id,
        kind: Kind::Promise,
        // Words the person did not say (a promise taken from someone's email) are not copied.
        title_public: matches!(source_type.as_str(), "conversation" | "manual"),
        title: if matches!(source_type.as_str(), "conversation" | "manual") {
            action
        } else {
            "Promise".to_string()
        },
        state: State::WaitingPerson,
        next_step: NextStep::Person,
        urgency: if has_deadline {
            UrgencyFn::dated(deadline)
        } else {
            UrgencyFn::undated_promise()
        },
        due_at: has_deadline.then_some(deadline),
        confidence,
        hold_until: 0.0,
        created_at: (created_at > 0.0).then_some(created_at),
    }))
}

fn map_thread(r: &Row, now: f64) -> rusqlite::Result<Mirrored> {
    let id: i64 = r.get(0)?;
    // Three of these are nullable in the source: a NULL is the default, not an unreadable row.
    let deadline: f64 = r.get::<_, Option<f64>>(1)?.unwrap_or(0.0);
    let status: String = r.get(2)?;
    let importance: f64 = r.get::<_, Option<f64>>(3)?.unwrap_or(0.5);
    let snooze_until: f64 = r.get::<_, Option<f64>>(4)?.unwrap_or(0.0);
    let source_id = id.to_string();
    match status.as_str() {
        "resolved" => return Ok(Mirrored::Closed { source_id, state: State::Done }),
        "archived" => return Ok(Mirrored::Closed { source_id, state: State::Dropped }),
        _ => {}
    }
    let held = status == "snoozed" || snooze_until > now;
    let has_deadline = deadline > 0.0;
    Ok(Mirrored::Active(NewEntry {
        source: THREADS.source,
        source_id,
        kind: Kind::Thread,
        // A thread's label can come from someone's email, so it is never copied.
        title: "Open thread".to_string(),
        title_public: false,
        state: if held { State::Snoozed } else { State::Open },
        next_step: NextStep::Person,
        urgency: if has_deadline {
            UrgencyFn::dated(deadline)
        } else {
            UrgencyFn::Constant { value: importance.clamp(0.0, 1.0) * THREAD_URGENCY_SCALE }
        },
        due_at: has_deadline.then_some(deadline),
        confidence: THREAD_CONFIDENCE,
        hold_until: if held { snooze_until } else { 0.0 },
        created_at: None,
    }))
}

fn map_conflict(r: &Row, _now: f64) -> rusqlite::Result<Mirrored> {
    let id: String = r.get(0)?;
    let priority: String = r.get(1)?;
    let status: String = r.get(2)?;
    match status.as_str() {
        "resolved" => return Ok(Mirrored::Closed { source_id: id, state: State::Done }),
        "dismissed" => return Ok(Mirrored::Closed { source_id: id, state: State::Dropped }),
        _ => {}
    }
    let value = match priority.as_str() {
        "low" => CONFLICT_URGENCY[0],
        "high" => CONFLICT_URGENCY[2],
        "critical" => CONFLICT_URGENCY[3],
        _ => CONFLICT_URGENCY[1],
    };
    Ok(Mirrored::Active(NewEntry {
        source: CONFLICTS.source,
        source_id: id,
        kind: Kind::Conflict,
        title: "Two memories disagree".to_string(),
        title_public: false,
        // Until a later slice can settle some by rule, a conflict needs the person.
        state: State::WaitingPerson,
        next_step: NextStep::Person,
        urgency: UrgencyFn::Constant { value },
        due_at: None,
        confidence: CONFLICT_CONFIDENCE,
        hold_until: 0.0,
        created_at: None,
    }))
}

fn map_schedule(r: &Row, _now: f64) -> rusqlite::Result<Mirrored> {
    let id: String = r.get(0)?;
    let next: Option<f64> = r.get(1)?;
    let status: Option<String> = r.get(2)?;
    let action: Option<String> = r.get(3)?;
    let urgency: Option<f64> = r.get(4)?;
    match status.as_deref().unwrap_or("active") {
        "completed" => return Ok(Mirrored::Closed { source_id: id, state: State::Done }),
        // A paused schedule is not something to do until it is resumed.
        "cancelled" | "paused" => return Ok(Mirrored::Closed { source_id: id, state: State::Dropped }),
        _ => {}
    }
    let needs_model = action.as_deref().is_some_and(|a| !a.trim().is_empty());
    Ok(Mirrored::Active(NewEntry {
        source: SCHEDULES.source,
        source_id: id,
        kind: Kind::Schedule,
        // Anyone may have written a schedule's label (the model, from an email), and the table
        // does not say who, so it is not copied.
        title: "Scheduled task".to_string(),
        title_public: false,
        state: State::Open,
        next_step: if needs_model { NextStep::Model } else { NextStep::Code },
        urgency: match next {
            Some(t) => UrgencyFn::dated(t),
            None => UrgencyFn::Constant { value: urgency.unwrap_or(SCHEDULE_DEFAULT_URGENCY) },
        },
        due_at: next,
        // A schedule has no "made by" column, so its author is unknown (see the label above).
        confidence: SCHEDULE_CONFIDENCE,
        hold_until: 0.0,
        created_at: None,
    }))
}

fn map_task(r: &Row, _now: f64) -> rusqlite::Result<Mirrored> {
    let id: String = r.get(0)?;
    let title: String = r.get(1)?;
    let priority: i64 = r.get(2)?;
    let status: String = r.get(3)?;
    let created_by: String = r.get(4)?;
    match status.as_str() {
        "completed" => return Ok(Mirrored::Closed { source_id: id, state: State::Done }),
        "failed" | "cancelled" => return Ok(Mirrored::Closed { source_id: id, state: State::Dropped }),
        _ => {}
    }
    Ok(Mirrored::Active(NewEntry {
        source: TASKS.source,
        source_id: id,
        kind: Kind::Task,
        // Only a task the person asked for keeps its title; the companion's own may carry
        // someone else's words.
        title_public: created_by == "user",
        title: if created_by == "user" { title } else { "Queued task".to_string() },
        state: State::WaitingModel,
        next_step: NextStep::Model,
        urgency: UrgencyFn::Constant { value: task_urgency(priority) },
        due_at: None,
        // What the person queued is certain; what a model queued is less so.
        confidence: if created_by == "user" { PERSON_CONFIDENCE } else { MODEL_CONFIDENCE },
        hold_until: 0.0,
        created_at: None,
    }))
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |_| Ok(()))
        .is_ok()
}

/// What one source read produced.
struct Read {
    rows: Vec<Mirrored>,
    skipped: usize,
}

fn read_source(conn: &Connection, m: &Mirror, now: f64) -> rusqlite::Result<Read> {
    let mut stmt = conn.prepare(m.sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Read { rows: Vec::new(), skipped: 0 };
    loop {
        match rows.next() {
            Ok(Some(row)) => match (m.map)(row, now) {
                Ok(x) => out.rows.push(x),
                Err(_) => out.skipped += 1,
            },
            Ok(None) => break,
            // A row the driver could not even step to: count it and stop, rather than loop.
            Err(_) => {
                out.skipped += 1;
                break;
            }
        }
    }
    Ok(out)
}

/// Mirrors the five sources into `agenda`. Idempotent: a second pass changes nothing.
pub fn gather(conn: &Connection, now: f64) -> rusqlite::Result<GatherReport> {
    store::migrate(conn)?;

    // Active entries by source, so a finished row can close its entry and a vanished row can be
    // noticed. One read of our own table.
    let mut active: HashMap<String, HashMap<String, i64>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT source, source_id, id FROM agenda WHERE is_open = 1",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
        for (s, sid, id) in rows.flatten() {
            active.entry(s).or_default().insert(sid, id);
        }
    }

    let mut reports = Vec::new();
    let mut reads: Vec<(&Mirror, Option<Read>)> = Vec::new();
    for m in MIRRORS {
        let mut rep = SourceReport { source: m.source, ..Default::default() };
        rep.present = m.tables.iter().all(|t| table_exists(conn, t));
        let read = if rep.present {
            match read_source(conn, m, now) {
                Ok(r) => {
                    rep.skipped = r.skipped;
                    rep.read = r.rows.len() + r.skipped;
                    Some(r)
                }
                Err(_) => {
                    rep.failed = true;
                    None
                }
            }
        } else {
            None
        };
        reports.push(rep);
        reads.push((m, read));
    }

    let tx = conn.unchecked_transaction()?;
    for ((m, read), rep) in reads.into_iter().zip(reports.iter_mut()) {
        let Some(read) = read else { continue };
        let mut seen: HashSet<String> = HashSet::new();
        let mine = active.get(m.source);
        for item in read.rows {
            match item {
                Mirrored::Active(e) => {
                    seen.insert(e.source_id.clone());
                    if store::upsert(&tx, &e, now).is_ok() {
                        rep.upserted += 1;
                    } else {
                        rep.skipped += 1;
                    }
                }
                Mirrored::Closed { source_id, state } => {
                    seen.insert(source_id.clone());
                    if let Some(id) = mine.and_then(|a| a.get(&source_id)) {
                        if store::move_to(&tx, *id, state, store::By::Source, now)? {
                            rep.closed += 1;
                        }
                    }
                }
            }
        }
        // A row that vanished from its source (deleted, or no longer visible, which is what
        // happens to a conflict whose memory moved to a private namespace) is dropped. Not done
        // when any row was unreadable: its id is unknown, so we cannot tell it from a vanished one.
        rep.drop_check_skipped = rep.skipped > 0 && mine.is_some_and(|a| a.keys().any(|k| !seen.contains(k)));
        if rep.skipped == 0 {
            if let Some(a) = mine {
                for (sid, id) in a {
                    if !seen.contains(sid) && store::move_to(&tx, *id, State::Dropped, store::By::Source, now)? {
                        rep.closed += 1;
                    }
                }
            }
        }
    }
    tx.commit()?;
    Ok(GatherReport { sources: reports })
}
