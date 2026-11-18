//! Shared fixtures: the source tables as the companion and the memory store create them (only
//! the columns that matter here are all that is relied on; the rest are present so the shapes
//! match), and small helpers to put rows in them.

use rusqlite::{params, Connection};

pub const T0: f64 = 1_800_000_000.0;
pub const HOUR: f64 = 3600.0;
pub const DAY: f64 = 86_400.0;

pub fn mem() -> Connection {
    Connection::open_in_memory().expect("open")
}

pub fn make_promises(c: &Connection) {
    c.execute_batch(
        "CREATE TABLE wm_commitments (id INTEGER PRIMARY KEY AUTOINCREMENT, promisor TEXT NOT NULL,
         promisee TEXT NOT NULL, action TEXT NOT NULL, deadline REAL NOT NULL DEFAULT 0,
         status TEXT NOT NULL DEFAULT 'pending', confidence REAL NOT NULL DEFAULT 0.5,
         source_type TEXT NOT NULL DEFAULT 'conversation', source_data TEXT NOT NULL DEFAULT '{}',
         evidence_text TEXT NOT NULL DEFAULT '', related_entities TEXT NOT NULL DEFAULT '[]',
         completion_evidence TEXT, created_at REAL NOT NULL, updated_at REAL NOT NULL);",
    )
    .expect("promises");
}

pub fn add_promise(c: &Connection, action: &str, deadline: f64) -> i64 {
    c.execute(
        "INSERT INTO wm_commitments (promisor, promisee, action, deadline, confidence, created_at, updated_at)
         VALUES ('me', 'Sam', ?1, ?2, 0.8, ?3, ?3)",
        params![action, deadline, T0],
    )
    .expect("insert promise");
    c.last_insert_rowid()
}

pub fn add_promise_from(c: &Connection, action: &str, deadline: f64, source_type: &str) -> i64 {
    let id = add_promise(c, action, deadline);
    set(c, "UPDATE wm_commitments SET source_type = ?1 WHERE id = ?2", params![source_type, id]);
    id
}

pub fn make_threads(c: &Connection) {
    c.execute_batch(
        "CREATE TABLE life_threads (id INTEGER PRIMARY KEY AUTOINCREMENT, thread_type TEXT NOT NULL,
         entity_id TEXT NOT NULL, label TEXT NOT NULL, deadline_ts REAL DEFAULT 0,
         status TEXT NOT NULL DEFAULT 'open', importance REAL DEFAULT 0.5, days_open INTEGER DEFAULT 0,
         last_action_ts REAL DEFAULT 0, snooze_until REAL DEFAULT 0, source TEXT DEFAULT '',
         context_json TEXT DEFAULT '{}', created_at REAL NOT NULL, updated_at REAL NOT NULL,
         UNIQUE(thread_type, entity_id));",
    )
    .expect("threads");
}

pub fn add_thread(c: &Connection, label: &str, deadline: f64, snooze_until: f64) -> i64 {
    c.execute(
        "INSERT INTO life_threads (thread_type, entity_id, label, deadline_ts, snooze_until, created_at, updated_at)
         VALUES ('email', ?1, ?1, ?2, ?3, ?4, ?4)",
        params![label, deadline, snooze_until, T0],
    )
    .expect("insert thread");
    c.last_insert_rowid()
}

pub fn make_memory_and_conflicts(c: &Connection) {
    c.execute_batch(
        "CREATE TABLE memories (rid TEXT PRIMARY KEY, text TEXT NOT NULL,
         namespace TEXT NOT NULL DEFAULT 'default');
         CREATE TABLE conflicts (conflict_id TEXT PRIMARY KEY, conflict_type TEXT NOT NULL,
         priority TEXT NOT NULL DEFAULT 'medium', status TEXT NOT NULL DEFAULT 'open',
         memory_a TEXT NOT NULL, memory_b TEXT NOT NULL);",
    )
    .expect("conflicts");
}

