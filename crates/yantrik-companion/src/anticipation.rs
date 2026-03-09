//! Anticipatory Preparation — quiet preemption before user asks.
//!
//! The companion notices likely future needs and stages them in advance:
//! - Upcoming meeting → prefetch notes, participant summaries, relevant docs
//! - Travel email detected → suggest travel workspace
//! - Low battery before commute → queue offline content
//! - Repeated morning routine → open the right apps
//! - Deadline approaching → surface exact files and unresolved commitments
//!
//! Implementation: event-driven preparation tasks with low-priority background
//! execution. Results cached, surfaced only when contextually relevant.

use std::collections::HashMap;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Preparation Tasks ───────────────────────────────────────────────────────

/// A prepared item — something the companion staged in advance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedItem {
    pub id: String,
    /// What triggered this preparation.
    pub trigger: PreparationTrigger,
    /// What was prepared.
    pub kind: PreparedKind,
    /// Human-readable description.
    pub description: String,
    /// Cached data (JSON — could be summaries, file paths, etc.).
    pub data: serde_json::Value,
    /// When this preparation becomes relevant.
    pub relevant_at: f64,
    /// When this expires (no longer useful).
    pub expires_at: f64,
    /// Was this surfaced to the user?
    pub surfaced: bool,
    /// Was this useful? (tracked for learning)
    pub outcome: Option<PrepOutcome>,
    pub created_at: f64,
}

/// What triggered the preparation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreparationTrigger {
    /// Calendar event approaching.
    CalendarEvent { event_id: String, title: String },
    /// Commitment deadline approaching.
    CommitmentDeadline { commitment_id: String },
    /// Routine pattern detected (user usually does X at this time).
    RoutinePattern { routine_id: String },
    /// System condition (low battery, going offline).
    SystemCondition { condition: String },
    /// Travel detected (from email, calendar).
    TravelDetected,
    /// Manual request.
    UserRequested,
}

impl PreparationTrigger {
    pub fn type_tag(&self) -> &'static str {
        match self {
            Self::CalendarEvent { .. } => "calendar",
            Self::CommitmentDeadline { .. } => "commitment",
            Self::RoutinePattern { .. } => "routine",
            Self::SystemCondition { .. } => "system",
            Self::TravelDetected => "travel",
            Self::UserRequested => "user",
        }
    }
}

/// What was prepared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreparedKind {
    /// Participant summaries for a meeting.
    MeetingBrief,
    /// Files staged for a task.
    StagedFiles,
    /// Draft reply to an email/message.
    DraftReply,
    /// Commitment status summary.
    CommitmentSummary,
    /// Offline content bundle.
    OfflineBundle,
    /// Workspace suggestion.
    WorkspaceSuggestion,
    /// General context/summary.
    ContextSummary,
}

impl PreparedKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MeetingBrief => "meeting_brief",
            Self::StagedFiles => "staged_files",
            Self::DraftReply => "draft_reply",
            Self::CommitmentSummary => "commitment_summary",
            Self::OfflineBundle => "offline_bundle",
            Self::WorkspaceSuggestion => "workspace_suggestion",
            Self::ContextSummary => "context_summary",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "meeting_brief" => Self::MeetingBrief,
            "staged_files" => Self::StagedFiles,
            "draft_reply" => Self::DraftReply,
            "commitment_summary" => Self::CommitmentSummary,
            "offline_bundle" => Self::OfflineBundle,
            "workspace_suggestion" => Self::WorkspaceSuggestion,
            _ => Self::ContextSummary,
        }
    }
}

/// Outcome of a prepared item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrepOutcome {
    /// User engaged with it (opened, used, followed).
    Useful,
    /// User saw it but didn't engage.
    Ignored,
    /// Expired before user saw it.
    Expired,
    /// User dismissed it.
    Dismissed,
}

impl PrepOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Useful => "useful",
            Self::Ignored => "ignored",
            Self::Expired => "expired",
            Self::Dismissed => "dismissed",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "useful" => Self::Useful,
            "ignored" => Self::Ignored,
            "expired" => Self::Expired,
            "dismissed" => Self::Dismissed,
            _ => Self::Ignored,
        }
    }
}

// ── Anticipation Engine ─────────────────────────────────────────────────────

pub struct AnticipationEngine;

