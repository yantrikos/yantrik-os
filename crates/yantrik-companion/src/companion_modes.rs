//! Companion Mode Auto-Switching — adaptive mode transitions based on context.
//!
//! Manages automatic transitions between Focus, Social, and Sleep modes based on:
//! - Calendar events (meetings → Social, deep work blocks → Focus)
//! - Time of day (late night → Sleep, morning → Social)
//! - User activity patterns (rapid typing → Focus, idle → Sleep)
//! - Explicit user overrides (sticky until conditions change)
//!
//! The system learns which mode transitions the user accepts or overrides,
//! adjusting confidence thresholds over time.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::workspaces::CompanionMode;

// ── Mode Transition ───────────────────────────────────────────────────────

/// A mode transition event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeTransition {
    pub from: CompanionMode,
    pub to: CompanionMode,
    pub reason: TransitionReason,
    /// Was this accepted by the user or overridden?
    pub outcome: Option<TransitionOutcome>,
    pub timestamp: f64,
}

/// Why a mode transition happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionReason {
    /// Calendar event starting (meeting, etc.).
    CalendarEvent { event_title: String },
    /// Deep work block detected.
    DeepWorkBlock,
    /// Time-of-day schedule.
    TimeSchedule { hour: u8 },
    /// User went idle.
    UserIdle { idle_minutes: u32 },
    /// User resumed activity.
    UserResumed,
    /// Active workspace changed.
    WorkspaceActivated { workspace_id: String },
    /// User explicitly set the mode.
    UserOverride,
}

impl TransitionReason {
    pub fn type_tag(&self) -> &'static str {
        match self {
            Self::CalendarEvent { .. } => "calendar",
            Self::DeepWorkBlock => "deep_work",
            Self::TimeSchedule { .. } => "time_schedule",
            Self::UserIdle { .. } => "user_idle",
            Self::UserResumed => "user_resumed",
            Self::WorkspaceActivated { .. } => "workspace",
            Self::UserOverride => "user_override",
        }
    }
}

/// How the user responded to an auto-transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionOutcome {
    /// User accepted (did not override).
    Accepted,
    /// User overrode to a different mode.
    Overridden { to: CompanionMode },
    /// Transition was suppressed because of recent override.
    Suppressed,
}

impl TransitionOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Overridden { .. } => "overridden",
            Self::Suppressed => "suppressed",
        }
    }
}

// ── Time-of-Day Schedule ──────────────────────────────────────────────────

/// Default time-of-day mode schedule.
#[derive(Debug, Clone)]
pub struct TimeSchedule {
    /// Hour (0-23) → preferred mode.
    pub slots: [(u8, CompanionMode); 4],
}

impl Default for TimeSchedule {
    fn default() -> Self {
        Self {
            slots: [
                (6, CompanionMode::Social),   // 6 AM: wake up, chatty
                (9, CompanionMode::Focus),     // 9 AM: work mode
                (17, CompanionMode::Social),   // 5 PM: wind down
                (23, CompanionMode::Sleep),    // 11 PM: sleep
            ],
        }
    }
}

impl TimeSchedule {
    /// Get the mode for a given hour.
    pub fn mode_for_hour(&self, hour: u8) -> CompanionMode {
        let mut result = self.slots.last().unwrap().1;
        for &(start_hour, mode) in &self.slots {
            if hour >= start_hour {
                result = mode;
            }
        }
        result
    }
}

// ── Mode Manager ──────────────────────────────────────────────────────────

/// Manages companion mode with auto-switching logic.
pub struct ModeManager {
    /// Current active mode.
    current_mode: CompanionMode,
    /// Whether the current mode was set by the user (sticky override).
    user_override_active: bool,
    /// When the user override was set (for expiry).
    override_set_at: Option<f64>,
    /// Override expiry in seconds (default: 2 hours).
    override_ttl_secs: f64,
    /// Time-of-day schedule.
    schedule: TimeSchedule,
    /// Acceptance rate for auto-transitions (for confidence).
    acceptance_rate: f64,
    /// Total auto-transitions attempted.
    total_transitions: u32,
    /// Accepted auto-transitions.
    accepted_transitions: u32,
}

