//! SQLite-backed urge queue.
//!
//! Urges are instinct outputs: "something the companion should bring up."
//! They have urgency, cooldown deduplication, expiry, and boost mechanics.

use rusqlite::{params, Connection};

use crate::config::UrgeQueueConfig;
use crate::types::{Urge, UrgeSpec};

/// Priority queue for companion urges, stored in YantrikDB's SQLite database.
pub struct UrgeQueue {
    config: UrgeQueueConfig,
}

impl UrgeQueue {
    pub fn new(conn: &Connection, config: UrgeQueueConfig) -> Self {
        Self::ensure_table(conn);
        Self { config }
    }

    fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS urges (
                urge_id TEXT PRIMARY KEY,
                instinct_name TEXT NOT NULL,
                reason TEXT NOT NULL,
                urgency REAL NOT NULL,
                suggested_message TEXT DEFAULT '',
                action TEXT,
                context TEXT DEFAULT '{}',
                cooldown_key TEXT NOT NULL,
                status TEXT DEFAULT 'pending',
                created_at REAL NOT NULL,
                delivered_at REAL,
                expires_at REAL,
                boost_count INTEGER DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_urges_status ON urges(status);
            CREATE INDEX IF NOT EXISTS idx_urges_cooldown ON urges(cooldown_key, status);
            CREATE INDEX IF NOT EXISTS idx_urges_urgency ON urges(status, urgency DESC);",
        )
        .expect("failed to create urges table");
    }

    /// Push an urge. If a matching pending cooldown_key exists, boost it.
    /// If a delivered urge exists with the same key, recycle it to pending
    /// so it can be re-delivered after the ProactiveEngine cooldown expires.
    /// Returns the urge_id if a new urge was created.
    pub fn push(&self, conn: &Connection, spec: &UrgeSpec) -> Option<String> {
        let now = now_ts();

        // "Don't use this memory" (#754): an urge made from a memory the person excluded is never
        // queued. A safety signal is never held back.
        if !spec.safety && crate::proactive_backoff::MemoryExclusions::any_excluded(conn, &spec.memory_ids) {
            tracing::debug!(instinct = spec.instinct_name, "Urge skipped: made from a memory the person excluded");
            return None;
        }
        let context_json = serde_json::to_string(&stored_context(spec)).unwrap_or_default();

        // Check for existing urge with same cooldown_key
        if !spec.cooldown_key.is_empty() {
            let existing: Option<(String, f64, String)> = conn
                .query_row(
                    "SELECT urge_id, urgency, status FROM urges
                     WHERE cooldown_key = ?1 AND status IN ('pending', 'delivered')",
                    params![spec.cooldown_key],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .ok();

            if let Some((urge_id, old_urgency, status)) = existing {
                if status == "pending" {
                    // Still pending — just boost urgency
                    let new_urgency = (old_urgency + self.config.boost_increment).min(1.0);
                    conn.execute(
                        "UPDATE urges SET urgency = ?1, boost_count = boost_count + 1
                         WHERE urge_id = ?2",
                        params![new_urgency, urge_id],
                    )
                    .ok();
                    tracing::debug!(urge_id, old_urgency, new_urgency, "Boosted pending urge");
                } else {
                    // Already delivered — recycle to pending only if enough time passed
                    // (2 hours cooldown prevents re-sending repeatedly)
                    let delivered_at: f64 = conn
                        .query_row(
                            "SELECT COALESCE(delivered_at, 0.0) FROM urges WHERE urge_id = ?1",
                            params![urge_id],
                            |row| row.get(0),
                        )
                        .unwrap_or(0.0);
                    let hours_since = (now - delivered_at) / 3600.0;
                    if hours_since >= 2.0 {
                        conn.execute(
                            "UPDATE urges SET status = 'pending', urgency = ?1, reason = ?2,
                             suggested_message = ?3, created_at = ?4, delivered_at = NULL,
                             boost_count = 0, context = ?6
                             WHERE urge_id = ?5",
                            params![
                                spec.urgency,
                                spec.reason,
                                spec.suggested_message,
                                now,
                                urge_id,
                                context_json
                            ],
                        )
                        .ok();
                        tracing::info!(
                            urge_id,
                            hours_since = format!("{hours_since:.1}"),
                            "Recycled delivered urge back to pending"
                        );
                    } else {
                        tracing::debug!(
                            urge_id,
                            hours_since = format!("{hours_since:.1}"),
                            "Skipping push — delivered too recently"
                        );
                    }
                }
                return None;
            }
        }

        // Enforce max pending — expire lowest urgency if at capacity
        let pending_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM urges WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if pending_count >= self.config.max_pending as i64 {
            conn.execute(
                "UPDATE urges SET status = 'expired'
                 WHERE urge_id = (
                     SELECT urge_id FROM urges
                     WHERE status = 'pending'
                     ORDER BY urgency ASC LIMIT 1
                 )",
                [],
            )
            .ok();
        }

        // Insert new urge
        let urge_id = uuid7::uuid7().to_string();
        let expires_at = now + self.config.expiry_hours * 3600.0;

        conn.execute(
            "INSERT INTO urges (urge_id, instinct_name, reason, urgency,
             suggested_message, action, context, cooldown_key, status,
             created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10)",
            params![
                urge_id,
                spec.instinct_name,
                spec.reason,
                spec.urgency,
                spec.suggested_message,
                spec.action,
                context_json,
                spec.cooldown_key,
                now,
                expires_at,
            ],
        )
        .ok();

        tracing::debug!(
            urge_id,
            instinct = spec.instinct_name,
            urgency = spec.urgency,
            "Pushed new urge"
        );

        Some(urge_id)
    }

    /// Pop top N urges by urgency for delivery during interaction.
    /// Marks them as "delivered".
    pub fn pop_for_interaction(&self, conn: &Connection, limit: usize) -> Vec<Urge> {
        let now = now_ts();
        let mut stmt = conn
            .prepare(
                "SELECT urge_id, instinct_name, reason, urgency, suggested_message,
                 action, context, cooldown_key, status, created_at, delivered_at,
                 expires_at, boost_count
                 FROM urges WHERE status = 'pending'
                 ORDER BY urgency DESC LIMIT ?1",
            )
            .expect("prepare pop_for_interaction");

        let urges: Vec<Urge> = stmt
            .query_map(params![limit as i64], |row| {
                urge_from_row(row)
            })
            .expect("query pop_for_interaction")
            .filter_map(|r| r.ok())
            .collect();

        // Mark as delivered
        for urge in &urges {
            conn.execute(
                "UPDATE urges SET status = 'delivered', delivered_at = ?1 WHERE urge_id = ?2",
                params![now, urge.urge_id],
            )
            .ok();
        }

        urges
    }

    /// Expire one pending urge, by id: it will never be said.
    pub fn expire(&self, conn: &Connection, urge_id: &str) {
        let _ = conn.execute(
            "UPDATE urges SET status = 'expired' WHERE urge_id = ?1 AND status = 'pending'",
            params![urge_id],
        );
    }

    /// Mark one pending urge delivered, by id. True when it was pending.
    pub fn mark_delivered(&self, conn: &Connection, urge_id: &str) -> bool {
        conn.execute(
            "UPDATE urges SET status = 'delivered', delivered_at = ?1
             WHERE urge_id = ?2 AND status = 'pending'",
            params![now_ts(), urge_id],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    /// Get all pending urges (for display).
    pub fn get_pending(&self, conn: &Connection, limit: usize) -> Vec<Urge> {
        let mut stmt = conn
            .prepare(
                "SELECT urge_id, instinct_name, reason, urgency, suggested_message,
                 action, context, cooldown_key, status, created_at, delivered_at,
                 expires_at, boost_count
                 FROM urges WHERE status = 'pending'
                 ORDER BY urgency DESC LIMIT ?1",
            )
            .expect("prepare get_pending");

        stmt.query_map(params![limit as i64], |row| {
            urge_from_row(row)
        })
        .expect("query get_pending")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Suppress a specific urge.
    pub fn suppress(&self, conn: &Connection, urge_id: &str) -> bool {
        let changes = conn
            .execute(
                "UPDATE urges SET status = 'suppressed'
                 WHERE urge_id = ?1 AND status IN ('pending', 'delivered')",
                params![urge_id],
            )
            .unwrap_or(0);
        changes > 0
    }

    /// Expire old urges past their expires_at time. Also expire delivered
    /// urges older than 2 hours so their cooldown keys become available again.
    /// Returns count expired.
    pub fn expire_old(&self, conn: &Connection) -> usize {
        let now = now_ts();
        let pending_expired = conn.execute(
            "UPDATE urges SET status = 'expired'
             WHERE status = 'pending' AND expires_at < ?1",
            params![now],
        )
        .unwrap_or(0);

        // Expire delivered urges after 2 hours — frees cooldown keys for
        // future urges while preventing rapid re-delivery
        let delivered_expired = conn.execute(
            "UPDATE urges SET status = 'expired'
             WHERE status = 'delivered' AND delivered_at < ?1",
            params![now - 7200.0], // 2 hours
        )
        .unwrap_or(0);

        pending_expired + delivered_expired
    }

    /// Count of pending urges.
    pub fn count_pending(&self, conn: &Connection) -> usize {
        conn.query_row(
            "SELECT COUNT(*) FROM urges WHERE status = 'pending'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Where an urge's safety marker and memory ids are kept: in its stored context, under keys no
/// instinct writes (#754). The urges table has no columns for them, and the context already
/// travels with the urge everywhere it goes.
const SAFETY_KEY: &str = "_safety";
const MEMORY_IDS_KEY: &str = "_memory_ids";

/// The context an urge is stored with: its own, plus its safety marker and memory ids.
fn stored_context(spec: &UrgeSpec) -> serde_json::Value {
    let mut ctx = match &spec.context {
        serde_json::Value::Null => serde_json::Value::Object(Default::default()),
        other => other.clone(),
    };
    if let Some(obj) = ctx.as_object_mut() {
        if spec.safety {
            obj.insert(SAFETY_KEY.into(), serde_json::Value::Bool(true));
        }
        if !spec.memory_ids.is_empty() {
            obj.insert(MEMORY_IDS_KEY.into(), serde_json::json!(spec.memory_ids));
        }
    }
    ctx
}

/// An urge from its stored row, its safety marker and memory ids read back from its context.
fn urge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Urge> {
    let context: serde_json::Value =
        serde_json::from_str(&row.get::<_, String>(6).unwrap_or_default()).unwrap_or_default();
    let safety = context.get(SAFETY_KEY).and_then(|v| v.as_bool()).unwrap_or(false);
    let memory_ids = context
        .get(MEMORY_IDS_KEY)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    Ok(Urge {
        urge_id: row.get(0)?,
        instinct_name: row.get(1)?,
        reason: row.get(2)?,
        urgency: row.get(3)?,
        suggested_message: row.get::<_, String>(4)?,
        action: row.get(5)?,
        context,
        cooldown_key: row.get(7)?,
        status: row.get(8)?,
        created_at: row.get(9)?,
        delivered_at: row.get(10)?,
        expires_at: row.get(11)?,
        boost_count: row.get(12)?,
        safety,
        memory_ids,
    })
}

