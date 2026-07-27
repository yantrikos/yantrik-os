//! Native Brain Detectors — pure data-driven signal generators.
//!
//! Unlike instincts (which are heuristic rules evaluated against CompanionState),
//! detectors query the database directly for statistical anomalies, pattern breaks,
//! and forward projections. They emit BrainCandidates with native signal typing.
//!
//! # Detectors
//!
//! 1. **PatternBreakDetector**: detects what STOPPED happening — a friend who
//!    usually messages weekly went silent, a morning routine was skipped 3 days.
//!
//! 2. **BaselineDeviationDetector**: statistical anomaly detection using Welford
//!    online algorithm — spending spikes, sleep pattern changes, unusual gaps.
//!
//! 3. **ForwardProjectionDetector**: linear extrapolation of trends to anticipate
//!    future problems — deadline velocity, budget burn rate, inbox growth.
//!
//! All detectors operate on two new tables:
//! - `entity_expectations`: tracks expected event intervals per entity
//! - `entity_metric_baselines`: tracks rolling statistics per entity/metric

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::brain::{BrainCandidate, CandidateSource, SignalType};

// ══════════════════════════════════════════════════════════════════════════════
// § 1  Schema
// ══════════════════════════════════════════════════════════════════════════════

/// Ensure detector tables exist. Called during DB initialization.
pub fn ensure_detector_tables(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS entity_expectations (
            entity_id       TEXT    NOT NULL,
            expectation_kind TEXT   NOT NULL,
            expected_interval_sec REAL NOT NULL DEFAULT 86400.0,
            confidence      REAL   NOT NULL DEFAULT 0.3,
            last_seen_at    REAL   NOT NULL DEFAULT 0.0,
            miss_count      INTEGER NOT NULL DEFAULT 0,
            -- Welford stats for interval learning
            n               INTEGER NOT NULL DEFAULT 0,
            mean_interval   REAL   NOT NULL DEFAULT 0.0,
            m2_interval     REAL   NOT NULL DEFAULT 0.0,
            created_at      REAL   NOT NULL,
            updated_at      REAL   NOT NULL,
            PRIMARY KEY (entity_id, expectation_kind)
        );

        CREATE TABLE IF NOT EXISTS entity_metric_baselines (
            entity_id       TEXT    NOT NULL,
            metric_name     TEXT    NOT NULL,
            window_kind     TEXT    NOT NULL DEFAULT 'rolling',
            n               INTEGER NOT NULL DEFAULT 0,
            mean            REAL   NOT NULL DEFAULT 0.0,
            m2              REAL   NOT NULL DEFAULT 0.0,
            last_value      REAL   NOT NULL DEFAULT 0.0,
            last_seen_ts    REAL   NOT NULL DEFAULT 0.0,
            updated_at      REAL   NOT NULL,
            PRIMARY KEY (entity_id, metric_name, window_kind)
        );

        CREATE INDEX IF NOT EXISTS idx_expectations_last_seen
            ON entity_expectations(last_seen_at);
        CREATE INDEX IF NOT EXISTS idx_baselines_metric
            ON entity_metric_baselines(metric_name);
        ",
    )
    .ok();
}

// ══════════════════════════════════════════════════════════════════════════════
// § 2  PatternBreakDetector
// ══════════════════════════════════════════════════════════════════════════════

/// Detects what STOPPED happening. Scans entity_expectations for events that
/// are overdue based on their learned interval. Emits PredictionError signals.
///
/// Examples:
/// - "Your mom usually calls on Sundays — no call this week"
/// - "You've skipped your morning walk for 3 days"
/// - "Team standup hasn't happened today (usually at 9am)"
pub struct PatternBreakDetector;

