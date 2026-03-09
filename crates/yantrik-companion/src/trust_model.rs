//! 3-Axis Trust Model — replaces the single bond scalar for permission decisions.
//!
//! Three independent trust axes:
//!
//! 1. **Action Trust** — How much can Yantrik do without asking?
//!    Earned by successful autonomous actions, lost by mistakes.
//!    Feeds into: tool permission escalation, autonomous execution threshold.
//!
//! 2. **Personal Trust** — How intimate/vulnerable can the interaction become?
//!    Earned by time + appropriate responses to sensitive topics.
//!    Feeds into: proactive depth, emotional awareness activation.
//!
//! 3. **Taste Trust** — How often are suggestions welcomed/useful?
//!    Earned by accepted suggestions, lost by dismissed ones.
//!    Feeds into: proactive scoring, recommendation confidence.
//!
//! Each axis: 0.0-1.0, updated by events, decays slowly, stored in SQLite.
//! The existing BondLevel (1-5 scalar) remains for personality — trust is orthogonal.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// The three trust axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrustAxis {
    /// Can Yantrik act autonomously?
    Action,
    /// Can the interaction be deep/vulnerable?
    Personal,
    /// Are suggestions/recommendations welcome?
    Taste,
}

impl TrustAxis {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Personal => "personal",
            Self::Taste => "taste",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "action" => Some(Self::Action),
            "personal" => Some(Self::Personal),
            "taste" => Some(Self::Taste),
            _ => None,
        }
    }
}

/// Snapshot of all three trust axes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustState {
    pub action: f64,
    pub personal: f64,
    pub taste: f64,
    /// Last update timestamp.
    pub updated_at: f64,
}

impl Default for TrustState {
    fn default() -> Self {
        Self {
            action: 0.2,    // Start cautious — user must earn action trust
            personal: 0.1,  // Start very cautious — personal trust is slow
            taste: 0.5,     // Start neutral — suggestions are 50/50
            updated_at: 0.0,
        }
    }
}

impl TrustState {
    /// Composite trust score (weighted average — action is most safety-critical).
    pub fn composite(&self) -> f64 {
        self.action * 0.4 + self.personal * 0.3 + self.taste * 0.3
    }

    /// Can Yantrik perform autonomous actions (no confirmation)?
    pub fn can_act_autonomously(&self) -> bool {
        self.action >= 0.7
    }

    /// Can Yantrik bring up sensitive/emotional topics proactively?
    pub fn can_be_personal(&self) -> bool {
        self.personal >= 0.5
    }

    /// Are proactive suggestions likely to be welcome?
    pub fn suggestions_welcome(&self) -> bool {
        self.taste >= 0.4
    }
}

/// Events that update trust axes.
#[derive(Debug, Clone)]
pub enum TrustEvent {
    // ── Action Trust ──
    /// Autonomous action completed successfully (e.g., auto-filed email).
    AutonomousSuccess { action: String },
    /// Autonomous action caused a problem (user had to fix/undo).
    AutonomousMistake { action: String, severity: f64 },
    /// User explicitly approved an action that was auto-suggested.
    ActionApproved,
    /// User explicitly denied/reverted an action.
    ActionDenied,

    // ── Personal Trust ──
    /// User shared something vulnerable (detected by vulnerability patterns).
    VulnerabilityShared,
    /// Companion responded appropriately to vulnerability.
    AppropriateResponse,
    /// Companion overstepped (brought up topic user didn't want to discuss).
    Overstepped,
    /// Interaction duration: longer conversations build personal trust.
    ExtendedConversation { minutes: f64 },
    /// Daily interaction (time together builds personal trust).
    DailyInteraction,

    // ── Taste Trust ──
    /// User accepted/engaged with a suggestion.
    SuggestionAccepted { source: String },
    /// User dismissed a suggestion.
    SuggestionDismissed { source: String },
    /// User explicitly said a suggestion was helpful.
    SuggestionPraised,
    /// User expressed frustration at a suggestion.
    SuggestionFrustration,
}

/// Manages trust state persistence and updates.
pub struct TrustModel;

/// Trust update deltas for each event type.
const ACTION_SUCCESS_DELTA: f64 = 0.03;
const ACTION_MISTAKE_BASE_DELTA: f64 = -0.10;
const ACTION_APPROVED_DELTA: f64 = 0.02;
const ACTION_DENIED_DELTA: f64 = -0.05;

