//! Co-occurrence pattern miner — discovers temporal associations.
//!
//! Mines the pulse stream to find patterns like:
//! - "When you view ticket X, you email person Y within 30min"
//! - "Commits spike after standup meetings"
//! - "You always do invoicing on Monday mornings"
//! - "When supplier Z emails, you check inventory within 1h"
//!
//! Uses association rule mining on (event_type, entity) pairs within
//! configurable time windows. Runs hourly — pure SQLite, no LLM.
//!
//! Patterns are stored in `cortex_patterns` and surfaced as attention
//! items when the antecedent fires but the consequent hasn't happened yet.

use rusqlite::{params, Connection};

use super::rules::AttentionItem;

// ── Core Types ───────────────────────────────────────────────────────

/// A discovered temporal pattern.
#[derive(Debug, Clone)]
pub struct TemporalPattern {
    pub id: i64,
    pub pattern_type: String,
    pub antecedent: String,  // e.g. "email_received:person:sarah"
    pub consequent: String,  // e.g. "email_sent:person:sarah"
    pub time_window_sec: f64,
    pub support: i64,        // how many times this co-occurrence was observed
    pub confidence: f64,     // P(consequent | antecedent)
}

// ── Pattern Miner ────────────────────────────────────────────────────

pub struct PatternMiner {
    /// Minimum co-occurrence count to consider a pattern real.
    min_support: i64,
    /// Minimum confidence (0.0-1.0) to surface a pattern.
    min_confidence: f64,
    /// Time windows to check for co-occurrence (in seconds).
    time_windows: Vec<f64>,
}

impl PatternMiner {
    pub fn new() -> Self {
        Self {
            min_support: 3,
            min_confidence: 0.5,
            time_windows: vec![
                1800.0,   // 30 minutes
                3600.0,   // 1 hour
                14400.0,  // 4 hours
                86400.0,  // 1 day
            ],
        }
    }

    /// Mine for new patterns in the pulse history.
    ///
    /// Called hourly. Looks for event pairs that frequently co-occur
    /// within time windows.
    pub fn mine(&self, conn: &Connection) {
        let now = now_ts();
        let lookback = now - 14.0 * 86400.0; // 2 weeks of history

        for &window in &self.time_windows {
            self.mine_event_pairs(conn, lookback, window);
            self.mine_entity_cooccurrence(conn, lookback, window);
        }

        // Prune stale patterns (not observed in 30 days, low confidence)
        let _ = conn.execute(
            "DELETE FROM cortex_patterns
             WHERE last_observed_ts < ?1 AND confidence < 0.4 AND suppressed = 0",
            params![now - 30.0 * 86400.0],
        );
    }

