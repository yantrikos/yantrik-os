//! The score: a readable sum of named terms. No randomness, no clock (time is passed in).
//!
//! ```text
//! u     = urgency in 0..1 (see `UrgencyFn`; a dated entry: 1 / (1 + left / H), no flat region)
//! c     = confidence clamped to [0.3, 1] ([0.5, 1] for a promise); it multiplies, it is not a term
//!
//! deadline = W_DEADLINE * u * c
//! overdue  = W_OVERDUE  * min(days overdue / OVERDUE_FULL_DAYS, 1)
//! stale    = W_STALE    * min(days since last moved / STALE_FULL_DAYS, 1)
//! blocked  = W_BLOCKED  * min(days blocked / BLOCKED_FULL_DAYS, 1)
//! drives   = 0 (this slice)        fit = 0 (this slice)
//! total    = the sum of the six terms
//! ```
//!
//! A blocked entry earns both `stale` and `blocked` over the same days (`blocked_since` is set
//! when it starts waiting, which is also when it last moved), so one that is blocked and
//! untouched reaches `W_STALE + W_BLOCKED` on purpose: it is the strongest nudge.
//!
//! Ties break by the nearer deadline, then the older entry, then the id, so the order is total
//! and never changes between calls.
//!
//! Every number that shapes the order is in the block of constants below and nowhere else.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::store::{read_entry, Entry, COLUMNS};
use super::Kind;

const DAY: f64 = 86_400.0;

// ---- The weights and shapes: the one table ----
pub const W_DEADLINE: f64 = 3.0;
pub const W_OVERDUE: f64 = 1.0;
pub const W_STALE: f64 = 0.3;
pub const W_BLOCKED: f64 = 0.3;
/// Days overdue after which the `overdue` term is at full strength.
pub const OVERDUE_FULL_DAYS: f64 = 3.0;
/// Days of not moving after which `stale` is at full strength.
pub const STALE_FULL_DAYS: f64 = 14.0;
/// Days of being blocked after which `blocked` is at full strength.
pub const BLOCKED_FULL_DAYS: f64 = 7.0;
/// `H` in `u = 1 / (1 + left / H)`: the time left at which a dated entry is half urgent.
pub const DEADLINE_HALF_DAYS: f64 = 2.0;
/// An undated promise: `min(base + age_days / UNDATED_DAYS, cap)`. The cap keeps it below a
/// dated promise within eight days (`1 / (1 + 8 / 2) = 0.2`).
pub const UNDATED_BASE: f64 = 0.10;
pub const UNDATED_DAYS: f64 = 365.0;
pub const UNDATED_CAP: f64 = 0.20;
/// A queued task's urgency by priority 1 (least urgent) up to 4 (most). A promise is what a
/// person forgets; a queued task is something the machine was going to do anyway, so these sit
/// low.
pub const TASK_URGENCY: [f64; 4] = [0.08, 0.15, 0.25, 0.35];
/// Confidence is clamped to at least this, so weak evidence still counts for something.
pub const MIN_CONFIDENCE: f64 = 0.3;
/// A promise found at all is worth at least half weight, however weakly it was extracted.
pub const PROMISE_MIN_CONFIDENCE: f64 = 0.5;
/// Confidence of a task the person queued.
pub const PERSON_CONFIDENCE: f64 = 1.0;
/// Confidence of a task a model created.
pub const MODEL_CONFIDENCE: f64 = 0.7;
/// A schedule's author is not recorded, so it is treated as unknown (its label is withheld for
/// the same reason): one unknown, one direction.
pub const SCHEDULE_CONFIDENCE: f64 = MODEL_CONFIDENCE;
/// A schedule with no next run takes the urgency its row gives, or this.
pub const SCHEDULE_DEFAULT_URGENCY: f64 = 0.6;
/// An open thread: a constant confidence, and urgency `importance * THREAD_URGENCY_SCALE`
/// when it has no deadline. Importance counts once (in the urgency), not twice.
pub const THREAD_CONFIDENCE: f64 = 0.8;
pub const THREAD_URGENCY_SCALE: f64 = 0.5;
/// A memory conflict's urgency by its priority: low, medium (and anything else), high,
/// critical; and its confidence.
pub const CONFLICT_URGENCY: [f64; 4] = [0.1, 0.3, 0.6, 0.9];
pub const CONFLICT_CONFIDENCE: f64 = 0.5;

