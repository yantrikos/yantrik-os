//! Tool Reliability Metrics — tracks per-tool success rates, durations, and failure patterns.
//!
//! Uses a dedicated SQLite table to accumulate rolling metrics for every tool invocation.
//! The agent loop records each tool call outcome, and these metrics inform:
//! - Tool selection (prefer reliable tools for critical tasks)
//! - Health monitoring (surface degraded tools in system dashboard)
//! - Auto-disable (quarantine tools failing > 80% over 50+ calls)

use rusqlite::{params, Connection};

/// Per-tool reliability statistics.
#[derive(Debug, Clone)]
pub struct ToolStats {
    pub tool_name: String,
    pub total_calls: u64,
    pub successes: u64,
    pub failures: u64,
    pub avg_duration_ms: f64,
    pub p95_duration_ms: f64,
    pub success_rate: f64,
    pub last_failure_reason: Option<String>,
    pub last_called_at: f64,
    pub quarantined: bool,
}

/// Manages tool reliability metrics persistence and querying.
pub struct ToolMetrics;

/// Threshold for quarantine consideration (must have at least this many calls).
const MIN_CALLS_FOR_QUARANTINE: u64 = 50;
/// Success rate below this triggers quarantine.
const QUARANTINE_THRESHOLD: f64 = 0.20;
/// Number of recent calls to consider for rolling metrics.
const ROLLING_WINDOW: u64 = 100;

