//! The score: a readable sum of named terms. No randomness, no clock (time is passed in).
//!
//! ```text
//! total = W_DEADLINE   * urgency_fn(created_at, now, dismiss_count)         in 0..1
//!       + W_STALE      * min(hours since last moved / STALE_FULL_HOURS, 1)
//!       + W_BLOCKED    * min(hours blocked / BLOCKED_FULL_HOURS, 1)
//!       + W_CONFIDENCE * confidence                                         in 0..1
//!       + drive_pressure (0 for now)
//!       + fit_with_activity (0 for now)
//! ```

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::store::{read_entry, Entry, COLUMNS};

/// Every weight lives here, so changing the order of the agenda means changing this block.
pub const W_DEADLINE: f64 = 3.0;
pub const W_STALE: f64 = 0.5;
pub const W_BLOCKED: f64 = 0.5;
pub const W_CONFIDENCE: f64 = 1.0;
/// Hours of not moving after which the "stale" term is at full strength.
pub const STALE_FULL_HOURS: f64 = 72.0;
/// Hours of being blocked after which the "blocked" term is at full strength.
pub const BLOCKED_FULL_HOURS: f64 = 72.0;

/// How the deadline term moves with time. The names and formulas follow the memory store's
/// `UrgencyFn` so the two can become one later without changing what anything means.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UrgencyFn {
    /// `base + slope * (now - created_at)`.
    Linear { base: f64, slope: f64 },
    /// `1 / (1 + exp(steepness * (deadline - now - offset)))`: low until near, then a rapid ramp.
    Sigmoid { deadline: f64, steepness: f64, offset: f64 },
    /// 0 until `deadline - buffer_secs`, then 1.
    StepAtDeadline { deadline: f64, buffer_secs: f64 },
    /// `initial * decay_factor ^ dismiss_count`: what the person keeps dismissing fades.
    DecayIfIgnored { initial: f64, decay_factor: f64 },
    /// Never changes with time.
    Constant { value: f64 },
}

impl UrgencyFn {
    pub fn evaluate(&self, created_at: f64, now: f64, dismiss_count: u32) -> f64 {
        let raw = match self {
            Self::Linear { base, slope } => base + slope * (now - created_at).max(0.0),
            Self::Sigmoid { deadline, steepness, offset } => {
                1.0 / (1.0 + (steepness * (deadline - now - offset)).exp())
            }
            Self::StepAtDeadline { deadline, buffer_secs } => {
                if now >= deadline - buffer_secs {
                    1.0
                } else {
                    0.0
                }
            }
            Self::DecayIfIgnored { initial, decay_factor } => {
                initial * decay_factor.powi(dismiss_count.min(i32::MAX as u32) as i32)
            }
            Self::Constant { value } => *value,
        };
        // A NaN (from a bad stored number) must not poison the ordering.
        if raw.is_nan() {
            0.0
        } else {
            raw.clamp(0.0, 1.0)
        }
    }
}

/// The named terms and their sum.
#[derive(Debug, Clone, PartialEq)]
pub struct Salience {
    pub terms: Vec<(&'static str, f64)>,
    pub total: f64,
}

pub fn salience(e: &Entry, now: f64) -> Salience {
    let deadline = W_DEADLINE * e.urgency.evaluate(e.created_at, now, e.dismiss_count);
    let stale_hours = ((now - e.last_moved_at) / 3600.0).max(0.0);
    let stale = W_STALE * (stale_hours / STALE_FULL_HOURS).min(1.0);
    let blocked_hours = e.blocked_since.map_or(0.0, |b| ((now - b) / 3600.0).max(0.0));
    let blocked = W_BLOCKED * (blocked_hours / BLOCKED_FULL_HOURS).min(1.0);
    let confidence = W_CONFIDENCE * e.confidence.clamp(0.0, 1.0);
    let terms = vec![
        ("deadline", deadline),
        ("time_since_moved", stale),
        ("time_blocked", blocked),
        ("evidence_confidence", confidence),
        // Named now so they have a place; the drives and the executive fill them in later.
        ("drive_pressure", 0.0),
        ("fit_with_activity", 0.0),
    ];
    let total = terms.iter().map(|(_, v)| v).sum();
    Salience { terms, total }
}

/// Active entries, best first. Closed entries, snoozed ones and ones that cannot be read are
/// left out. Ties break on id so the order never changes between calls. One SQL read.
pub fn ranked(conn: &Connection, now: f64, limit: usize) -> rusqlite::Result<Vec<(Entry, Salience)>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM agenda WHERE state NOT IN ('done','dropped') AND snoozed_until <= ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([now], |r| Ok(read_entry(r)))?;
    let mut scored: Vec<(Entry, Salience)> = Vec::new();
    for row in rows {
        if let Ok(Some(e)) = row {
            let s = salience(&e, now);
            scored.push((e, s));
        }
    }
    scored.sort_by(|a, b| {
        b.1.total.partial_cmp(&a.1.total).unwrap_or(std::cmp::Ordering::Equal).then(a.0.id.cmp(&b.0.id))
    });
    scored.truncate(limit);
    Ok(scored)
}
