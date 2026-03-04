//! Automation engine — persistent automation rules with triggers, conditions, and actions.
//!
//! Automations can be:
//! - **Manual**: user-triggered workflows ("run my deploy workflow")
//! - **Schedule-triggered**: linked to a scheduled_task ("every Monday, check weather")
//! - **Event-triggered**: fired by SystemEvents ("when WiFi connects, turn on lights")
//!
//! Actions are stored as natural language instructions. The LLM is the execution engine —
//! when an automation fires, its steps become urges that the LLM reads and acts on.

use rusqlite::{params, Connection};

/// A stored automation rule.
#[derive(Debug, Clone)]
pub struct Automation {
    pub automation_id: String,
    pub name: String,
    pub description: String,
    pub trigger_type: String,
    pub trigger_config: serde_json::Value,
    pub condition: Option<String>,
    pub steps: String,
    pub enabled: bool,
    pub run_count: i64,
    pub last_run: Option<f64>,
    pub created_at: f64,
    pub status: String,
}

/// Stateless automation storage — all methods take a `&Connection`.
pub struct AutomationStore;

impl AutomationStore {
    /// Create the automations table if it doesn't exist.
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS automations (
                automation_id   TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                description     TEXT DEFAULT '',
                trigger_type    TEXT NOT NULL,
                trigger_config  TEXT DEFAULT '{}',
                condition       TEXT,
                steps           TEXT NOT NULL,
                enabled         INTEGER DEFAULT 1,
                run_count       INTEGER DEFAULT 0,
                last_run        REAL,
                created_at      REAL NOT NULL,
                status          TEXT DEFAULT 'active'
            );
            CREATE INDEX IF NOT EXISTS idx_auto_trigger
                ON automations(trigger_type, status, enabled);",
        )
        .expect("failed to create automations table");
    }

    /// Create a new automation. Returns the automation_id.
    pub fn create(
        conn: &Connection,
        name: &str,
        description: &str,
        trigger_type: &str,
        trigger_config: &serde_json::Value,
        condition: Option<&str>,
        steps: &str,
    ) -> String {
        let id = uuid7::uuid7().to_string();
        let now = now_ts();
        let config_json = serde_json::to_string(trigger_config).unwrap_or_default();

        conn.execute(
            "INSERT INTO automations
             (automation_id, name, description, trigger_type, trigger_config,
              condition, steps, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, name, description, trigger_type, config_json, condition, steps, now],
        )
        .expect("failed to insert automation");

        tracing::info!(
            automation_id = %id,
            name = %name,
            trigger_type = %trigger_type,
            "Automation created"
        );

        id
    }

    /// Get a single automation by ID.
    pub fn get(conn: &Connection, automation_id: &str) -> Option<Automation> {
        conn.query_row(
            "SELECT automation_id, name, description, trigger_type, trigger_config,
             condition, steps, enabled, run_count, last_run, created_at, status
             FROM automations WHERE automation_id = ?1",
            params![automation_id],
            row_to_automation,
        )
        .ok()
    }

    /// Find an automation by name (case-insensitive).
    pub fn find_by_name(conn: &Connection, name: &str) -> Option<Automation> {
        conn.query_row(
            "SELECT automation_id, name, description, trigger_type, trigger_config,
             condition, steps, enabled, run_count, last_run, created_at, status
             FROM automations WHERE LOWER(name) = LOWER(?1) AND status = 'active'",
            params![name],
            row_to_automation,
        )
        .ok()
    }

    /// List automations, optionally filtered.
    pub fn list(
        conn: &Connection,
        trigger_type: Option<&str>,
        status: Option<&str>,
    ) -> Vec<Automation> {
        let status = status.unwrap_or("active");

        if let Some(tt) = trigger_type {
            let mut stmt = conn
                .prepare(
                    "SELECT automation_id, name, description, trigger_type, trigger_config,
                     condition, steps, enabled, run_count, last_run, created_at, status
                     FROM automations WHERE trigger_type = ?1 AND status = ?2
                     ORDER BY created_at DESC",
                )
                .expect("prepare list");
            stmt.query_map(params![tt, status], row_to_automation)
                .expect("query list")
                .filter_map(|r| r.ok())
                .collect()
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT automation_id, name, description, trigger_type, trigger_config,
                     condition, steps, enabled, run_count, last_run, created_at, status
                     FROM automations WHERE status = ?1
                     ORDER BY created_at DESC",
                )
                .expect("prepare list");
            stmt.query_map(params![status], row_to_automation)
                .expect("query list")
                .filter_map(|r| r.ok())
                .collect()
        }
    }

    /// Get all enabled event-triggered automations matching an event type.
    pub fn get_event_automations(conn: &Connection, event_type: &str) -> Vec<Automation> {
        let mut stmt = conn
            .prepare(
                "SELECT automation_id, name, description, trigger_type, trigger_config,
                 condition, steps, enabled, run_count, last_run, created_at, status
                 FROM automations
                 WHERE trigger_type = 'event' AND status = 'active' AND enabled = 1",
            )
            .expect("prepare event automations");

        stmt.query_map([], row_to_automation)
            .expect("query event automations")
            .filter_map(|r| r.ok())
            .filter(|a| {
                // Match event_type from trigger_config
                a.trigger_config
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    == Some(event_type)
            })
            .collect()
    }

    /// Check if an event matches an automation's trigger_config filters.
    pub fn event_matches(
        trigger_config: &serde_json::Value,
        event_data: &serde_json::Value,
    ) -> bool {
        let match_obj = match trigger_config.get("match") {
            Some(serde_json::Value::Object(m)) => m,
            _ => return true, // No match filter → always matches
        };

        for (key, expected) in match_obj {
            let actual = event_data.get(key);
            match actual {
                Some(v) if v == expected => continue,
                _ => return false,
            }
        }
        true
    }

    /// Record that an automation ran.
    pub fn record_run(conn: &Connection, automation_id: &str) {
        let now = now_ts();
        conn.execute(
            "UPDATE automations SET run_count = run_count + 1, last_run = ?1
             WHERE automation_id = ?2",
            params![now, automation_id],
        )
        .ok();
    }

    /// Toggle enabled state.
    pub fn set_enabled(conn: &Connection, automation_id: &str, enabled: bool) -> bool {
        let changes = conn
            .execute(
                "UPDATE automations SET enabled = ?1 WHERE automation_id = ?2 AND status = 'active'",
                params![enabled as i32, automation_id],
            )
            .unwrap_or(0);
        changes > 0
    }

    /// Archive (soft-delete) an automation.
    pub fn archive(conn: &Connection, automation_id: &str) -> bool {
        let changes = conn
            .execute(
                "UPDATE automations SET status = 'archived' WHERE automation_id = ?1",
                params![automation_id],
            )
            .unwrap_or(0);
        changes > 0
    }

    /// Format summary for system context.
    pub fn format_summary(conn: &Connection) -> String {
        let active = Self::list(conn, None, Some("active"));
        if active.is_empty() {
            return String::new();
        }

        let mut lines = vec!["Automations:".to_string()];
        for a in active.iter().take(10) {
            let trigger = match a.trigger_type.as_str() {
                "manual" => "manual".to_string(),
                "schedule" => "scheduled".to_string(),
                "event" => {
                    let et = a
                        .trigger_config
                        .get("event_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    format!("on {}", et)
                }
                _ => a.trigger_type.clone(),
            };
            let enabled = if a.enabled { "" } else { " [disabled]" };
            lines.push(format!(
                "  - {} ({}) runs: {}{} [{}]",
                a.name, trigger, a.run_count, enabled, a.automation_id
            ));
        }
        lines.join("\n")
    }
}

fn row_to_automation(row: &rusqlite::Row) -> rusqlite::Result<Automation> {
    let config_str: String = row.get(4)?;
    let condition: Option<String> = row.get(5)?;
    let enabled_int: i32 = row.get(7)?;

    Ok(Automation {
        automation_id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        trigger_type: row.get(3)?,
        trigger_config: serde_json::from_str(&config_str).unwrap_or_default(),
        condition,
        steps: row.get(6)?,
        enabled: enabled_int != 0,
        run_count: row.get(8)?,
        last_run: row.get(9)?,
        created_at: row.get(10)?,
        status: row.get(11)?,
    })
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