/// Urgency of a queued task by its priority (1..4; anything else is clamped into range).
pub fn task_urgency(priority: i64) -> f64 {
    TASK_URGENCY[(priority.clamp(1, 4) - 1) as usize]
}

/// How the deadline term moves with time. The names and formulas of the first five follow the
/// memory store's `UrgencyFn` so the two can become one later without changing what anything
/// means; they stay available for later senses. This slice's mirrors use `Hyperbolic`,
/// `CappedLinear` and `Constant`.
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
    /// `1 / (1 + left / half_secs)` for `left = deadline - now >= 0`, and 1 once due. Strictly
    /// decreasing in `left`: it never goes flat, so a nearer deadline always scores higher.
    Hyperbolic { deadline: f64, half_secs: f64 },
    /// `min(base + slope * (now - created_at), cap)`.
    CappedLinear { base: f64, slope: f64, cap: f64 },
}

impl UrgencyFn {
    /// A dated entry's urgency, with `H` from the constants above.
    pub fn dated(deadline: f64) -> Self {
        Self::Hyperbolic { deadline, half_secs: DEADLINE_HALF_DAYS * DAY }
    }

    /// An undated promise's urgency.
    pub fn undated_promise() -> Self {
        Self::CappedLinear { base: UNDATED_BASE, slope: 1.0 / (UNDATED_DAYS * DAY), cap: UNDATED_CAP }
    }

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
            Self::Hyperbolic { deadline, half_secs } => {
                let left = (deadline - now).max(0.0);
                1.0 / (1.0 + left / half_secs)
            }
            Self::CappedLinear { base, slope, cap } => {
                (base + slope * (now - created_at).max(0.0)).min(*cap)
            }
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
    // A NaN confidence counts as the weakest evidence, so the total is never NaN.
    let floor = if e.kind == Kind::Promise { PROMISE_MIN_CONFIDENCE } else { MIN_CONFIDENCE };
    let c = if e.confidence.is_nan() { floor } else { e.confidence.clamp(floor, 1.0) };
    let u = e.urgency.evaluate(e.created_at, now, e.dismiss_count);
    let deadline = W_DEADLINE * u * c;
    let days_overdue = e.due_at.map_or(0.0, |d| ((now - d) / DAY).max(0.0));
    let overdue = W_OVERDUE * (days_overdue / OVERDUE_FULL_DAYS).min(1.0);
    let days_moved = ((now - e.last_moved_at) / DAY).max(0.0);
    let stale = W_STALE * (days_moved / STALE_FULL_DAYS).min(1.0);
    let days_blocked = e.blocked_since.map_or(0.0, |b| ((now - b) / DAY).max(0.0));
    let blocked = W_BLOCKED * (days_blocked / BLOCKED_FULL_DAYS).min(1.0);
    let terms = vec![
        ("deadline", deadline),
        ("overdue", overdue),
        ("stale", stale),
        ("blocked", blocked),
        // Named now so they have a place; the drives and the executive fill them in later.
        ("drives", 0.0),
        ("fit", 0.0),
    ];
    let total = terms.iter().map(|(_, v)| v).sum();
    Salience { terms, total }
}

/// Active entries, best first. Closed entries, snoozed ones and ones that cannot be read are
/// left out. One SQL read.
pub fn ranked(conn: &Connection, now: f64, limit: usize) -> rusqlite::Result<Vec<(Entry, Salience)>> {
    let sql = format!("SELECT {COLUMNS} FROM agenda WHERE is_open = 1 AND snoozed_until <= ?1");
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
        b.1.total
            .total_cmp(&a.1.total)
            // No deadline sorts after any deadline.
            .then(a.0.due_at.unwrap_or(f64::INFINITY).total_cmp(&b.0.due_at.unwrap_or(f64::INFINITY)))
            .then(a.0.created_at.total_cmp(&b.0.created_at))
            .then(a.0.id.cmp(&b.0.id))
    });
    scored.truncate(limit);
    Ok(scored)
}

