//! Nightly Consolidation — the brain's "sleep" cycle.
//!
//! Runs during idle periods (long idle + nighttime) to:
//! 1. Strengthen patterns confirmed by positive feedback
//! 2. Decay source weights for consistently poor outcomes
//! 3. Prune stale entity expectations (confidence < 0.05, not seen in 60+ days)
//! 4. Update entity baselines with batch Welford corrections
//! 5. Recalculate curiosity source TTLs from yield history
//! 6. Compress BrainState feedback data (remove zero-confidence entries)
//! 7. Report on what was consolidated
//!
//! Integrates with the existing memory evolution nightly cycle.

use rusqlite::Connection;

use serde::{Deserialize, Serialize};

// ══════════════════════════════════════════════════════════════════════════════
// § 1  Consolidation Report
// ══════════════════════════════════════════════════════════════════════════════

/// Summary of what the nightly consolidation did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsolidationReport {
    /// Expectations pruned (too stale or low confidence).
    pub expectations_pruned: usize,
    /// Expectations decayed (lowered confidence).
    pub expectations_decayed: usize,
    /// Baselines pruned (zero variance or very old).
    pub baselines_pruned: usize,
    /// Curiosity sources backed off or disabled.
    pub sources_adjusted: usize,
    /// Feedback entries cleaned from BrainState.
    pub feedback_cleaned: usize,
    /// Whether consolidation ran at all.
    pub ran: bool,
}

// ══════════════════════════════════════════════════════════════════════════════
// § 2  Should Consolidate Check
// ══════════════════════════════════════════════════════════════════════════════

const CONSOLIDATION_KEY: &str = "__brain_last_consolidation";
const CONSOLIDATION_INTERVAL: f64 = 86400.0; // 24 hours

/// Check if consolidation should run (once per 24h, during idle).
pub fn should_consolidate(conn: &Connection, now: f64, idle_secs: f64) -> bool {
    // Require at least 30 minutes of idle
    if idle_secs < 1800.0 {
        return false;
    }

    let last: f64 = conn
        .query_row(
            "SELECT COALESCE(
                (SELECT activation FROM cognitive_nodes WHERE node_id = ?1),
                0.0
            )",
            rusqlite::params![CONSOLIDATION_KEY],
            |row| row.get(0),
        )
        .unwrap_or(0.0);

    now - last > CONSOLIDATION_INTERVAL
}

// ══════════════════════════════════════════════════════════════════════════════
// § 3  Main Consolidation
// ══════════════════════════════════════════════════════════════════════════════

/// Run the nightly consolidation pass.
pub fn run_consolidation(conn: &Connection, now: f64) -> ConsolidationReport {
    let mut report = ConsolidationReport::default();
    report.ran = true;

    // 1. Prune stale entity expectations
    report.expectations_pruned = prune_stale_expectations(conn, now);

    // 2. Decay old expectations
    report.expectations_decayed = decay_old_expectations(conn, now);

    // 3. Prune dead baselines
    report.baselines_pruned = prune_dead_baselines(conn, now);

    // 4. Adjust curiosity sources
    report.sources_adjusted = adjust_curiosity_sources(conn, now);

    // 5. Clean brain state feedback
    report.feedback_cleaned = clean_brain_feedback(conn);

    // Record consolidation timestamp
    conn.execute(
        "INSERT INTO cognitive_nodes (node_id, node_type, label, context, confidence, activation, salience, created_at, updated_at)
         VALUES (?1, 'consolidation', 'brain_consolidation', '{}', 1.0, ?2, 0.0, ?2, ?2)
         ON CONFLICT(node_id) DO UPDATE SET activation = ?2, updated_at = ?2",
        rusqlite::params![CONSOLIDATION_KEY, now],
    )
    .ok();

    tracing::info!(
        pruned_expectations = report.expectations_pruned,
        decayed_expectations = report.expectations_decayed,
        pruned_baselines = report.baselines_pruned,
        adjusted_sources = report.sources_adjusted,
        cleaned_feedback = report.feedback_cleaned,
        "Brain consolidation complete"
    );

    report
}

// ══════════════════════════════════════════════════════════════════════════════
// § 4  Individual Consolidation Steps
// ══════════════════════════════════════════════════════════════════════════════

/// Prune expectations with very low confidence or not seen in 90+ days.
fn prune_stale_expectations(conn: &Connection, now: f64) -> usize {
    let pruned = conn
        .execute(
            "DELETE FROM entity_expectations
             WHERE confidence < 0.05
                OR (?1 - last_seen_at > 7776000.0 AND confidence < 0.3)",
            rusqlite::params![now],
        )
        .unwrap_or(0);
    pruned
}

