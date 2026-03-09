//! Memory Lifecycle — states, contradiction detection, and consolidation windows.
//!
//! Adds lifecycle management to YantrikDB memories:
//!
//! **States**: observed → inferred → confirmed → reinforced → contradicted → stale → archived → forgotten
//!
//! **Contradiction Detection**: When new info conflicts with existing beliefs,
//! flag both and keep the contradiction set. Let the user or time resolve it.
//!
//! **Consolidation Windows**:
//! - Immediate: scratchpad (working memory, ephemeral)
//! - Short-term: recent observations (< 24h, higher recall priority)
//! - Long-term: confirmed/reinforced beliefs (nightly consolidation)
//!
//! **Scoping**: Each memory can have a scope (global, at_work, personal, travel)
//! to prevent context leakage across domains.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Memory States ───────────────────────────────────────────────────────────

/// Lifecycle state of a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryState {
    /// Just observed — raw data, not yet validated.
    Observed,
    /// Inferred from patterns (not directly stated by user).
    Inferred,
    /// Confirmed by user or repeated observation.
    Confirmed,
    /// Reinforced: seen multiple times, high confidence.
    Reinforced,
    /// Contradicted by newer information.
    Contradicted,
    /// Stale: hasn't been relevant in a long time.
    Stale,
    /// Archived: explicitly set aside by user or consolidation.
    Archived,
    /// Forgotten: soft-deleted, not recalled unless explicitly searched.
    Forgotten,
}

impl MemoryState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Inferred => "inferred",
            Self::Confirmed => "confirmed",
            Self::Reinforced => "reinforced",
            Self::Contradicted => "contradicted",
            Self::Stale => "stale",
            Self::Archived => "archived",
            Self::Forgotten => "forgotten",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "observed" => Self::Observed,
            "inferred" => Self::Inferred,
            "confirmed" => Self::Confirmed,
            "reinforced" => Self::Reinforced,
            "contradicted" => Self::Contradicted,
            "stale" => Self::Stale,
            "archived" => Self::Archived,
            "forgotten" => Self::Forgotten,
            _ => Self::Observed,
        }
    }

    /// Is this memory active (should be recalled)?
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Observed | Self::Inferred | Self::Confirmed | Self::Reinforced)
    }

    /// Can this memory be auto-consolidated?
    pub fn can_consolidate(&self) -> bool {
        matches!(self, Self::Observed | Self::Inferred)
    }

    /// Recall priority multiplier (higher = recalled first).
    pub fn recall_priority(&self) -> f64 {
        match self {
            Self::Reinforced => 1.2,
            Self::Confirmed => 1.0,
            Self::Observed => 0.8,
            Self::Inferred => 0.6,
            Self::Contradicted => 0.3, // Low but not zero — useful context
            Self::Stale => 0.2,
            Self::Archived => 0.1,
            Self::Forgotten => 0.0,
        }
    }
}

// ── Memory Scope ────────────────────────────────────────────────────────────

/// Scope of a memory — prevents context leakage across domains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryScope {
    /// Applies everywhere.
    Global,
    /// Only relevant at work.
    Work,
    /// Personal/home context.
    Personal,
    /// Travel-related.
    Travel,
    /// Custom scope.
    Custom(String),
}

impl MemoryScope {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Global => "global",
            Self::Work => "work",
            Self::Personal => "personal",
            Self::Travel => "travel",
            Self::Custom(s) => s,
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "global" => Self::Global,
            "work" => Self::Work,
            "personal" => Self::Personal,
            "travel" => Self::Travel,
            s => Self::Custom(s.to_string()),
        }
    }
}

// ── Lifecycle Metadata ──────────────────────────────────────────────────────

/// Extended lifecycle metadata for a memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleMetadata {
    pub memory_id: String,
    pub state: MemoryState,
    pub scope: MemoryScope,
    pub confidence: f64,
    pub reinforcement_count: u32,
    pub contradiction_ids: Vec<String>,
    pub source: MemorySource,
    pub last_recalled_at: Option<f64>,
    pub recall_count: u32,
    pub privacy_flag: bool,
    pub created_at: f64,
    pub updated_at: f64,
}

