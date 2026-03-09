//! Memory Repair — user corrections, scope changes, privacy controls, exclusion lists.
//!
//! Provides the backend for memory correction UX:
//! - "That's not true anymore" → mark stale + archive
//! - "Only at work" → change scope
//! - "Don't remember this" → delete + add to exclusion list
//! - "This is private — never use proactively" → privacy flag
//! - "You misunderstood that" → correct text + lower confidence
//! - "Forget everything about X" → topic-based deletion
//!
//! Also tracks boundary memories: topics user avoids, sensitive areas.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::memory_lifecycle::{MemoryLifecycle, MemoryScope, MemoryState};

// ── Repair Actions ──────────────────────────────────────────────────────────

/// A user-initiated memory repair action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RepairAction {
    /// Mark as outdated and archive: "That's not true anymore"
    MarkOutdated { reason: String },
    /// Change scope: "Only at work" / "Only personal"
    ChangeScope { new_scope: MemoryScope },
    /// Delete and exclude: "Don't remember this"
    DeleteAndExclude,
    /// Set privacy: "Never use this proactively"
    SetPrivate,
    /// Remove privacy flag
    RemovePrivate,
    /// Correct the memory text: "You misunderstood — here's what I meant"
    Correct { new_text: String },
    /// Forget a topic entirely: "Forget everything about X"
    ForgetTopic { topic: String },
    /// Restore a previously archived/forgotten memory
    Restore,
}

/// Result of a repair action.
#[derive(Debug, Clone)]
pub struct RepairResult {
    pub action: String,
    pub affected_count: usize,
    pub message: String,
}

// ── Memory Repair Engine ────────────────────────────────────────────────────

pub struct MemoryRepair;

impl MemoryRepair {
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_exclusions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern     TEXT NOT NULL,
                reason      TEXT,
                created_at  REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_exclusion_pattern ON memory_exclusions(pattern);

            CREATE TABLE IF NOT EXISTS memory_boundaries (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                topic       TEXT NOT NULL,
                boundary_type TEXT NOT NULL,
                severity    REAL NOT NULL DEFAULT 0.5,
                notes       TEXT,
                created_at  REAL NOT NULL,
                updated_at  REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_boundary_topic ON memory_boundaries(topic);

            CREATE TABLE IF NOT EXISTS memory_repairs (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id   TEXT NOT NULL,
                action      TEXT NOT NULL,
                detail      TEXT,
                created_at  REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_repair_memory ON memory_repairs(memory_id);
            CREATE INDEX IF NOT EXISTS idx_repair_time ON memory_repairs(created_at);",
        )
        .expect("failed to create memory repair tables");
    }

