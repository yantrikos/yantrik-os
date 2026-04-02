//! Recipe Engine — structured automation with tool bypass.
//!
//! Recipes are ordered lists of steps with conditional jumps. Unlike the task queue
//! (open-ended LLM work), recipes have predetermined steps where Tool steps execute
//! directly via the tool registry without LLM involvement.
//!
//! Architecture:
//! - Recipe created from natural language via `create_recipe` tool
//! - Steps stored in normalized SQLite tables (debuggable, queryable)
//! - Tool steps bypass LLM entirely for speed
//! - Think steps call LLM for decision-making
//! - Triggers: manual, time-based (cron), event-based, signal-based
//! - Self-signals via ProcessRecipeStep for continuous execution

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Step Types ──

/// Comparison operator for Filter steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FilterOp {
    Equals,
    NotEquals,
    Contains,
    GreaterThan,
    LessThan,
}

/// Aggregation operator for Aggregate steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AggregateOp {
    Count,
    Sum,
    Min,
    Max,
    Avg,
}

/// A single step in a recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RecipeStep {
    /// Direct tool call — no LLM needed.
    Tool {
        tool_name: String,
        args: serde_json::Value,
        store_as: String,
        #[serde(default)]
        on_error: ErrorAction,
    },
    /// LLM decides what to do with context. Variable references like {{var}} are resolved.
    /// When `fallback_template` is set and no LLM is available, the template is used instead.
    Think {
        prompt: String,
        store_as: String,
        /// Template string with {{var}} substitution, used when LLM is unavailable.
        #[serde(default)]
        fallback_template: Option<String>,
    },
    /// Jump to target_step if condition is true. Evaluated in pure Rust, no LLM.
    JumpIf {
        condition: Condition,
        target_step: usize,
    },
    /// Wait for an external condition. Persists state and pauses recipe.
    WaitFor {
        condition: WaitCondition,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    /// Notify user via proactive message. Supports {{var}} templates.
    Notify {
        message: String,
    },
    /// Pause recipe and ask user a question. Stores response in variable.
    /// Recipe transitions to Waiting until user responds.
    AskUser {
        question: String,
        store_as: String,
        /// Optional choices for multiple-choice (UI can render as buttons).
        #[serde(default)]
        choices: Option<Vec<String>>,
    },
    /// LLM synthesis with per-claim citations. Requires source variables from Tool steps.
    /// Outputs structured CitedOutput JSON stored in `store_as`.
    ThinkCited {
        prompt: String,
        store_as: String,
        /// Variable names that serve as sources (from prior Tool steps).
        source_vars: Vec<String>,
    },
    /// Deterministic validation of cited output. Strips uncited claims,
    /// computes evidence strength, produces a cleaned result. No LLM needed.
    Validate {
        /// Variable containing CitedOutput JSON (from ThinkCited).
        input_var: String,
        store_as: String,
    },
    /// Format validated data for user presentation.
    /// Supports multiple output formats.
    Render {
        /// Variable containing validated output.
        input_var: String,
        store_as: String,
        /// Output format.
        #[serde(default)]
        format: RenderFormat,
    },

    // ── Deterministic steps (no LLM needed) ──

    /// Format data using a template string with {{variable}} substitution.
    Format {
        input_vars: Vec<String>,
        template: String,
        store_as: String,
    },
    /// Filter a collection (JSON array in a variable) by a predicate.
    Filter {
        input_var: String,
        field: String,
        op: FilterOp,
        value: String,
        store_as: String,
    },
    /// Sort a collection by a field.
    Sort {
        input_var: String,
        by_field: String,
        #[serde(default)]
        descending: bool,
        store_as: String,
    },
    /// Aggregate a collection (count, sum, min, max, avg).
    Aggregate {
        input_var: String,
        op: AggregateOp,
        field: Option<String>,
        store_as: String,
    },
    /// Extract a value from structured output using a key path or regex.
    Extract {
        input_var: String,
        pattern: String,
        store_as: String,
    },
    /// Branch: if condition is true run then_steps, else run else_steps.
    Branch {
        condition: String,
        then_steps: Vec<RecipeStep>,
        else_steps: Vec<RecipeStep>,
    },
}

