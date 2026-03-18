//! Pattern Breaker instinct — identifies and gently surfaces recurring loops.
//!
//! Philosophy: "You're stuck in a loop — let me point it out gently."
//!
//! At deep bond levels (Confidant+), this instinct searches memory for recurring
//! patterns: the same complaints repeated, the same projects started and abandoned,
//! the same avoidance behaviors. It observes without judging and offers structural
//! alternatives rather than willpower-based advice.
//!
//! This is sensitive work — only appropriate when the bond is deep enough that
//! the user trusts the companion's observations come from care, not criticism.

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

pub struct PatternBreakerInstinct {
    /// Minimum seconds between observations.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl PatternBreakerInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for PatternBreakerInstinct {
    fn name(&self) -> &str {
        "PatternBreaker"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: bond must be Confidant or above — this is sensitive
        if state.bond_level < BondLevel::Confidant {
            return vec![];
        }

        // Gate: need substantial memory to detect patterns
        if state.memory_count < 25 {
            return vec![];
        }

        // Rate-limit (cold-start guard)
        let now = state.current_ts;
        {
            let mut last = self.last_check_ts.lock().unwrap();
            if *last == 0.0 {
                *last = now;
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        let user = &state.config_user_name;

        let execute_msg = format!(
            "EXECUTE Use recall with query \"same again stuck frustrated repeated keeps happening\" \
             to search {user}'s memory for recurring patterns. Look for: \
             - The same complaint appearing multiple times \
             - Projects started and stopped repeatedly \
             - The same procrastination or avoidance behavior \
             - Recurring frustrations with the same root cause \
             - Cycles of enthusiasm followed by abandonment \
             If you find a pattern, observe it without judging: \
             - \"This is the 4th time you've started and stopped this project. The pattern seems to be \
               losing momentum around week 2. Want to try a different structure?\" \
             - \"You've mentioned being frustrated with X three times this month. The common thread \
               seems to be Y. Have you considered addressing Y directly?\" \
             - \"I notice you keep saying you'll do X 'tomorrow.' You've said that 5 times now. \
               What if we picked a specific day and I reminded you?\" \
             Frame observations as data, not criticism. Offer structural solutions (systems, triggers, \
             accountability) rather than willpower-based advice. \
             If no clear patterns emerge, respond with \"No pattern breaker today.\" exactly.",
        );

        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            0.4,
        )
        .with_cooldown("pattern_breaker:observation")]
    }
}