impl PatternBreakDetector {
    /// Scan for pattern breaks and return BrainCandidates.
    pub fn detect(conn: &Connection, now: f64) -> Vec<BrainCandidate> {
        let mut candidates = Vec::new();

        let mut stmt = match conn.prepare(
            "SELECT entity_id, expectation_kind, expected_interval_sec,
                    confidence, last_seen_at, miss_count, n, mean_interval, m2_interval
             FROM entity_expectations
             WHERE confidence > 0.3
             ORDER BY last_seen_at ASC
             LIMIT 200"
        ) {
            Ok(s) => s,
            Err(_) => return candidates,
        };

        let rows = stmt.query_map([], |row| {
            Ok(Expectation {
                entity_id: row.get(0)?,
                kind: row.get(1)?,
                expected_interval: row.get(2)?,
                confidence: row.get(3)?,
                last_seen_at: row.get(4)?,
                miss_count: row.get(5)?,
                n: row.get(6)?,
                mean_interval: row.get(7)?,
                m2_interval: row.get(8)?,
            })
        });

        let rows = match rows {
            Ok(r) => r,
            Err(_) => return candidates,
        };

        for row in rows.flatten() {
            // Use learned mean if we have enough data, otherwise configured interval
            let effective_interval = if row.n >= 5 && row.mean_interval > 0.0 {
                row.mean_interval
            } else {
                row.expected_interval
            };

            let elapsed = now - row.last_seen_at;
            if effective_interval <= 0.0 {
                continue;
            }

            // lateness: how many intervals overdue (0 = on time, 1 = one full interval late)
            let lateness = (elapsed - effective_interval) / effective_interval;

            if lateness < 0.5 {
                continue; // not late enough to care
            }

            // Compute standard deviation for confidence-weighted urgency
            let stddev = if row.n >= 3 && row.m2_interval > 0.0 {
                (row.m2_interval / (row.n as f64 - 1.0)).sqrt()
            } else {
                effective_interval * 0.3 // assume 30% CV if unknown
            };

            // Z-score: how many standard deviations late
            let z = if stddev > 0.0 {
                (elapsed - effective_interval) / stddev
            } else {
                lateness * 2.0
            };

            // Urgency scales with z-score, dampened by confidence
            let urgency = (z / 4.0).clamp(0.0, 1.0) * row.confidence;

            if urgency < 0.2 {
                continue;
            }

            let human_interval = humanize_interval(effective_interval);
            let human_elapsed = humanize_interval(elapsed);

            let reason = format!(
                "Pattern break: {} usually happens every {} but it's been {}",
                row.kind, human_interval, human_elapsed,
            );

            let suggested = format!(
                "{} hasn't happened in {} (usually every {})",
                format_entity_kind(&row.entity_id, &row.kind),
                human_elapsed, human_interval,
            );

            candidates.push(BrainCandidate {
                candidate_id: format!("pb:{}:{}:{:.0}", row.entity_id, row.kind, now),
                source: CandidateSource::Detector {
                    detector_name: "pattern_break".to_string(),
                },
                signal_type: if row.miss_count >= 3 {
                    SignalType::Tension // persistent absence becomes tension
                } else {
                    SignalType::PredictionError
                },
                raw_urgency: urgency,
                brain_score: 0.0,
                reason,
                suggested_message: suggested,
                action: None,
                context: serde_json::json!({
                    "entity_id": row.entity_id,
                    "expectation_kind": row.kind,
                    "expected_interval_sec": effective_interval,
                    "elapsed_sec": elapsed,
                    "lateness": lateness,
                    "z_score": z,
                    "miss_count": row.miss_count,
                    "confidence": row.confidence,
                }),
                cooldown_key: format!("pattern_break:{}:{}", row.entity_id, row.kind),
                orientation: None,
                created_at: now,
            });
        }

        candidates
    }