/// Output format for Render steps.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum RenderFormat {
    /// Bullet-point summary (default).
    #[default]
    Summary,
    /// Markdown table.
    Table,
    /// Side-by-side comparison grid.
    Comparison,
    /// Numbered card layout.
    Cards,
}

// ── Citation Types ──

/// A single claim with source citations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitedClaim {
    /// The claim text.
    pub text: String,
    /// Source variable names that back this claim.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Confidence: "high", "medium", "low", "uncited".
    #[serde(default = "default_confidence")]
    pub confidence: String,
}

fn default_confidence() -> String { "uncited".to_string() }

/// Structured output from ThinkCited steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitedOutput {
    /// Section title.
    pub title: String,
    /// Claims with source citations.
    pub claims: Vec<CitedClaim>,
    /// Overall evidence strength.
    #[serde(default)]
    pub evidence_status: EvidenceStatus,
}

/// How well-supported the evidence is.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum EvidenceStatus {
    /// 3+ independent sources confirm.
    Strong,
    /// 2 sources or 1 high-quality source.
    Moderate,
    /// 1 source only.
    Thin,
    /// Sources disagree.
    Conflicting,
    /// Not enough data.
    #[default]
    Insufficient,
}

/// What to do when a Tool step fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum ErrorAction {
    Fail,
    Skip,
    Retry { max: u8 },
    JumpTo { step: usize },
    /// Ask the LLM to diagnose the failure and replan remaining steps.
    Replan,
}

impl Default for ErrorAction {
    fn default() -> Self { Self::Fail }
}

/// Conditions evaluable in pure Rust — no LLM cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Condition {
    VarEquals { var: String, value: serde_json::Value },
    VarContains { var: String, substring: String },
    VarExists { var: String },
    VarGt { var: String, threshold: f64 },
    VarEmpty { var: String },
    TimeAfter { hour: u8, minute: u8 },
    TimeBefore { hour: u8, minute: u8 },
    Not { inner: Box<Condition> },
    And { conditions: Vec<Condition> },
    Or { conditions: Vec<Condition> },
}

impl Condition {
    /// Evaluate the condition against recipe variables. Pure Rust, zero LLM cost.
    pub fn evaluate(&self, vars: &std::collections::HashMap<String, serde_json::Value>) -> bool {
        match self {
            Self::VarEquals { var, value } => {
                vars.get(var).map_or(false, |v| v == value)
            }
            Self::VarContains { var, substring } => {
                vars.get(var)
                    .and_then(|v| v.as_str())
                    .map_or(false, |s| s.contains(substring.as_str()))
            }
            Self::VarExists { var } => vars.contains_key(var),
            Self::VarEmpty { var } => {
                vars.get(var).map_or(true, |v| {
                    v.is_null() || v.as_str().map_or(false, |s| s.is_empty())
                        || v.as_array().map_or(false, |a| a.is_empty())
                })
            }
            Self::VarGt { var, threshold } => {
                vars.get(var)
                    .and_then(|v| v.as_f64())
                    .map_or(false, |n| n > *threshold)
            }
            Self::TimeAfter { hour, minute } => {
                let now = chrono_now();
                now.0 > *hour || (now.0 == *hour && now.1 >= *minute)
            }
            Self::TimeBefore { hour, minute } => {
                let now = chrono_now();
                now.0 < *hour || (now.0 == *hour && now.1 < *minute)
            }
            Self::Not { inner } => !inner.evaluate(vars),
            Self::And { conditions } => conditions.iter().all(|c| c.evaluate(vars)),
            Self::Or { conditions } => conditions.iter().any(|c| c.evaluate(vars)),
        }
    }
}

/// Conditions for WaitFor steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WaitCondition {
    /// Wait until a specific time. Format: seconds from now.
    Duration { seconds: u64 },
    /// Wait until a cron-like time expression fires.
    Time { hour: u8, minute: u8 },
}

// ── Trigger Types ──

/// What starts a recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TriggerType {
    /// User manually runs it.
    Manual,
    /// Cron-like schedule.
    Cron { expression: String },
    /// Fired by an event (email:new, file:created, etc.)
    Event { event_type: String, filter: Option<serde_json::Value> },
    /// Fired when another recipe completes.
    RecipeComplete { recipe_id: String },
}

