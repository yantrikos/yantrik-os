//! Right to Remain Silent — dismissal tracking and proactivity dampening.
//!
//! Tracks every proactive intervention outcome (accepted, dismissed, ignored,
//! caused_frustration) and learns when silence is the correct choice.
//!
//! Key features:
//! - Dismissal pattern tracking by: time of day, intervention type, topic/source
//! - Adaptive silence baseline: increases when dismissals are frequent
//! - Interruptibility estimation: focused work → high bar, idle → low bar
//! - Per-source dampening: reduce suggestions of a type that gets dismissed often
//! - Frustration detection: back-to-back dismissals trigger extended quiet period

use std::collections::HashMap;
use rusqlite::{params, Connection};

// ── Outcome Tracking ────────────────────────────────────────────────────────

/// Possible outcomes for a proactive intervention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterventionOutcome {
    /// User engaged with the content (clicked, responded, followed suggestion).
    Accepted,
    /// User explicitly dismissed (X button, swipe away).
    Dismissed,
    /// User saw it but didn't interact (timed out).
    Ignored,
    /// User opened/read it after initial ignore (delayed acceptance).
    OpenedLater,
    /// User expressed frustration ("stop", "not now", rapid multi-dismiss).
    CausedFrustration,
}

impl InterventionOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Dismissed => "dismissed",
            Self::Ignored => "ignored",
            Self::OpenedLater => "opened_later",
            Self::CausedFrustration => "frustration",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "accepted" => Self::Accepted,
            "dismissed" => Self::Dismissed,
            "ignored" => Self::Ignored,
            "opened_later" => Self::OpenedLater,
            "frustration" => Self::CausedFrustration,
            _ => Self::Ignored,
        }
    }

    /// Is this a positive outcome?
    pub fn is_positive(&self) -> bool {
        matches!(self, Self::Accepted | Self::OpenedLater)
    }

    /// Is this a negative signal?
    pub fn is_negative(&self) -> bool {
        matches!(self, Self::Dismissed | Self::CausedFrustration)
    }
}

// ── Silence Policy Engine ───────────────────────────────────────────────────

/// The silence policy engine — learns when to shut up.
pub struct SilencePolicy {
    /// Rolling window of recent outcomes (newest first).
    recent_outcomes: Vec<OutcomeRecord>,
    /// Max outcomes to keep in memory.
    window_size: usize,
    /// Current silence baseline (fed to proactive pipeline scorer).
    /// Higher = harder for any candidate to beat silence.
    silence_baseline: f64,
    /// Extended quiet period: if set, no proactive messages until this timestamp.
    quiet_until: f64,
    /// Per-source dampening factors (0.0 = fully dampened, 1.0 = normal).
    source_dampening: HashMap<String, f64>,
}

#[derive(Debug, Clone)]
struct OutcomeRecord {
    source: String,
    outcome: InterventionOutcome,
    hour_of_day: u8,
    timestamp: f64,
}

impl SilencePolicy {
    pub fn new() -> Self {
        Self {
            recent_outcomes: Vec::new(),
            window_size: 50,
            silence_baseline: 0.5,
            quiet_until: 0.0,
            source_dampening: HashMap::new(),
        }
    }

    /// Record an intervention outcome.
    pub fn record_outcome(
        &mut self,
        source: &str,
        outcome: InterventionOutcome,
        conn: &Connection,
    ) {
        let now = now_ts();
        let hour = ((now as i64 % 86400) / 3600) as u8;

        let record = OutcomeRecord {
            source: source.to_string(),
            outcome: outcome.clone(),
            hour_of_day: hour,
            timestamp: now,
        };

        self.recent_outcomes.insert(0, record);
        if self.recent_outcomes.len() > self.window_size {
            self.recent_outcomes.pop();
        }

        // Persist to SQLite
        Self::persist_outcome(conn, source, &outcome, hour);

        // Update policy based on new data
        self.update_policy();
    }

    /// Get the current silence baseline for the proactive scorer.
    pub fn silence_baseline(&self) -> f64 {
        self.silence_baseline
    }

    /// Check if we're in a quiet period.
    pub fn is_quiet_period(&self) -> bool {
        now_ts() < self.quiet_until
    }

    /// Get dampening factor for a source (0.0-1.0).
    pub fn dampening_for(&self, source: &str) -> f64 {
        self.source_dampening.get(source).copied().unwrap_or(1.0)
    }

