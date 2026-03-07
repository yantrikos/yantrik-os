//! Bond tracking — measures and evolves the relationship between companion and user.
//!
//! Bond score grows through interaction signals: frequency, depth, vulnerability,
//! memory callbacks, and daily streaks. Bond level unlocks personality behaviors:
//!
//! | Level | Score   | Name             | Behavior                                    |
//! |-------|---------|------------------|---------------------------------------------|
//! | 1     | 0-0.5   | Stranger         | Polite, formal, no opinions, no humor       |
//! | 2     | 0.5-1.5 | Acquaintance     | Remembers preferences, light warmth         |
//! | 3     | 1.5-2.5 | Friend           | Humor, opinions, gentle teasing, callbacks  |
//! | 4     | 2.5-3.5 | Confidant        | Deep emotional awareness, inside references |
//! | 5     | 3.5+    | Partner-in-Crime | Full Jexi mode — snarky, opinionated, real  |

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Bond level enum — unlocks progressively more personality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum BondLevel {
    Stranger = 1,
    Acquaintance = 2,
    Friend = 3,
    Confidant = 4,
    PartnerInCrime = 5,
}

impl BondLevel {
    pub fn from_score(score: f64) -> Self {
        match score {
            s if s < 0.5 => BondLevel::Stranger,
            s if s < 1.5 => BondLevel::Acquaintance,
            s if s < 2.5 => BondLevel::Friend,
            s if s < 3.5 => BondLevel::Confidant,
            _ => BondLevel::PartnerInCrime,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            BondLevel::Stranger => "Stranger",
            BondLevel::Acquaintance => "Acquaintance",
            BondLevel::Friend => "Friend",
            BondLevel::Confidant => "Confidant",
            BondLevel::PartnerInCrime => "Partner-in-Crime",
        }
    }

    pub fn as_u8(&self) -> u8 {
        *self as u8
    }
}

/// Snapshot of bond state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BondState {
    pub bond_score: f64,
    pub bond_level: BondLevel,
    pub total_interactions: i64,
    pub vulnerability_events: i64,
    pub humor_successes: i64,
    pub humor_attempts: i64,
    pub deep_conversations: i64,
    pub shared_references: i64,
    pub current_streak_days: i64,
    pub longest_streak_days: i64,
    pub first_interaction_at: Option<f64>,
    pub days_together: f64,
}

/// Tracks and evolves the bond between companion and user.
pub struct BondTracker;

/// Default vulnerability keywords.
const VULNERABILITY_PATTERNS: &[&str] = &[
    "i feel",
    "i'm scared",
    "i'm afraid",
    "i'm worried",
    "nobody understands",
    "i can't",
    "i lost",
    "i miss",
    "i'm lonely",
    "i'm sad",
    "i'm depressed",
    "i'm anxious",
    "i'm stressed",
    "i hate myself",
    "i love you",
    "thank you for listening",
    "you're the only one",
    "i trust you",
    "i've never told",
    "don't tell anyone",
];

impl BondTracker {
    /// Ensure bond tables exist. Called once on startup.
    pub fn ensure_tables(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS bond_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                bond_score REAL NOT NULL DEFAULT 0.0,
                bond_level INTEGER NOT NULL DEFAULT 1,
                total_interactions INTEGER NOT NULL DEFAULT 0,
                total_minutes REAL NOT NULL DEFAULT 0.0,
                vulnerability_events INTEGER NOT NULL DEFAULT 0,
                humor_successes INTEGER NOT NULL DEFAULT 0,
                humor_attempts INTEGER NOT NULL DEFAULT 0,
                deep_conversations INTEGER NOT NULL DEFAULT 0,
                shared_references INTEGER NOT NULL DEFAULT 0,
                first_interaction_at REAL,
                last_interaction_at REAL,
                longest_streak_days INTEGER NOT NULL DEFAULT 0,
                current_streak_days INTEGER NOT NULL DEFAULT 0,
                last_interaction_date TEXT,
                updated_at REAL NOT NULL DEFAULT 0.0
            );
            INSERT OR IGNORE INTO bond_state (id) VALUES (1);