    /// Find event type pairs that frequently follow each other.
    ///
    /// "After event_type A, event_type B usually happens within T seconds"
    fn mine_event_pairs(&self, conn: &Connection, since: f64, window: f64) {
        // Find pairs: pulse A followed by pulse B within the window,
        // sharing at least one common entity.
        let mut stmt = match conn.prepare(
            "SELECT
                a.event_type || ':' || pe_a.entity_id AS antecedent,
                b.event_type || ':' || pe_b.entity_id AS consequent,
                COUNT(*) AS support
             FROM cortex_pulses a
             JOIN cortex_pulse_entities pe_a ON pe_a.pulse_id = a.id
             JOIN cortex_pulses b ON b.ts > a.ts AND b.ts <= a.ts + ?1
             JOIN cortex_pulse_entities pe_b ON pe_b.pulse_id = b.id
             WHERE a.ts >= ?2
               AND a.event_type != b.event_type
               AND a.id != b.id
               AND pe_a.entity_id = pe_b.entity_id
             GROUP BY antecedent, consequent
             HAVING COUNT(*) >= ?3
             ORDER BY support DESC
             LIMIT 20",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        let pairs: Vec<(String, String, i64)> = stmt
            .query_map(params![window, since, self.min_support], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .ok()
            .map(|r| r.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();

        let now = now_ts();
        for (ante, cons, support) in pairs {
            // Calculate confidence: P(cons | ante)
            let ante_event = ante.split(':').next().unwrap_or("");
            let ante_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM cortex_pulses WHERE event_type = ?1 AND ts >= ?2",
                    params![ante_event, since],
                    |row| row.get(0),
                )
                .unwrap_or(1);

            let confidence = support as f64 / ante_count.max(1) as f64;
            if confidence < self.min_confidence {
                continue;
            }

            // Upsert pattern
            let _ = conn.execute(
                "INSERT INTO cortex_patterns
                    (pattern_type, antecedent, consequent, time_window_sec,
                     support, confidence, last_observed_ts, discovered_ts)
                 VALUES ('temporal', ?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT DO NOTHING",
                params![ante, cons, window, support, confidence, now],
            );

            // Update existing pattern if it exists
            let _ = conn.execute(
                "UPDATE cortex_patterns SET
                    support = ?1, confidence = ?2, last_observed_ts = ?3
                 WHERE pattern_type = 'temporal' AND antecedent = ?4 AND consequent = ?5
                   AND time_window_sec = ?6",
                params![support, confidence, now, ante, cons, window],
            );
        }
    }

    /// Find entity pairs that frequently appear together.
    ///
    /// "Person X and Ticket Y always appear in the same context"
    fn mine_entity_cooccurrence(&self, conn: &Connection, since: f64, window: f64) {
        // Find entity pairs that co-occur in the same pulse or nearby pulses
        let mut stmt = match conn.prepare(
            "SELECT
                pe_a.entity_id AS entity_a,
                pe_b.entity_id AS entity_b,
                COUNT(DISTINCT pe_a.pulse_id) AS support
             FROM cortex_pulse_entities pe_a
             JOIN cortex_pulse_entities pe_b
                ON pe_a.pulse_id = pe_b.pulse_id
                AND pe_a.entity_id < pe_b.entity_id
             JOIN cortex_pulses p ON p.id = pe_a.pulse_id
             WHERE p.ts >= ?1
             GROUP BY entity_a, entity_b
             HAVING COUNT(DISTINCT pe_a.pulse_id) >= ?2
             ORDER BY support DESC
             LIMIT 20",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        let pairs: Vec<(String, String, i64)> = stmt
            .query_map(params![since, self.min_support], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .ok()
            .map(|r| r.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();

        let now = now_ts();
        for (entity_a, entity_b, support) in pairs {
            let total_a: i64 = conn
                .query_row(
                    "SELECT COUNT(DISTINCT pulse_id) FROM cortex_pulse_entities
                     WHERE entity_id = ?1",
                    params![entity_a],
                    |row| row.get(0),
                )
                .unwrap_or(1);

            let confidence = support as f64 / total_a.max(1) as f64;
            if confidence < self.min_confidence {
                continue;
            }

            let _ = conn.execute(
                "INSERT INTO cortex_patterns
                    (pattern_type, antecedent, consequent, time_window_sec,
                     support, confidence, last_observed_ts, discovered_ts)
                 VALUES ('cooccurrence', ?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT DO NOTHING",
                params![entity_a, entity_b, window, support, confidence, now],
            );
        }
    }

    /// Check if any known patterns have fired (antecedent happened)
    /// but the expected consequent hasn't followed yet.
    ///
    /// This is the attention-generating step — "you usually do X after Y,
    /// but you haven't yet."
    pub fn check_missing_consequents(&self, conn: &Connection) -> Vec<AttentionItem> {
        let now = now_ts();
        let mut items = Vec::new();

        // Get active temporal patterns with reasonable confidence
        let mut stmt = match conn.prepare(
            "SELECT id, antecedent, consequent, time_window_sec, confidence, support
             FROM cortex_patterns
             WHERE pattern_type = 'temporal'
               AND confidence >= ?1
               AND support >= ?2
               AND suppressed = 0
             ORDER BY confidence DESC
             LIMIT 30",
        ) {
            Ok(s) => s,
            Err(e) => {
                // Table might not have 'active' column yet — fall back
                tracing::debug!("Pattern query failed (expected on first run): {e}");
                return items;
            }
        };

        let patterns: Vec<(i64, String, String, f64, f64, i64)> = stmt
            .query_map(params![self.min_confidence, self.min_support], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .ok()
            .map(|r| r.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();

        for (id, antecedent, consequent, window, confidence, support) in &patterns {
            // Parse antecedent: "event_type:entity_id"
            let (ante_event, ante_entity) = match antecedent.split_once(':') {
                Some((e, ent)) => (e, ent),
                None => continue,
            };

            // Check if antecedent happened recently (within window)
            let ante_happened = conn
                .query_row(
                    "SELECT MAX(p.ts) FROM cortex_pulses p
                     JOIN cortex_pulse_entities pe ON pe.pulse_id = p.id
                     WHERE p.event_type = ?1 AND pe.entity_id = ?2 AND p.ts >= ?3",
                    params![ante_event, ante_entity, now - window],
                    |row| row.get::<_, Option<f64>>(0),
                )
                .ok()
                .flatten();

            let ante_ts = match ante_happened {
                Some(ts) => ts,
                None => continue, // Antecedent hasn't fired recently
            };

            // Check if consequent happened after the antecedent
            let (cons_event, cons_entity) = match consequent.split_once(':') {
                Some((e, ent)) => (e, ent),
                None => continue,
            };

            let cons_happened: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM cortex_pulses p
                     JOIN cortex_pulse_entities pe ON pe.pulse_id = p.id
                     WHERE p.event_type = ?1 AND pe.entity_id = ?2 AND p.ts > ?3",
                    params![cons_event, cons_entity, ante_ts],
                    |row| row.get::<_, i64>(0).map(|c| c > 0),
                )
                .unwrap_or(true); // Default to "happened" to avoid false positives

            if !cons_happened {
                // The usual follow-up hasn't happened yet
                let elapsed = now - ante_ts;
                let pct_window = elapsed / window;

                // Only alert if we're >50% through the expected window
                if pct_window < 0.5 {
                    continue;
                }

                let ante_name = ante_entity.split(':').last().unwrap_or(ante_entity).replace('-', " ");
                let cons_name = cons_event.replace('_', " ");

                items.push(AttentionItem {
                    rule_name: "pattern_missing",
                    priority: 0.5 + 0.2 * confidence.min(1.0),
                    summary: format!(
                        "You usually {} after activity on {}, but haven't yet ({} observed, {:.0}% confidence)",
                        cons_name, ante_name, support, confidence * 100.0
                    ),
                    suggested_action: format!("Consider: {}", cons_name),
                    entity_ids: vec![ante_entity.to_string()],
                    systems_involved: vec!["system"],
                });
            }
        }

        items.sort_by(|a, b| b.priority.partial_cmp(&a.priority).unwrap_or(std::cmp::Ordering::Equal));
        items.truncate(2);
        items
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