// ── Recipe Instance (runtime state) ──

/// Status of a recipe execution.
#[derive(Debug, Clone, PartialEq)]
pub enum RecipeStatus {
    /// Created but not started.
    Pending,
    /// Currently executing steps.
    Running,
    /// Paused waiting for a condition.
    Waiting,
    /// Successfully completed all steps.
    Done,
    /// Failed with an error.
    Failed,
}

impl RecipeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "waiting" => Self::Waiting,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// A recipe definition + runtime state.
#[derive(Debug, Clone)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: RecipeStatus,
    pub current_step: usize,
    pub created_at: f64,
    pub updated_at: f64,
    pub enabled: bool,
    pub error_message: Option<String>,
}

/// A loaded step from SQLite with its execution result.
#[derive(Debug, Clone)]
pub struct StoredStep {
    pub step_index: usize,
    pub step: RecipeStep,
    pub status: String,    // "pending", "done", "failed", "skipped"
    pub result: Option<String>,
}

// ── SQLite Persistence ──

pub struct RecipeStore;

impl RecipeStore {
    /// Create all recipe tables.
    pub fn ensure_tables(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS recipes (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                description TEXT DEFAULT '',
                status      TEXT NOT NULL DEFAULT 'pending',
                current_step INTEGER DEFAULT 0,
                enabled     INTEGER DEFAULT 1,
                error_msg   TEXT,
                created_at  REAL NOT NULL,
                updated_at  REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS recipe_steps (
                recipe_id   TEXT NOT NULL REFERENCES recipes(id),
                step_index  INTEGER NOT NULL,
                step_json   TEXT NOT NULL,
                status      TEXT DEFAULT 'pending',
                result      TEXT,
                PRIMARY KEY (recipe_id, step_index)
            );

            CREATE TABLE IF NOT EXISTS recipe_vars (
                recipe_id   TEXT NOT NULL REFERENCES recipes(id),
                key         TEXT NOT NULL,
                value       TEXT NOT NULL,
                PRIMARY KEY (recipe_id, key)
            );

            CREATE TABLE IF NOT EXISTS recipe_triggers (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                recipe_id   TEXT NOT NULL REFERENCES recipes(id),
                trigger_json TEXT NOT NULL,
                enabled     INTEGER DEFAULT 1,
                last_fired  REAL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_recipes_status ON recipes(status);
            CREATE INDEX IF NOT EXISTS idx_triggers_enabled ON recipe_triggers(enabled);",
        )
        .expect("failed to create recipe tables");
    }

    /// Create a new recipe with steps and optional trigger.
    pub fn create(
        conn: &Connection,
        name: &str,
        description: &str,
        steps: &[RecipeStep],
        trigger: Option<&TriggerType>,
    ) -> String {
        let id = format!("rcp_{}", &uuid7::uuid7().to_string()[..8]);
        let now = now_ts();

        conn.execute(
            "INSERT INTO recipes (id, name, description, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?4)",
            params![id, name, description, now],
        )
        .expect("insert recipe");

        for (i, step) in steps.iter().enumerate() {
            let step_json = serde_json::to_string(step).unwrap_or_default();
            conn.execute(
                "INSERT INTO recipe_steps (recipe_id, step_index, step_json)
                 VALUES (?1, ?2, ?3)",
                params![id, i as i64, step_json],
            )
            .expect("insert recipe step");
        }

        if let Some(trigger) = trigger {
            let trigger_json = serde_json::to_string(trigger).unwrap_or_default();
            conn.execute(
                "INSERT INTO recipe_triggers (recipe_id, trigger_json) VALUES (?1, ?2)",
                params![id, trigger_json],
            )
            .expect("insert recipe trigger");
        }

        tracing::info!(recipe_id = %id, name = %name, steps = steps.len(), "Recipe created");
        id
    }

    /// Register or update a built-in recipe with a fixed ID. Idempotent — safe to call on every boot.
    /// Updates step definitions if the recipe already exists (handles version upgrades).
    pub fn ensure_builtin(
        conn: &Connection,
        id: &str,
        name: &str,
        description: &str,
        steps: &[RecipeStep],
    ) {
        if Self::get(conn, id).is_some() {
            // Update steps in place (definition may change across versions)
            conn.execute("DELETE FROM recipe_steps WHERE recipe_id = ?1", params![id]).ok();
            for (i, step) in steps.iter().enumerate() {
                let step_json = serde_json::to_string(step).unwrap_or_default();
                conn.execute(
                    "INSERT INTO recipe_steps (recipe_id, step_index, step_json) VALUES (?1, ?2, ?3)",
                    params![id, i as i64, step_json],
                ).ok();
            }
            return;
        }
        let now = now_ts();
        conn.execute(
            "INSERT INTO recipes (id, name, description, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?4)",
            params![id, name, description, now],
        ).ok();
        for (i, step) in steps.iter().enumerate() {
            let step_json = serde_json::to_string(step).unwrap_or_default();
            conn.execute(
                "INSERT INTO recipe_steps (recipe_id, step_index, step_json) VALUES (?1, ?2, ?3)",
                params![id, i as i64, step_json],
            ).ok();
        }
        tracing::info!(recipe_id = %id, name = %name, steps = steps.len(), "Built-in recipe registered");
    }

    /// Get a recipe by ID.
    pub fn get(conn: &Connection, recipe_id: &str) -> Option<Recipe> {
        conn.query_row(
            "SELECT id, name, description, status, current_step, enabled, error_msg, created_at, updated_at
             FROM recipes WHERE id = ?1",
            params![recipe_id],
            |row| {
                let enabled_i: i32 = row.get(5)?;
                Ok(Recipe {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    status: RecipeStatus::from_str(&row.get::<_, String>(3)?),
                    current_step: row.get::<_, i64>(4)? as usize,
                    enabled: enabled_i != 0,
                    error_message: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        )
        .ok()
    }

    /// Get a recipe by name (case-insensitive).
    pub fn find_by_name(conn: &Connection, name: &str) -> Option<Recipe> {
        conn.query_row(
            "SELECT id, name, description, status, current_step, enabled, error_msg, created_at, updated_at
             FROM recipes WHERE LOWER(name) = LOWER(?1)",
            params![name],
            |row| {
                let enabled_i: i32 = row.get(5)?;
                Ok(Recipe {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    status: RecipeStatus::from_str(&row.get::<_, String>(3)?),
                    current_step: row.get::<_, i64>(4)? as usize,
                    enabled: enabled_i != 0,
                    error_message: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        )
        .ok()
    }

    /// Load all steps for a recipe.
    pub fn get_steps(conn: &Connection, recipe_id: &str) -> Vec<StoredStep> {
        let mut stmt = conn
            .prepare(
                "SELECT step_index, step_json, status, result
                 FROM recipe_steps WHERE recipe_id = ?1 ORDER BY step_index",
            )
            .expect("prepare get_steps");

        stmt.query_map(params![recipe_id], |row| {
            let step_json: String = row.get(1)?;
            let step: RecipeStep = serde_json::from_str(&step_json)
                .unwrap_or(RecipeStep::Notify { message: format!("PARSE ERROR: {}", step_json) });
            Ok(StoredStep {
                step_index: row.get::<_, i64>(0)? as usize,
                step,
                status: row.get(2)?,
                result: row.get(3)?,
            })
        })
        .expect("query get_steps")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Get recipe variables.
    pub fn get_vars(conn: &Connection, recipe_id: &str) -> std::collections::HashMap<String, serde_json::Value> {
        let mut stmt = conn
            .prepare("SELECT key, value FROM recipe_vars WHERE recipe_id = ?1")
            .expect("prepare get_vars");

        stmt.query_map(params![recipe_id], |row| {
            let key: String = row.get(0)?;
            let val_str: String = row.get(1)?;
            let val: serde_json::Value = serde_json::from_str(&val_str).unwrap_or(serde_json::Value::String(val_str));
            Ok((key, val))
        })
        .expect("query get_vars")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Set a recipe variable.
    pub fn set_var(conn: &Connection, recipe_id: &str, key: &str, value: &serde_json::Value) {
        let val_str = serde_json::to_string(value).unwrap_or_default();
        conn.execute(
            "INSERT OR REPLACE INTO recipe_vars (recipe_id, key, value) VALUES (?1, ?2, ?3)",
            params![recipe_id, key, val_str],
        )
        .ok();
    }

    /// Update recipe status and current step.
    pub fn update_status(conn: &Connection, recipe_id: &str, status: &RecipeStatus, current_step: usize) {
        let now = now_ts();
        conn.execute(
            "UPDATE recipes SET status = ?1, current_step = ?2, updated_at = ?3 WHERE id = ?4",
            params![status.as_str(), current_step as i64, now, recipe_id],
        )
        .ok();
    }

    /// Set error message on a recipe.
    pub fn set_error(conn: &Connection, recipe_id: &str, error: &str) {
        let now = now_ts();
        conn.execute(
            "UPDATE recipes SET status = 'failed', error_msg = ?1, updated_at = ?2 WHERE id = ?3",
            params![error, now, recipe_id],
        )
        .ok();
    }

    /// Mark a step as done with a result.
    pub fn complete_step(conn: &Connection, recipe_id: &str, step_index: usize, result: &str) {
        conn.execute(
            "UPDATE recipe_steps SET status = 'done', result = ?1
             WHERE recipe_id = ?2 AND step_index = ?3",
            params![result, recipe_id, step_index as i64],
        )
        .ok();
    }

    /// Mark a step as failed.
    pub fn fail_step(conn: &Connection, recipe_id: &str, step_index: usize, error: &str) {
        conn.execute(
            "UPDATE recipe_steps SET status = 'failed', result = ?1
             WHERE recipe_id = ?2 AND step_index = ?3",
            params![error, recipe_id, step_index as i64],
        )
        .ok();
    }

    /// Mark a step as skipped.
    pub fn skip_step(conn: &Connection, recipe_id: &str, step_index: usize) {
        conn.execute(
            "UPDATE recipe_steps SET status = 'skipped' WHERE recipe_id = ?1 AND step_index = ?2",
            params![recipe_id, step_index as i64],
        )
        .ok();
    }

    /// Replace remaining steps from `from_step` onwards with new steps (for replanning).
    /// Keeps completed steps intact, replaces pending/failed ones.
    pub fn replace_remaining_steps(conn: &Connection, recipe_id: &str, from_step: usize, new_steps: &[RecipeStep]) {
        // Delete old steps from from_step onwards
        conn.execute(
            "DELETE FROM recipe_steps WHERE recipe_id = ?1 AND step_index >= ?2",
            params![recipe_id, from_step as i64],
        )
        .ok();
        // Insert new steps
        for (i, step) in new_steps.iter().enumerate() {
            let step_json = serde_json::to_string(step).unwrap_or_default();
            conn.execute(
                "INSERT INTO recipe_steps (recipe_id, step_index, step_json) VALUES (?1, ?2, ?3)",
                params![recipe_id, (from_step + i) as i64, step_json],
            )
            .ok();
        }
        tracing::info!(
            recipe_id = %recipe_id,
            from_step,
            new_count = new_steps.len(),
            "Replaced remaining recipe steps (replan)"
        );
    }

    /// Record a recipe failure for learning. Stores the failure context so future
    /// recipe creation can avoid the same mistakes.
    pub fn record_failure_learning(conn: &Connection, recipe_id: &str, step_index: usize,
                                    tool_name: &str, error: &str, resolution: &str) {
        // Use recipe_vars to store learning data (avoid new table)
        let learning = serde_json::json!({
            "step": step_index,
            "tool": tool_name,
            "error": error,
            "resolution": resolution,
            "timestamp": now_ts(),
        });
        let key = format!("_learning_{}", step_index);
        Self::set_var(conn, recipe_id, &key, &learning);
    }

    /// List recipes with optional status filter.
    pub fn list(conn: &Connection, status_filter: Option<&str>, limit: usize) -> Vec<Recipe> {
        let (sql, p): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match status_filter {
            Some(s) => (
                format!(
                    "SELECT id, name, description, status, current_step, enabled, error_msg, created_at, updated_at
                     FROM recipes WHERE status = ?1 ORDER BY updated_at DESC LIMIT {limit}"
                ),
                vec![Box::new(s.to_string()) as Box<dyn rusqlite::types::ToSql>],
            ),
            None => (
                format!(
                    "SELECT id, name, description, status, current_step, enabled, error_msg, created_at, updated_at
                     FROM recipes ORDER BY updated_at DESC LIMIT {limit}"
                ),
                vec![],
            ),
        };

        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let refs: Vec<&dyn rusqlite::types::ToSql> = p.iter().map(|x| x.as_ref()).collect();

        stmt.query_map(refs.as_slice(), |row| {
            let enabled_i: i32 = row.get(5)?;
            Ok(Recipe {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                status: RecipeStatus::from_str(&row.get::<_, String>(3)?),
                current_step: row.get::<_, i64>(4)? as usize,
                enabled: enabled_i != 0,
                error_message: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Get all enabled triggers.
    pub fn get_enabled_triggers(conn: &Connection) -> Vec<(String, TriggerType, f64)> {
        let mut stmt = conn
            .prepare(
                "SELECT t.recipe_id, t.trigger_json, t.last_fired
                 FROM recipe_triggers t
                 JOIN recipes r ON r.id = t.recipe_id
                 WHERE t.enabled = 1 AND r.enabled = 1 AND r.status IN ('pending', 'done')",
            )
            .expect("prepare triggers");

        stmt.query_map([], |row| {
            let recipe_id: String = row.get(0)?;
            let trigger_json: String = row.get(1)?;
            let last_fired: f64 = row.get(2)?;
            let trigger: TriggerType = serde_json::from_str(&trigger_json)
                .unwrap_or(TriggerType::Manual);
            Ok((recipe_id, trigger, last_fired))
        })
        .expect("query triggers")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Record that a trigger fired.
    pub fn record_trigger_fired(conn: &Connection, recipe_id: &str) {
        let now = now_ts();
        conn.execute(
            "UPDATE recipe_triggers SET last_fired = ?1 WHERE recipe_id = ?2",
            params![now, recipe_id],
        )
        .ok();
    }

    /// Count running/waiting recipes.
    pub fn active_count(conn: &Connection) -> usize {
        conn.query_row(
            "SELECT COUNT(*) FROM recipes WHERE status IN ('running', 'waiting', 'pending')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    /// Get recipes that need processing (pending or running).
    pub fn get_resumable(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT id FROM recipes WHERE status IN ('pending', 'running')")
            .expect("prepare resumable");

        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query resumable")
            .filter_map(|r| r.ok())
            .collect()
    }

    /// Get waiting recipes whose WaitFor timeout has expired.
    /// Returns recipe IDs that should be resumed.
    pub fn get_expired_waiting(conn: &Connection) -> Vec<String> {
        let now = now_ts();
        // Find waiting recipes
        let mut stmt = conn
            .prepare("SELECT id, current_step, updated_at FROM recipes WHERE status = 'waiting'")
            .expect("prepare expired waiting");

        let rows: Vec<(String, usize, f64)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as usize,
                    row.get::<_, f64>(2)?,
                ))
            })
            .expect("query expired waiting")
            .filter_map(|r| r.ok())
            .collect();

        let mut expired = Vec::new();
        for (id, current_step, updated_at) in rows {
            // The WaitFor step is the one BEFORE current_step (it was completed and current_step advanced)
            let wait_step_idx = current_step.saturating_sub(1);
            let steps = Self::get_steps(conn, &id);
            if let Some(stored) = steps.get(wait_step_idx) {
                match &stored.step {
                    RecipeStep::WaitFor { condition, timeout_secs } => {
                        let should_resume = match condition {
                            WaitCondition::Duration { seconds } => {
                                (now - updated_at) >= *seconds as f64
                            }
                            WaitCondition::Time { hour, minute } => {
                                let (h, m) = chrono_now();
                                h > *hour || (h == *hour && m >= *minute)
                            }
                        };
                        // Also check global timeout if set
                        let timed_out = timeout_secs
                            .map(|t| (now - updated_at) >= t as f64)
                            .unwrap_or(false);
                        if should_resume || timed_out {
                            expired.push(id);
                        }
                    }
                    _ => {
                        // Stuck in waiting but not on a WaitFor step — resume it
                        expired.push(id);
                    }
                }
            }
        }
        expired
    }

    /// Collect failure learnings across all recipes for context injection.
    /// Returns a human-readable summary of past recipe failures and how they were resolved.
    pub fn get_failure_learnings(conn: &Connection, limit: usize) -> Vec<String> {
        // Query learning vars from all recipes (keys starting with _learning_)
        let mut stmt = conn
            .prepare(
                "SELECT rv.recipe_id, r.name, rv.value
                 FROM recipe_vars rv
                 JOIN recipes r ON r.id = rv.recipe_id
                 WHERE rv.key LIKE '_learning_%'
                 ORDER BY ROWID DESC
                 LIMIT ?1"
            )
            .unwrap_or_else(|_| conn.prepare("SELECT '', '', '' FROM recipe_vars LIMIT 0").unwrap());

        stmt.query_map(params![limit as i64], |row| {
            let recipe_name: String = row.get(1)?;
            let value_str: String = row.get(2)?;
            Ok((recipe_name, value_str))
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .map(|(name, val)| {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&val) {
                format!(
                    "Recipe '{}': tool '{}' failed with '{}'. Resolution: {}",
                    name,
                    v.get("tool").and_then(|t| t.as_str()).unwrap_or("?"),
                    v.get("error").and_then(|t| t.as_str()).unwrap_or("?"),
                    v.get("resolution").and_then(|t| t.as_str()).map(|s|
                        if s.len() > 200 { format!("{}...", &s[..s.floor_char_boundary(200)]) } else { s.to_string() }
                    ).unwrap_or_default(),
                )
            } else {
                format!("Recipe '{}': {}", name, val)
            }
        })
        .collect()
    }

    /// Format summary for system context injection.
    pub fn format_summary(conn: &Connection) -> String {
        let recipes = Self::list(conn, None, 10);
        let active: Vec<&Recipe> = recipes
            .iter()
            .filter(|r| r.status != RecipeStatus::Done && r.status != RecipeStatus::Failed)
            .collect();

        if active.is_empty() {
            return String::new();
        }

        let mut lines = vec!["Recipes:".to_string()];
        for r in active {
            let icon = match r.status {
                RecipeStatus::Running => "▶",
                RecipeStatus::Waiting => "⏸",
                RecipeStatus::Pending => "○",
                _ => "?",
            };
            lines.push(format!("  {} [{}] {} (step {}) — {}", icon, r.id, r.name, r.current_step, r.status.as_str()));
        }
        lines.join("\n")
    }
}

// ── Step Executor ──

/// Result of executing a single step.
pub enum StepResult {
    /// Step completed, advance to next step.
    Continue,
    /// Jump to a specific step index.
    JumpTo(usize),
    /// Recipe is waiting for a condition. Persist and pause.
    Waiting,
    /// Recipe completed (reached end or Notify was the last step).
    Done,
    /// Step failed with error message.
    Failed(String),
    /// Step produced a notification to deliver.
    Notify(String),
}

/// Resolve {{variable}} references in a string using recipe variables.
pub fn resolve_vars(template: &str, vars: &std::collections::HashMap<String, serde_json::Value>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        let replacement = match value {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        result = result.replace(&placeholder, &replacement);
    }
    result
}

/// Resolve {{variable}} references in JSON args.
pub fn resolve_vars_in_json(
    args: &serde_json::Value,
    vars: &std::collections::HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    match args {
        serde_json::Value::String(s) => {
            // Check if it's a pure variable reference like "{{emails}}"
            let trimmed = s.trim();
            if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
                let var_name = &trimmed[2..trimmed.len() - 2];
                if let Some(val) = vars.get(var_name) {
                    return val.clone();
                }
            }
            serde_json::Value::String(resolve_vars(s, vars))
        }
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(k.clone(), resolve_vars_in_json(v, vars));
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| resolve_vars_in_json(v, vars)).collect())
        }
        other => other.clone(),
    }
}

// ── Helpers ──

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Get current hour and minute (local time).
fn chrono_now() -> (u8, u8) {
    let secs = now_ts() as i64;
    // Simple UTC-based time (good enough for single-user OS)
    let hour = ((secs % 86400) / 3600) as u8;
    let minute = ((secs % 3600) / 60) as u8;
    (hour, minute)
}
