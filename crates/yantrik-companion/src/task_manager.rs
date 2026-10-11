//! Background task manager — spawn, track, poll, and stop long-running processes.
//!
//! Tools can spawn background tasks via `TaskManager::spawn()`, which uses
//! `std::process::Command::spawn()` with stdout/stderr redirected to a temp file.
//! The manager is polled lazily (on each message + system context update) to
//! detect completions. SQLite table persists task metadata across restarts.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::process::{Child, Command, Stdio};

use rusqlite::Connection;

/// Maximum concurrent running tasks.
const MAX_CONCURRENT: usize = 10;

/// A tracked background task's metadata (from SQLite).
#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub task_id: String,
    pub command: String,
    pub label: String,
    pub pid: Option<u32>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub started_at: f64,
    pub finished_at: Option<f64>,
    pub output_tail: String,
}

/// Manages background child processes with SQLite-backed persistence.
pub struct TaskManager {
    /// Live child processes keyed by task_id.
    live: HashMap<String, Child>,
    /// Simple counter for task ID generation.
    next_id: u64,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            live: HashMap::new(),
            next_id: 1,
        }
    }

    /// Create the background_tasks table if it doesn't exist.
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS background_tasks (
                task_id     TEXT PRIMARY KEY,
                command     TEXT NOT NULL,
                label       TEXT NOT NULL,
                pid         INTEGER,
                status      TEXT NOT NULL DEFAULT 'running',
                exit_code   INTEGER,
                output_file TEXT NOT NULL,
                started_at  REAL NOT NULL,
                finished_at REAL,
                recorded    INTEGER NOT NULL DEFAULT 0
            );",
        )
        .expect("failed to create background_tasks table");
    }

    /// Spawn a command in the background.
    ///
    /// Returns the task_id on success. Stdout and stderr are redirected to
    /// `yantrik-task-{id}.out` in the private scratch dir (see `output_path`).
    pub fn spawn(
        &mut self,
        conn: &Connection,
        command: &str,
        label: &str,
    ) -> Result<String, String> {
        // Check concurrent limit
        let running: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM background_tasks WHERE status = 'running'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if running >= MAX_CONCURRENT as i64 {
            return Err(format!(
                "Too many concurrent tasks ({running}/{MAX_CONCURRENT}). Stop or wait for some to finish."
            ));
        }

        let task_id = format!("t{:04x}", self.next_id);
        self.next_id += 1;

        let output_path = Self::output_path(&task_id)?;
        // Not a plain create: the file tools can write into scratch too, and a link or second
        // hard link left at this predictable name must not be followed and emptied.
        let log_file = yantrik_ml::private_dir::create_scratch(&Self::output_name(&task_id))
            .map_err(|e| format!("Failed to create output file: {e}"))?;
        let stderr_file = log_file
            .try_clone()
            .map_err(|e| format!("Failed to clone file handle: {e}"))?;

        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .map_err(|e| format!("Failed to spawn process: {e}"))?;

        let pid = child.id();
        let now = now_ts();

        conn.execute(
            "INSERT INTO background_tasks (task_id, command, label, pid, status, output_file, started_at)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6)",
            rusqlite::params![task_id, command, label, pid, output_path, now],
        )
        .map_err(|e| format!("Failed to record task: {e}"))?;

        self.live.insert(task_id.clone(), child);

        tracing::info!(
            task_id = %task_id,
            pid = pid,
            command = %command,
            "Background task spawned"
        );

        Ok(task_id)
    }

    /// Poll all live tasks. Returns task_ids of newly completed tasks.
    pub fn poll(&mut self, conn: &Connection) -> Vec<String> {
        let mut completed = Vec::new();
        let now = now_ts();

        let finished: Vec<(String, i32)> = self
            .live
            .iter_mut()
            .filter_map(|(id, child)| {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let code = status.code().unwrap_or(-1);
                        Some((id.clone(), code))
                    }
                    Ok(None) => None, // Still running
                    Err(e) => {
                        tracing::warn!(task_id = %id, error = %e, "Error polling task");
                        Some((id.clone(), -1))
                    }
                }
            })
            .collect();

        for (id, code) in finished {
            self.live.remove(&id);
            let status_str = if code == 0 { "completed" } else { "failed" };
            conn.execute(
                "UPDATE background_tasks SET status = ?1, exit_code = ?2, finished_at = ?3 WHERE task_id = ?4",
                rusqlite::params![status_str, code, now, id],
            )
            .ok();
            tracing::info!(task_id = %id, exit_code = code, status = status_str, "Background task finished");
            completed.push(id);
        }

        completed
    }

    /// Get status of a specific task.
    pub fn get_status(&self, conn: &Connection, task_id: &str) -> Option<TaskInfo> {
        conn.query_row(
            "SELECT task_id, command, label, pid, status, exit_code, started_at, finished_at
             FROM background_tasks WHERE task_id = ?1",
            [task_id],
            |row| {
                let task_id: String = row.get(0)?;
                Ok(TaskInfo {
                    output_tail: Self::read_output(&task_id, 20),
                    task_id,
                    command: row.get(1)?,
                    label: row.get(2)?,
                    pid: row.get(3)?,
                    status: row.get(4)?,
                    exit_code: row.get(5)?,
                    started_at: row.get(6)?,
                    finished_at: row.get(7)?,
                })
            },
        )
        .ok()
    }

    /// List tasks, optionally filtered by status.
    pub fn list(&self, conn: &Connection, status_filter: Option<&str>) -> Vec<TaskInfo> {
        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match status_filter {
            Some(s) => (
                "SELECT task_id, command, label, pid, status, exit_code, started_at, finished_at
                 FROM background_tasks WHERE status = ?1 ORDER BY started_at DESC LIMIT 20",
                vec![Box::new(s.to_string()) as Box<dyn rusqlite::types::ToSql>],
            ),
            None => (
                "SELECT task_id, command, label, pid, status, exit_code, started_at, finished_at
                 FROM background_tasks ORDER BY started_at DESC LIMIT 20",
                vec![],
            ),
        };

        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        stmt.query_map(param_refs.as_slice(), |row| {
            let task_id: String = row.get(0)?;
            Ok(TaskInfo {
                output_tail: Self::read_output(&task_id, 5),
                task_id,
                command: row.get(1)?,
                label: row.get(2)?,
                pid: row.get(3)?,
                status: row.get(4)?,
                exit_code: row.get(5)?,
                started_at: row.get(6)?,
                finished_at: row.get(7)?,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Stop a running task.
    pub fn stop(&mut self, conn: &Connection, task_id: &str) -> Result<String, String> {
        if let Some(mut child) = self.live.remove(task_id) {
            let pid = child.id();
            match child.kill() {
                Ok(()) => {
                    let _ = child.wait(); // Reap the zombie
                    let now = now_ts();
                    conn.execute(
                        "UPDATE background_tasks SET status = 'stopped', finished_at = ?1 WHERE task_id = ?2",
                        rusqlite::params![now, task_id],
                    )
                    .ok();
                    tracing::info!(task_id = %task_id, pid = pid, "Background task stopped");
                    Ok(format!("Task {task_id} stopped (pid {pid})"))
                }
                Err(e) => Err(format!("Failed to kill task {task_id}: {e}")),
            }
        } else {
            // Check if it exists but already finished
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM background_tasks WHERE task_id = ?1",
                    [task_id],
                    |row| row.get(0),
                )
                .ok();
            match status.as_deref() {
                Some("completed") => Err(format!("Task {task_id} already completed")),
                Some("failed") => Err(format!("Task {task_id} already failed")),
                Some("stopped") => Err(format!("Task {task_id} already stopped")),
                Some(_) => Err(format!("Task {task_id} not found in live processes")),
                None => Err(format!("Unknown task: {task_id}")),
            }
        }
    }

    /// Read the last N lines from a task's output file.
    ///
    /// Opened through `read_scratch`, not a plain open: the names are predictable and the file
    /// tools can write into scratch, so `yantrik-task-t0001.out` could be a link to a key that
    /// "the task's output" would then hand to the model.
    pub fn read_output(task_id: &str, tail_lines: usize) -> String {
        match yantrik_ml::private_dir::read_scratch(&Self::output_name(task_id)) {
            Ok(file) => Self::read_output_tail(file, tail_lines),
            Err(_) => String::new(),
        }
    }

    /// Where a task's stdout and stderr go, and where `read_output` looks for them.
    ///
    /// Task ids are a counter (`t0001`, `t0002`, …) that restarts with the process, so under
    /// `/tmp` every name was guessable in advance: another account could plant a symlink there
    /// and have `File::create` truncate a file of their choosing, or leave output of its own for
    /// the agent to read back as the result of a command it ran. The private scratch dir is ours
    /// alone. The id is also refused if it is not a plain name, since `read_output` takes it from
    /// the model's tool call. A string, because it is stored in the tasks table as one.
    ///
    /// The path is always worked out from the id, never read back from the tasks table: rows
    /// written before this change point into /tmp, where the file at that name may now be anyone's.
    fn output_path(task_id: &str) -> Result<String, String> {
        yantrik_ml::private_dir::scratch_file_string(&Self::output_name(task_id))
            .map_err(|e| format!("No private directory for task output: {e}"))
    }

    fn output_name(task_id: &str) -> String {
        format!("yantrik-task-{task_id}.out")
    }

    fn read_output_tail(file: std::fs::File, tail_lines: usize) -> String {
        // Read last N lines by seeking backwards
        let metadata = match file.metadata() {
            Ok(m) => m,
            Err(_) => return String::new(),
        };

        let size = metadata.len();
        if size == 0 {
            return String::new();
        }

        // A small file is read whole; a larger one only its last few kilobytes.
        let mut file = file;
        let seek_pos = if size < 8192 {
            0
        } else {
            size.saturating_sub(4096)
        };
        if file.seek(SeekFrom::Start(seek_pos)).is_err() {
            return String::new();
        }

        // The bytes are decoded in one go rather than line by line. A seek can land inside a
        // multi-byte character, and `BufRead::lines` drops that whole line as undecodable, so the
        // skip below threw away a second, complete line with the partial one. Decoding lossily
        // leaves the broken character on the first line, which is the line being skipped, and a
        // read error ends the read instead of failing again on every retry.
        let mut chunk = Vec::new();
        if file.read_to_end(&mut chunk).is_err() {
            return String::new();
        }
        let text = String::from_utf8_lossy(&chunk);
        let lines: Vec<&str> = text.lines().collect();
        // The first line is partial whenever the read started inside the file.
        let skip = if seek_pos > 0 { 1 } else { 0 };
        let valid = &lines[skip.min(lines.len())..];
        let start = valid.len().saturating_sub(tail_lines);
        valid[start..].join("\n")
    }

    /// Compact summary of active tasks for system context injection.
    pub fn format_active_summary(&self, conn: &Connection) -> String {
        let tasks = self.list(conn, Some("running"));
        if tasks.is_empty() {
            return String::new();
        }

        let now = now_ts();
        let mut lines = vec!["Background tasks:".to_string()];
        for t in &tasks {
            let elapsed = (now - t.started_at).max(0.0);
            let elapsed_str = format_duration(elapsed);
            lines.push(format!("- [{}] \"{}\" running ({})", t.task_id, t.label, elapsed_str));
        }
        lines.join("\n")
    }

    /// Mark a completed task as recorded (result written to memory).
    pub fn mark_recorded(conn: &Connection, task_id: &str) {
        conn.execute(
            "UPDATE background_tasks SET recorded = 1 WHERE task_id = ?1",
            [task_id],
        )
        .ok();
    }

    /// On startup, mark any "running" tasks as failed (process lost on restart).
    pub fn recover_stale(&mut self, conn: &Connection) {
        let now = now_ts();
        let count = conn
            .execute(
                "UPDATE background_tasks SET status = 'failed', finished_at = ?1
                 WHERE status = 'running'",
                [now],
            )
            .unwrap_or(0);
        if count > 0 {
            tracing::info!(count, "Recovered stale background tasks (marked as failed)");
        }
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn format_duration(secs: f64) -> String {
    let s = secs as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::TaskManager;
    use std::io::Write;

    /// A log big enough that the tail is read from `size - 4096`, with that cut landing inside a
    /// multi-byte character of the third-to-last line. The two lines after the cut are complete,
    /// so they are the last two lines of the log and both must come back: the old code dropped the
    /// partial line as undecodable and then skipped a complete one on top of it.
    #[test]
    fn a_tail_cut_inside_a_character_loses_no_line() {
        let mut content = String::new();
        while content.len() + 1024 <= 8192 {
            content.push_str(&"r".repeat(1023));
            content.push('\n');
        }
        // The cut sits 4096 bytes from the end, so it falls inside the first of these three lines
        // and, since they are made of 3-byte characters, inside a character.
        content.push_str(&"字".repeat(1400));
        content.push('\n');
        content.push_str("second-to-last line: 日本語のテキスト\n");
        let last_line = "last line: 日本語\n";
        content.push_str(last_line);
        let bytes = content.as_bytes();

        // The assertions below are only worth anything if the read really starts mid-character.
        let cut = bytes.len() - 4096;
        assert!(bytes.len() >= 8192, "big enough to seek: {}", bytes.len());
        assert!(
            (bytes[cut] & 0xc0) == 0x80,
            "the cut at {cut} must land inside a character, not on its first byte"
        );

        let last_two = format!("second-to-last line: 日本語のテキスト\n{}", last_line.trim_end());
        assert_eq!(tail(&content, 2), last_two);
        assert_eq!(tail(&content, 1), "last line: 日本語");
    }

    /// A log under the size where nothing is skipped, since all of it is read.
    #[test]
    fn a_small_log_tails_its_last_lines() {
        let content = "one\ntwo\nthree\nfour\n";
        assert_eq!(tail(content, 2), "three\nfour");
        assert_eq!(tail(content, 99), content.trim_end());
    }

    /// An empty log has no tail, and a log with fewer lines than asked for returns all of them.
    #[test]
    fn an_empty_log_tails_to_nothing() {
        assert_eq!(tail("", 5), "");
        assert_eq!(tail("only\n", 5), "only");
    }

    /// Writes `content` where `read_output_tail` can read it back. Each call gets its own name:
    /// the tests run in parallel and one log's content was landing in another's file.
    fn tail(content: &str, tail_lines: usize) -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yantrik-task-tail-{}-{n}",
            std::process::id()
        ));
        {
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(content.as_bytes()).unwrap();
            file.flush().unwrap();
        }
        let file = std::fs::File::open(&path).unwrap();
        let got = TaskManager::read_output_tail(file, tail_lines);
        let _ = std::fs::remove_file(&path);
        got
    }
}