    /// Estimate user interruptibility based on context.
    ///
    /// Returns 0.0 (deeply focused, do not interrupt) to 1.0 (idle, available).
    pub fn estimate_interruptibility(
        idle_seconds: u64,
        interactions_last_hour: u32,
        current_hour: u32,
    ) -> f64 {
        let mut score: f64 = 0.5; // Neutral baseline

        // Idle time: more idle = more interruptible
        if idle_seconds > 600 {
            score += 0.3; // 10+ minutes idle
        } else if idle_seconds > 120 {
            score += 0.15; // 2+ minutes idle
        } else if idle_seconds < 30 {
            score -= 0.2; // Recently active = probably focused
        }

        // Interaction frequency: lots of activity = user is engaged
        if interactions_last_hour > 10 {
            score -= 0.15; // Very active — don't interrupt workflow
        } else if interactions_last_hour == 0 && idle_seconds > 300 {
            score += 0.1; // No interaction + idle = available
        }

        // Time of day: late night / early morning = lower interruptibility
        if current_hour >= 23 || current_hour < 6 {
            score -= 0.2; // Late night — probably winding down
        } else if (9..=11).contains(&current_hour) || (14..=16).contains(&current_hour) {
            score += 0.05; // Peak work hours — likely at desk
        }

        score.clamp(0.0, 1.0)
    }

    /// Detect frustration: back-to-back dismissals within short window.
    pub fn detect_frustration(&self) -> bool {
        let now = now_ts();
        let recent_dismissals = self.recent_outcomes.iter()
            .take(5)
            .filter(|r| now - r.timestamp < 300.0) // Last 5 minutes
            .filter(|r| r.outcome.is_negative())
            .count();

        recent_dismissals >= 3
    }

    /// Update internal policy based on recent outcome patterns.
    fn update_policy(&mut self) {
        let now = now_ts();

        // Check for frustration → trigger quiet period
        if self.detect_frustration() {
            // 30 minute quiet period
            self.quiet_until = now + 1800.0;
            self.silence_baseline = 1.5; // Very high bar
            tracing::info!("Frustration detected — entering 30min quiet period");
            return;
        }

        // Calculate overall dismissal rate from recent outcomes
        let recent_count = self.recent_outcomes.len();
        if recent_count < 3 {
            return; // Not enough data
        }

        let negative_count = self.recent_outcomes.iter()
            .filter(|r| r.outcome.is_negative())
            .count();
        let positive_count = self.recent_outcomes.iter()
            .filter(|r| r.outcome.is_positive())
            .count();

        let negative_rate = negative_count as f64 / recent_count as f64;

        // Adjust silence baseline based on dismissal rate
        // High dismissal → higher baseline → harder to beat silence
        self.silence_baseline = 0.3 + negative_rate * 1.2;
        self.silence_baseline = self.silence_baseline.clamp(0.2, 1.8);

        // Per-source dampening
        let mut source_counts: HashMap<String, (u32, u32)> = HashMap::new();
        for record in &self.recent_outcomes {
            let entry = source_counts.entry(record.source.clone()).or_insert((0, 0));
            entry.0 += 1; // total
            if record.outcome.is_negative() {
                entry.1 += 1; // negative
            }
        }

        for (source, (total, negative)) in &source_counts {
            if *total >= 3 {
                let dismiss_rate = *negative as f64 / *total as f64;
                // Dampening: 1.0 at 0% dismissal, 0.2 at 80%+ dismissal
                let dampening = (1.0 - dismiss_rate * 1.0).max(0.2);
                self.source_dampening.insert(source.clone(), dampening);
            }
        }

        // Decay quiet period if expired
        if now >= self.quiet_until && self.quiet_until > 0.0 {
            self.quiet_until = 0.0;
            // Gradually reduce silence baseline back to normal
            self.silence_baseline = (self.silence_baseline * 0.8).max(0.3);
        }

        // Time-of-day pattern: if most dismissals happen at certain hours,
        // boost silence baseline during those hours
        let current_hour = ((now as i64 % 86400) / 3600) as u8;
        let hour_dismissals = self.recent_outcomes.iter()
            .filter(|r| r.hour_of_day == current_hour && r.outcome.is_negative())
            .count();
        let hour_total = self.recent_outcomes.iter()
            .filter(|r| r.hour_of_day == current_hour)
            .count();

        if hour_total >= 3 && hour_dismissals as f64 / hour_total as f64 > 0.6 {
            self.silence_baseline += 0.2; // Extra quiet during this hour
        }
    }

