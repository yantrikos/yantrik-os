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

// Re-export data types from core
pub use yantrik_companion_core::bond::{BondLevel, BondState};

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

        // Calendar days on the same UTC clock the streak uses, counting the day
        // the two met as day one. This used to be elapsed 24-hour blocks off
        // SystemTime — a different clock and a different counting rule — so it
        // read 0 for the entire first day while the streak counter beside it
        // on the Bond screen read 1 from the first interaction.
        let days_together = first
            .and_then(|f| chrono::DateTime::from_timestamp(f as i64, 0))
            .map(|first_dt| {
                // from_timestamp yields UTC, the zone the streak dates are in.
                let days = (chrono::Utc::now().date_naive() - first_dt.date_naive()).num_days();
                // max(0): a clock that jumped (NTP sync after the first
                // interaction was stamped) must not read as negative days.
                days.max(0) as f64 + 1.0
            })
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

    /// When the person was last here: the newest `interaction` event in the store.
    ///
    /// `None` when there has never been one — the two have not spoken yet, and no caller
    /// may invent a time. The shell reads this at startup to set the clock the proactive
    /// engine answers "how long has the person been away" from; a startup is not the
    /// person having been around (#156). Only `interaction` events count: the store also
    /// holds events the machine wrote by itself (humor attempts, milestones), and those
    /// are not the person talking.
    pub fn last_interaction_at(conn: &Connection) -> Option<f64> {
        // MAX over an empty set is NULL, read as None; a missing table reads as None too.
        conn.query_row(
            "SELECT MAX(created_at) FROM bond_events WHERE event_type = 'interaction'",
            [],
            |row| row.get::<_, Option<f64>>(0),
        )
        .ok()
        .flatten()
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

    /// Score one conversation turn with the desktop, whichever mind answered it.
    ///
    /// The bond is the person's relationship with Yantrik, not with one of its minds — the
    /// Bond screen and `describe shell` say so — but only the built-in companion ever called
    /// `score_interaction`, from inside its own turn. With Hermes answering, forty minutes of
    /// conversation left the store untouched. What every mind's turn has in common is the part
    /// that scores: the person said something and was answered. The memory-callback bonus is
    /// the built-in's alone, because an attached harness keeps its own memory and the shell
    /// cannot see what it recalled; the reply text has never counted.
    pub fn score_conversation_turn(conn: &Connection, user_text: &str) -> (BondLevel, bool) {
        Self::score_interaction(conn, user_text, "", 0)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A bond database whose only history is one first interaction at `first`.
    fn conn_with_first_interaction(first: Option<f64>) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        BondTracker::ensure_tables(&conn);
        if let Some(ts) = first {
            conn.execute(
                "UPDATE bond_state SET first_interaction_at = ?1 WHERE id = 1",
                params![ts],
            )
            .unwrap();
        }
        conn
    }

    /// Midnight UTC `days` calendar days ago, as a UNIX timestamp.
    fn midnight_utc_days_ago(days: i64) -> f64 {
        (chrono::Utc::now().date_naive() - chrono::Duration::days(days))
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp() as f64
    }

    #[test]
    fn the_day_you_meet_is_day_one() {
        // The screen showed "0 days together" beside "1 days streak" because
        // days_together counted elapsed 24-hour blocks (zero until a full day
        // passed) while the streak counts calendar days from the first
        // interaction (one, immediately).
        let conn = conn_with_first_interaction(Some(midnight_utc_days_ago(0)));
        // What score_interaction records on that first interaction:
        conn.execute(
            "UPDATE bond_state SET current_streak_days = 1, last_interaction_date = ?1 WHERE id = 1",
            params![chrono::Utc::now().format("%Y-%m-%d").to_string()],
        )
        .unwrap();
        let state = BondTracker::get_state(&conn);
        assert_eq!(state.current_streak_days, 1);
        assert!(
            state.days_together >= 1.0,
            "a live streak must not outcount the days together"
        );
    }

    #[test]
    fn days_together_counts_calendar_days_from_first_interaction() {
        let conn = conn_with_first_interaction(Some(midnight_utc_days_ago(1)));
        assert_eq!(BondTracker::get_state(&conn).days_together, 2.0);

        let conn = conn_with_first_interaction(Some(midnight_utc_days_ago(2)));
        assert_eq!(BondTracker::get_state(&conn).days_together, 3.0);
    }

    #[test]
    fn no_first_interaction_means_no_days_together() {
        let conn = conn_with_first_interaction(None);
        assert_eq!(BondTracker::get_state(&conn).days_together, 0.0);
    }

    #[test]
    fn a_turn_answered_by_any_mind_counts_toward_the_bond() {
        // No memories recalled and no reply text: that is all the shell knows about a turn an
        // attached harness answered, and it has to be enough to move the bond, or a machine
        // whose mind is Hermes stays "Stranger, 0.0" no matter how long the person talks.
        let conn = conn_with_first_interaction(None);
        let before = BondTracker::get_state(&conn);
        assert_eq!(before.total_interactions, 0);

        let (level, _) = BondTracker::score_conversation_turn(&conn, "how is the build going?");
        let after = BondTracker::get_state(&conn);
        assert_eq!(after.total_interactions, 1, "a harness turn is an interaction");
        assert!(after.bond_score > before.bond_score, "a harness turn moves the score");
        assert_eq!(level, after.bond_level);
        assert!(after.first_interaction_at.is_some(), "the first turn is the day the two met");
        let logged: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bond_events WHERE event_type = 'interaction'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(logged, 1, "scored through the same path as the built-in, so it is logged like one");
    }

    #[test]
    fn the_newest_interaction_event_is_when_the_person_was_last_here() {
        let conn = conn_with_first_interaction(None);
        assert_eq!(
            BondTracker::last_interaction_at(&conn),
            None,
            "a store nobody has spoken into says so instead of naming a time"
        );

        // An event the machine wrote by itself is not the person talking, even when
        // it is newer than the last turn: the humor row stays at "now" while the
        // interaction row below is backdated two days.
        BondTracker::record_humor(&conn, true);
        assert_eq!(BondTracker::last_interaction_at(&conn), None);

        BondTracker::score_conversation_turn(&conn, "goodnight");
        let two_days_ago = now_ts() - 2.0 * 86400.0;
        conn.execute(
            "UPDATE bond_events SET created_at = ?1 WHERE event_type = 'interaction'",
            params![two_days_ago],
        )
        .unwrap();
        let last = BondTracker::last_interaction_at(&conn).expect("one interaction now");
        assert!(
            (last - two_days_ago).abs() < 1.0,
            "the last interaction was two days ago and reads as two days ago, got {last}"
        );
    }

    #[test]
    fn a_clock_jump_cannot_make_the_days_negative() {
        // first_interaction_at stamped into the future (NTP correction after
        // the interaction was recorded) still reads as day one, never zero —
        // zero beside a streak of one is the contradiction being fixed.
        let future = (chrono::Utc::now() + chrono::Duration::hours(6)).timestamp() as f64;
        let conn = conn_with_first_interaction(Some(future));
        assert_eq!(BondTracker::get_state(&conn).days_together, 1.0);
    }
}