/// Decay confidence for expectations not seen recently.
fn decay_old_expectations(conn: &Connection, now: f64) -> usize {
    // 30-60 days without event: multiply confidence by 0.85
    let decayed = conn
        .execute(
            "UPDATE entity_expectations
             SET confidence = confidence * 0.85, updated_at = ?1
             WHERE (?1 - last_seen_at) > 2592000.0
               AND confidence > 0.1",
            rusqlite::params![now],
        )
        .unwrap_or(0);
    decayed
}

/// Prune baselines with zero variance (no variation = no anomaly detection value)
/// or not updated in 90+ days.
fn prune_dead_baselines(conn: &Connection, now: f64) -> usize {
    let pruned = conn
        .execute(
            "DELETE FROM entity_metric_baselines
             WHERE (m2 < 0.001 AND n > 20)
                OR (?1 - last_seen_ts > 7776000.0)",
            rusqlite::params![now],
        )
        .unwrap_or(0);
    pruned
}

/// Disable curiosity sources with persistently low yield.
fn adjust_curiosity_sources(conn: &Connection, _now: f64) -> usize {
    // Disable sources with <0.05 yield EMA and >20 fetches
    let disabled = conn
        .execute(
            "UPDATE curiosity_sources
             SET enabled = 0
             WHERE yield_ema < 0.05 AND fetch_count > 20",
            [],
        )
        .unwrap_or(0);

    // Increase TTL for low-yield sources (slow them down instead of disabling)
    let slowed = conn
        .execute(
            "UPDATE curiosity_sources
             SET ttl_secs = ttl_secs * 1.5
             WHERE yield_ema < 0.2 AND yield_ema >= 0.05
               AND fetch_count > 10
               AND ttl_secs < 86400.0",
            [],
        )
        .unwrap_or(0);

    disabled + slowed
}

/// Clean BrainState feedback data — remove zero-count entries.
fn clean_brain_feedback(conn: &Connection) -> usize {
    use super::brain::BrainState;

    let json: Option<String> = conn
        .query_row(
            "SELECT context FROM cognitive_nodes WHERE node_id = '__brain_state_v1'",
            [],
            |row| row.get(0),
        )
        .ok();

    let mut state: BrainState = match json {
        Some(s) => serde_json::from_str(&s).unwrap_or_default(),
        None => return 0,
    };

    let before = state.feedback.ema.len();

    // Remove sources with 0 outcome count or very low EMA across all signal types
    state.feedback.ema.retain(|source_key, signal_map| {
        let count = state.feedback.outcome_count.get(source_key).copied().unwrap_or(0);
        if count == 0 {
            return false;
        }
        // Remove if all signal EMAs are near zero
        let all_low = signal_map.values().all(|&v| v < 0.02);
        !all_low
    });

    state.feedback.outcome_count.retain(|_, &mut v| v > 0);

    let cleaned = before.saturating_sub(state.feedback.ema.len());

    if cleaned > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        if let Ok(json) = serde_json::to_string(&state) {
            conn.execute(
                "UPDATE cognitive_nodes SET context = ?1, updated_at = ?2
                 WHERE node_id = '__brain_state_v1'",
                rusqlite::params![json, now],
            )
            .ok();
        }
    }

    cleaned
}

// ══════════════════════════════════════════════════════════════════════════════
// § 5  Tests
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cognitive_nodes (
                node_id TEXT PRIMARY KEY,
                node_type TEXT,
                label TEXT,
                context TEXT DEFAULT '{}',
                confidence REAL DEFAULT 0.5,
                activation REAL DEFAULT 0.5,
                salience REAL DEFAULT 0.5,
                created_at REAL,
                updated_at REAL
            );",
        ).unwrap();
        super::super::detectors::ensure_detector_tables(&conn);
        super::super::curiosity::ensure_curiosity_tables(&conn);
        conn
    }

    #[test]
    fn test_should_consolidate() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Should consolidate if idle enough and never run before
        assert!(should_consolidate(&conn, now, 3600.0));

        // Not if idle time is short
        assert!(!should_consolidate(&conn, now, 60.0));
    }

    #[test]
    fn test_consolidation_runs() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Insert some stale expectations
        conn.execute(
            "INSERT INTO entity_expectations VALUES
             ('stale', 'msg', 86400.0, 0.01, ?1, 5, 3, 86400.0, 1000.0, ?1, ?1)",
            rusqlite::params![now - 10_000_000.0],
        ).unwrap();

        let report = run_consolidation(&conn, now);
        assert!(report.ran);
        assert!(report.expectations_pruned > 0, "Should prune stale expectation");
    }

    #[test]
    fn test_no_double_consolidation() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Run consolidation
        run_consolidation(&conn, now);

        // Should not run again within 24h
        assert!(!should_consolidate(&conn, now + 3600.0, 3600.0));

        // Should run after 24h
        assert!(should_consolidate(&conn, now + 90000.0, 3600.0));
    }
}