    /// Record that an expected event occurred. Updates interval statistics
    /// using Welford's online algorithm and resets the miss counter.
    pub fn record_event(conn: &Connection, entity_id: &str, kind: &str, now: f64) {
        // Get current expectation
        let existing: Option<(f64, i64, f64, f64)> = conn
            .query_row(
                "SELECT last_seen_at, n, mean_interval, m2_interval
                 FROM entity_expectations
                 WHERE entity_id = ?1 AND expectation_kind = ?2",
                rusqlite::params![entity_id, kind],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok();

        match existing {
            Some((last_seen, n, mean, m2)) if last_seen > 0.0 => {
                let interval = now - last_seen;
                if interval < 10.0 {
                    return; // ignore rapid re-fires
                }

                // Welford update
                let new_n = n + 1;
                let delta = interval - mean;
                let new_mean = mean + delta / new_n as f64;
                let delta2 = interval - new_mean;
                let new_m2 = m2 + delta * delta2;

                // Update confidence: rises with more observations, caps at 0.95
                let conf = (1.0 - (-0.15 * new_n as f64).exp()).min(0.95);

                conn.execute(
                    "UPDATE entity_expectations
                     SET last_seen_at = ?1, miss_count = 0,
                         n = ?2, mean_interval = ?3, m2_interval = ?4,
                         expected_interval_sec = ?3, confidence = ?5, updated_at = ?1
                     WHERE entity_id = ?6 AND expectation_kind = ?7",
                    rusqlite::params![now, new_n, new_mean, new_m2, conf, entity_id, kind],
                )
                .ok();
            }
            Some(_) => {
                // First real event — just update last_seen
                conn.execute(
                    "UPDATE entity_expectations
                     SET last_seen_at = ?1, updated_at = ?1
                     WHERE entity_id = ?2 AND expectation_kind = ?3",
                    rusqlite::params![now, entity_id, kind],
                )
                .ok();
            }
            None => {
                // New expectation — insert with default interval
                conn.execute(
                    "INSERT INTO entity_expectations
                     (entity_id, expectation_kind, expected_interval_sec, confidence,
                      last_seen_at, miss_count, n, mean_interval, m2_interval, created_at, updated_at)
                     VALUES (?1, ?2, 86400.0, 0.3, ?3, 0, 0, 0.0, 0.0, ?3, ?3)",
                    rusqlite::params![entity_id, kind, now],
                )
                .ok();
            }
        }
    }

    /// Increment miss count for expectations that haven't been seen.
    /// Called during brain tick to track consecutive misses.
    pub fn increment_misses(conn: &Connection, now: f64) {
        // Only increment for expectations that are at least 1 full interval overdue
        conn.execute(
            "UPDATE entity_expectations
             SET miss_count = miss_count + 1, updated_at = ?1
             WHERE confidence > 0.3
               AND (?1 - last_seen_at) > expected_interval_sec * 1.5
               AND updated_at < ?1 - 3600.0",  // at most once per hour
            rusqlite::params![now],
        )
        .ok();
    }

    /// Decay confidence for stale expectations (not seen in 60+ days).
    pub fn decay_stale(conn: &Connection, now: f64) {
        conn.execute(
            "UPDATE entity_expectations
             SET confidence = confidence * 0.9, updated_at = ?1
             WHERE (?1 - last_seen_at) > 5184000.0
               AND confidence > 0.1",
            rusqlite::params![now],
        )
        .ok();

        // Prune very low confidence
        conn.execute(
            "DELETE FROM entity_expectations WHERE confidence < 0.05",
            [],
        )
        .ok();
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// § 3  BaselineDeviationDetector
// ══════════════════════════════════════════════════════════════════════════════

/// Detects statistical anomalies using Welford online statistics. When a new
/// observation is far from the running mean (high z-score), it emits a
/// PredictionError signal.
///
/// Examples:
/// - "Your spending today is 3x your daily average"
/// - "You went to bed 2 hours later than usual"
/// - "Unusually high number of emails today"
pub struct BaselineDeviationDetector;

impl BaselineDeviationDetector {
    /// Record a new observation for an entity/metric pair.
    /// Uses Welford's online algorithm for incremental mean/variance.
    /// Returns the deviation score if n >= 5, or None.
    pub fn observe(
        conn: &Connection,
        entity_id: &str,
        metric_name: &str,
        value: f64,
        now: f64,
    ) -> Option<DeviationResult> {
        let existing: Option<(i64, f64, f64)> = conn
            .query_row(
                "SELECT n, mean, m2 FROM entity_metric_baselines
                 WHERE entity_id = ?1 AND metric_name = ?2 AND window_kind = 'rolling'",
                rusqlite::params![entity_id, metric_name],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();

        let (n, mean, m2) = existing.unwrap_or((0, 0.0, 0.0));

        // Welford update
        let new_n = n + 1;
        let delta = value - mean;
        let new_mean = mean + delta / new_n as f64;
        let delta2 = value - new_mean;
        let new_m2 = m2 + delta * delta2;

        // Upsert
        conn.execute(
            "INSERT INTO entity_metric_baselines
             (entity_id, metric_name, window_kind, n, mean, m2, last_value, last_seen_ts, updated_at)
             VALUES (?1, ?2, 'rolling', ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(entity_id, metric_name, window_kind) DO UPDATE SET
               n = ?3, mean = ?4, m2 = ?5, last_value = ?6, last_seen_ts = ?7, updated_at = ?7",
            rusqlite::params![entity_id, metric_name, new_n, new_mean, new_m2, value, now],
        )
        .ok();

        // Need at least 10 observations for reliable deviation
        if new_n < 10 {
            return None;
        }

        let variance = new_m2 / (new_n as f64 - 1.0);
        let stddev = variance.sqrt();
        if stddev < 1e-9 {
            return None; // no variation
        }

        let z_score = (value - new_mean) / stddev;
        let deviation_score = (z_score.abs() / 3.0).clamp(0.0, 1.0);

        Some(DeviationResult {
            entity_id: entity_id.to_string(),
            metric_name: metric_name.to_string(),
            value,
            mean: new_mean,
            stddev,
            z_score,
            deviation_score,
            n: new_n,
        })
    }

    /// Scan all baselines for deviations and emit BrainCandidates.
    /// Called during brain tick to detect anomalies in recently-updated metrics.
    pub fn detect(conn: &Connection, now: f64, lookback_secs: f64) -> Vec<BrainCandidate> {
        let mut candidates = Vec::new();

        let mut stmt = match conn.prepare(
            "SELECT entity_id, metric_name, n, mean, m2, last_value, last_seen_ts
             FROM entity_metric_baselines
             WHERE window_kind = 'rolling'
               AND n >= 10
               AND last_seen_ts > ?1
             ORDER BY last_seen_ts DESC
             LIMIT 100"
        ) {
            Ok(s) => s,
            Err(_) => return candidates,
        };

        let cutoff = now - lookback_secs;
        let rows = stmt.query_map(rusqlite::params![cutoff], |row| {
            Ok(BaselineRow {
                entity_id: row.get(0)?,
                metric_name: row.get(1)?,
                n: row.get(2)?,
                mean: row.get(3)?,
                m2: row.get(4)?,
                last_value: row.get(5)?,
                last_seen_ts: row.get(6)?,
            })
        });

        let rows = match rows {
            Ok(r) => r,
            Err(_) => return candidates,
        };

        for row in rows.flatten() {
            let variance = row.m2 / (row.n as f64 - 1.0);
            let stddev = variance.sqrt();
            if stddev < 1e-9 {
                continue;
            }

            let z_score = (row.last_value - row.mean) / stddev;
            let deviation_score = (z_score.abs() / 3.0).clamp(0.0, 1.0);

            if deviation_score < 0.5 {
                continue; // not anomalous enough
            }

            let direction = if z_score > 0.0 { "higher" } else { "lower" };
            let reason = format!(
                "Baseline deviation: {} for {} is {:.1}σ {} than usual (value: {:.1}, avg: {:.1})",
                row.metric_name, row.entity_id, z_score.abs(), direction,
                row.last_value, row.mean,
            );

            let suggested = format!(
                "{} is unusually {} — {:.1} vs average {:.1}",
                format_metric(&row.entity_id, &row.metric_name),
                direction, row.last_value, row.mean,
            );

            candidates.push(BrainCandidate {
                candidate_id: format!("bd:{}:{}:{:.0}", row.entity_id, row.metric_name, now),
                source: CandidateSource::Detector {
                    detector_name: "baseline_deviation".to_string(),
                },
                signal_type: SignalType::PredictionError,
                raw_urgency: deviation_score,
                brain_score: 0.0,
                reason,
                suggested_message: suggested,
                action: None,
                context: serde_json::json!({
                    "entity_id": row.entity_id,
                    "metric_name": row.metric_name,
                    "value": row.last_value,
                    "mean": row.mean,
                    "stddev": stddev,
                    "z_score": z_score,
                    "deviation_score": deviation_score,
                    "n": row.n,
                }),
                cooldown_key: format!("deviation:{}:{}", row.entity_id, row.metric_name),
                orientation: None,
                created_at: now,
            });
        }

        candidates
    }
}

/// Result of a single deviation observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviationResult {
    pub entity_id: String,
    pub metric_name: String,
    pub value: f64,
    pub mean: f64,
    pub stddev: f64,
    pub z_score: f64,
    pub deviation_score: f64,
    pub n: i64,
}

// ══════════════════════════════════════════════════════════════════════════════
// § 4  ForwardProjectionDetector
// ══════════════════════════════════════════════════════════════════════════════

/// Projects current trends forward to detect upcoming problems. Uses linear
/// extrapolation and threshold intersection to anticipate issues before they hit.
///
/// Examples:
/// - "At current pace, you'll miss the Friday deadline"
/// - "Monthly spending trend projects to $X over budget"
/// - "Unread emails growing faster than you're processing them"
/// - "Meeting in 2 hours with no prep notes"
pub struct ForwardProjectionDetector;

impl ForwardProjectionDetector {
    /// Scan for projectable trends and emit BrainCandidates.
    pub fn detect(conn: &Connection, now: f64) -> Vec<BrainCandidate> {
        let mut candidates = Vec::new();

        // Projection 1: Deadline velocity
        Self::check_deadline_velocity(conn, now, &mut candidates);

        // Projection 2: Accumulation rate (inbox, tasks, etc.)
        Self::check_accumulation_rate(conn, now, &mut candidates);

        // Projection 3: Upcoming events needing prep
        Self::check_upcoming_prep(conn, now, &mut candidates);

        candidates
    }

    /// Check if task/commitment completion rate will meet deadlines.
    fn check_deadline_velocity(conn: &Connection, now: f64, out: &mut Vec<BrainCandidate>) {
        // Find commitments with deadlines in the future
        let mut stmt = match conn.prepare(
            "SELECT id, action, deadline, status
             FROM commitments
             WHERE deadline > ?1
               AND deadline < ?1 + 604800.0
               AND status IN ('pending', 'in_progress')
             ORDER BY deadline ASC
             LIMIT 20"
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        let rows: Vec<(i64, String, f64, String)> = stmt
            .query_map(rusqlite::params![now], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .ok()
            .map(|r| r.flatten().collect())
            .unwrap_or_default();

        for (id, action, deadline, status) in rows {
            let remaining_secs = deadline - now;
            let remaining_hours = remaining_secs / 3600.0;

            // Simple urgency: closer deadline + still pending = more urgent
            let time_urgency: f64 = if remaining_hours < 2.0 {
                0.95
            } else if remaining_hours < 12.0 {
                0.75
            } else if remaining_hours < 48.0 {
                0.5
            } else {
                0.3
            };

            // If status is still "pending" (not started), boost urgency
            let status_boost: f64 = if status == "pending" { 1.3 } else { 1.0 };
            let urgency = (time_urgency * status_boost).min(1.0);

            if urgency < 0.4 {
                continue;
            }

            let human_remaining = humanize_interval(remaining_secs);

            out.push(BrainCandidate {
                candidate_id: format!("fp:deadline:{}:{:.0}", id, now),
                source: CandidateSource::Detector {
                    detector_name: "forward_projection".to_string(),
                },
                signal_type: if remaining_hours < 12.0 {
                    SignalType::Tension
                } else {
                    SignalType::Opportunity // still time to act
                },
                raw_urgency: urgency,
                brain_score: 0.0,
                reason: format!(
                    "Deadline approaching: \"{}\" due in {} (status: {})",
                    truncate(&action, 60), human_remaining, status,
                ),
                suggested_message: format!(
                    "\"{}\" is due in {} and still {}",
                    truncate(&action, 50), human_remaining, status,
                ),
                action: None,
                context: serde_json::json!({
                    "projection_type": "deadline_velocity",
                    "commitment_id": id,
                    "deadline": deadline,
                    "remaining_hours": remaining_hours,
                    "status": status,
                }),
                cooldown_key: format!("deadline:{}", id),
                orientation: None,
                created_at: now,
            });
        }
    }

    /// Check for growing backlogs (unread emails, pending tasks, etc.).
    fn check_accumulation_rate(conn: &Connection, now: f64, out: &mut Vec<BrainCandidate>) {
        // Check attention items accumulation
        let pending_attention: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM attention_items WHERE handled = 0 AND replied = 0",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Get rate: items added in last 24h vs items handled
        let added_24h: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM attention_items WHERE created_at > ?1",
                rusqlite::params![now - 86400.0],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let handled_24h: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM attention_items WHERE handled = 1 AND updated_at > ?1",
                rusqlite::params![now - 86400.0],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if pending_attention > 5 && added_24h > handled_24h {
            let net_growth = added_24h - handled_24h;
            let urgency = ((pending_attention as f64 / 20.0) + (net_growth as f64 / 10.0))
                .clamp(0.3, 0.9);

            out.push(BrainCandidate {
                candidate_id: format!("fp:inbox:{:.0}", now),
                source: CandidateSource::Detector {
                    detector_name: "forward_projection".to_string(),
                },
                signal_type: SignalType::Tension,
                raw_urgency: urgency,
                brain_score: 0.0,
                reason: format!(
                    "Inbox growing: {} unhandled items, +{} added vs {} handled in 24h",
                    pending_attention, added_24h, handled_24h,
                ),
                suggested_message: format!(
                    "You have {} unhandled messages and they're arriving faster than you're processing them",
                    pending_attention,
                ),
                action: None,
                context: serde_json::json!({
                    "projection_type": "accumulation_rate",
                    "pending": pending_attention,
                    "added_24h": added_24h,
                    "handled_24h": handled_24h,
                    "net_growth": net_growth,
                }),
                cooldown_key: "inbox_accumulation".to_string(),
                orientation: None,
                created_at: now,
            });
        }

        // Check open loops accumulation
        let open_loops: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM life_threads WHERE status IN ('open', 'stalled', 'overdue')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if open_loops > 10 {
            let urgency = (open_loops as f64 / 25.0).clamp(0.3, 0.85);
            out.push(BrainCandidate {
                candidate_id: format!("fp:loops:{:.0}", now),
                source: CandidateSource::Detector {
                    detector_name: "forward_projection".to_string(),
                },
                signal_type: SignalType::Tension,
                raw_urgency: urgency,
                brain_score: 0.0,
                reason: format!("{} open threads/loops — cognitive load risk", open_loops),
                suggested_message: format!(
                    "You have {} open items building up — might be worth reviewing which ones can be closed",
                    open_loops,
                ),
                action: None,
                context: serde_json::json!({
                    "projection_type": "open_loops",
                    "count": open_loops,
                }),
                cooldown_key: "open_loops_pressure".to_string(),
                orientation: None,
                created_at: now,
            });
        }
    }

    /// Check for upcoming calendar events that need preparation.
    fn check_upcoming_prep(conn: &Connection, now: f64, out: &mut Vec<BrainCandidate>) {
        // Look for events in the next 2-6 hours
        let mut stmt = match conn.prepare(
            "SELECT event_id, title, start_ts, attendees
             FROM calendar_events
             WHERE start_ts > ?1 AND start_ts < ?2
             ORDER BY start_ts ASC
             LIMIT 5"
        ) {
            Ok(s) => s,
            Err(_) => return, // table may not exist
        };

        let window_start = now + 7200.0;  // 2 hours from now
        let window_end = now + 21600.0;    // 6 hours from now

        let rows: Vec<(String, String, f64, String)> = stmt
            .query_map(rusqlite::params![window_start, window_end], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, String>(3).unwrap_or_default()))
            })
            .ok()
            .map(|r| r.flatten().collect())
            .unwrap_or_default();

        for (event_id, title, start_ts, attendees) in rows {
            let hours_until = (start_ts - now) / 3600.0;

            // Only flag meetings with attendees (not solo blocks)
            if attendees.is_empty() || attendees == "[]" {
                continue;
            }

            let urgency = if hours_until < 3.0 { 0.65 } else { 0.45 };

            out.push(BrainCandidate {
                candidate_id: format!("fp:prep:{}:{:.0}", event_id, now),
                source: CandidateSource::Detector {
                    detector_name: "forward_projection".to_string(),
                },
                signal_type: SignalType::Opportunity,
                raw_urgency: urgency,
                brain_score: 0.0,
                reason: format!(
                    "Upcoming meeting \"{}\" in {:.1}h — prep opportunity",
                    truncate(&title, 50), hours_until,
                ),
                suggested_message: format!(
                    "\"{}\" starts in {:.0} hours — want me to pull up relevant notes?",
                    truncate(&title, 40), hours_until,
                ),
                action: Some("meeting_prep".to_string()),
                context: serde_json::json!({
                    "projection_type": "meeting_prep",
                    "event_id": event_id,
                    "title": title,
                    "start_ts": start_ts,
                    "hours_until": hours_until,
                }),
                cooldown_key: format!("meeting_prep:{}", event_id),
                orientation: None,
                created_at: now,
            });
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// § 5  Detector Runner (unified entry point)
// ══════════════════════════════════════════════════════════════════════════════

/// Run all detectors and collect their BrainCandidates.
/// Called by the brain loop alongside instinct evaluation.
pub fn run_all_detectors(conn: &Connection, now: f64) -> Vec<BrainCandidate> {
    let mut all = Vec::new();

    // Pattern break detection
    PatternBreakDetector::increment_misses(conn, now);
    all.extend(PatternBreakDetector::detect(conn, now));

    // Baseline deviation (look at metrics updated in last 10 minutes)
    all.extend(BaselineDeviationDetector::detect(conn, now, 600.0));

    // Forward projection
    all.extend(ForwardProjectionDetector::detect(conn, now));

    tracing::debug!(
        pattern_breaks = all.iter().filter(|c| matches!(&c.source, CandidateSource::Detector { detector_name } if detector_name == "pattern_break")).count(),
        deviations = all.iter().filter(|c| matches!(&c.source, CandidateSource::Detector { detector_name } if detector_name == "baseline_deviation")).count(),
        projections = all.iter().filter(|c| matches!(&c.source, CandidateSource::Detector { detector_name } if detector_name == "forward_projection")).count(),
        "Detectors scan complete"
    );

    all
}

// ══════════════════════════════════════════════════════════════════════════════
// § 6  Internal helpers
// ══════════════════════════════════════════════════════════════════════════════

struct Expectation {
    entity_id: String,
    kind: String,
    expected_interval: f64,
    confidence: f64,
    last_seen_at: f64,
    miss_count: i64,
    n: i64,
    mean_interval: f64,
    m2_interval: f64,
}

struct BaselineRow {
    entity_id: String,
    metric_name: String,
    n: i64,
    mean: f64,
    m2: f64,
    last_value: f64,
    #[allow(dead_code)]
    last_seen_ts: f64,
}

/// Format an interval in seconds as a human-readable string.
fn humanize_interval(secs: f64) -> String {
    if secs < 60.0 {
        format!("{:.0} seconds", secs)
    } else if secs < 3600.0 {
        let mins = secs / 60.0;
        if mins < 2.0 { "1 minute".into() } else { format!("{:.0} minutes", mins) }
    } else if secs < 86400.0 {
        let hours = secs / 3600.0;
        if hours < 2.0 { "1 hour".into() } else { format!("{:.0} hours", hours) }
    } else {
        let days = secs / 86400.0;
        if days < 2.0 { "1 day".into() } else { format!("{:.0} days", days) }
    }
}

/// Format an entity + kind for user-facing display.
fn format_entity_kind(entity_id: &str, kind: &str) -> String {
    // Try to make it readable: "contact:mom" + "message" → "Messages from mom"
    if let Some(name) = entity_id.strip_prefix("contact:") {
        match kind {
            "message" | "msg" => format!("Messages from {name}"),
            "call" => format!("Calls from {name}"),
            "email" => format!("Emails from {name}"),
            _ => format!("{kind} from {name}"),
        }
    } else if let Some(routine) = entity_id.strip_prefix("routine:") {
        format!("Your {routine}")
    } else if let Some(app) = entity_id.strip_prefix("app:") {
        format!("{app} usage")
    } else {
        format!("{kind} ({entity_id})")
    }
}

/// Format a metric for user-facing display.
fn format_metric(entity_id: &str, metric_name: &str) -> String {
    if entity_id == "self" || entity_id == "user" {
        match metric_name {
            "daily_spending" => "Your daily spending".into(),
            "sleep_start_hour" => "Your bedtime".into(),
            "emails_per_day" => "Daily email volume".into(),
            "active_hours" => "Your active hours".into(),
            _ => format!("Your {}", metric_name.replace('_', " ")),
        }
    } else {
        format!("{} for {}", metric_name.replace('_', " "), entity_id)
    }
}

/// Truncate a string with ellipsis.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let end = s.char_indices()
            .nth(max.saturating_sub(3))
            .map(|(i, _)| i)
            .unwrap_or(max);
        &s[..end]
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// § 7  Tests
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_detector_tables(&conn);
        // Create minimal tables that detectors query
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS commitments (
                id INTEGER PRIMARY KEY, action TEXT, deadline REAL, status TEXT,
                promisor TEXT, promisee TEXT, confidence REAL, source TEXT,
                evidence_text TEXT, created_at REAL, updated_at REAL,
                completion_evidence TEXT
            );
            CREATE TABLE IF NOT EXISTS attention_items (
                id INTEGER PRIMARY KEY, channel TEXT, sender TEXT, sender_name TEXT,
                subject TEXT, preview TEXT, received_ts REAL, importance TEXT,
                needs_reply INTEGER, replied INTEGER DEFAULT 0, handled INTEGER DEFAULT 0,
                external_id TEXT, context_json TEXT, created_at REAL, updated_at REAL
            );
            CREATE TABLE IF NOT EXISTS life_threads (
                id INTEGER PRIMARY KEY, status TEXT, title TEXT, created_at REAL
            );"
        ).unwrap();
        conn
    }

    #[test]
    fn test_pattern_break_no_expectations() {
        let conn = setup_db();
        let candidates = PatternBreakDetector::detect(&conn, 1000.0);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_pattern_break_event_tracking() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Record first event
        PatternBreakDetector::record_event(&conn, "contact:mom", "call", now);

        // Record subsequent events at ~weekly intervals
        for i in 1..=6 {
            let t = now + (i as f64) * 604800.0; // weekly
            PatternBreakDetector::record_event(&conn, "contact:mom", "call", t);
        }

        // Check: after 6 weekly events, confidence should be reasonable
        let (conf, n): (f64, i64) = conn
            .query_row(
                "SELECT confidence, n FROM entity_expectations WHERE entity_id = 'contact:mom'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(n >= 5, "Should have recorded intervals: n={n}");
        assert!(conf > 0.5, "Confidence should build: {conf}");
    }

    #[test]
    fn test_pattern_break_detection() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Manually insert a well-established expectation
        conn.execute(
            "INSERT INTO entity_expectations VALUES
             ('contact:alice', 'message', 86400.0, 0.8, ?1, 2, 10, 86400.0, 1000.0, ?1, ?1)",
            rusqlite::params![now - 200000.0], // last seen 2.3 days ago, expected daily
        ).unwrap();

        let candidates = PatternBreakDetector::detect(&conn, now);
        assert!(!candidates.is_empty(), "Should detect alice's silence");
        assert_eq!(candidates[0].signal_type, SignalType::PredictionError);
        assert!(candidates[0].raw_urgency > 0.3);
    }

    #[test]
    fn test_baseline_deviation_observe() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Record 15 normal observations (mean ~100, low variance)
        for i in 0..15 {
            let value = 100.0 + (i as f64 % 3.0) - 1.0; // 99-101 range
            BaselineDeviationDetector::observe(&conn, "user", "daily_spending", value, now + i as f64 * 86400.0);
        }

        // Record an anomalous value
        let result = BaselineDeviationDetector::observe(
            &conn, "user", "daily_spending", 300.0, now + 16.0 * 86400.0,
        );

        assert!(result.is_some());
        let r = result.unwrap();
        assert!(r.z_score > 2.0, "300 should be >2σ from mean ~100: z={}", r.z_score);
        assert!(r.deviation_score > 0.5, "Should flag as anomalous: dev={}", r.deviation_score);
    }

    #[test]
    fn test_baseline_deviation_detect() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Build baseline with 15 observations
        for i in 0..15 {
            BaselineDeviationDetector::observe(
                &conn, "user", "sleep_hour", 23.0 + (i as f64 % 2.0) * 0.5,
                now + i as f64 * 86400.0,
            );
        }
        // Anomalous late night
        BaselineDeviationDetector::observe(
            &conn, "user", "sleep_hour", 3.0, now + 15.0 * 86400.0,
        );

        let candidates = BaselineDeviationDetector::detect(&conn, now + 15.0 * 86400.0, 86400.0);
        // Should detect the sleep anomaly
        assert!(!candidates.is_empty(), "Should detect sleep anomaly");
    }

    #[test]
    fn test_forward_projection_deadlines() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Insert a commitment due in 6 hours, still pending
        conn.execute(
            "INSERT INTO commitments (id, action, deadline, status, promisor, promisee, confidence, source, evidence_text, created_at, updated_at)
             VALUES (1, 'Review PR #42', ?1, 'pending', 'user', 'team', 0.9, 'conversation', '', ?2, ?2)",
            rusqlite::params![now + 21600.0, now],
        ).unwrap();

        let candidates = ForwardProjectionDetector::detect(&conn, now);
        assert!(!candidates.is_empty(), "Should flag upcoming deadline");
        assert!(candidates[0].raw_urgency > 0.5);
    }

