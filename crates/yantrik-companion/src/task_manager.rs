//! Background task manager — spawn, track, poll, and stop long-running processes.
//!
//! Tools can spawn background tasks via `TaskManager::spawn()`, which uses
//! `std::process::Command::spawn()` with stdout/stderr redirected to a temp file.
//! The manager is polled lazily (on each message + system context update) to
//! detect completions. SQLite table persists task metadata across restarts.

use std::collections::HashMap;
use std::process::{Child, Stdio};

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
}

/// Manages background child processes with SQLite-backed persistence.
pub struct TaskManager {
    /// Live child processes keyed by task_id.
    live: HashMap<String, Child>,
    /// Simple counter for task ID generation.
    next_id: u64,
}

/// A vetted program as a command (#1124, review R11-H2): it starts through the Mind door, in the outside
/// view, with the workshops root and the person's `/run/user` hidden, directly with its words and a plain
/// environment, as `run_command` starts one: no shell reads the line. On a machine that holds workshops
/// and has no `bwrap` it is refused, and nothing has run. The one way [`TaskManager::spawn`] starts a program.
fn checked_command(home: &std::path::Path, argv: &[String]) -> Result<std::process::Command, String> {
    let (program, words) = argv.split_first().ok_or("The command was not run: it is empty")?;
    let mut command = yantrik_companion_tools::spawn::mind_command(home, program, words).map_err(|e| format!("The command was not run: {e}"))?;
    yantrik_companion_tools::safe_command::plain_environment(&mut command);
    Ok(command)
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
                recorded    INTEGER NOT NULL DEFAULT 0,
                vetted      INTEGER NOT NULL DEFAULT 0,
                argv        TEXT NOT NULL DEFAULT ''
            );",
        )
        .expect("failed to create background_tasks table");
        // A table made before `vetted` has none; its rows were not vetted.
        let _ = conn.execute("ALTER TABLE background_tasks ADD COLUMN vetted INTEGER NOT NULL DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE background_tasks ADD COLUMN argv TEXT NOT NULL DEFAULT ''", []);
    }

    /// Spawn a vetted program (`safe_command::vet`) in the background.
    ///
    /// Returns the task_id on success. Stdout and stderr are redirected to
    /// `yantrik-task-{id}.out` in the private scratch dir (see `output_path`).
    pub fn spawn(
        &mut self,
        conn: &Connection,
        argv: &[String],
        label: &str,
    ) -> Result<String, String> {
        let command = argv.join(" ");
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

        // Refused before anything is created or counted: a refused line leaves no trace.
        let mut shell = checked_command(&yantrik_companion_tools::spawn::home(), argv)?;

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

        shell.stdout(Stdio::from(log_file)).stderr(Stdio::from(stderr_file));
        let child = yantrik_companion_tools::spawn::spawn_long_lived(shell)
            .map_err(|e| format!("Failed to spawn process: {e}"))?;

        let pid = child.id();
        let now = now_ts();

        conn.execute(
            "INSERT INTO background_tasks (task_id, command, label, pid, status, output_file, started_at, vetted, argv)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, 1, ?7)",
            rusqlite::params![task_id, command, label, pid, output_path, now, serde_json::to_string(argv).unwrap_or_default()],
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
    /// The tail of a task's output, for a task `run_background` vetted and started; nothing for any other
    /// row (one from before the gate, or one no known tool started): its output may hold what the gate
    /// would not have let a command read (review R11-H2).
    ///
    /// What is shown is what `run_command` shows of the same program (N-H1): its listing is filtered by
    /// [`yantrik_companion_tools::safe_command_listing::shown_together`], from the argv stored with the row.
    pub fn output_of(conn: &Connection, task_id: &str, tail_lines: usize) -> String {
        let row = conn.query_row("SELECT vetted, argv FROM background_tasks WHERE task_id = ?1", [task_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)));
        let Ok((vetted, argv)) = row else { return String::new() };
        let argv: Vec<String> = serde_json::from_str(&argv).unwrap_or_default();
        if vetted == 0 || argv.is_empty() {
            return String::new();
        }
        yantrik_companion_tools::safe_command_listing::tail_shown(&argv, Self::read_output(task_id), tail_lines)
    }

    /// The start of a task's output, up to the cap `run_command` reads: it is filtered before it is cut
    /// to its last lines (R12-H2), so the whole of it is read, never only its tail.
    fn read_output(task_id: &str) -> Vec<u8> {
        use std::io::Read as _;
        let Ok(file) = yantrik_ml::private_dir::read_scratch(&Self::output_name(task_id)) else { return Vec::new() };
        let mut raw = Vec::new();
        let _ = file.take(yantrik_companion_tools::safe_command::OUT_CAP as u64).read_to_end(&mut raw);
        raw
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
    use super::checked_command;
    use std::path::Path;
    use yantrik_ipc_contracts::home_paths::workshops::{base, find_bwrap, root, roots_under_home_for_tests};

    /// A home whose workshops root holds one workshop, as on a machine that has granted one.
    fn home_holding_a_workshop() -> std::path::PathBuf {
        roots_under_home_for_tests();
        let home = std::env::temp_dir().join(format!("yantrik-task-manager-{}", std::process::id()));
        std::fs::create_dir_all(root(&home).join("w")).unwrap();
        home
    }

    fn argv(c: &std::process::Command) -> Vec<String> {
        c.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    /// `run_background` starts a vetted program directly, never `sh -c <line>`: with bwrap the program is
    /// bwrap with the workshops base masked by a tmpfs; without it, a machine that holds workshops refuses.
    #[test]
    fn a_background_program_runs_in_the_outside_view_or_not_at_all() {
        let home = home_holding_a_workshop();
        let words = ["ls".to_string(), "/var/lib/yantrik/workshops".to_string()];
        match checked_command(&home, &words) {
            Ok(c) => {
                assert!(find_bwrap().is_some());
                assert!(c.get_program().to_string_lossy().ends_with("bwrap"), "{:?}", c.get_program());
                let args = argv(&c);
                assert!(args.windows(2).any(|w| w[0] == "--tmpfs" && Path::new(&w[1]) == base(&home)), "{args:?}");
                assert_eq!(args[args.len() - 2..], words, "no shell between the outside view and the program");
            }
            Err(e) => {
                assert!(find_bwrap().is_none());
                assert!(e.contains("The command was not run") && e.contains("nothing was run"), "{e}");
            }
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The one place the task manager starts a program is that door: no `Command::new` of its own.
    #[test]
    fn the_task_manager_builds_no_command_of_its_own() {
        let src = include_str!("task_manager.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!code.contains("Command::new") && code.contains("spawn::mind_command"));
    }
}