    /// Load historical data from SQLite to warm the policy on startup.
    pub fn load_history(&mut self, conn: &Connection) {
        let since = now_ts() - 7.0 * 86400.0; // Last 7 days

        let mut stmt = match conn.prepare(
            "SELECT source, outcome, hour_of_day, recorded_at
             FROM silence_outcomes
             WHERE recorded_at >= ?1
             ORDER BY recorded_at DESC
             LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        if let Ok(rows) = stmt.query_map(params![since, self.window_size as i64], |row| {
            Ok(OutcomeRecord {
                source: row.get(0)?,
                outcome: InterventionOutcome::from_str(&row.get::<_, String>(1)?),
                hour_of_day: row.get::<_, i64>(2)? as u8,
                timestamp: row.get(3)?,
            })
        }) {
            self.recent_outcomes = rows.flatten().collect();
        }

        self.update_policy();
    }

    fn persist_outcome(conn: &Connection, source: &str, outcome: &InterventionOutcome, hour: u8) {
        let _ = conn.execute(
            "INSERT INTO silence_outcomes (source, outcome, hour_of_day, recorded_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![source, outcome.as_str(), hour as i64, now_ts()],
        );
    }

    /// Create required tables.
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS silence_outcomes (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                source      TEXT NOT NULL,
                outcome     TEXT NOT NULL,
                hour_of_day INTEGER NOT NULL,
                recorded_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_silence_source ON silence_outcomes(source);
            CREATE INDEX IF NOT EXISTS idx_silence_time ON silence_outcomes(recorded_at);
            CREATE INDEX IF NOT EXISTS idx_silence_hour ON silence_outcomes(hour_of_day);",
        )
        .expect("failed to create silence_outcomes table");
    }

    /// Get dismissal statistics for the dashboard.
    pub fn stats(conn: &Connection, since_hours: f64) -> SilenceStats {
        let since = now_ts() - since_hours * 3600.0;

        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM silence_outcomes WHERE recorded_at >= ?1",
            params![since], |r| r.get(0),
        ).unwrap_or(0);

        let accepted: i64 = conn.query_row(
            "SELECT COUNT(*) FROM silence_outcomes WHERE recorded_at >= ?1 AND outcome = 'accepted'",
            params![since], |r| r.get(0),
        ).unwrap_or(0);