    #[test]
    fn test_forward_projection_inbox_growth() {
        let conn = setup_db();
        let now = 1_000_000.0;

        // Add 15 unhandled attention items, 10 created in last 24h
        for i in 0..15 {
            let created = if i < 10 { now - 3600.0 } else { now - 172800.0 };
            conn.execute(
                "INSERT INTO attention_items (channel, sender, sender_name, subject, preview, received_ts, importance, needs_reply, handled, external_id, context_json, created_at, updated_at)
                 VALUES ('email', 'test', 'Test', 'Subject', '', ?1, 'normal', 1, 0, '', '{}', ?1, ?1)",
                rusqlite::params![created],
            ).unwrap();
        }

        let candidates = ForwardProjectionDetector::detect(&conn, now);
        let inbox_candidates: Vec<_> = candidates.iter()
            .filter(|c| c.cooldown_key == "inbox_accumulation")
            .collect();
        assert!(!inbox_candidates.is_empty(), "Should detect inbox growth");
    }

    #[test]
    fn test_humanize_interval() {
        assert_eq!(humanize_interval(30.0), "30 seconds");
        assert_eq!(humanize_interval(90.0), "2 minutes");
        assert_eq!(humanize_interval(7200.0), "2 hours");
        assert_eq!(humanize_interval(172800.0), "2 days");
    }

    #[test]
    fn test_run_all_detectors_empty() {
        let conn = setup_db();
        let candidates = run_all_detectors(&conn, 1_000_000.0);
        // Should run without errors, may be empty
        assert!(candidates.len() < 100);
    }
}