pub fn add_memory(c: &Connection, rid: &str, text: &str, ns: &str) {
    c.execute("INSERT INTO memories (rid, text, namespace) VALUES (?1, ?2, ?3)", params![rid, text, ns])
        .expect("insert memory");
}

pub fn add_conflict(c: &Connection, id: &str, a: &str, b: &str) {
    c.execute(
        "INSERT INTO conflicts (conflict_id, conflict_type, memory_a, memory_b) VALUES (?1, 'preference', ?2, ?3)",
        params![id, a, b],
    )
    .expect("insert conflict");
}

pub fn make_schedules(c: &Connection) {
    c.execute_batch(
        "CREATE TABLE scheduled_tasks (task_id TEXT PRIMARY KEY, label TEXT NOT NULL,
         description TEXT DEFAULT '', schedule_type TEXT NOT NULL, interval_secs INTEGER,
         cron_expr TEXT, next_invoke REAL, last_invoked REAL, invocation_count INTEGER DEFAULT 0,
         max_invocations INTEGER, urgency REAL DEFAULT 0.6, status TEXT DEFAULT 'active',
         action TEXT, metadata TEXT DEFAULT '{}', created_at REAL NOT NULL, updated_at REAL NOT NULL);",
    )
    .expect("schedules");
}

pub fn add_schedule(c: &Connection, id: &str, label: &str, next: f64, action: Option<&str>) {
    c.execute(
        "INSERT INTO scheduled_tasks (task_id, label, schedule_type, next_invoke, action, created_at, updated_at)
         VALUES (?1, ?2, 'once', ?3, ?4, ?5, ?5)",
        params![id, label, next, action, T0],
    )
    .expect("insert schedule");
}

pub fn make_tasks(c: &Connection) {
    c.execute_batch(
        "CREATE TABLE task_queue (task_id TEXT PRIMARY KEY, title TEXT NOT NULL,
         description TEXT NOT NULL DEFAULT '', priority INTEGER NOT NULL DEFAULT 2,
         status TEXT NOT NULL DEFAULT 'pending', progress TEXT NOT NULL DEFAULT '',
         result TEXT NOT NULL DEFAULT '', steps_completed INTEGER NOT NULL DEFAULT 0,
         steps_total INTEGER NOT NULL DEFAULT 0, created_at REAL NOT NULL, updated_at REAL NOT NULL,
         completed_at REAL, created_by TEXT NOT NULL DEFAULT 'user');",
    )
    .expect("tasks");
}

pub fn add_task(c: &Connection, id: &str, title: &str, priority: i64) {
    c.execute(
        "INSERT INTO task_queue (task_id, title, priority, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        params![id, title, priority, T0],
    )
    .expect("insert task");
}

pub fn add_task_by(c: &Connection, id: &str, title: &str, created_by: &str) {
    add_task(c, id, title, 2);
    set(c, "UPDATE task_queue SET created_by = ?1 WHERE task_id = ?2", params![created_by, id]);
}

pub fn set(c: &Connection, sql: &str, p: impl rusqlite::Params) {
    c.execute(sql, p).expect("update");
}

/// Every table except `agenda`, as text, to prove nothing else was written.
pub fn dump_all_but_agenda(c: &Connection) -> String {
    let names: Vec<String> = {
        let mut s = c
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != 'agenda' ORDER BY name")
            .expect("names");
        let rows = s.query_map([], |r| r.get::<_, String>(0)).expect("q");
        rows.flatten().collect()
    };
    let mut out = String::new();
    for n in names {
        out.push_str(&format!("## {n}\n"));
        let mut s = c.prepare(&format!("SELECT * FROM \"{n}\" ORDER BY 1")).expect("sel");
        let cols = s.column_count();
        let rows = s
            .query_map([], |r| {
                let mut line = String::new();
                for i in 0..cols {
                    let v: rusqlite::types::Value = r.get(i)?;
                    line.push_str(&format!("{v:?}|"));
                }
                Ok(line)
            })
            .expect("rows");
        for l in rows.flatten() {
            out.push_str(&l);
            out.push('\n');
        }
    }
    out
}