const PERSONAL_VULNERABILITY_DELTA: f64 = 0.04;
const PERSONAL_APPROPRIATE_DELTA: f64 = 0.02;
const PERSONAL_OVERSTEPPED_DELTA: f64 = -0.08;
const PERSONAL_DAILY_DELTA: f64 = 0.005;

const TASTE_ACCEPTED_DELTA: f64 = 0.03;
const TASTE_DISMISSED_DELTA: f64 = -0.02;
const TASTE_PRAISED_DELTA: f64 = 0.05;
const TASTE_FRUSTRATION_DELTA: f64 = -0.08;

/// Daily decay rate: trust slowly decays without reinforcement.
const DAILY_DECAY: f64 = 0.002;

impl TrustModel {
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS trust_state (
                id          INTEGER PRIMARY KEY CHECK (id = 1),
                action_trust    REAL NOT NULL DEFAULT 0.2,
                personal_trust  REAL NOT NULL DEFAULT 0.1,
                taste_trust     REAL NOT NULL DEFAULT 0.5,
                updated_at      REAL NOT NULL DEFAULT 0.0
            );
            INSERT OR IGNORE INTO trust_state (id) VALUES (1);

            CREATE TABLE IF NOT EXISTS trust_events (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                axis        TEXT NOT NULL,
                event_type  TEXT NOT NULL,
                delta       REAL NOT NULL,
                context     TEXT,
                created_at  REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_trust_axis ON trust_events(axis);
            CREATE INDEX IF NOT EXISTS idx_trust_time ON trust_events(created_at);",
        )
        .expect("failed to create trust tables");
    }

    /// Get current trust state.
    pub fn get_state(conn: &Connection) -> TrustState {
        conn.query_row(
            "SELECT action_trust, personal_trust, taste_trust, updated_at
             FROM trust_state WHERE id = 1",
            [],
            |row| {
                Ok(TrustState {
                    action: row.get(0)?,
                    personal: row.get(1)?,
                    taste: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        )
        .unwrap_or_default()
    }

    /// Apply a trust event and update state.
    pub fn apply_event(conn: &Connection, event: &TrustEvent) -> TrustState {
        let (axis, delta, event_type, context) = match event {
            TrustEvent::AutonomousSuccess { action } => {
                (TrustAxis::Action, ACTION_SUCCESS_DELTA, "autonomous_success", Some(action.clone()))
            }
            TrustEvent::AutonomousMistake { action, severity } => {
                let delta = ACTION_MISTAKE_BASE_DELTA * severity;
                (TrustAxis::Action, delta, "autonomous_mistake", Some(action.clone()))
            }
            TrustEvent::ActionApproved => {
                (TrustAxis::Action, ACTION_APPROVED_DELTA, "action_approved", None)
            }
            TrustEvent::ActionDenied => {
                (TrustAxis::Action, ACTION_DENIED_DELTA, "action_denied", None)
            }
            TrustEvent::VulnerabilityShared => {
                (TrustAxis::Personal, PERSONAL_VULNERABILITY_DELTA, "vulnerability_shared", None)
            }
            TrustEvent::AppropriateResponse => {
                (TrustAxis::Personal, PERSONAL_APPROPRIATE_DELTA, "appropriate_response", None)
            }
            TrustEvent::Overstepped => {
                (TrustAxis::Personal, PERSONAL_OVERSTEPPED_DELTA, "overstepped", None)
            }
            TrustEvent::ExtendedConversation { minutes } => {
                let delta = (minutes / 30.0).min(1.0) * 0.01; // Cap at 0.01 per 30 mins
                (TrustAxis::Personal, delta, "extended_conversation", None)
            }
            TrustEvent::DailyInteraction => {
                (TrustAxis::Personal, PERSONAL_DAILY_DELTA, "daily_interaction", None)
            }
            TrustEvent::SuggestionAccepted { source } => {
                (TrustAxis::Taste, TASTE_ACCEPTED_DELTA, "suggestion_accepted", Some(source.clone()))
            }
            TrustEvent::SuggestionDismissed { source } => {
                (TrustAxis::Taste, TASTE_DISMISSED_DELTA, "suggestion_dismissed", Some(source.clone()))
            }
            TrustEvent::SuggestionPraised => {
                (TrustAxis::Taste, TASTE_PRAISED_DELTA, "suggestion_praised", None)
            }
            TrustEvent::SuggestionFrustration => {
                (TrustAxis::Taste, TASTE_FRUSTRATION_DELTA, "suggestion_frustration", None)
            }
        };

        // Log the event
        let now = now_ts();
        let _ = conn.execute(
            "INSERT INTO trust_events (axis, event_type, delta, context, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![axis.as_str(), event_type, delta, context, now],
        );

        // Update the appropriate axis
        let column = match axis {
            TrustAxis::Action => "action_trust",
            TrustAxis::Personal => "personal_trust",
            TrustAxis::Taste => "taste_trust",
        };

        let _ = conn.execute(
            &format!(
                "UPDATE trust_state SET {col} = MAX(0.0, MIN(1.0, {col} + ?1)), updated_at = ?2 WHERE id = 1",
                col = column
            ),
            params![delta, now],
        );

        Self::get_state(conn)
    }

    /// Apply daily decay to all axes (call once per day from background loop).
    pub fn apply_daily_decay(conn: &Connection) -> TrustState {
        let now = now_ts();
        let state = Self::get_state(conn);

        // Only decay if we haven't decayed recently
        if now - state.updated_at < 43200.0 {
            return state; // Less than 12 hours since last update
        }

        let days_since = ((now - state.updated_at) / 86400.0).min(7.0);
        let decay = DAILY_DECAY * days_since;

        let _ = conn.execute(
            "UPDATE trust_state SET
                action_trust = MAX(0.05, action_trust - ?1),
                personal_trust = MAX(0.05, personal_trust - ?1),
                taste_trust = MAX(0.1, taste_trust - ?1),
                updated_at = ?2
             WHERE id = 1",
            params![decay, now],
        );

        Self::get_state(conn)
    }

    /// Get trust event history for a specific axis.
    pub fn axis_history(
        conn: &Connection,
        axis: TrustAxis,
        limit: usize,
    ) -> Vec<TrustEventRecord> {
        let mut stmt = match conn.prepare(
            "SELECT event_type, delta, context, created_at
             FROM trust_events
             WHERE axis = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![axis.as_str(), limit as i64], |row| {
            Ok(TrustEventRecord {
                event_type: row.get(0)?,
                delta: row.get(1)?,
                context: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Format a trust summary for the system prompt / dashboard.
    pub fn summary(conn: &Connection) -> String {
        let state = Self::get_state(conn);
        format!(
            "Trust: action={:.0}% personal={:.0}% taste={:.0}% (composite={:.0}%)",
            state.action * 100.0,
            state.personal * 100.0,
            state.taste * 100.0,
            state.composite() * 100.0,
        )
    }
}

/// A recorded trust event.
#[derive(Debug, Clone)]
pub struct TrustEventRecord {
    pub event_type: String,
    pub delta: f64,
    pub context: Option<String>,
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
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        TrustModel::ensure_table(&conn);
        conn
    }

    #[test]
    fn initial_state_is_cautious() {
        let conn = setup();
        let state = TrustModel::get_state(&conn);
        assert!((state.action - 0.2).abs() < 0.01);
        assert!((state.personal - 0.1).abs() < 0.01);
        assert!((state.taste - 0.5).abs() < 0.01);
        assert!(!state.can_act_autonomously());
        assert!(!state.can_be_personal());
        assert!(state.suggestions_welcome());
    }

    #[test]
    fn action_trust_grows_with_successes() {
        let conn = setup();

        for _ in 0..20 {
            TrustModel::apply_event(&conn, &TrustEvent::AutonomousSuccess {
                action: "auto_file_email".into(),
            });
        }

        let state = TrustModel::get_state(&conn);
        assert!(state.action > 0.5, "Action trust should grow: {}", state.action);
    }

    #[test]
    fn mistake_drops_action_trust() {
        let conn = setup();

        // Build up some trust first
        for _ in 0..10 {
            TrustModel::apply_event(&conn, &TrustEvent::AutonomousSuccess {
                action: "test".into(),
            });
        }
        let before = TrustModel::get_state(&conn).action;

        // One significant mistake
        TrustModel::apply_event(&conn, &TrustEvent::AutonomousMistake {
            action: "deleted_wrong_file".into(),
            severity: 1.0,
        });

        let after = TrustModel::get_state(&conn).action;
        assert!(after < before, "Action trust should drop after mistake: {} → {}", before, after);
    }

    #[test]
    fn personal_trust_builds_slowly() {
        let conn = setup();

        // Daily interactions over "weeks"
        for _ in 0..30 {
            TrustModel::apply_event(&conn, &TrustEvent::DailyInteraction);
        }

        let state = TrustModel::get_state(&conn);
        assert!(state.personal > 0.1, "Personal trust should grow: {}", state.personal);
        // But shouldn't grow too fast from daily interactions alone
        assert!(state.personal < 0.5, "Personal trust shouldn't be too high from dailies alone: {}", state.personal);
    }

    #[test]
    fn vulnerability_accelerates_personal_trust() {
        let conn = setup();

        TrustModel::apply_event(&conn, &TrustEvent::VulnerabilityShared);
        TrustModel::apply_event(&conn, &TrustEvent::AppropriateResponse);

        let state = TrustModel::get_state(&conn);
        let expected_min = 0.1 + PERSONAL_VULNERABILITY_DELTA + PERSONAL_APPROPRIATE_DELTA;
        assert!(state.personal >= expected_min - 0.001,
            "Personal trust should reflect vulnerability events: {}", state.personal);
    }

    #[test]
    fn overstepping_drops_personal_trust() {
        let conn = setup();

        // Build some trust
        for _ in 0..10 {
            TrustModel::apply_event(&conn, &TrustEvent::VulnerabilityShared);
        }
        let before = TrustModel::get_state(&conn).personal;

        TrustModel::apply_event(&conn, &TrustEvent::Overstepped);

        let after = TrustModel::get_state(&conn).personal;
        assert!(after < before, "Personal trust should drop: {} → {}", before, after);
    }

    #[test]
    fn taste_trust_tracks_suggestions() {
        let conn = setup();

        // Mix of accepted and dismissed
        for _ in 0..5 {
            TrustModel::apply_event(&conn, &TrustEvent::SuggestionAccepted {
                source: "weather".into(),
            });
        }
        for _ in 0..3 {
            TrustModel::apply_event(&conn, &TrustEvent::SuggestionDismissed {
                source: "check_in".into(),
            });
        }

        let state = TrustModel::get_state(&conn);
        // Net positive: 5 * 0.03 - 3 * 0.02 = 0.09
        let expected = 0.5 + 0.09;
        assert!((state.taste - expected).abs() < 0.01,
            "Taste trust should reflect suggestion history: {} (expected ~{})",
            state.taste, expected);
    }

    #[test]
    fn frustration_hammers_taste_trust() {
        let conn = setup();

        TrustModel::apply_event(&conn, &TrustEvent::SuggestionFrustration);
        TrustModel::apply_event(&conn, &TrustEvent::SuggestionFrustration);

        let state = TrustModel::get_state(&conn);
        assert!(state.taste < 0.5, "Taste should drop: {}", state.taste);
    }

    #[test]
    fn axes_are_independent() {
        let conn = setup();

        // Only affect action trust
        for _ in 0..10 {
            TrustModel::apply_event(&conn, &TrustEvent::AutonomousSuccess {
                action: "test".into(),
            });
        }

        let state = TrustModel::get_state(&conn);
        assert!(state.action > 0.4);
        // Personal and taste should be unchanged
        assert!((state.personal - 0.1).abs() < 0.01);
        assert!((state.taste - 0.5).abs() < 0.01);
    }

    #[test]
    fn trust_clamped_to_range() {
        let conn = setup();

        // Try to push action trust above 1.0
        for _ in 0..100 {
            TrustModel::apply_event(&conn, &TrustEvent::AutonomousSuccess {
                action: "test".into(),
            });
        }
        let state = TrustModel::get_state(&conn);
        assert!(state.action <= 1.0, "Action trust should be clamped: {}", state.action);

        // Try to push taste trust below 0.0
        for _ in 0..100 {
            TrustModel::apply_event(&conn, &TrustEvent::SuggestionFrustration);
        }
        let state = TrustModel::get_state(&conn);
        assert!(state.taste >= 0.0, "Taste trust should be clamped: {}", state.taste);
    }

    #[test]
    fn event_history_recorded() {
        let conn = setup();

        TrustModel::apply_event(&conn, &TrustEvent::AutonomousSuccess {
            action: "email_auto_file".into(),
        });
        TrustModel::apply_event(&conn, &TrustEvent::ActionDenied);

        let history = TrustModel::axis_history(&conn, TrustAxis::Action, 10);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].event_type, "action_denied"); // Most recent first
        assert_eq!(history[1].event_type, "autonomous_success");
    }

    #[test]
    fn summary_formats_correctly() {
        let conn = setup();
        let summary = TrustModel::summary(&conn);
        assert!(summary.contains("action="));
        assert!(summary.contains("personal="));
        assert!(summary.contains("taste="));
        assert!(summary.contains("composite="));
    }
}