            CREATE TABLE IF NOT EXISTS bond_events (
                event_id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                delta REAL NOT NULL,
                context TEXT DEFAULT '{}',
                created_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_bond_events_type ON bond_events(event_type);
            CREATE INDEX IF NOT EXISTS idx_bond_events_created ON bond_events(created_at);",
        )
        .expect("failed to create bond tables");
    }

    /// Get current bond state.
    pub fn get_state(conn: &Connection) -> BondState {
        let (score, _level, total, vuln, humor_s, humor_a, deep, refs, streak, longest, first): (
            f64, i64, i64, i64, i64, i64, i64, i64, i64, i64, Option<f64>,
        ) = conn
            .query_row(
                "SELECT bond_score, bond_level, total_interactions, vulnerability_events,
                 humor_successes, humor_attempts, deep_conversations, shared_references,
                 current_streak_days, longest_streak_days, first_interaction_at
                 FROM bond_state WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                    ))
                },
            )
            .unwrap_or((0.0, 1, 0, 0, 0, 0, 0, 0, 0, 0, None));

        let days_together = first
            .map(|f| (now_ts() - f) / 86400.0)
            .unwrap_or(0.0);

        BondState {
            bond_score: score,
            bond_level: BondLevel::from_score(score),
            total_interactions: total,
            vulnerability_events: vuln,
            humor_successes: humor_s,
            humor_attempts: humor_a,
            deep_conversations: deep,
            shared_references: refs,
            current_streak_days: streak,
            longest_streak_days: longest,
            first_interaction_at: first,
            days_together,
        }
    }

    /// Score an interaction and update bond state. Returns the new bond level
    /// and whether the level changed.
    pub fn score_interaction(
        conn: &Connection,
        user_text: &str,
        _response_text: &str,
        memories_recalled: usize,
    ) -> (BondLevel, bool) {
        let now = now_ts();
        let old_state = Self::get_state(conn);
        let old_level = old_state.bond_level;

        let mut delta = 0.01; // Base delta for any interaction

        // Depth bonus — longer messages indicate engagement
        if user_text.len() > 200 {
            delta += 0.02;
        } else if user_text.len() > 100 {
            delta += 0.01;
        }

        // Memory callback bonus — shared history acknowledgment
        let mem_bonus = (memories_recalled.min(3) as f64) * 0.01;
        delta += mem_bonus;

        // Vulnerability detection
        let user_lower = user_text.to_lowercase();
        let is_vulnerable = VULNERABILITY_PATTERNS
            .iter()
            .any(|p| user_lower.contains(p));
        if is_vulnerable {
            delta += 0.05;
            Self::log_event(conn, "vulnerability", delta, "{}");
            conn.execute(
                "UPDATE bond_state SET vulnerability_events = vulnerability_events + 1 WHERE id = 1",
                [],
            )
            .ok();
        }

        // Deep conversation detection (long + emotional)
        if user_text.len() > 300 {
            delta += 0.01;
            conn.execute(
                "UPDATE bond_state SET deep_conversations = deep_conversations + 1 WHERE id = 1",
                [],
            )
            .ok();
        }

        // Streak tracking
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let last_date: Option<String> = conn
            .query_row(
                "SELECT last_interaction_date FROM bond_state WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .ok();

        let (new_streak, streak_bonus) = match last_date {
            Some(ref d) if d == &today => {
                // Same day — no streak change
                (old_state.current_streak_days, 0.0)
            }
            Some(ref d) => {
                // Check if yesterday
                let yesterday = (chrono::Utc::now() - chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string();
                if d == &yesterday {
                    let new = old_state.current_streak_days + 1;
                    (new, 0.005 * new.min(7) as f64) // Streak bonus caps at 7 days
                } else {
                    (1, 0.0) // Streak broken
                }
            }
            None => (1, 0.0), // First interaction
        };
        delta += streak_bonus;

        // Compute new score (capped at 5.0)
        let new_score = (old_state.bond_score + delta).min(5.0);
        let new_level = BondLevel::from_score(new_score);
        let longest = new_streak.max(old_state.longest_streak_days);

        // Update state
        conn.execute(
            "UPDATE bond_state SET
                bond_score = ?1,
                bond_level = ?2,
                total_interactions = total_interactions + 1,
                last_interaction_at = ?3,
                last_interaction_date = ?4,
                current_streak_days = ?5,
                longest_streak_days = ?6,
                first_interaction_at = COALESCE(first_interaction_at, ?3),
                updated_at = ?3
             WHERE id = 1",
            params![
                new_score,
                new_level.as_u8() as i64,
                now,
                today,
                new_streak,
                longest,
            ],
        )
        .ok();

        // Log interaction event
        Self::log_event(
            conn,
            "interaction",
            delta,
            &serde_json::json!({
                "msg_len": user_text.len(),
                "memories": memories_recalled,
                "vulnerable": is_vulnerable,
                "streak": new_streak,
            })
            .to_string(),
        );

        let level_changed = new_level != old_level;
        if level_changed {
            tracing::info!(
                old = old_level.name(),
                new = new_level.name(),
                score = new_score,
                "Bond level changed!"
            );
            Self::log_event(
                conn,
                "milestone",
                0.0,
                &serde_json::json!({
                    "from": old_level.name(),
                    "to": new_level.name(),
                    "score": new_score,
                })
                .to_string(),
            );
        }

        (new_level, level_changed)
    }

    /// Record a humor attempt outcome.
    pub fn record_humor(conn: &Connection, success: bool) {
        if success {
            conn.execute(
                "UPDATE bond_state SET humor_successes = humor_successes + 1,
                 humor_attempts = humor_attempts + 1 WHERE id = 1",
                [],
            )
            .ok();
            Self::log_event(conn, "humor_success", 0.02, "{}");
            // Small bond bonus for successful humor
            conn.execute(
                "UPDATE bond_state SET bond_score = MIN(bond_score + 0.02, 5.0) WHERE id = 1",
                [],
            )
            .ok();
        } else {
            conn.execute(
                "UPDATE bond_state SET humor_attempts = humor_attempts + 1 WHERE id = 1",
                [],
            )
            .ok();
            Self::log_event(conn, "humor_fail", 0.0, "{}");
        }
    }

    /// Increment shared reference count.
    pub fn record_shared_reference(conn: &Connection) {
        conn.execute(
            "UPDATE bond_state SET shared_references = shared_references + 1 WHERE id = 1",
            [],
        )
        .ok();
    }

    /// Get the target formality for the current bond level.
    pub fn target_formality(level: BondLevel) -> f64 {
        match level {
            BondLevel::Stranger => 0.8,
            BondLevel::Acquaintance => 0.6,
            BondLevel::Friend => 0.4,
            BondLevel::Confidant => 0.2,
            BondLevel::PartnerInCrime => 0.1,
        }
    }

    fn log_event(conn: &Connection, event_type: &str, delta: f64, context: &str) {
        let event_id = uuid7::uuid7().to_string();
        conn.execute(
            "INSERT INTO bond_events (event_id, event_type, delta, context, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event_id, event_type, delta, context, now_ts()],
        )
        .ok();
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
