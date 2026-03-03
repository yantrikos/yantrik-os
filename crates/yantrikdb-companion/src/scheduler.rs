//! Native scheduler — persistent scheduled tasks that fire as urges.
//!
//! Supports three schedule types:
//! - `once` — one-shot (replaces old memory-based reminders)
//! - `interval` — every N seconds
//! - `cron` — 5-field cron expression (minute hour day month weekday)
//!
//! Evaluated during think cycles: due tasks are injected as triggers,
//! converted to urges by SchedulerInstinct, and delivered via ProactiveEngine.

use rusqlite::{params, Connection};

/// A scheduled task row from SQLite.
#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub task_id: String,
    pub label: String,
    pub description: String,
    pub schedule_type: String,
    pub interval_secs: Option<i64>,
    pub cron_expr: Option<String>,
    pub next_invoke: Option<f64>,
    pub last_invoked: Option<f64>,
    pub invocation_count: i64,
    pub max_invocations: Option<i64>,
    pub urgency: f64,
    pub status: String,
    pub action: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: f64,
    pub updated_at: f64,
}

/// Stateless scheduler — all methods take a `&Connection`.
pub struct Scheduler;

impl Scheduler {
    /// Create the scheduled_tasks table if it doesn't exist.
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS scheduled_tasks (
                task_id           TEXT PRIMARY KEY,
                label             TEXT NOT NULL,
                description       TEXT DEFAULT '',
                schedule_type     TEXT NOT NULL,
                interval_secs     INTEGER,
                cron_expr         TEXT,
                next_invoke       REAL,
                last_invoked      REAL,
                invocation_count  INTEGER DEFAULT 0,
                max_invocations   INTEGER,
                urgency           REAL DEFAULT 0.6,
                status            TEXT DEFAULT 'active',
                action            TEXT,
                metadata          TEXT DEFAULT '{}',
                created_at        REAL NOT NULL,
                updated_at        REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sched_next
                ON scheduled_tasks(status, next_invoke);",
        )
        .expect("failed to create scheduled_tasks table");
    }

    /// Create a new scheduled task. Returns the task_id.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        conn: &Connection,
        label: &str,
        description: &str,
        schedule_type: &str,
        interval_secs: Option<i64>,
        cron_expr: Option<&str>,
        next_invoke: f64,
        max_invocations: Option<i64>,
        urgency: f64,
        action: Option<&str>,
        metadata: &serde_json::Value,
    ) -> String {
        let task_id = uuid7::uuid7().to_string();
        let now = now_ts();
        let meta_json = serde_json::to_string(metadata).unwrap_or_default();

        conn.execute(
            "INSERT INTO scheduled_tasks
             (task_id, label, description, schedule_type, interval_secs, cron_expr,
              next_invoke, max_invocations, urgency, action, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                task_id,
                label,
                description,
                schedule_type,
                interval_secs,
                cron_expr,
                next_invoke,
                max_invocations,
                urgency,
                action,
                meta_json,
                now,
                now,
            ],
        )
        .expect("failed to insert scheduled task");

        tracing::info!(
            task_id = %task_id,
            label = %label,
            schedule_type = %schedule_type,
            "Scheduled task created"
        );

        task_id
    }

    /// Get all tasks that are due (next_invoke <= now AND status = 'active').
    pub fn get_due(conn: &Connection) -> Vec<ScheduledTask> {
        let now = now_ts();
        let mut stmt = conn
            .prepare(
                "SELECT task_id, label, description, schedule_type, interval_secs,
                 cron_expr, next_invoke, last_invoked, invocation_count, max_invocations,
                 urgency, status, action, metadata, created_at, updated_at
                 FROM scheduled_tasks
                 WHERE status = 'active' AND next_invoke IS NOT NULL AND next_invoke <= ?1",
            )
            .expect("prepare get_due");

        stmt.query_map(params![now], row_to_task)
            .expect("query get_due")
            .filter_map(|r| r.ok())
            .collect()
    }

    /// Advance a task after it fires: update next_invoke, increment count, complete if done.
    pub fn advance(conn: &Connection, task_id: &str) {
        let task = match Self::get(conn, task_id) {
            Some(t) => t,
            None => return,
        };

        let now = now_ts();
        let new_count = task.invocation_count + 1;

        // Check if max invocations reached
        if let Some(max) = task.max_invocations {
            if new_count >= max {
                conn.execute(
                    "UPDATE scheduled_tasks SET status = 'completed',
                     last_invoked = ?1, invocation_count = ?2, updated_at = ?1
                     WHERE task_id = ?3",
                    params![now, new_count, task_id],
                )
                .ok();
                tracing::info!(task_id, label = %task.label, "Scheduled task completed (max invocations)");
                return;
            }
        }

        // Compute next invoke time
        let next = match task.schedule_type.as_str() {
            "once" => {
                // One-shot: complete after firing
                conn.execute(
                    "UPDATE scheduled_tasks SET status = 'completed',
                     last_invoked = ?1, invocation_count = ?2, updated_at = ?1
                     WHERE task_id = ?3",
                    params![now, new_count, task_id],
                )
                .ok();
                tracing::info!(task_id, label = %task.label, "One-shot task completed");
                return;
            }
            "interval" => {
                // Next = now + interval_secs
                let interval = task.interval_secs.unwrap_or(3600) as f64;
                Some(now + interval)
            }
            "cron" => {
                // Compute next cron time
                if let Some(ref expr) = task.cron_expr {
                    crate::cron_mini::next_cron(expr, now)
                } else {
                    None
                }
            }
            _ => None,
        };

        conn.execute(
            "UPDATE scheduled_tasks SET next_invoke = ?1, last_invoked = ?2,
             invocation_count = ?3, updated_at = ?2
             WHERE task_id = ?4",
            params![next, now, new_count, task_id],
        )
        .ok();

        tracing::debug!(
            task_id,
            label = %task.label,
            next_invoke = ?next,
            "Scheduled task advanced"
        );
    }

    /// List tasks, optionally filtered by status.
    pub fn list(conn: &Connection, status_filter: Option<&str>) -> Vec<ScheduledTask> {
        let (sql, filter_val);
        if let Some(status) = status_filter {
            sql = "SELECT task_id, label, description, schedule_type, interval_secs,
                   cron_expr, next_invoke, last_invoked, invocation_count, max_invocations,
                   urgency, status, action, metadata, created_at, updated_at
                   FROM scheduled_tasks WHERE status = ?1
                   ORDER BY next_invoke ASC NULLS LAST";
            filter_val = Some(status.to_string());
        } else {
            sql = "SELECT task_id, label, description, schedule_type, interval_secs,
                   cron_expr, next_invoke, last_invoked, invocation_count, max_invocations,
                   urgency, status, action, metadata, created_at, updated_at
                   FROM scheduled_tasks
                   ORDER BY next_invoke ASC NULLS LAST";
            filter_val = None;
        }

        let mut stmt = conn.prepare(sql).expect("prepare list");

        if let Some(ref val) = filter_val {
            stmt.query_map(params![val], row_to_task)
                .expect("query list")
                .filter_map(|r| r.ok())
                .collect()
        } else {
            stmt.query_map([], row_to_task)
                .expect("query list")
                .filter_map(|r| r.ok())
                .collect()
        }
    }

    /// Get a single task by ID.
    pub fn get(conn: &Connection, task_id: &str) -> Option<ScheduledTask> {
        conn.query_row(
            "SELECT task_id, label, description, schedule_type, interval_secs,
             cron_expr, next_invoke, last_invoked, invocation_count, max_invocations,
             urgency, status, action, metadata, created_at, updated_at
             FROM scheduled_tasks WHERE task_id = ?1",
            params![task_id],
            row_to_task,
        )
        .ok()
    }

    /// Update fields on a task. Only non-null JSON fields are applied.
    pub fn update(conn: &Connection, task_id: &str, updates: &serde_json::Value) {
        let now = now_ts();
        let mut sets = vec!["updated_at = ?1".to_string()];
        let mut idx = 2u32;

        // Collect updates dynamically
        macro_rules! maybe_set {
            ($field:expr, $json_key:expr) => {
                if updates.get($json_key).is_some() {
                    sets.push(format!("{} = ?{}", $field, idx));
                    idx += 1;
                }
            };
        }

        maybe_set!("label", "label");
        maybe_set!("description", "description");
        maybe_set!("next_invoke", "next_invoke");
        maybe_set!("urgency", "urgency");
        maybe_set!("status", "status");
        maybe_set!("interval_secs", "interval_seconds");
        maybe_set!("cron_expr", "cron");

        if sets.len() == 1 {
            return; // nothing to update besides updated_at
        }

        let sql = format!(
            "UPDATE scheduled_tasks SET {} WHERE task_id = ?{}",
            sets.join(", "),
            idx
        );

        // Build parameter list — this is a bit manual but avoids dynamic dispatch
        // We'll use a simpler approach: just execute individual updates
        // For each field, execute a targeted update
        if let Some(v) = updates.get("label").and_then(|v| v.as_str()) {
            conn.execute(
                "UPDATE scheduled_tasks SET label = ?1, updated_at = ?2 WHERE task_id = ?3",
                params![v, now, task_id],
            )
            .ok();
        }
        if let Some(v) = updates.get("description").and_then(|v| v.as_str()) {
            conn.execute(
                "UPDATE scheduled_tasks SET description = ?1, updated_at = ?2 WHERE task_id = ?3",
                params![v, now, task_id],
            )
            .ok();
        }
        if let Some(v) = updates.get("next_invoke").and_then(|v| v.as_f64()) {
            conn.execute(
                "UPDATE scheduled_tasks SET next_invoke = ?1, updated_at = ?2 WHERE task_id = ?3",
                params![v, now, task_id],
            )
            .ok();
        }
        if let Some(v) = updates.get("urgency").and_then(|v| v.as_f64()) {
            conn.execute(
                "UPDATE scheduled_tasks SET urgency = ?1, updated_at = ?2 WHERE task_id = ?3",
                params![v, now, task_id],
            )
            .ok();
        }
        if let Some(v) = updates.get("status").and_then(|v| v.as_str()) {
            conn.execute(
                "UPDATE scheduled_tasks SET status = ?1, updated_at = ?2 WHERE task_id = ?3",
                params![v, now, task_id],
            )
            .ok();
        }
        if let Some(v) = updates.get("interval_seconds").and_then(|v| v.as_i64()) {
            conn.execute(
                "UPDATE scheduled_tasks SET interval_secs = ?1, updated_at = ?2 WHERE task_id = ?3",
                params![v, now, task_id],
            )
            .ok();
        }
        if let Some(v) = updates.get("cron").and_then(|v| v.as_str()) {
            // Also recompute next_invoke from new cron expression
            let next = crate::cron_mini::next_cron(v, now);
            conn.execute(
                "UPDATE scheduled_tasks SET cron_expr = ?1, next_invoke = ?2, updated_at = ?3 WHERE task_id = ?4",
                params![v, next, now, task_id],
            )
            .ok();
        }

        // Suppress unused variable warning from macro expansion
        let _ = sql;
    }

    /// Cancel a task.
    pub fn cancel(conn: &Connection, task_id: &str) -> bool {
        let now = now_ts();
        let changes = conn
            .execute(
                "UPDATE scheduled_tasks SET status = 'cancelled', updated_at = ?1
                 WHERE task_id = ?2 AND status IN ('active', 'paused')",
                params![now, task_id],
            )
            .unwrap_or(0);
        changes > 0
    }

    /// Format a human-readable summary of active scheduled tasks (for system context).
    pub fn format_summary(conn: &Connection) -> String {
        let active = Self::list(conn, Some("active"));
        if active.is_empty() {
            return String::new();
        }

        let mut lines = vec!["Scheduled tasks:".to_string()];
        for task in active.iter().take(10) {
            let next_str = task
                .next_invoke
                .map(|ts| format_ts(ts))
                .unwrap_or_else(|| "—".to_string());
            lines.push(format!(
                "  - {} ({}) next: {} [{}]",
                task.label, task.schedule_type, next_str, task.task_id
            ));
        }
        lines.join("\n")
    }
}

/// Map a SQLite row to ScheduledTask.
fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<ScheduledTask> {
    let meta_str: String = row.get(13)?;
    Ok(ScheduledTask {
        task_id: row.get(0)?,
        label: row.get(1)?,
        description: row.get(2)?,
        schedule_type: row.get(3)?,
        interval_secs: row.get(4)?,
        cron_expr: row.get(5)?,
        next_invoke: row.get(6)?,
        last_invoked: row.get(7)?,
        invocation_count: row.get(8)?,
        max_invocations: row.get(9)?,
        urgency: row.get(10)?,
        status: row.get(11)?,
        action: row.get(12)?,
        metadata: serde_json::from_str(&meta_str).unwrap_or_default(),
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Format a unix timestamp as ISO-like string (UTC).
fn format_ts(ts: f64) -> String {
    let secs = ts as i64;
    let days = secs.div_euclid(86400);
    let time_of_day = secs.rem_euclid(86400);
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;

    // Reuse the same date algorithm as cron_mini
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}Z", y, m, d, hour, minute)
}
