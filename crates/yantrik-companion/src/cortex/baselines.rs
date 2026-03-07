//! Baseline tracker — learns what's "normal" and detects deviations.
//!
//! Uses Welford's online algorithm to maintain running mean + variance
//! for arbitrary (entity, metric) pairs. No hardcoded metrics — the
//! system learns whatever patterns emerge from the pulse stream.
//!
//! Examples of auto-discovered baselines:
//! - "person:sarah → emails_per_day" (mean=3.2, stddev=1.1)
//! - "activity:coding → duration_minutes" (mean=45, stddev=20)
//! - "ticket:* → commits_per_day" (mean=2.5, stddev=1.5)
//! - "activity:invoicing → session_count_per_week" (mean=2, stddev=0.5)
//!
//! Deviation detection fires when current value is >2σ from the mean.

use rusqlite::{params, Connection};

use super::rules::AttentionItem;

// ── Core Types ───────────────────────────────────────────────────────

/// A baseline measurement for an entity+metric pair.
#[derive(Debug, Clone)]
struct Baseline {
    entity_id: String,
    metric: String,
    sample_count: i64,
    mean: f64,
    stddev: f64,
    last_value: f64,
}

/// A detected deviation from normal.
#[derive(Debug, Clone)]
pub struct Deviation {
    pub entity_id: String,
    pub metric: String,
    pub current: f64,
    pub expected_mean: f64,
    pub stddev: f64,
    pub z_score: f64,
    pub direction: DeviationDirection,
}

#[derive(Debug, Clone)]
pub enum DeviationDirection {
    AboveNormal,
    BelowNormal,
}

// ── Baseline Tracker ─────────────────────────────────────────────────

pub struct BaselineTracker {
    /// Minimum samples before we start flagging deviations.
    min_samples: i64,
    /// Z-score threshold for flagging (default: 2.0 = ~95% confidence).
    z_threshold: f64,
}

impl BaselineTracker {
    pub fn new() -> Self {
        Self {
            min_samples: 5,
            z_threshold: 2.0,
        }
    }

