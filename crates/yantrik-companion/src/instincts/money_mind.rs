//! MoneyMind instinct — "Financial awareness, not financial advice."
//!
//! Connects to the user's financial interests and conversations. If they watch
//! markets, it provides deeper analysis than surface-level stock tickers. If they
//! mention spending, saving, or investing decisions, it researches the specific
//! decision with real data.
//!
//! The key difference from a finance news feed: MoneyMind is contextual. It only
//! activates when the user has demonstrated financial interest, and it delivers
//! the specific insight that's relevant to THEIR situation — not generic tips.
//!
//! Example output: "You've been tracking REITs. Here's something most investors
//! miss: REIT dividend tax treatment changed in 2025 — the 199A deduction now
//! applies differently to..."

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// Finance-related keywords to detect in user interests or conversation.
const FINANCE_KEYWORDS: &[&str] = &[
    "finance", "investing", "stocks", "crypto", "budget", "saving",
    "money", "market", "trading", "portfolio", "retirement", "401k",
    "reit", "etf", "bonds", "dividend", "real estate", "mortgage",
    "financial", "economics", "inflation", "interest rate",
];

pub struct MoneyMindInstinct {
    /// Seconds between financial awareness checks.
    interval_secs: f64,
    /// Last evaluation timestamp.
    last_check_ts: Mutex<f64>,
}

impl MoneyMindInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for MoneyMindInstinct {
    fn name(&self) -> &str {
        "MoneyMind"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // Rate-limit with cold-start guard
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

        // Gate: bond level — at least Acquaintance to discuss finances
        if state.bond_level < BondLevel::Acquaintance {
            return vec![];
        }

        // Gate: user must have finance-related interests OR recent conversation
        // mentions money/investing/budget topics
        let has_finance_interest = state.user_interests.iter().any(|interest| {
            let lower = interest.to_lowercase();
            FINANCE_KEYWORDS.iter().any(|kw| lower.contains(kw))
        });

        let has_finance_conversation = state.recent_sent_messages.iter().any(|msg| {
            let lower = msg.to_lowercase();
            FINANCE_KEYWORDS.iter().any(|kw| lower.contains(kw))
        });

        if !has_finance_interest && !has_finance_conversation {
            return vec![];
        }

        let user = &state.config_user_name;

        let execute_msg = format!(
            "EXECUTE First, use recall with query \"money budget investing saving spending financial\" \
             to find {user}'s financial mentions and interests.\n\
             \n\
             Identify the specific financial topic {user} cares about most right now:\n\
             - Are they tracking specific investments or markets?\n\
             - Have they mentioned a spending or saving decision?\n\
             - Is there a financial goal or concern they've expressed?\n\
             \n\
             Then use web_search to find a deeper insight or recent research related to that \
             specific topic. Look for the angle that most people miss — the hidden fee, the \
             tax implication, the counterintuitive data point.\n\
             \n\
             Deliver ONE actionable insight in 2-3 sentences. It must be specific to what \
             {user} mentioned — NOT generic financial advice.\n\
             \n\
             Example tone: \"You've been tracking REITs. Here's something most investors miss: \
             REIT dividend tax treatment changed in 2025 — the 199A deduction now applies \
             differently to...\"\n\
             \n\
             If no financial topics found in memory, respond with just \"No money mind today.\"\n\
             After you're done, call browser_cleanup to free resources.",
        );

        vec![UrgeSpec::new(self.name(), &execute_msg, 0.4)
            .with_cooldown("money_mind:insight")]
    }
}