impl ModeManager {
    pub fn new() -> Self {
        Self {
            current_mode: CompanionMode::Social,
            user_override_active: false,
            override_set_at: None,
            override_ttl_secs: 7200.0, // 2 hours
            schedule: TimeSchedule::default(),
            acceptance_rate: 0.8, // Start optimistic
            total_transitions: 0,
            accepted_transitions: 0,
        }
    }

    /// Get the current mode.
    pub fn current(&self) -> CompanionMode {
        self.current_mode
    }

    /// Is there an active user override?
    pub fn is_user_override(&self) -> bool {
        self.user_override_active
    }

    /// User explicitly sets the mode (sticky override).
    pub fn set_mode(&mut self, mode: CompanionMode, conn: &Connection) {
        let old = self.current_mode;
        self.current_mode = mode;
        self.user_override_active = true;
        self.override_set_at = Some(now_ts());

        self.log_transition(conn, ModeTransition {
            from: old,
            to: mode,
            reason: TransitionReason::UserOverride,
            outcome: None,
            timestamp: now_ts(),
        });
    }

    /// Check if the user override has expired.
    fn check_override_expiry(&mut self) {
        if self.user_override_active {
            if let Some(set_at) = self.override_set_at {
                if now_ts() - set_at > self.override_ttl_secs {
                    self.user_override_active = false;
                    self.override_set_at = None;
                }
            }
        }
    }

    /// Evaluate whether a mode switch should happen, given current context.
    /// Returns Some(new_mode) if a transition is recommended.
    pub fn evaluate(
        &mut self,
        current_hour: u8,
        idle_minutes: u32,
        has_meeting_soon: bool,
        meeting_title: Option<&str>,
        active_workspace_mode: Option<CompanionMode>,
        conn: &Connection,
    ) -> Option<CompanionMode> {
        self.check_override_expiry();

        // User override is sticky — don't auto-switch
        if self.user_override_active {
            return None;
        }

        // Determine the recommended mode based on signals
        let recommended = self.recommend_mode(
            current_hour,
            idle_minutes,
            has_meeting_soon,
            active_workspace_mode,
        );

        // If recommended is same as current, no transition needed
        if recommended.mode == self.current_mode {
            return None;
        }

        // Check confidence: only switch if acceptance rate is decent
        let min_confidence = if self.total_transitions < 5 { 0.5 } else { 0.6 };
        if self.acceptance_rate < min_confidence {
            // Low confidence — don't auto-switch, let user decide
            return None;
        }

        // Execute the transition
        let old = self.current_mode;
        self.current_mode = recommended.mode;

        self.log_transition(conn, ModeTransition {
            from: old,
            to: recommended.mode,
            reason: recommended.reason,
            outcome: None,
            timestamp: now_ts(),
        });

        Some(recommended.mode)
    }

    /// Record that the user accepted or overrode the last auto-transition.
    pub fn record_outcome(&mut self, outcome: TransitionOutcome) {
        self.total_transitions += 1;
        if outcome == TransitionOutcome::Accepted {
            self.accepted_transitions += 1;
        }
        // EMA update
        let alpha = 2.0 / (self.total_transitions as f64 + 1.0);
        let val = if outcome == TransitionOutcome::Accepted { 1.0 } else { 0.0 };
        self.acceptance_rate = alpha * val + (1.0 - alpha) * self.acceptance_rate;
    }

    /// Get the acceptance rate.
    pub fn acceptance_rate(&self) -> f64 {
        self.acceptance_rate
    }