impl AnticipationEngine {
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS prepared_items (
                id          TEXT PRIMARY KEY,
                trigger_type TEXT NOT NULL,
                trigger_data TEXT NOT NULL DEFAULT '{}',
                kind        TEXT NOT NULL,
                description TEXT NOT NULL,
                data        TEXT NOT NULL DEFAULT '{}',
                relevant_at REAL NOT NULL,
                expires_at  REAL NOT NULL,
                surfaced    INTEGER NOT NULL DEFAULT 0,
                outcome     TEXT,
                created_at  REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_prep_relevant ON prepared_items(relevant_at);
            CREATE INDEX IF NOT EXISTS idx_prep_expires ON prepared_items(expires_at);
            CREATE INDEX IF NOT EXISTS idx_prep_surfaced ON prepared_items(surfaced);",
        )
        .expect("failed to create prepared_items table");
    }

    /// Store a prepared item.
    pub fn store(conn: &Connection, item: &PreparedItem) {
        let trigger_data = serde_json::to_string(&item.trigger).unwrap_or_default();
        let data = serde_json::to_string(&item.data).unwrap_or_default();

        let _ = conn.execute(
            "INSERT OR REPLACE INTO prepared_items
             (id, trigger_type, trigger_data, kind, description, data, relevant_at, expires_at, surfaced, outcome, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                item.id, item.trigger.type_tag(), trigger_data,
                item.kind.as_str(), item.description,
                data, item.relevant_at, item.expires_at,
                item.surfaced as i32,
                item.outcome.as_ref().map(|o| o.as_str()),
                item.created_at,
            ],
        );
    }

    /// Get items that are currently relevant (relevant_at <= now < expires_at, not surfaced).
    pub fn get_relevant(conn: &Connection) -> Vec<PreparedItem> {
        let now = now_ts();
        let mut stmt = match conn.prepare(
            "SELECT id, trigger_type, trigger_data, kind, description, data,
                    relevant_at, expires_at, surfaced, outcome, created_at
             FROM prepared_items
             WHERE relevant_at <= ?1 AND expires_at > ?1 AND surfaced = 0
             ORDER BY relevant_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![now], Self::row_to_item)
            .ok()
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Mark an item as surfaced.
    pub fn mark_surfaced(conn: &Connection, id: &str) {
        let _ = conn.execute(
            "UPDATE prepared_items SET surfaced = 1 WHERE id = ?1",
            params![id],
        );
    }

    /// Record the outcome of a prepared item.
    pub fn record_outcome(conn: &Connection, id: &str, outcome: PrepOutcome) {
        let _ = conn.execute(
            "UPDATE prepared_items SET outcome = ?1 WHERE id = ?2",
            params![outcome.as_str(), id],
        );
    }

    /// Clean up expired items.
    pub fn cleanup_expired(conn: &Connection) -> u64 {
        let now = now_ts();

        // Mark unsurfaced expired items
        let _ = conn.execute(
            "UPDATE prepared_items SET outcome = 'expired'
             WHERE expires_at < ?1 AND surfaced = 0 AND outcome IS NULL",
            params![now],
        );

        // Delete very old items (30 days)
        let old = now - 30.0 * 86400.0;
        conn.execute(
            "DELETE FROM prepared_items WHERE created_at < ?1",
            params![old],
        ).unwrap_or(0) as u64
    }

    /// Get anticipation success rate (for learning).
    pub fn success_rate(conn: &Connection, since_days: f64) -> f64 {
        let since = now_ts() - since_days * 86400.0;

        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM prepared_items WHERE created_at >= ?1 AND outcome IS NOT NULL",
            params![since], |r| r.get(0),
        ).unwrap_or(0);

        let useful: i64 = conn.query_row(
            "SELECT COUNT(*) FROM prepared_items WHERE created_at >= ?1 AND outcome = 'useful'",
            params![since], |r| r.get(0),
        ).unwrap_or(0);

        if total > 0 { useful as f64 / total as f64 } else { 0.0 }
    }

    /// Get stats by trigger type.
    pub fn stats_by_trigger(conn: &Connection) -> HashMap<String, (u64, u64)> {
        let mut stats = HashMap::new();

        let mut stmt = match conn.prepare(
            "SELECT trigger_type,
                    COUNT(*) as total,
                    SUM(CASE WHEN outcome = 'useful' THEN 1 ELSE 0 END) as useful
             FROM prepared_items WHERE outcome IS NOT NULL
             GROUP BY trigger_type",
        ) {
            Ok(s) => s,
            Err(_) => return stats,
        };

        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
            ))
        }) {
            for row in rows.flatten() {
                stats.insert(row.0, (row.1, row.2));
            }
        }

        stats
    }

    fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<PreparedItem> {
        let trigger_data: String = row.get(2)?;
        let data: String = row.get(5)?;
        let outcome: Option<String> = row.get(9)?;

        Ok(PreparedItem {
            id: row.get(0)?,
            trigger: serde_json::from_str(&trigger_data).unwrap_or(PreparationTrigger::UserRequested),
            kind: PreparedKind::from_str(&row.get::<_, String>(3)?),
            description: row.get(4)?,
            data: serde_json::from_str(&data).unwrap_or(serde_json::json!({})),
            relevant_at: row.get(6)?,
            expires_at: row.get(7)?,
            surfaced: row.get::<_, i32>(8)? != 0,
            outcome: outcome.map(|o| PrepOutcome::from_str(&o)),
            created_at: row.get(10)?,
        })
    }
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
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        AnticipationEngine::ensure_table(&conn);
        conn
    }

    #[test]
    fn store_and_retrieve() {
        let conn = setup();
        let now = now_ts();

        let item = PreparedItem {
            id: "p1".into(),
            trigger: PreparationTrigger::CalendarEvent {
                event_id: "evt1".into(),
                title: "Standup".into(),
            },
            kind: PreparedKind::MeetingBrief,
            description: "Brief for standup meeting".into(),
            data: serde_json::json!({"participants": ["Alice", "Bob"]}),
            relevant_at: now - 60.0, // Already relevant
            expires_at: now + 3600.0,
            surfaced: false,
            outcome: None,
            created_at: now - 120.0,
        };

        AnticipationEngine::store(&conn, &item);

        let relevant = AnticipationEngine::get_relevant(&conn);
        assert_eq!(relevant.len(), 1);
        assert_eq!(relevant[0].id, "p1");
        assert_eq!(relevant[0].kind, PreparedKind::MeetingBrief);
    }

    #[test]
    fn expired_items_not_returned() {
        let conn = setup();
        let now = now_ts();

        let item = PreparedItem {
            id: "p_old".into(),
            trigger: PreparationTrigger::UserRequested,
            kind: PreparedKind::ContextSummary,
            description: "Old prep".into(),
            data: serde_json::json!({}),
            relevant_at: now - 7200.0,
            expires_at: now - 3600.0, // Already expired
            surfaced: false,
            outcome: None,
            created_at: now - 7200.0,
        };

        AnticipationEngine::store(&conn, &item);

        let relevant = AnticipationEngine::get_relevant(&conn);
        assert!(relevant.is_empty());
    }

    #[test]
    fn surfaced_items_not_returned() {
        let conn = setup();
        let now = now_ts();

        let item = PreparedItem {
            id: "p2".into(),
            trigger: PreparationTrigger::UserRequested,
            kind: PreparedKind::ContextSummary,
            description: "Already shown".into(),
            data: serde_json::json!({}),
            relevant_at: now - 60.0,
            expires_at: now + 3600.0,
            surfaced: false,
            outcome: None,
            created_at: now,
        };

        AnticipationEngine::store(&conn, &item);
        AnticipationEngine::mark_surfaced(&conn, "p2");

        let relevant = AnticipationEngine::get_relevant(&conn);
        assert!(relevant.is_empty());
    }

    #[test]
    fn outcome_tracking() {
        let conn = setup();
        let now = now_ts();

        for (id, outcome) in &[("p1", "useful"), ("p2", "useful"), ("p3", "ignored")] {
            let item = PreparedItem {
                id: id.to_string(),
                trigger: PreparationTrigger::UserRequested,
                kind: PreparedKind::ContextSummary,
                description: "test".into(),
                data: serde_json::json!({}),
                relevant_at: now, expires_at: now + 100.0,
                surfaced: true, outcome: None, created_at: now,
            };
            AnticipationEngine::store(&conn, &item);
            AnticipationEngine::record_outcome(&conn, id, PrepOutcome::from_str(outcome));
        }

        let rate = AnticipationEngine::success_rate(&conn, 1.0);
        assert!((rate - 0.667).abs() < 0.01, "Expected ~0.667, got {rate}");
    }

    #[test]
    fn trigger_stats() {
        let conn = setup();
        let now = now_ts();

        for i in 0..5 {
            let item = PreparedItem {
                id: format!("cal_{i}"),
                trigger: PreparationTrigger::CalendarEvent {
                    event_id: format!("e{i}"), title: "meeting".into(),
                },
                kind: PreparedKind::MeetingBrief,
                description: "test".into(),
                data: serde_json::json!({}),
                relevant_at: now, expires_at: now + 100.0,
                surfaced: true, outcome: None, created_at: now,
            };
            AnticipationEngine::store(&conn, &item);
            let outcome = if i < 3 { PrepOutcome::Useful } else { PrepOutcome::Ignored };
            AnticipationEngine::record_outcome(&conn, &format!("cal_{i}"), outcome);
        }

        let stats = AnticipationEngine::stats_by_trigger(&conn);
        let (total, useful) = stats.get("calendar").unwrap();
        assert_eq!(*total, 5);
        assert_eq!(*useful, 3);
    }
}
