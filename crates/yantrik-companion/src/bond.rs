//! Bond — what the person's own actions have earned, and nothing they said.
//!
//! The score is the sum of **receipts**: a card Allowed, a For-you item marked Useful, a prepared
//! item opened or used (it rises); a card refused, a Not useful, an undo (it falls). See
//! [`ReceiptKind`]. It is computed from the receipt rows every time it is read, so there is
//! nothing to decay while the person is away and no turn's text can move it. The old counters
//! (messages, length, keywords, streaks) and the events behind them stay in the store as history
//! and are no longer read.
//!
//! The level only gates the *tone* of the instincts. It gates no action, and it is never shown
//! as a score or announced when it changes; per-action autonomy lives in `bond_table`, where the
//! person alone applies an offer.
//!
//! | Level | Score   | Name             |
//! |-------|---------|------------------|
//! | 1     | 0-0.5   | Stranger         |
//! | 2     | 0.5-1.5 | Acquaintance     |
//! | 3     | 1.5-2.5 | Friend           |
//! | 4     | 2.5-3.5 | Confidant        |
//! | 5     | 3.5+    | Partner-in-Crime |

use rusqlite::{params, Connection};

// Re-export data types from core
pub use yantrik_companion_core::bond::{
    BondLevel, BondRow, BondState, Receipt, ReceiptKind, OFFER_CLEAN_STREAK, SCORE_CEILING,
};

/// The `bond_events.event_type` a receipt is stored under.
pub(crate) const RECEIPT_EVENT: &str = "receipt";

/// Reads and writes the bond.
pub struct BondTracker;

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

    /// The bond as the receipts have it.
    pub fn get_state(conn: &Connection) -> BondState {
        let (score, receipts) = Self::receipt_score(conn);

        let (total, first): (i64, Option<f64>) = conn
            .query_row(
                "SELECT total_interactions, first_interaction_at FROM bond_state WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((0, None));

        // Calendar days on the same UTC clock, counting the day the two met as day one.
        let days_together = first
            .and_then(|f| chrono::DateTime::from_timestamp(f as i64, 0))
            .map(|first_dt| {
                let days = (chrono::Utc::now().date_naive() - first_dt.date_naive()).num_days();
                // max(0): a clock that jumped (NTP sync after the first interaction was
                // stamped) must not read as negative days.
                days.max(0) as f64 + 1.0
            })
            .unwrap_or(0.0);

        BondState {
            bond_score: score,
            bond_level: BondLevel::from_score(score),
            receipts,
            total_interactions: total,
            first_interaction_at: first,
            days_together,
        }
    }

    /// The score and how many receipts it is made of: each receipt in the order it happened,
    /// the running total held between zero and [`SCORE_CEILING`]. Time never enters.
    fn receipt_score(conn: &Connection) -> (f64, i64) {
        let Ok(mut stmt) = conn.prepare(
            "SELECT delta FROM bond_events WHERE event_type = ?1 ORDER BY created_at, rowid",
        ) else {
            return (0.0, 0);
        };
        let Ok(deltas) = stmt.query_map(params![RECEIPT_EVENT], |row| row.get::<_, f64>(0)) else {
            return (0.0, 0);
        };
        deltas
            .flatten()
            .fold((0.0, 0), |(score, n), d| ((score + d).clamp(0.0, SCORE_CEILING), n + 1))
    }

    /// Write down one receipt. Returns `false` when a receipt from the same record is already
    /// there: the same press is counted once. A receipt that lowers the score carries the
    /// sentence that says why.
    pub fn record_receipt(conn: &Connection, receipt: &Receipt) -> bool {
        let event_id = if receipt.source_id.is_empty() {
            uuid7::uuid7().to_string()
        } else {
            format!("{RECEIPT_EVENT}:{}:{}", receipt.kind.as_str(), receipt.source_id)
        };
        let context = serde_json::json!({
            "kind": receipt.kind.as_str(),
            "app": receipt.app,
            "action": receipt.action,
            "leaves_machine": receipt.leaves_machine,
            "source_id": receipt.source_id,
            "why": receipt.why_it_fell(),
        })
        .to_string();
        conn.execute(
            "INSERT OR IGNORE INTO bond_events (event_id, event_type, delta, context, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event_id, RECEIPT_EVENT, receipt.kind.delta(), context, now_ts()],
        )
        .map(|written| written > 0)
        .unwrap_or(false)
    }

    /// Why the score fell, newest first: the sentence stored with each receipt that lowered it.
    pub fn why_it_fell(conn: &Connection, limit: usize) -> Vec<String> {
        let Ok(mut stmt) = conn.prepare(
            "SELECT json_extract(context, '$.why') FROM bond_events
             WHERE event_type = ?1 AND delta < 0 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        ) else {
            return Vec::new();
        };
        let Ok(rows) =
            stmt.query_map(params![RECEIPT_EVENT, limit as i64], |row| row.get::<_, Option<String>>(0))
        else {
            return Vec::new();
        };
        rows.flatten().flatten().collect()
    }

    /// When the person was last here: the newest `interaction` event in the store.
    ///
    /// `None` when there has never been one — the two have not spoken yet, and no caller
    /// may invent a time. The shell reads this at startup to set the clock the proactive
    /// engine answers "how long has the person been away" from; a startup is not the
    /// person having been around (#156). Only `interaction` events count: the store also
    /// holds events the machine wrote by itself, and those are not the person talking.
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

    /// Note that the person spoke and was answered, by whichever mind answered.
    ///
    /// This is presence, not trust: it takes no text, and it moves the score by nothing. It
    /// keeps the day the two met and the time the person was last here, which the proactive
    /// engine reads to know whether to speak.
    pub fn note_turn(conn: &Connection) {
        let now = now_ts();
        conn.execute(
            "UPDATE bond_state SET
                total_interactions = total_interactions + 1,
                last_interaction_at = ?1,
                first_interaction_at = COALESCE(first_interaction_at, ?1),
                updated_at = ?1
             WHERE id = 1",
            params![now],
        )
        .ok();
        conn.execute(
            "INSERT INTO bond_events (event_id, event_type, delta, context, created_at)
             VALUES (?1, 'interaction', 0.0, '{}', ?2)",
            params![uuid7::uuid7().to_string(), now],
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
}

pub(crate) fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

#[cfg(test)]
#[path = "bond_tests.rs"]
mod tests;