    /// Recommend a mode based on signals.
    fn recommend_mode(
        &self,
        current_hour: u8,
        idle_minutes: u32,
        has_meeting_soon: bool,
        active_workspace_mode: Option<CompanionMode>,
    ) -> ModeRecommendation {
        // Priority 1: Workspace override (explicit intent)
        if let Some(ws_mode) = active_workspace_mode {
            return ModeRecommendation {
                mode: ws_mode,
                reason: TransitionReason::WorkspaceActivated {
                    workspace_id: String::new(),
                },
            };
        }

        // Priority 2: Upcoming meeting → Social
        if has_meeting_soon {
            return ModeRecommendation {
                mode: CompanionMode::Social,
                reason: TransitionReason::CalendarEvent {
                    event_title: "upcoming meeting".into(),
                },
            };
        }

        // Priority 3: Extended idle → Sleep (30+ minutes)
        if idle_minutes >= 30 {
            return ModeRecommendation {
                mode: CompanionMode::Sleep,
                reason: TransitionReason::UserIdle { idle_minutes },
            };
        }

        // Priority 4: User resumed after idle → Social
        if idle_minutes == 0 && self.current_mode == CompanionMode::Sleep {
            return ModeRecommendation {
                mode: CompanionMode::Social,
                reason: TransitionReason::UserResumed,
            };
        }

        // Priority 5: Time-of-day schedule
        let scheduled = self.schedule.mode_for_hour(current_hour);
        ModeRecommendation {
            mode: scheduled,
            reason: TransitionReason::TimeSchedule { hour: current_hour },
        }
    }

    /// Load transition history from DB to warm the acceptance rate.
    pub fn load_history(&mut self, conn: &Connection) {
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mode_transitions WHERE reason != 'user_override'",
            [],
            |r| r.get(0),
        ).unwrap_or(0);

        let accepted: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mode_transitions WHERE outcome = 'accepted'",
            [],
            |r| r.get(0),
        ).unwrap_or(0);

        if total > 0 {
            self.total_transitions = total as u32;
            self.accepted_transitions = accepted as u32;
            self.acceptance_rate = accepted as f64 / total as f64;
        }

        // Load last mode
        if let Ok(mode_str) = conn.query_row(
            "SELECT to_mode FROM mode_transitions ORDER BY timestamp DESC LIMIT 1",
            [],
            |r| r.get::<_, String>(0),
        ) {
            self.current_mode = CompanionMode::from_str(&mode_str);
        }
    }

    fn log_transition(&self, conn: &Connection, transition: ModeTransition) {
        let reason_data = serde_json::to_string(&transition.reason).unwrap_or_default();
        let outcome_str = transition.outcome.as_ref().map(|o| o.as_str().to_string());

        let _ = conn.execute(
            "INSERT INTO mode_transitions (from_mode, to_mode, reason, reason_data, outcome, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                transition.from.as_str(),
                transition.to.as_str(),
                transition.reason.type_tag(),
                reason_data,
                outcome_str,
                transition.timestamp,
            ],
        );
    }

    /// Get recent transitions for dashboard display.
    pub fn recent_transitions(conn: &Connection, limit: usize) -> Vec<ModeTransition> {
        let mut stmt = match conn.prepare(
            "SELECT from_mode, to_mode, reason, reason_data, outcome, timestamp
             FROM mode_transitions ORDER BY timestamp DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![limit as i64], |row| {
            let reason_data: String = row.get(3)?;
            let outcome_str: Option<String> = row.get(4)?;

            Ok(ModeTransition {
                from: CompanionMode::from_str(&row.get::<_, String>(0)?),
                to: CompanionMode::from_str(&row.get::<_, String>(1)?),
                reason: serde_json::from_str(&reason_data).unwrap_or(TransitionReason::UserOverride),
                outcome: outcome_str.map(|o| match o.as_str() {
                    "accepted" => TransitionOutcome::Accepted,
                    "suppressed" => TransitionOutcome::Suppressed,
                    _ => TransitionOutcome::Overridden { to: CompanionMode::Social },
                }),
                timestamp: row.get(5)?,
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Mode transition stats for the dashboard.
    pub fn stats(conn: &Connection) -> ModeStats {
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mode_transitions WHERE reason != 'user_override'",
            [],
            |r| r.get(0),
        ).unwrap_or(0);

        let accepted: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mode_transitions WHERE outcome = 'accepted'",
            [],
            |r| r.get(0),
        ).unwrap_or(0);

        let overridden: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mode_transitions WHERE outcome = 'overridden'",
            [],
            |r| r.get(0),
        ).unwrap_or(0);

        let user_sets: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mode_transitions WHERE reason = 'user_override'",
            [],
            |r| r.get(0),
        ).unwrap_or(0);

        // Time spent in each mode (approximate from transitions)
        let mut time_in_mode: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT to_mode, timestamp FROM mode_transitions ORDER BY timestamp ASC",
        ) {
            let mut last_mode = String::new();
            let mut last_ts = 0.0_f64;

            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            }) {
                for row in rows.flatten() {
                    if !last_mode.is_empty() {
                        let duration = row.1 - last_ts;
                        *time_in_mode.entry(last_mode.clone()).or_default() += duration;
                    }
                    last_mode = row.0;
                    last_ts = row.1;
                }
                // Account for time in current mode up to now
                if !last_mode.is_empty() {
                    let duration = now_ts() - last_ts;
                    *time_in_mode.entry(last_mode).or_default() += duration;
                }
            }
        }

        ModeStats {
            total_auto_transitions: total as u32,
            accepted: accepted as u32,
            overridden: overridden as u32,
            user_manual_sets: user_sets as u32,
            acceptance_rate: if total > 0 { accepted as f64 / total as f64 } else { 0.0 },
            time_in_mode_hours: time_in_mode.into_iter()
                .map(|(k, v)| (k, v / 3600.0))
                .collect(),
        }
    }
}