    /// Record a new measurement for an entity+metric pair.
    ///
    /// Uses Welford's online algorithm for numerically stable
    /// incremental mean + variance calculation.
    pub fn record(&self, conn: &Connection, entity_id: &str, metric: &str, value: f64) {
        let now = now_ts();

        // Try to get existing baseline
        let existing: Option<(i64, f64, f64)> = conn
            .query_row(
                "SELECT sample_count, running_mean, running_m2 FROM cortex_baselines
                 WHERE entity_id = ?1 AND metric = ?2 AND window = 'day'",
                params![entity_id, metric],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();

        if let Some((n, mean, m2)) = existing {
            // Welford's update
            let n_new = n + 1;
            let delta = value - mean;
            let new_mean = mean + delta / n_new as f64;
            let delta2 = value - new_mean;
            let new_m2 = m2 + delta * delta2;

            let _ = conn.execute(
                "UPDATE cortex_baselines SET
                    sample_count = ?1, running_mean = ?2, running_m2 = ?3,
                    last_value = ?4, last_updated_ts = ?5
                 WHERE entity_id = ?6 AND metric = ?7 AND window = 'day'",
                params![n_new, new_mean, new_m2, value, now, entity_id, metric],
            );
        } else {
            // First observation
            let _ = conn.execute(
                "INSERT INTO cortex_baselines
                    (entity_id, metric, window, sample_count, running_mean, running_m2, last_value, last_updated_ts)
                 VALUES (?1, ?2, 'day', 1, ?3, 0.0, ?3, ?4)",
                params![entity_id, metric, value, now],
            );
        }
    }

    /// Check all baselines for deviations. Returns attention items.
    ///
    /// Called from the think cycle (every 60s). Only flags baselines
    /// that have enough samples and significant deviation.
    pub fn check_deviations(&self, conn: &Connection) -> Vec<AttentionItem> {
        let now = now_ts();
        let mut items = Vec::new();

        // Get all baselines with enough samples
        let mut stmt = match conn.prepare(
            "SELECT entity_id, metric, sample_count, running_mean, running_m2, last_value
             FROM cortex_baselines
             WHERE sample_count >= ?1 AND last_updated_ts > ?2",
        ) {
            Ok(s) => s,
            Err(_) => return items,
        };

        // Only check baselines updated in the last day
        let cutoff = now - 86400.0;
        let rows: Vec<Baseline> = stmt
            .query_map(params![self.min_samples, cutoff], |row| {
                let entity_id: String = row.get(0)?;
                let metric: String = row.get(1)?;
                let n: i64 = row.get(2)?;
                let mean: f64 = row.get(3)?;
                let m2: f64 = row.get(4)?;
                let last_value: f64 = row.get(5)?;
                let variance = if n > 1 { m2 / (n - 1) as f64 } else { 0.0 };
                Ok(Baseline {
                    entity_id,
                    metric,
                    sample_count: n,
                    mean,
                    stddev: variance.sqrt(),
                    last_value,
                })
            })
            .ok()
            .map(|r| r.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();

        for baseline in &rows {
            if baseline.stddev < 0.01 {
                // No variance — can't detect deviations
                continue;
            }

            let z_score = (baseline.last_value - baseline.mean) / baseline.stddev;

            if z_score.abs() >= self.z_threshold {
                let direction = if z_score > 0.0 {
                    DeviationDirection::AboveNormal
                } else {
                    DeviationDirection::BelowNormal
                };

                let (summary, action) = format_deviation(
                    &baseline.entity_id,
                    &baseline.metric,
                    baseline.last_value,
                    baseline.mean,
                    baseline.stddev,
                    &direction,
                );

                items.push(AttentionItem {
                    rule_name: "baseline_deviation",
                    priority: (0.4 + 0.1 * z_score.abs().min(3.0)).min(0.9),
                    summary,
                    suggested_action: action,
                    entity_ids: vec![baseline.entity_id.clone()],
                    systems_involved: infer_systems(&baseline.metric),
                });
            }
        }

        // Sort by priority, return top 3
        items.sort_by(|a, b| b.priority.partial_cmp(&a.priority).unwrap_or(std::cmp::Ordering::Equal));
        items.truncate(3);
        items
    }

    /// Update baselines from recent pulse data.
    ///
    /// Called periodically (every hour). Aggregates pulse counts into
    /// per-entity-per-metric measurements.
    pub fn update_from_pulses(&self, conn: &Connection) {
        let now = now_ts();
        let one_day_ago = now - 86400.0;

        // Count pulses per entity per event_type in the last 24h
        let mut stmt = match conn.prepare(
            "SELECT pe.entity_id, p.event_type, COUNT(*)
             FROM cortex_pulses p
             JOIN cortex_pulse_entities pe ON pe.pulse_id = p.id
             WHERE p.ts >= ?1
             GROUP BY pe.entity_id, p.event_type",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        let counts: Vec<(String, String, f64)> = stmt
            .query_map(params![one_day_ago], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as f64,
                ))
            })
            .ok()
            .map(|r| r.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();

        for (entity_id, event_type, count) in counts {
            let metric = format!("{}_per_day", event_type);
            self.record(conn, &entity_id, &metric, count);
        }

        // Activity-level baselines from focus detector
        // (duration of current activity type)
        // These come in via separate calls from the focus detector
    }
}

// ── Pulse-to-Baseline Extractors ─────────────────────────────────────

/// Auto-extract baseline metrics from a tool result.
///
/// Called after every pulse ingestion. Discovers metrics organically
/// from the data — no hardcoded metric list.
pub fn extract_metrics_from_pulse(
    conn: &Connection,
    tracker: &BaselineTracker,
    event_type: &str,
    entity_ids: &[String],
    _metadata: &serde_json::Value,
) {
    // For each entity involved in this pulse, record the event
    for entity_id in entity_ids {
        // Track event frequency per entity
        let metric = format!("{}_frequency", event_type);
        tracker.record(conn, entity_id, &metric, 1.0);
    }

    // Track time-of-day patterns
    let hour = (now_ts() as u64 / 3600) % 24;
    let day_metric = format!("{}_hour", event_type);
    for entity_id in entity_ids {
        tracker.record(conn, entity_id, &day_metric, hour as f64);
    }
}

// ── Response Time Tracking ───────────────────────────────────────────

/// Track response time between related events.
///
/// e.g., "time from email_received to email_sent for person:sarah"
/// Called when we detect a "response" event that matches a prior event.
pub fn track_response_time(
    conn: &Connection,
    tracker: &BaselineTracker,
    entity_id: &str,
    trigger_event: &str,
    response_event: &str,
    elapsed_seconds: f64,
) {
    let metric = format!("response_{}_{}", trigger_event, response_event);
    tracker.record(conn, entity_id, &metric, elapsed_seconds);
}

// ── Formatting ──────────────────────────────────────────────────────

fn format_deviation(
    entity_id: &str,
    metric: &str,
    current: f64,
    mean: f64,
    _stddev: f64,
    direction: &DeviationDirection,
) -> (String, String) {
    let entity_name = entity_id
        .split(':')
        .nth(1)
        .unwrap_or(entity_id)
        .replace('-', " ");

    let metric_readable = metric
        .replace('_', " ")
        .replace("per day", "/day")
        .replace("frequency", "activity");

    let dir_word = match direction {
        DeviationDirection::AboveNormal => "higher",
        DeviationDirection::BelowNormal => "lower",
    };

    let summary = format!(
        "{} — {} is {:.0} today, {} than usual ({:.1} avg)",
        entity_name, metric_readable, current, dir_word, mean
    );

    let action = match direction {
        DeviationDirection::AboveNormal => {
            format!("Unusually high {} for {} — worth checking", metric_readable, entity_name)
        }
        DeviationDirection::BelowNormal => {
            format!("Unusually low {} for {} — might be blocked or forgotten", metric_readable, entity_name)
        }
    };

    (summary, action)
}

fn infer_systems(metric: &str) -> Vec<&'static str> {
    let mut systems = Vec::new();
    if metric.contains("email") {
        systems.push("email");
    }
    if metric.contains("ticket") || metric.contains("transition") {
        systems.push("jira");
    }
    if metric.contains("commit") || metric.contains("pr_") {
        systems.push("git");
    }
    if metric.contains("meeting") || metric.contains("calendar") {
        systems.push("calendar");
    }
    if metric.contains("file") {
        systems.push("filesystem");
    }
    if systems.is_empty() {
        systems.push("system");
    }
    systems
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