impl ToolMetrics {
    /// Create the tool_metrics table if it doesn't exist.
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tool_metrics (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                tool_name       TEXT NOT NULL,
                success         INTEGER NOT NULL,
                duration_ms     INTEGER NOT NULL,
                failure_reason  TEXT,
                called_at       REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tool_metrics_name ON tool_metrics(tool_name);
            CREATE INDEX IF NOT EXISTS idx_tool_metrics_time ON tool_metrics(called_at);

            CREATE TABLE IF NOT EXISTS tool_quarantine (
                tool_name       TEXT PRIMARY KEY,
                quarantined_at  REAL NOT NULL,
                reason          TEXT NOT NULL,
                success_rate    REAL NOT NULL
            );",
        )
        .expect("failed to create tool_metrics tables");
    }

    /// Record a tool call outcome.
    pub fn record(
        conn: &Connection,
        tool_name: &str,
        success: bool,
        duration_ms: u64,
        failure_reason: Option<&str>,
    ) {
        let now = now_ts();
        let _ = conn.execute(
            "INSERT INTO tool_metrics (tool_name, success, duration_ms, failure_reason, called_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![tool_name, success as i32, duration_ms as i64, failure_reason, now],
        );

        // Check quarantine after recording
        Self::check_quarantine(conn, tool_name);
    }

    /// Get reliability stats for a specific tool (rolling window).
    pub fn stats_for(conn: &Connection, tool_name: &str) -> Option<ToolStats> {
        let mut stmt = conn
            .prepare(
                "SELECT success, duration_ms, failure_reason, called_at
                 FROM tool_metrics
                 WHERE tool_name = ?1
                 ORDER BY called_at DESC
                 LIMIT ?2",
            )
            .ok()?;

        let rows: Vec<(bool, u64, Option<String>, f64)> = stmt
            .query_map(params![tool_name, ROLLING_WINDOW], |row| {
                Ok((
                    row.get::<_, i32>(0)? != 0,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();

        if rows.is_empty() {
            return None;
        }

        let total = rows.len() as u64;
        let successes = rows.iter().filter(|(s, _, _, _)| *s).count() as u64;
        let failures = total - successes;
        let success_rate = successes as f64 / total as f64;

        let durations: Vec<u64> = rows.iter().map(|(_, d, _, _)| *d).collect();
        let avg_duration_ms = durations.iter().sum::<u64>() as f64 / total as f64;

        // P95 duration
        let mut sorted_durations = durations.clone();
        sorted_durations.sort_unstable();
        let p95_idx = ((total as f64 * 0.95) as usize).min(sorted_durations.len() - 1);
        let p95_duration_ms = sorted_durations[p95_idx] as f64;

        let last_failure_reason = rows
            .iter()
            .find(|(s, _, _, _)| !*s)
            .and_then(|(_, _, r, _)| r.clone());

        let last_called_at = rows.first().map(|(_, _, _, t)| *t).unwrap_or(0.0);

        let quarantined = conn
            .query_row(
                "SELECT 1 FROM tool_quarantine WHERE tool_name = ?1",
                params![tool_name],
                |_| Ok(()),
            )
            .is_ok();

        Some(ToolStats {
            tool_name: tool_name.to_string(),
            total_calls: total,
            successes,
            failures,
            avg_duration_ms,
            p95_duration_ms,
            success_rate,
            last_failure_reason,
            last_called_at,
            quarantined,
        })
    }

    /// Get stats for all tools that have been called, sorted by success rate ascending.
    pub fn all_stats(conn: &Connection) -> Vec<ToolStats> {
        let tool_names: Vec<String> = conn
            .prepare("SELECT DISTINCT tool_name FROM tool_metrics ORDER BY tool_name")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get(0))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        let mut stats: Vec<ToolStats> = tool_names
            .iter()
            .filter_map(|name| Self::stats_for(conn, name))
            .collect();

        // Sort: worst success rate first (most attention-worthy)
        stats.sort_by(|a, b| a.success_rate.partial_cmp(&b.success_rate).unwrap_or(std::cmp::Ordering::Equal));
        stats
    }

    /// Get list of quarantined tools.
    pub fn quarantined(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT tool_name FROM tool_quarantine")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get(0))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    /// Check if a tool should be quarantined based on its failure rate.
    fn check_quarantine(conn: &Connection, tool_name: &str) {
        if let Some(stats) = Self::stats_for(conn, tool_name) {
            if stats.total_calls >= MIN_CALLS_FOR_QUARANTINE
                && stats.success_rate < QUARANTINE_THRESHOLD
            {
                let reason = format!(
                    "Success rate {:.1}% over {} calls (threshold: {:.0}%)",
                    stats.success_rate * 100.0,
                    stats.total_calls,
                    QUARANTINE_THRESHOLD * 100.0,
                );
                tracing::warn!(
                    tool = tool_name,
                    success_rate = stats.success_rate,
                    total_calls = stats.total_calls,
                    "Tool quarantined due to low reliability"
                );
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO tool_quarantine (tool_name, quarantined_at, reason, success_rate)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![tool_name, now_ts(), reason, stats.success_rate],
                );
            }
        }
    }

    /// Remove a tool from quarantine (manual rehabilitation).
    pub fn unquarantine(conn: &Connection, tool_name: &str) {
        let _ = conn.execute(
            "DELETE FROM tool_quarantine WHERE tool_name = ?1",
            params![tool_name],
        );
    }

    /// Compact old metrics (keep only last N per tool).
    pub fn compact(conn: &Connection, keep_per_tool: u64) {
        let tool_names: Vec<String> = conn
            .prepare("SELECT DISTINCT tool_name FROM tool_metrics")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get(0))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        for name in &tool_names {
            let _ = conn.execute(
                "DELETE FROM tool_metrics WHERE tool_name = ?1 AND id NOT IN (
                    SELECT id FROM tool_metrics WHERE tool_name = ?1
                    ORDER BY called_at DESC LIMIT ?2
                )",
                params![name, keep_per_tool],
            );
        }
    }

    /// Format a health summary for system prompt injection.
    pub fn health_summary(conn: &Connection) -> String {
        let stats = Self::all_stats(conn);
        if stats.is_empty() {
            return String::new();
        }

        let degraded: Vec<&ToolStats> = stats
            .iter()
            .filter(|s| s.success_rate < 0.8 && s.total_calls >= 10)
            .collect();

        if degraded.is_empty() {
            return String::new();
        }

        let mut summary = String::from("Tool health warnings:\n");
        for s in &degraded {
            let status = if s.quarantined { "QUARANTINED" } else { "degraded" };
            summary.push_str(&format!(
                "- {} [{}]: {:.0}% success ({}/{} calls, avg {:.0}ms)",
                s.tool_name, status,
                s.success_rate * 100.0,
                s.successes, s.total_calls,
                s.avg_duration_ms,
            ));
            if let Some(ref reason) = s.last_failure_reason {
                let short = if reason.len() > 80 { &reason[..80] } else { reason };
                summary.push_str(&format!(" — last error: {}", short));
            }
            summary.push('\n');
        }
        summary
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ToolMetrics::ensure_table(&conn);
        conn
    }

    #[test]
    fn record_and_stats() {
        let conn = setup();
        for _ in 0..8 {
            ToolMetrics::record(&conn, "recall", true, 50, None);
        }
        ToolMetrics::record(&conn, "recall", false, 200, Some("timeout"));
        ToolMetrics::record(&conn, "recall", true, 60, None);

        let stats = ToolMetrics::stats_for(&conn, "recall").unwrap();
        assert_eq!(stats.total_calls, 10);
        assert_eq!(stats.successes, 9);
        assert_eq!(stats.failures, 1);
        assert!((stats.success_rate - 0.9).abs() < 0.01);
        assert!(stats.avg_duration_ms > 0.0);
        assert_eq!(stats.last_failure_reason, Some("timeout".into()));
    }

    #[test]
    fn quarantine_on_high_failure() {
        let conn = setup();
        // 50 calls, 90% failure
        for _ in 0..45 {
            ToolMetrics::record(&conn, "bad_tool", false, 100, Some("broken"));
        }
        for _ in 0..5 {
            ToolMetrics::record(&conn, "bad_tool", true, 100, None);
        }
        let stats = ToolMetrics::stats_for(&conn, "bad_tool").unwrap();
        assert!(stats.quarantined);
        assert!(stats.success_rate < 0.2);

        // Unquarantine
        ToolMetrics::unquarantine(&conn, "bad_tool");
        let stats2 = ToolMetrics::stats_for(&conn, "bad_tool").unwrap();
        assert!(!stats2.quarantined);
    }

    #[test]
    fn health_summary_reports_degraded() {
        let conn = setup();
        // 90% success — healthy, should NOT appear
        for _ in 0..9 {
            ToolMetrics::record(&conn, "good_tool", true, 50, None);
        }
        ToolMetrics::record(&conn, "good_tool", false, 50, None);

        // 60% success — degraded, SHOULD appear
        for _ in 0..6 {
            ToolMetrics::record(&conn, "flaky_tool", true, 100, None);
        }
        for _ in 0..4 {
            ToolMetrics::record(&conn, "flaky_tool", false, 100, Some("network error"));
        }

        let summary = ToolMetrics::health_summary(&conn);
        assert!(summary.contains("flaky_tool"));
        assert!(!summary.contains("good_tool"));
    }

    #[test]
    fn compact_keeps_recent() {
        let conn = setup();
        for i in 0..200 {
            ToolMetrics::record(&conn, "tool_a", i % 3 != 0, (i * 10) as u64, None);
        }
        ToolMetrics::compact(&conn, 50);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tool_metrics WHERE tool_name = 'tool_a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 50);
    }
}
