//! Conflict alerting instinct — surfaces when too many memory conflicts pile up.
//!
//! v4: Hash-dedup to prevent firing every evaluation cycle with the same conflicts.
//! Only generates an urge when the conflict set changes or 6h TTL expires.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

pub struct ConflictAlertingInstinct {
    threshold: usize,
    /// Hash of the last conflict state we reported on.
    last_conflict_hash: Mutex<u64>,
    /// Timestamp of last urge generation (for 6h TTL).
    last_fire_ts: Mutex<f64>,
}

impl ConflictAlertingInstinct {
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold,
            last_conflict_hash: Mutex::new(0),
            last_fire_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for ConflictAlertingInstinct {
    fn name(&self) -> &str {
        "conflict_alerting"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        if state.open_conflicts_count < self.threshold {
            return vec![];
        }

        // Hash-dedup: compute a signature from the conflict count.
        // Only fire if the conflict state changed OR 6h TTL expired.
        let mut hasher = DefaultHasher::new();
        state.open_conflicts_count.hash(&mut hasher);
        let current_hash = hasher.finish();

        let now = state.current_ts;
        {
            let last_hash = *self.last_conflict_hash.lock().unwrap();
            let last_ts = *self.last_fire_ts.lock().unwrap();

            let same_conflicts = current_hash == last_hash;
            let ttl_ok = last_ts > 0.0 && (now - last_ts) < 6.0 * 3600.0; // 6h TTL

            if same_conflicts && ttl_ok {
                return vec![]; // Same conflicts, not expired — skip
            }
        }

        // Update tracking state
        *self.last_conflict_hash.lock().unwrap() = current_hash;
        *self.last_fire_ts.lock().unwrap() = now;

        let urgency = (state.open_conflicts_count as f64 / 10.0).min(0.8);

        vec![UrgeSpec::new(
            "conflict_alerting",
            &format!(
                "{} memory conflicts need your help",
                state.open_conflicts_count
            ),
            urgency,
        )
        .with_cooldown("conflict_alert")
        .with_message(&format!(
            "I have some conflicting memories that I could use your help sorting out — {} things don't quite add up.",
            state.open_conflicts_count
        ))
        .with_context(serde_json::json!({
            "open_count": state.open_conflicts_count,
        }))]
    }
}