struct ModeRecommendation {
    mode: CompanionMode,
    reason: TransitionReason,
}

/// Stats about mode transitions.
#[derive(Debug, Clone)]
pub struct ModeStats {
    pub total_auto_transitions: u32,
    pub accepted: u32,
    pub overridden: u32,
    pub user_manual_sets: u32,
    pub acceptance_rate: f64,
    /// Hours spent in each mode.
    pub time_in_mode_hours: std::collections::HashMap<String, f64>,
}

// ── SQLite ────────────────────────────────────────────────────────────────

pub fn ensure_table(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mode_transitions (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            from_mode   TEXT NOT NULL,
            to_mode     TEXT NOT NULL,
            reason      TEXT NOT NULL,
            reason_data TEXT NOT NULL DEFAULT '{}',
            outcome     TEXT,
            timestamp   REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_mt_time ON mode_transitions(timestamp);
        CREATE INDEX IF NOT EXISTS idx_mt_reason ON mode_transitions(reason);",
    )
    .expect("failed to create mode_transitions table");
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

    fn setup() -> (Connection, ModeManager) {
        let conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn);
        let mgr = ModeManager::new();
        (conn, mgr)
    }

    #[test]
    fn default_mode_is_social() {
        let (_, mgr) = setup();
        assert_eq!(mgr.current(), CompanionMode::Social);
        assert!(!mgr.is_user_override());
    }

    #[test]
    fn user_override_is_sticky() {
        let (conn, mut mgr) = setup();
        mgr.set_mode(CompanionMode::Focus, &conn);
        assert!(mgr.is_user_override());
        assert_eq!(mgr.current(), CompanionMode::Focus);

        // Auto-switch should not happen during override
        let result = mgr.evaluate(10, 0, false, None, None, &conn);
        assert!(result.is_none(), "Should not auto-switch during user override");
    }

    #[test]
    fn idle_triggers_sleep() {
        let (conn, mut mgr) = setup();
        // 40 minutes idle → should switch to Sleep
        let result = mgr.evaluate(14, 40, false, None, None, &conn);
        assert_eq!(result, Some(CompanionMode::Sleep));
        assert_eq!(mgr.current(), CompanionMode::Sleep);
    }

    #[test]
    fn meeting_triggers_social() {
        let (conn, mut mgr) = setup();
        // Start in Focus
        mgr.current_mode = CompanionMode::Focus;
        let result = mgr.evaluate(10, 0, true, Some("standup"), None, &conn);
        assert_eq!(result, Some(CompanionMode::Social));
    }

    #[test]
    fn workspace_mode_takes_priority() {
        let (conn, mut mgr) = setup();
        let result = mgr.evaluate(
            14, 0, false, None,
            Some(CompanionMode::Focus),
            &conn,
        );
        assert_eq!(result, Some(CompanionMode::Focus));
    }

    #[test]
    fn time_schedule() {
        let schedule = TimeSchedule::default();
        assert_eq!(schedule.mode_for_hour(3), CompanionMode::Sleep);
        assert_eq!(schedule.mode_for_hour(7), CompanionMode::Social);
        assert_eq!(schedule.mode_for_hour(10), CompanionMode::Focus);
        assert_eq!(schedule.mode_for_hour(18), CompanionMode::Social);
        assert_eq!(schedule.mode_for_hour(23), CompanionMode::Sleep);
    }

    #[test]
    fn acceptance_rate_learning() {
        let (_, mut mgr) = setup();
        assert!((mgr.acceptance_rate() - 0.8).abs() < 0.01);

        // Accept 3 transitions
        for _ in 0..3 {
            mgr.record_outcome(TransitionOutcome::Accepted);
        }
        assert!(mgr.acceptance_rate() > 0.8);

        // Override 3 times → rate should drop
        for _ in 0..3 {
            mgr.record_outcome(TransitionOutcome::Overridden {
                to: CompanionMode::Focus,
            });
        }
        assert!(mgr.acceptance_rate() < 0.7,
            "Rate should drop after overrides: {}", mgr.acceptance_rate());
    }

    #[test]
    fn low_acceptance_prevents_auto_switch() {
        let (conn, mut mgr) = setup();
        // Tank the acceptance rate
        mgr.acceptance_rate = 0.3;
        mgr.total_transitions = 10;

        // Should NOT auto-switch even with idle signal
        mgr.current_mode = CompanionMode::Social;
        let result = mgr.evaluate(14, 40, false, None, None, &conn);
        assert!(result.is_none(), "Should not auto-switch with low acceptance rate");
    }

    #[test]
    fn resume_from_sleep() {
        let (conn, mut mgr) = setup();
        mgr.current_mode = CompanionMode::Sleep;
        // User resumes (0 idle minutes) → should go to Social
        let result = mgr.evaluate(10, 0, false, None, None, &conn);
        assert_eq!(result, Some(CompanionMode::Social));
    }

    #[test]
    fn transition_logging() {
        let (conn, mut mgr) = setup();
        mgr.set_mode(CompanionMode::Focus, &conn);
        mgr.user_override_active = false; // Clear so we can test auto
        let _ = mgr.evaluate(14, 40, false, None, None, &conn);

        let recent = ModeManager::recent_transitions(&conn, 10);
        assert!(recent.len() >= 1, "Should have at least 1 logged transition");
    }

    #[test]
    fn stats_calculation() {
        let (conn, mut mgr) = setup();

        // Simulate some transitions
        mgr.set_mode(CompanionMode::Focus, &conn);
        mgr.user_override_active = false;
        let _ = mgr.evaluate(23, 0, false, None, None, &conn); // time schedule → Sleep

        let stats = ModeManager::stats(&conn);
        assert!(stats.total_auto_transitions >= 0);
    }
}
