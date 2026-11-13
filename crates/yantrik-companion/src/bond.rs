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
    ActionFacts, BondLevel, BondRow, BondState, Receipt, ReceiptKind, ScoreFold, OFFER_CLEAN_STREAK,
    CARD_RISE_CAP, PAIR_RISE_CAP, RECEIPT_RETENTION_DAYS, SCORE_CEILING,
};

/// The `bond_events.event_type` a receipt is stored under.
pub const RECEIPT_EVENT: &str = "receipt";

/// The earliest date the clock can truly show: 2026-11-01 00:00 UTC. This code did not exist
/// before it, so a clock reading earlier (a dead RTC at 1970 or 2000, before time sync) is
/// impossible and is not trusted to judge a receipt "too far ahead".
pub(crate) const EARLIEST_TRUE_CLOCK: f64 = 1_793_491_200.0;

/// The reserved `app` that prepared-work receipts are filed under. The leading `~` can never be
/// a Mind's name, so prepared work shares one rise cap and collides with no sender.
pub(crate) const PREPARED_APP: &str = "~prepared";

/// The moment a person pressed. Only [`PressStamp::now`] makes one, so no caller can write a
/// receipt with a time it chose, while a retry can still carry the time of the press.
#[derive(Clone, Copy, Debug)]
pub struct PressStamp(f64);

impl PressStamp {
    pub fn now() -> Self {
        Self(now_ts())
    }

    /// Seconds since the epoch, as stored.
    pub fn secs(self) -> f64 {
        self.0
    }
}

/// Reads and writes the bond.
pub struct BondTracker;

impl BondTracker {
    /// Ensure bond tables exist. Called once on startup.
    pub fn ensure_tables(conn: &Connection) {
        Self::ensure_tables_at(conn, now_ts());
    }