        let dismissed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM silence_outcomes WHERE recorded_at >= ?1 AND outcome = 'dismissed'",
            params![since], |r| r.get(0),
        ).unwrap_or(0);

        let frustrated: i64 = conn.query_row(
            "SELECT COUNT(*) FROM silence_outcomes WHERE recorded_at >= ?1 AND outcome = 'frustration'",
            params![since], |r| r.get(0),
        ).unwrap_or(0);

        let acceptance_rate = if total > 0 { accepted as f64 / total as f64 } else { 0.0 };
        let dismissal_rate = if total > 0 { dismissed as f64 / total as f64 } else { 0.0 };

        SilenceStats {
            total_interventions: total as u64,
            accepted: accepted as u64,
            dismissed: dismissed as u64,
            frustrated: frustrated as u64,
            acceptance_rate,
            dismissal_rate,
        }
    }

    /// Get per-source dismissal rates for the dashboard.
    pub fn source_stats(conn: &Connection, since_hours: f64) -> Vec<SourceSilenceStats> {
        let since = now_ts() - since_hours * 3600.0;

        let mut stmt = match conn.prepare(
            "SELECT source,
                    COUNT(*) as total,
                    SUM(CASE WHEN outcome = 'accepted' THEN 1 ELSE 0 END) as accepted,
                    SUM(CASE WHEN outcome = 'dismissed' THEN 1 ELSE 0 END) as dismissed
             FROM silence_outcomes
             WHERE recorded_at >= ?1
             GROUP BY source
             ORDER BY total DESC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![since], |row| {
            let total: i64 = row.get(1)?;
            let accepted: i64 = row.get(2)?;
            let dismissed: i64 = row.get(3)?;
            Ok(SourceSilenceStats {
                source: row.get(0)?,
                total: total as u64,
                accepted: accepted as u64,
                dismissed: dismissed as u64,
                acceptance_rate: if total > 0 { accepted as f64 / total as f64 } else { 0.0 },
                dismissal_rate: if total > 0 { dismissed as f64 / total as f64 } else { 0.0 },
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }
}

/// Aggregate silence/dismissal stats.
#[derive(Debug, Clone)]
pub struct SilenceStats {
    pub total_interventions: u64,
    pub accepted: u64,
    pub dismissed: u64,
    pub frustrated: u64,
    pub acceptance_rate: f64,
    pub dismissal_rate: f64,
}

/// Per-source silence stats.
#[derive(Debug, Clone)]
pub struct SourceSilenceStats {
    pub source: String,
    pub total: u64,
    pub accepted: u64,
    pub dismissed: u64,
    pub acceptance_rate: f64,
    pub dismissal_rate: f64,
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
        SilencePolicy::ensure_table(&conn);
        conn
    }

    #[test]
    fn outcome_tracking_basic() {
        let conn = setup();
        let mut policy = SilencePolicy::new();

        policy.record_outcome("check_in", InterventionOutcome::Accepted, &conn);
        policy.record_outcome("check_in", InterventionOutcome::Dismissed, &conn);
        policy.record_outcome("weather", InterventionOutcome::Accepted, &conn);

        assert_eq!(policy.recent_outcomes.len(), 3);
    }

    #[test]
    fn dismissal_increases_silence_baseline() {
        let conn = setup();
        let mut policy = SilencePolicy::new();

        let initial_baseline = policy.silence_baseline();

        // Record many dismissals
        for _ in 0..10 {
            policy.record_outcome("check_in", InterventionOutcome::Dismissed, &conn);
        }

        assert!(policy.silence_baseline() > initial_baseline,
            "Baseline should increase after dismissals: {} vs {}",
            policy.silence_baseline(), initial_baseline);
    }

    #[test]
    fn frustration_triggers_quiet_period() {
        let conn = setup();
        let mut policy = SilencePolicy::new();

        // Rapid dismissals → frustration
        for _ in 0..4 {
            policy.record_outcome("annoying", InterventionOutcome::CausedFrustration, &conn);
        }

        assert!(policy.is_quiet_period());
        assert!(policy.silence_baseline() >= 1.0,
            "Baseline should be very high after frustration: {}",
            policy.silence_baseline());
    }

    #[test]
    fn per_source_dampening() {
        let conn = setup();
        let mut policy = SilencePolicy::new();

        // check_in gets mostly dismissed
        for _ in 0..5 {
            policy.record_outcome("check_in", InterventionOutcome::Dismissed, &conn);
        }
        // weather gets mostly accepted
        for _ in 0..5 {
            policy.record_outcome("weather", InterventionOutcome::Accepted, &conn);
        }

        let check_in_dampening = policy.dampening_for("check_in");
        let weather_dampening = policy.dampening_for("weather");

        assert!(check_in_dampening < weather_dampening,
            "check_in ({:.2}) should be dampened more than weather ({:.2})",
            check_in_dampening, weather_dampening);
    }

    #[test]
    fn interruptibility_estimation() {
        // User is idle for 10 minutes at 10am
        let idle_interruptibility = SilencePolicy::estimate_interruptibility(600, 0, 10);
        // User is actively working at 10am
        let active_interruptibility = SilencePolicy::estimate_interruptibility(10, 15, 10);
        // Late night
        let night_interruptibility = SilencePolicy::estimate_interruptibility(300, 0, 2);

        assert!(idle_interruptibility > active_interruptibility,
            "Idle ({:.2}) should be more interruptible than active ({:.2})",
            idle_interruptibility, active_interruptibility);
        assert!(idle_interruptibility > night_interruptibility,
            "Daytime idle ({:.2}) should be more interruptible than late night ({:.2})",
            idle_interruptibility, night_interruptibility);
    }

    #[test]
    fn stats_from_database() {
        let conn = setup();
        let mut policy = SilencePolicy::new();

        policy.record_outcome("check_in", InterventionOutcome::Accepted, &conn);
        policy.record_outcome("check_in", InterventionOutcome::Dismissed, &conn);
        policy.record_outcome("weather", InterventionOutcome::Accepted, &conn);
        policy.record_outcome("weather", InterventionOutcome::Accepted, &conn);

        let stats = SilencePolicy::stats(&conn, 1.0);
        assert_eq!(stats.total_interventions, 4);
        assert_eq!(stats.accepted, 3);
        assert_eq!(stats.dismissed, 1);
        assert!((stats.acceptance_rate - 0.75).abs() < 0.01);

        let source_stats = SilencePolicy::source_stats(&conn, 1.0);
        assert_eq!(source_stats.len(), 2);
    }

    #[test]
    fn history_loads_on_startup() {
        let conn = setup();

        // Simulate historical data
        for _ in 0..5 {
            let _ = conn.execute(
                "INSERT INTO silence_outcomes (source, outcome, hour_of_day, recorded_at)
                 VALUES ('check_in', 'dismissed', 14, ?1)",
                params![now_ts()],
            );
        }

        let mut policy = SilencePolicy::new();
        policy.load_history(&conn);

        assert_eq!(policy.recent_outcomes.len(), 5);
        // After loading 5 dismissals, baseline should be elevated
        assert!(policy.silence_baseline() > 0.5);
    }
}