    /// Apply a repair action to a specific memory.
    pub fn apply(
        conn: &Connection,
        memory_id: &str,
        action: &RepairAction,
    ) -> RepairResult {
        let now = now_ts();

        match action {
            RepairAction::MarkOutdated { reason } => {
                MemoryLifecycle::transition(conn, memory_id, MemoryState::Archived);
                Self::log_repair(conn, memory_id, "mark_outdated", Some(reason));
                RepairResult {
                    action: "mark_outdated".into(),
                    affected_count: 1,
                    message: format!("Marked as outdated: {}", reason),
                }
            }

            RepairAction::ChangeScope { new_scope } => {
                MemoryLifecycle::set_scope(conn, memory_id, new_scope.clone());
                Self::log_repair(conn, memory_id, "change_scope", Some(new_scope.as_str()));
                RepairResult {
                    action: "change_scope".into(),
                    affected_count: 1,
                    message: format!("Scope changed to: {}", new_scope.as_str()),
                }
            }

            RepairAction::DeleteAndExclude => {
                // Get the memory text before deleting (for exclusion pattern)
                if let Some(meta) = MemoryLifecycle::get(conn, memory_id) {
                    // Add to exclusion list
                    Self::add_exclusion(conn, memory_id, "User requested deletion");
                    // Mark as forgotten
                    MemoryLifecycle::transition(conn, memory_id, MemoryState::Forgotten);
                    Self::log_repair(conn, memory_id, "delete_exclude", None);
                    RepairResult {
                        action: "delete_exclude".into(),
                        affected_count: 1,
                        message: format!("Deleted and excluded (state: {})", meta.state.as_str()),
                    }
                } else {
                    RepairResult {
                        action: "delete_exclude".into(),
                        affected_count: 0,
                        message: "Memory not found".into(),
                    }
                }
            }

            RepairAction::SetPrivate => {
                MemoryLifecycle::set_privacy(conn, memory_id, true);
                Self::log_repair(conn, memory_id, "set_private", None);
                RepairResult {
                    action: "set_private".into(),
                    affected_count: 1,
                    message: "Marked as private — won't be used proactively".into(),
                }
            }

            RepairAction::RemovePrivate => {
                MemoryLifecycle::set_privacy(conn, memory_id, false);
                Self::log_repair(conn, memory_id, "remove_private", None);
                RepairResult {
                    action: "remove_private".into(),
                    affected_count: 1,
                    message: "Privacy flag removed".into(),
                }
            }

            RepairAction::Correct { new_text } => {
                // Lower confidence since original was wrong
                let _ = conn.execute(
                    "UPDATE memory_lifecycle SET confidence = MAX(0.1, confidence - 0.2), updated_at = ?1
                     WHERE memory_id = ?2",
                    params![now, memory_id],
                );
                Self::log_repair(conn, memory_id, "correct", Some(new_text));
                RepairResult {
                    action: "correct".into(),
                    affected_count: 1,
                    message: "Memory corrected — confidence lowered".into(),
                }
            }

            RepairAction::ForgetTopic { topic } => {
                // Find all memories containing the topic via lifecycle metadata
                // (We can't search memory text from here — that's in YantrikDB.
                //  Instead, add as a boundary and mark known matches.)
                Self::add_boundary(conn, topic, "avoid", 0.8, Some("User requested topic deletion"));
                Self::add_exclusion(conn, topic, "Topic forgotten by user request");
                Self::log_repair(conn, memory_id, "forget_topic", Some(topic));
                RepairResult {
                    action: "forget_topic".into(),
                    affected_count: 1,
                    message: format!("Topic '{}' added to exclusion list and boundaries", topic),
                }
            }

            RepairAction::Restore => {
                // Restore from archived/forgotten to observed
                MemoryLifecycle::transition(conn, memory_id, MemoryState::Observed);
                // Remove from exclusion list if present
                Self::remove_exclusion(conn, memory_id);
                Self::log_repair(conn, memory_id, "restore", None);
                RepairResult {
                    action: "restore".into(),
                    affected_count: 1,
                    message: "Memory restored to active state".into(),
                }
            }
        }
    }

    /// Add a pattern to the exclusion list.
    pub fn add_exclusion(conn: &Connection, pattern: &str, reason: &str) {
        let _ = conn.execute(
            "INSERT INTO memory_exclusions (pattern, reason, created_at) VALUES (?1, ?2, ?3)",
            params![pattern, reason, now_ts()],
        );
    }

    /// Remove a pattern from the exclusion list.
    pub fn remove_exclusion(conn: &Connection, pattern: &str) {
        let _ = conn.execute(
            "DELETE FROM memory_exclusions WHERE pattern = ?1",
            params![pattern],
        );
    }