/// Where a memory came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemorySource {
    /// User directly stated it.
    UserStatement,
    /// Extracted from conversation context.
    ConversationInference,
    /// Extracted from email.
    Email,
    /// Extracted from calendar.
    Calendar,
    /// System observation (tool result, pattern detection).
    SystemObservation,
    /// Consolidation (merged from multiple observations).
    Consolidation,
}

impl MemorySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UserStatement => "user_statement",
            Self::ConversationInference => "conversation_inference",
            Self::Email => "email",
            Self::Calendar => "calendar",
            Self::SystemObservation => "system_observation",
            Self::Consolidation => "consolidation",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "user_statement" => Self::UserStatement,
            "conversation_inference" => Self::ConversationInference,
            "email" => Self::Email,
            "calendar" => Self::Calendar,
            "system_observation" => Self::SystemObservation,
            "consolidation" => Self::Consolidation,
            _ => Self::ConversationInference,
        }
    }
}

// ── Lifecycle Manager ───────────────────────────────────────────────────────

/// Manages memory lifecycle operations.
pub struct MemoryLifecycle;

impl MemoryLifecycle {
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_lifecycle (
                memory_id           TEXT PRIMARY KEY,
                state               TEXT NOT NULL DEFAULT 'observed',
                scope               TEXT NOT NULL DEFAULT 'global',
                confidence          REAL NOT NULL DEFAULT 0.5,
                reinforcement_count INTEGER NOT NULL DEFAULT 0,
                contradiction_ids   TEXT NOT NULL DEFAULT '[]',
                source              TEXT NOT NULL DEFAULT 'conversation_inference',
                last_recalled_at    REAL,
                recall_count        INTEGER NOT NULL DEFAULT 0,
                privacy_flag        INTEGER NOT NULL DEFAULT 0,
                created_at          REAL NOT NULL,
                updated_at          REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ml_state ON memory_lifecycle(state);
            CREATE INDEX IF NOT EXISTS idx_ml_scope ON memory_lifecycle(scope);
            CREATE INDEX IF NOT EXISTS idx_ml_confidence ON memory_lifecycle(confidence);
            CREATE INDEX IF NOT EXISTS idx_ml_recalled ON memory_lifecycle(last_recalled_at);

            CREATE TABLE IF NOT EXISTS memory_contradictions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_a_id TEXT NOT NULL,
                memory_b_id TEXT NOT NULL,
                reason      TEXT NOT NULL,
                resolved    INTEGER NOT NULL DEFAULT 0,
                resolved_by TEXT,
                created_at  REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_mc_a ON memory_contradictions(memory_a_id);
            CREATE INDEX IF NOT EXISTS idx_mc_b ON memory_contradictions(memory_b_id);",
        )
        .expect("failed to create memory_lifecycle tables");
    }

    /// Register lifecycle metadata for a new memory.
    pub fn register(
        conn: &Connection,
        memory_id: &str,
        source: MemorySource,
        scope: MemoryScope,
        confidence: f64,
    ) {
        let now = now_ts();
        let _ = conn.execute(
            "INSERT OR IGNORE INTO memory_lifecycle
             (memory_id, state, scope, confidence, source, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                memory_id,
                MemoryState::Observed.as_str(),
                scope.as_str(),
                confidence,
                source.as_str(),
                now,
            ],
        );
    }

    /// Get lifecycle metadata for a memory.
    pub fn get(conn: &Connection, memory_id: &str) -> Option<LifecycleMetadata> {
        conn.query_row(
            "SELECT memory_id, state, scope, confidence, reinforcement_count,
                    contradiction_ids, source, last_recalled_at, recall_count,
                    privacy_flag, created_at, updated_at
             FROM memory_lifecycle WHERE memory_id = ?1",
            params![memory_id],
            |row| {
                let contradictions_json: String = row.get(5)?;
                let contradictions: Vec<String> = serde_json::from_str(&contradictions_json)
                    .unwrap_or_default();
                Ok(LifecycleMetadata {
                    memory_id: row.get(0)?,
                    state: MemoryState::from_str(&row.get::<_, String>(1)?),
                    scope: MemoryScope::from_str(&row.get::<_, String>(2)?),
                    confidence: row.get(3)?,
                    reinforcement_count: row.get::<_, i64>(4)? as u32,
                    contradiction_ids: contradictions,
                    source: MemorySource::from_str(&row.get::<_, String>(6)?),
                    last_recalled_at: row.get(7)?,
                    recall_count: row.get::<_, i64>(8)? as u32,
                    privacy_flag: row.get::<_, i64>(9)? != 0,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                })
            },
        )
        .ok()
    }

    /// Transition a memory to a new state.
    pub fn transition(conn: &Connection, memory_id: &str, new_state: MemoryState) {
        let _ = conn.execute(
            "UPDATE memory_lifecycle SET state = ?1, updated_at = ?2 WHERE memory_id = ?3",
            params![new_state.as_str(), now_ts(), memory_id],
        );
    }

    /// Reinforce a memory (bump confidence + reinforcement count).
    /// Transitions: observed→confirmed, confirmed→reinforced.
    pub fn reinforce(conn: &Connection, memory_id: &str) {
        let now = now_ts();
        let _ = conn.execute(
            "UPDATE memory_lifecycle SET
                reinforcement_count = reinforcement_count + 1,
                confidence = MIN(1.0, confidence + 0.1),
                state = CASE
                    WHEN state = 'observed' THEN 'confirmed'
                    WHEN state = 'inferred' THEN 'confirmed'
                    WHEN state = 'confirmed' AND reinforcement_count >= 2 THEN 'reinforced'
                    ELSE state
                END,
                updated_at = ?1
             WHERE memory_id = ?2",
            params![now, memory_id],
        );
    }

    /// Record that a memory was recalled.
    pub fn record_recall(conn: &Connection, memory_id: &str) {
        let now = now_ts();
        let _ = conn.execute(
            "UPDATE memory_lifecycle SET
                last_recalled_at = ?1, recall_count = recall_count + 1, updated_at = ?1
             WHERE memory_id = ?2",
            params![now, memory_id],
        );
    }

    /// Flag a contradiction between two memories.
    pub fn flag_contradiction(
        conn: &Connection,
        memory_a: &str,
        memory_b: &str,
        reason: &str,
    ) {
        let now = now_ts();

        // Record the contradiction
        let _ = conn.execute(
            "INSERT INTO memory_contradictions (memory_a_id, memory_b_id, reason, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![memory_a, memory_b, reason, now],
        );

        // Update both memories' contradiction_ids
        for mid in &[memory_a, memory_b] {
            let other = if *mid == memory_a { memory_b } else { memory_a };
            if let Some(meta) = Self::get(conn, mid) {
                let mut ids = meta.contradiction_ids;
                if !ids.contains(&other.to_string()) {
                    ids.push(other.to_string());
                }
                let json = serde_json::to_string(&ids).unwrap_or_default();
                let _ = conn.execute(
                    "UPDATE memory_lifecycle SET
                        contradiction_ids = ?1, state = 'contradicted', updated_at = ?2
                     WHERE memory_id = ?3",
                    params![json, now, mid],
                );
            }
        }
    }

    /// Resolve a contradiction (user chose which is correct).
    pub fn resolve_contradiction(
        conn: &Connection,
        memory_correct: &str,
        memory_wrong: &str,
        resolution: &str,
    ) {
        let now = now_ts();

        // Mark the wrong memory as archived
        Self::transition(conn, memory_wrong, MemoryState::Archived);

        // Reinforce the correct one
        Self::reinforce(conn, memory_correct);

        // Clear contradiction state on the correct memory
        if let Some(meta) = Self::get(conn, memory_correct) {
            let ids: Vec<String> = meta.contradiction_ids.into_iter()
                .filter(|id| id != memory_wrong)
                .collect();
            let json = serde_json::to_string(&ids).unwrap_or_default();

            // Only revert from contradicted if no more contradictions remain
            let new_state = if ids.is_empty() { "confirmed" } else { "contradicted" };
            let _ = conn.execute(
                "UPDATE memory_lifecycle SET
                    contradiction_ids = ?1, state = ?2, updated_at = ?3
                 WHERE memory_id = ?4",
                params![json, new_state, now, memory_correct],
            );
        }

        // Mark the contradiction record as resolved
        let _ = conn.execute(
            "UPDATE memory_contradictions SET resolved = 1, resolved_by = ?1
             WHERE (memory_a_id = ?2 AND memory_b_id = ?3) OR (memory_a_id = ?3 AND memory_b_id = ?2)",
            params![resolution, memory_correct, memory_wrong],
        );
    }

    /// Set privacy flag (memory should not be used proactively).
    pub fn set_privacy(conn: &Connection, memory_id: &str, private: bool) {
        let _ = conn.execute(
            "UPDATE memory_lifecycle SET privacy_flag = ?1, updated_at = ?2 WHERE memory_id = ?3",
            params![private as i64, now_ts(), memory_id],
        );
    }

    /// Set scope for a memory.
    pub fn set_scope(conn: &Connection, memory_id: &str, scope: MemoryScope) {
        let _ = conn.execute(
            "UPDATE memory_lifecycle SET scope = ?1, updated_at = ?2 WHERE memory_id = ?3",
            params![scope.as_str(), now_ts(), memory_id],
        );
    }

    /// Run stale detection: mark old, never-recalled memories as stale.
    pub fn detect_stale(conn: &Connection, stale_days: f64) -> u64 {
        let threshold = now_ts() - stale_days * 86400.0;

        let affected = conn.execute(
            "UPDATE memory_lifecycle SET state = 'stale', updated_at = ?1
             WHERE state IN ('observed', 'inferred')
               AND (last_recalled_at IS NULL OR last_recalled_at < ?2)
               AND created_at < ?2
               AND recall_count = 0",
            params![now_ts(), threshold],
        ).unwrap_or(0);

        affected as u64
    }

    /// Nightly consolidation: promote high-confidence observations to confirmed.
    pub fn consolidate(conn: &Connection) -> ConsolidationReport {
        let now = now_ts();
        let mut report = ConsolidationReport::default();

        // Promote high-confidence observations (> 0.7) that are at least 24h old
        let day_ago = now - 86400.0;
        report.promoted = conn.execute(
            "UPDATE memory_lifecycle SET state = 'confirmed', updated_at = ?1
             WHERE state = 'observed' AND confidence >= 0.7 AND created_at < ?2",
            params![now, day_ago],
        ).unwrap_or(0) as u64;

        // Mark old, never-used inferred memories as stale (30 days)
        report.staled = Self::detect_stale(conn, 30.0);

        // Archive old stale memories (90 days stale)
        let archive_threshold = now - 90.0 * 86400.0;
        report.archived = conn.execute(
            "UPDATE memory_lifecycle SET state = 'archived', updated_at = ?1
             WHERE state = 'stale' AND updated_at < ?2",
            params![now, archive_threshold],
        ).unwrap_or(0) as u64;

        report
    }

    /// Get active memories for a scope (for recall filtering).
    pub fn active_ids_for_scope(conn: &Connection, scope: &MemoryScope) -> Vec<String> {
        let scope_str = scope.as_str();
        let mut stmt = match conn.prepare(
            "SELECT memory_id FROM memory_lifecycle
             WHERE state IN ('observed', 'inferred', 'confirmed', 'reinforced')
               AND privacy_flag = 0
               AND (scope = ?1 OR scope = 'global')
             ORDER BY confidence DESC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![scope_str], |row| row.get(0))
            .ok()
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Get unresolved contradictions.
    pub fn unresolved_contradictions(conn: &Connection) -> Vec<ContradictionRecord> {
        let mut stmt = match conn.prepare(
            "SELECT memory_a_id, memory_b_id, reason, created_at
             FROM memory_contradictions
             WHERE resolved = 0
             ORDER BY created_at DESC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map([], |row| {
            Ok(ContradictionRecord {
                memory_a: row.get(0)?,
                memory_b: row.get(1)?,
                reason: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Summary stats for the dashboard.
    pub fn stats(conn: &Connection) -> LifecycleStats {
        let count_state = |state: &str| -> u64 {
            conn.query_row(
                "SELECT COUNT(*) FROM memory_lifecycle WHERE state = ?1",
                params![state], |r| r.get::<_, i64>(0),
            ).unwrap_or(0) as u64
        };

        let contradictions: u64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_contradictions WHERE resolved = 0",
            [], |r| r.get::<_, i64>(0),
        ).unwrap_or(0) as u64;

        let private_count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_lifecycle WHERE privacy_flag = 1",
            [], |r| r.get::<_, i64>(0),
        ).unwrap_or(0) as u64;

        LifecycleStats {
            observed: count_state("observed"),
            inferred: count_state("inferred"),
            confirmed: count_state("confirmed"),
            reinforced: count_state("reinforced"),
            contradicted: count_state("contradicted"),
            stale: count_state("stale"),
            archived: count_state("archived"),
            forgotten: count_state("forgotten"),
            unresolved_contradictions: contradictions,
            private_memories: private_count,
        }
    }
}

/// Result of nightly consolidation.
#[derive(Debug, Clone, Default)]
pub struct ConsolidationReport {
    pub promoted: u64,
    pub staled: u64,
    pub archived: u64,
}

/// A contradiction between two memories.
#[derive(Debug, Clone)]
pub struct ContradictionRecord {
    pub memory_a: String,
    pub memory_b: String,
    pub reason: String,
    pub created_at: f64,
}

/// Memory lifecycle statistics.
#[derive(Debug, Clone)]
pub struct LifecycleStats {
    pub observed: u64,
    pub inferred: u64,
    pub confirmed: u64,
    pub reinforced: u64,
    pub contradicted: u64,
    pub stale: u64,
    pub archived: u64,
    pub forgotten: u64,
    pub unresolved_contradictions: u64,
    pub private_memories: u64,
}

impl LifecycleStats {
    pub fn total_active(&self) -> u64 {
        self.observed + self.inferred + self.confirmed + self.reinforced
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
        MemoryLifecycle::ensure_table(&conn);
        conn
    }

    #[test]
    fn register_and_get() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);
        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert_eq!(meta.state, MemoryState::Observed);
        assert_eq!(meta.scope, MemoryScope::Global);
        assert!((meta.confidence - 0.8).abs() < 0.01);
    }

    #[test]
    fn reinforcement_lifecycle() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.5);

        // First reinforce: observed → confirmed
        MemoryLifecycle::reinforce(&conn, "m1");
        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert_eq!(meta.state, MemoryState::Confirmed);
        assert!(meta.confidence > 0.5);

        // Second reinforce: still confirmed, count increases
        MemoryLifecycle::reinforce(&conn, "m1");
        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert_eq!(meta.reinforcement_count, 2);

        // Third reinforce: confirmed → reinforced (count >= 2 check happens on update)
        MemoryLifecycle::reinforce(&conn, "m1");
        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert_eq!(meta.state, MemoryState::Reinforced);
    }

    #[test]
    fn contradiction_detection_and_resolution() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);
        MemoryLifecycle::register(&conn, "m2", MemorySource::Email, MemoryScope::Global, 0.6);

        // Flag contradiction
        MemoryLifecycle::flag_contradiction(&conn, "m1", "m2", "m1 says X, m2 says Y");

        let m1 = MemoryLifecycle::get(&conn, "m1").unwrap();
        let m2 = MemoryLifecycle::get(&conn, "m2").unwrap();
        assert_eq!(m1.state, MemoryState::Contradicted);
        assert_eq!(m2.state, MemoryState::Contradicted);
        assert!(m1.contradiction_ids.contains(&"m2".to_string()));

        let contradictions = MemoryLifecycle::unresolved_contradictions(&conn);
        assert_eq!(contradictions.len(), 1);

        // Resolve: m1 is correct, m2 is wrong
        MemoryLifecycle::resolve_contradiction(&conn, "m1", "m2", "user_confirmed");

        let m1 = MemoryLifecycle::get(&conn, "m1").unwrap();
        let m2 = MemoryLifecycle::get(&conn, "m2").unwrap();
        assert_eq!(m1.state, MemoryState::Confirmed); // Restored
        assert_eq!(m2.state, MemoryState::Archived); // Deprecated
        assert!(m1.contradiction_ids.is_empty());

        let contradictions = MemoryLifecycle::unresolved_contradictions(&conn);
        assert!(contradictions.is_empty());
    }

    #[test]
    fn scope_filtering() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);
        MemoryLifecycle::register(&conn, "m2", MemorySource::UserStatement, MemoryScope::Work, 0.7);
        MemoryLifecycle::register(&conn, "m3", MemorySource::UserStatement, MemoryScope::Personal, 0.6);

        let work_ids = MemoryLifecycle::active_ids_for_scope(&conn, &MemoryScope::Work);
        assert!(work_ids.contains(&"m1".to_string())); // Global is always included
        assert!(work_ids.contains(&"m2".to_string()));
        assert!(!work_ids.contains(&"m3".to_string())); // Personal excluded from work

        let personal_ids = MemoryLifecycle::active_ids_for_scope(&conn, &MemoryScope::Personal);
        assert!(personal_ids.contains(&"m1".to_string())); // Global included
        assert!(!personal_ids.contains(&"m2".to_string())); // Work excluded
        assert!(personal_ids.contains(&"m3".to_string()));
    }

    #[test]
    fn privacy_flag_excludes_from_recall() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);
        MemoryLifecycle::register(&conn, "m2", MemorySource::UserStatement, MemoryScope::Global, 0.7);

        MemoryLifecycle::set_privacy(&conn, "m2", true);

        let active = MemoryLifecycle::active_ids_for_scope(&conn, &MemoryScope::Global);
        assert!(active.contains(&"m1".to_string()));
        assert!(!active.contains(&"m2".to_string())); // Private excluded
    }

    #[test]
    fn stale_detection() {
        let conn = setup();

        // Insert a memory with old timestamp (simulate 60 days ago)
        let old_ts = now_ts() - 60.0 * 86400.0;
        let _ = conn.execute(
            "INSERT INTO memory_lifecycle
             (memory_id, state, scope, confidence, source, created_at, updated_at)
             VALUES ('old_mem', 'observed', 'global', 0.5, 'conversation_inference', ?1, ?1)",
            params![old_ts],
        );

        // Insert a recent memory
        MemoryLifecycle::register(&conn, "new_mem", MemorySource::UserStatement, MemoryScope::Global, 0.8);

        let staled = MemoryLifecycle::detect_stale(&conn, 30.0);
        assert_eq!(staled, 1);

        let old = MemoryLifecycle::get(&conn, "old_mem").unwrap();
        assert_eq!(old.state, MemoryState::Stale);

        let new = MemoryLifecycle::get(&conn, "new_mem").unwrap();
        assert_eq!(new.state, MemoryState::Observed); // Not stale
    }

    #[test]
    fn consolidation_promotes_confident_memories() {
        let conn = setup();

        // High confidence, old enough
        let old_ts = now_ts() - 2.0 * 86400.0;
        let _ = conn.execute(
            "INSERT INTO memory_lifecycle
             (memory_id, state, scope, confidence, source, created_at, updated_at)
             VALUES ('high_conf', 'observed', 'global', 0.85, 'user_statement', ?1, ?1)",
            params![old_ts],
        );

        // Low confidence, old enough
        let _ = conn.execute(
            "INSERT INTO memory_lifecycle
             (memory_id, state, scope, confidence, source, created_at, updated_at)
             VALUES ('low_conf', 'observed', 'global', 0.3, 'conversation_inference', ?1, ?1)",
            params![old_ts],
        );

        let report = MemoryLifecycle::consolidate(&conn);
        assert_eq!(report.promoted, 1);

        let high = MemoryLifecycle::get(&conn, "high_conf").unwrap();
        assert_eq!(high.state, MemoryState::Confirmed);

        let low = MemoryLifecycle::get(&conn, "low_conf").unwrap();
        assert_eq!(low.state, MemoryState::Observed); // Not promoted
    }

    #[test]
    fn recall_tracking() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.5);

        MemoryLifecycle::record_recall(&conn, "m1");
        MemoryLifecycle::record_recall(&conn, "m1");

        let meta = MemoryLifecycle::get(&conn, "m1").unwrap();
        assert_eq!(meta.recall_count, 2);
        assert!(meta.last_recalled_at.is_some());
    }

    #[test]
    fn stats_are_correct() {
        let conn = setup();
        MemoryLifecycle::register(&conn, "m1", MemorySource::UserStatement, MemoryScope::Global, 0.8);
        MemoryLifecycle::register(&conn, "m2", MemorySource::UserStatement, MemoryScope::Global, 0.7);
        MemoryLifecycle::reinforce(&conn, "m1"); // observed → confirmed
        MemoryLifecycle::set_privacy(&conn, "m2", true);

        let stats = MemoryLifecycle::stats(&conn);
        assert_eq!(stats.confirmed, 1);
        assert_eq!(stats.observed, 1);
        assert_eq!(stats.private_memories, 1);
        assert_eq!(stats.total_active(), 2);
    }
}
