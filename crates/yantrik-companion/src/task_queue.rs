//! Persistent task queue — LLM tasks that span multiple think cycles.
//!
//! Unlike `task_manager.rs` (background OS processes), this tracks high-level
//! tasks the LLM should work on autonomously across idle periods.
//! Tasks persist in SQLite and are picked up during think cycles.
//!
//! Flow:
//! 1. User says "evaluate all tools" → LLM calls `queue_task` tool
//! 2. Task stored in SQLite with status "pending"
//! 3. Each think cycle: pick the highest-priority pending/in_progress task
//! 4. Run it through `handle_message` with task context
//! 5. LLM uses tools to do actual work, updates progress
//! 6. When done, mark completed. Results stored in `result` field.
//! 7. Notify user via proactive message.

use rusqlite::Connection;

/// A persistent task from the queue.
#[derive(Debug, Clone)]
pub struct QueuedTask {
    pub task_id: String,
    pub title: String,
    pub description: String,
    pub priority: i32,         // 1=low, 2=normal, 3=high, 4=urgent
    pub status: TaskStatus,
    pub progress: String,      // Running summary of what's been done
    pub result: String,        // Final result text
    pub steps_completed: i32,
    pub steps_total: i32,      // 0 = unknown
    pub created_at: f64,
    pub updated_at: f64,
    pub completed_at: Option<f64>,
    pub created_by: String,    // "user", "instinct", "system"
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

/// Persistent task queue backed by SQLite.
pub struct TaskQueue;

impl TaskQueue {
    /// Create the task_queue table.
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS task_queue (
                task_id         TEXT PRIMARY KEY,
                title           TEXT NOT NULL,
                description     TEXT NOT NULL DEFAULT '',
                priority        INTEGER NOT NULL DEFAULT 2,
                status          TEXT NOT NULL DEFAULT 'pending',
                progress        TEXT NOT NULL DEFAULT '',
                result          TEXT NOT NULL DEFAULT '',
                steps_completed INTEGER NOT NULL DEFAULT 0,
                steps_total     INTEGER NOT NULL DEFAULT 0,
                created_at      REAL NOT NULL,
                updated_at      REAL NOT NULL,
                completed_at    REAL,
                created_by      TEXT NOT NULL DEFAULT 'user'
            );
            CREATE INDEX IF NOT EXISTS idx_task_queue_status ON task_queue(status, priority DESC);",
        )
        .expect("failed to create task_queue table");
    }

    /// Add a new task to the queue.
    pub fn enqueue(
        conn: &Connection,
        title: &str,
        description: &str,
        priority: i32,
        created_by: &str,
    ) -> Result<String, String> {
        let task_id = format!("tq_{:08x}", rand_u32());
        let now = now_ts();
        conn.execute(
            "INSERT INTO task_queue (task_id, title, description, priority, status, created_at, updated_at, created_by)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?5, ?6)",
            rusqlite::params![task_id, title, description, priority.clamp(1, 4), now, created_by],
        )
        .map_err(|e| format!("Failed to enqueue task: {e}"))?;

        tracing::info!(task_id = %task_id, title = %title, priority, "Task queued");
        Ok(task_id)
    }

    /// Get the next task to work on: highest priority pending, or in_progress (resume).
    pub fn next_task(conn: &Connection) -> Option<QueuedTask> {
        // Prefer in_progress (resume) over pending (new)
        let sql = "SELECT task_id, title, description, priority, status, progress, result,
                          steps_completed, steps_total, created_at, updated_at, completed_at, created_by
                   FROM task_queue
                   WHERE status IN ('in_progress', 'pending')
                   ORDER BY
                     CASE status WHEN 'in_progress' THEN 0 ELSE 1 END,
                     priority DESC,
                     created_at ASC
                   LIMIT 1";
        conn.query_row(sql, [], |row| Ok(row_to_task(row)))
            .ok()
    }

    /// Update task progress.
    pub fn update_progress(
        conn: &Connection,
        task_id: &str,
        progress: &str,
        steps_completed: i32,
    ) {
        let now = now_ts();
        conn.execute(
            "UPDATE task_queue SET status = 'in_progress', progress = ?1, steps_completed = ?2, updated_at = ?3
             WHERE task_id = ?4",
            rusqlite::params![progress, steps_completed, now, task_id],
        )
        .ok();
    }

    /// Mark a task as completed.
    pub fn complete(conn: &Connection, task_id: &str, result: &str) {
        let now = now_ts();
        conn.execute(
            "UPDATE task_queue SET status = 'completed', result = ?1, completed_at = ?2, updated_at = ?2
             WHERE task_id = ?3",
            rusqlite::params![result, now, task_id],
        )
        .ok();
        tracing::info!(task_id = %task_id, "Task completed");
    }

    /// Mark a task as failed.
    pub fn fail(conn: &Connection, task_id: &str, reason: &str) {
        let now = now_ts();
        conn.execute(
            "UPDATE task_queue SET status = 'failed', result = ?1, completed_at = ?2, updated_at = ?2
             WHERE task_id = ?3",
            rusqlite::params![reason, now, task_id],
        )
        .ok();
        tracing::warn!(task_id = %task_id, reason = %reason, "Task failed");
    }

    /// Cancel a task.
    pub fn cancel(conn: &Connection, task_id: &str) {
        let now = now_ts();
        conn.execute(
            "UPDATE task_queue SET status = 'cancelled', updated_at = ?1
             WHERE task_id = ?2 AND status IN ('pending', 'in_progress')",
            rusqlite::params![now, task_id],
        )
        .ok();
    }

    /// List tasks, optionally filtered by status.
    pub fn list(conn: &Connection, status_filter: Option<&str>, limit: usize) -> Vec<QueuedTask> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match status_filter {
            Some(s) => (
                format!(
                    "SELECT task_id, title, description, priority, status, progress, result,
                            steps_completed, steps_total, created_at, updated_at, completed_at, created_by
                     FROM task_queue WHERE status = ?1
                     ORDER BY priority DESC, created_at DESC LIMIT {limit}"
                ),
                vec![Box::new(s.to_string()) as Box<dyn rusqlite::types::ToSql>],
            ),
            None => (
                format!(
                    "SELECT task_id, title, description, priority, status, progress, result,
                            steps_completed, steps_total, created_at, updated_at, completed_at, created_by
                     FROM task_queue
                     ORDER BY
                       CASE status WHEN 'in_progress' THEN 0 WHEN 'pending' THEN 1 ELSE 2 END,
                       priority DESC, created_at DESC
                     LIMIT {limit}"
                ),
                vec![],
            ),
        };

        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        stmt.query_map(param_refs.as_slice(), |row| Ok(row_to_task(row)))
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect()
    }

    /// Get a specific task by ID.
    pub fn get(conn: &Connection, task_id: &str) -> Option<QueuedTask> {
        conn.query_row(
            "SELECT task_id, title, description, priority, status, progress, result,
                    steps_completed, steps_total, created_at, updated_at, completed_at, created_by
             FROM task_queue WHERE task_id = ?1",
            [task_id],
            |row| Ok(row_to_task(row)),
        )
        .ok()
    }

    /// Count pending + in_progress tasks.
    pub fn active_count(conn: &Connection) -> usize {
        conn.query_row(
            "SELECT COUNT(*) FROM task_queue WHERE status IN ('pending', 'in_progress')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    /// Format a summary of active tasks for system prompt injection.
    pub fn format_active_summary(conn: &Connection) -> String {
        let tasks = Self::list(conn, None, 5);
        let active: Vec<&QueuedTask> = tasks.iter()
            .filter(|t| t.status == TaskStatus::Pending || t.status == TaskStatus::InProgress)
            .collect();

        if active.is_empty() {
            return String::new();
        }

        let mut lines = vec!["Queued tasks:".to_string()];
        for t in active {
            let status_icon = match t.status {
                TaskStatus::InProgress => "▶",
                _ => "○",
            };
            let progress = if t.progress.is_empty() {
                String::new()
            } else {
                let short = if t.progress.len() > 80 {
                    format!("{}...", &t.progress[..t.progress.floor_char_boundary(77)])
                } else {
                    t.progress.clone()
                };
                format!(" — {short}")
            };
            lines.push(format!("- {status_icon} [{}] {} (P{}){progress}", t.task_id, t.title, t.priority));
        }
        lines.join("\n")
    }
}

fn row_to_task(row: &rusqlite::Row) -> QueuedTask {
    QueuedTask {
        task_id: row.get(0).unwrap_or_default(),
        title: row.get(1).unwrap_or_default(),
        description: row.get(2).unwrap_or_default(),
        priority: row.get(3).unwrap_or(2),
        status: TaskStatus::from_str(&row.get::<_, String>(4).unwrap_or_default()),
        progress: row.get(5).unwrap_or_default(),
        result: row.get(6).unwrap_or_default(),
        steps_completed: row.get(7).unwrap_or(0),
        steps_total: row.get(8).unwrap_or(0),
        created_at: row.get(9).unwrap_or(0.0),
        updated_at: row.get(10).unwrap_or(0.0),
        completed_at: row.get(11).ok(),
        created_by: row.get(12).unwrap_or_default(),
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn rand_u32() -> u32 {
    // Simple pseudo-random from timestamp nanos
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    ns.wrapping_mul(2654435761) // Knuth's multiplicative hash
}