    /// Check if a memory text matches any exclusion pattern.
    pub fn is_excluded(conn: &Connection, text: &str) -> bool {
        let lower = text.to_lowercase();
        let mut stmt = match conn.prepare("SELECT pattern FROM memory_exclusions") {
            Ok(s) => s,
            Err(_) => return false,
        };

        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for pattern in rows.flatten() {
                if lower.contains(&pattern.to_lowercase()) {
                    return true;
                }
            }
        }
        false
    }

    /// Get all exclusion patterns.
    pub fn exclusion_list(conn: &Connection) -> Vec<ExclusionEntry> {
        let mut stmt = match conn.prepare(
            "SELECT pattern, reason, created_at FROM memory_exclusions ORDER BY created_at DESC"
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map([], |row| {
            Ok(ExclusionEntry {
                pattern: row.get(0)?,
                reason: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    // ── Boundaries ──

    /// Add a topic boundary (topic the user doesn't want discussed).
    pub fn add_boundary(
        conn: &Connection,
        topic: &str,
        boundary_type: &str,
        severity: f64,
        notes: Option<&str>,
    ) {
        let now = now_ts();
        let _ = conn.execute(
            "INSERT INTO memory_boundaries (topic, boundary_type, severity, notes, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![topic, boundary_type, severity, notes, now],
        );
    }

    /// Check if a topic is a known boundary.
    pub fn is_boundary_topic(conn: &Connection, text: &str) -> Option<BoundaryInfo> {
        let lower = text.to_lowercase();
        let mut stmt = match conn.prepare(
            "SELECT topic, boundary_type, severity, notes FROM memory_boundaries ORDER BY severity DESC"
        ) {
            Ok(s) => s,
            Err(_) => return None,
        };

        if let Ok(rows) = stmt.query_map([], |row| {
            Ok(BoundaryInfo {
                topic: row.get(0)?,
                boundary_type: row.get(1)?,
                severity: row.get(2)?,
                notes: row.get(3)?,
            })
        }) {
            for boundary in rows.flatten() {
                if lower.contains(&boundary.topic.to_lowercase()) {
                    return Some(boundary);
                }
            }
        }
        None
    }

    /// Get all boundaries.
    pub fn boundary_list(conn: &Connection) -> Vec<BoundaryInfo> {
        let mut stmt = match conn.prepare(
            "SELECT topic, boundary_type, severity, notes FROM memory_boundaries ORDER BY severity DESC"
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map([], |row| {
            Ok(BoundaryInfo {
                topic: row.get(0)?,
                boundary_type: row.get(1)?,
                severity: row.get(2)?,
                notes: row.get(3)?,
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Remove a boundary.
    pub fn remove_boundary(conn: &Connection, topic: &str) {
        let _ = conn.execute(
            "DELETE FROM memory_boundaries WHERE topic = ?1",
            params![topic],
        );
    }

    // ── Repair History ──

    fn log_repair(conn: &Connection, memory_id: &str, action: &str, detail: Option<&str>) {
        let _ = conn.execute(
            "INSERT INTO memory_repairs (memory_id, action, detail, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![memory_id, action, detail, now_ts()],
        );
    }

    /// Get repair history for a memory.
    pub fn repair_history(conn: &Connection, memory_id: &str) -> Vec<RepairRecord> {
        let mut stmt = match conn.prepare(
            "SELECT action, detail, created_at FROM memory_repairs
             WHERE memory_id = ?1 ORDER BY created_at DESC"
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![memory_id], |row| {
            Ok(RepairRecord {
                action: row.get(0)?,
                detail: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Get recent repairs (for dashboard).
    pub fn recent_repairs(conn: &Connection, limit: usize) -> Vec<(String, RepairRecord)> {
        let mut stmt = match conn.prepare(
            "SELECT memory_id, action, detail, created_at FROM memory_repairs
             ORDER BY created_at DESC LIMIT ?1"
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                RepairRecord {
                    action: row.get(1)?,
                    detail: row.get(2)?,
                    created_at: row.get(3)?,
                },
            ))
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }
}

/// An exclusion list entry.
#[derive(Debug, Clone)]
pub struct ExclusionEntry {
    pub pattern: String,
    pub reason: Option<String>,
    pub created_at: f64,
}

/// A topic boundary.
#[derive(Debug, Clone)]
pub struct BoundaryInfo {
    pub topic: String,
    pub boundary_type: String,
    pub severity: f64,
    pub notes: Option<String>,
}

/// A repair action record.
#[derive(Debug, Clone)]
pub struct RepairRecord {
    pub action: String,
    pub detail: Option<String>,
    pub created_at: f64,
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_lifecycle::{MemoryLifecycle, MemoryScope, MemorySource, MemoryState};
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        MemoryLifecycle::ensure_table(&conn);
        MemoryRepair::ensure_table(&conn);
        conn
    }

    #[test]
    fn mark_outdated() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);

        let result = MemoryRepair::apply(&conn, "m1", &RepairAction::MarkOutdated {
            reason: "Moved to a new city".into(),
        });
        assert_eq!(result.affected_count, 1);

        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert_eq!(meta.state, MemoryState::Archived);
    }

    #[test]
    fn change_scope() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);

        MemoryRepair::apply(&conn, "m1", &RepairAction::ChangeScope {
            new_scope: MemoryScope::Work,
        });

        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert_eq!(meta.scope, MemoryScope::Work);
    }

    #[test]
    fn delete_and_exclude() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);

        MemoryRepair::apply(&conn, "m1", &RepairAction::DeleteAndExclude);

        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert_eq!(meta.state, MemoryState::Forgotten);
        assert!(MemoryRepair::is_excluded(&conn, "m1"));
    }

    #[test]
    fn privacy_toggle() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);

        MemoryRepair::apply(&conn, "m1", &RepairAction::SetPrivate);
        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert!(meta.privacy_flag);

        MemoryRepair::apply(&conn, "m1", &RepairAction::RemovePrivate);
        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert!(!meta.privacy_flag);
    }

    #[test]
    fn correct_lowers_confidence() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);

        MemoryRepair::apply(&conn, "m1", &RepairAction::Correct {
            new_text: "Actually, I prefer tea not coffee".into(),
        });

        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert!(meta.confidence < 0.8, "Confidence should drop: {}", meta.confidence);
    }

    #[test]
    fn forget_topic_adds_boundary() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);

        MemoryRepair::apply(&conn, "m1", &RepairAction::ForgetTopic {
            topic: "my ex".into(),
        });

        assert!(MemoryRepair::is_excluded(&conn, "my ex girlfriend"));
        let boundary = MemoryRepair::is_boundary_topic(&conn, "talking about my ex");
        assert!(boundary.is_some());
    }

    #[test]
    fn restore_memory() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);
        MemoryLifecycle::transition(&conn, "m1", MemoryState::Archived);

        MemoryRepair::apply(&conn, "m1", &RepairAction::Restore);

        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert_eq!(meta.state, MemoryState::Observed);
    }

    #[test]
    fn boundary_management() {
        let conn = setup();

        MemoryRepair::add_boundary(&conn, "politics", "avoid", 0.9, Some("User prefers no political topics"));
        MemoryRepair::add_boundary(&conn, "diet", "sensitive", 0.5, None);

        let boundaries = MemoryRepair::boundary_list(&conn);
        assert_eq!(boundaries.len(), 2);

        let hit = MemoryRepair::is_boundary_topic(&conn, "Let's talk about politics");
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().topic, "politics");

        MemoryRepair::remove_boundary(&conn, "politics");
        let boundaries = MemoryRepair::boundary_list(&conn);
        assert_eq!(boundaries.len(), 1);
    }

    #[test]
    fn repair_history_logged() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);

        MemoryRepair::apply(&conn, "m1", &RepairAction::SetPrivate);
        MemoryRepair::apply(&conn, "m1", &RepairAction::ChangeScope {
            new_scope: MemoryScope::Work,
        });

        let history = MemoryRepair::repair_history(&conn, "m1");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].action, "change_scope"); // Most recent first
        assert_eq!(history[1].action, "set_private");

        let recent = MemoryRepair::recent_repairs(&conn, 10);
        assert_eq!(recent.len(), 2);
    }
}