    /// [`Self::ensure_tables`] with the clock given, so a test can set it.
    pub(crate) fn ensure_tables_at(conn: &Connection, now: f64) {
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
        Self::drop_old_receipts(conn, now);
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

    /// The score and how many receipts it is made of (see [`ScoreFold`]).
    fn receipt_score(conn: &Connection) -> (f64, i64) {
        // Order by `rowid` alone: the order they were written in. A clock stepped back must not
        // move a refusal ahead of older allows.
        let Ok(mut stmt) = conn.prepare(
            "SELECT delta, json_extract(context, '$.app'), json_extract(context, '$.action'),
                    json_extract(context, '$.kind')
             FROM bond_events WHERE event_type = ?1 ORDER BY rowid",
        ) else {
            return (0.0, 0);
        };
        let Ok(found) = stmt.query_map(params![RECEIPT_EVENT], |row| {
            Ok((
                row.get::<_, f64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        }) else {
            return (0.0, 0);
        };
        let mut fold = ScoreFold::default();
        for (delta, app, action, kind) in found.flatten() {
            let (app, action) = (app.as_deref().unwrap_or(""), action.as_deref().unwrap_or(""));
            match kind.as_deref().and_then(ReceiptKind::parse) {
                Some(ReceiptKind::CardAllowed | ReceiptKind::CardRefused) => fold.add_card(delta, app, action),
                _ => fold.add(delta, app, action),
            }
        }
        fold.finish()
    }

    /// Write down one receipt. Returns `false` when a receipt from the same record is already
    /// there: the same press is counted once. A receipt with no record of its own (an undo) is
    /// given a fresh id, so each one counts. A receipt that lowers the score carries the sentence
    /// that says why. Receipts past [`RECEIPT_RETENTION_DAYS`] are dropped as this one is written.
    pub fn record_receipt(conn: &Connection, receipt: &Receipt) -> bool {
        Self::record_receipt_at(conn, receipt, now_ts())
    }

    /// [`Self::record_receipt`] with the clock given, so a test can set it.
    pub(crate) fn record_receipt_at(conn: &Connection, receipt: &Receipt, now: f64) -> bool {
        Self::drop_old_receipts(conn, now);
        Self::try_record_receipt(conn, receipt, PressStamp(now)).unwrap_or(false)
    }

    /// The shell's press path: ONE bounded statement (the INSERT), no table creation and no
    /// prune (the worker prunes, [`Self::prune_old_receipts`]). The error is returned, never
    /// swallowed, so a write that timed out is the caller's to log and retry. The stamp is the
    /// press's own ([`PressStamp::now`]), so a retry lands with the time the person pressed.
    pub fn try_record_receipt(conn: &Connection, receipt: &Receipt, at: PressStamp) -> rusqlite::Result<bool> {
        let source_id = if receipt.source_id.is_empty() {
            uuid7::uuid7().to_string()
        } else {
            receipt.source_id.clone()
        };
        let event_id = format!("{RECEIPT_EVENT}:{}:{source_id}", receipt.kind.as_str());
        let context = serde_json::json!({
            "kind": receipt.kind.as_str(),
            "app": receipt.app,
            "action": receipt.action,
            "grade": receipt.facts.grade,
            "cannot_be_undone": receipt.facts.cannot_be_undone,
            "open_ended": receipt.facts.open_ended,
            "fetches_address": receipt.facts.fetches_address,
            "leaves_machine": receipt.facts.leaves_machine,
            "no_session_rule": receipt.facts.no_session_rule,
            "source_id": source_id,
            "why": receipt.why_it_fell(),
        })
        .to_string();
        conn.execute(
            "INSERT OR IGNORE INTO bond_events (event_id, event_type, delta, context, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event_id, RECEIPT_EVENT, receipt.kind.delta(), context, at.0],
        )
        .map(|written| written > 0)
    }

    /// Forget every receipt: the person's own act, from a button and nowhere else. The score
    /// returns to Stranger and the table empties. The old counters and presence stay. Returns
    /// how many were dropped; a store that would not let it (busy, locked) is an `Err`, so the
    /// caller can say nothing was forgotten.
    pub fn try_forget_receipts(conn: &Connection) -> rusqlite::Result<usize> {
        conn.execute("DELETE FROM bond_events WHERE event_type = ?1", params![RECEIPT_EVENT])
    }

    /// [`Self::try_forget_receipts`] for the worker's own thread, where a failure is not shown.
    pub fn forget_receipts(conn: &Connection) -> usize {
        Self::try_forget_receipts(conn).unwrap_or(0)
    }

    /// The retention prune, by the clock now. The worker runs it at its start and on each
    /// `Refresh`; a press never does.
    pub fn prune_old_receipts(conn: &Connection) {
        Self::drop_old_receipts(conn, now_ts());
    }

    /// Drop receipts more than [`RECEIPT_RETENTION_DAYS`] old by the clock, at startup and on each
    /// refresh (never on a press). That arm always runs. A second arm drops receipts stamped more than a day ahead of
    /// the clock, but only once the clock shows [`EARLIEST_TRUE_CLOCK`] or later, so a boot at
    /// 1970 or 2000 erases nothing, and a receipt written while the clock was impossible is
    /// dropped once the clock is right. Two faults still forget, both on the side of privacy: a
    /// clock set back by more than a day, to a date after [`EARLIEST_TRUE_CLOCK`], forgets every
    /// receipt written after the date it shows; a clock far ahead forgets older ones. Order is by
    /// `rowid`, never by stamp. Reading never calls this.
    fn drop_old_receipts(conn: &Connection, now: f64) {
        let oldest = now - RECEIPT_RETENTION_DAYS as f64 * 86_400.0;
        // Beyond any stamp, so the future arm cannot match, while the clock is impossible.
        let newest = if now >= EARLIEST_TRUE_CLOCK { now + 86_400.0 } else { f64::INFINITY };
        conn.execute(
            "DELETE FROM bond_events WHERE event_type = ?1 AND (created_at < ?2 OR created_at > ?3)",
            params![RECEIPT_EVENT, oldest, newest],
        )
        .ok();
    }

    /// Why the score fell, newest first: the sentence stored with each receipt that lowered it.
    pub fn why_it_fell(conn: &Connection, limit: usize) -> Vec<String> {
        let Ok(mut stmt) = conn.prepare(
            "SELECT json_extract(context, '$.why') FROM bond_events
             WHERE event_type = ?1 AND delta < 0 ORDER BY rowid DESC LIMIT ?2",
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

#[cfg(test)]
#[path = "bond_clock_tests.rs"]
mod clock_tests;
