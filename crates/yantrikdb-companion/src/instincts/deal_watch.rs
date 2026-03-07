//! Deal Watch instinct — proactive deal/price monitoring for items the user wants.
//!
//! When the user mentions wanting to buy something, Yantrik remembers it.
//! This instinct periodically checks for deals, price drops, flash sales,
//! and coupons for items on the user's wish list.
//!
//! Also monitors general deal sites for categories the user shops in.
//!
//! Examples:
//!   "I need a new laptop" → monitors laptop deals on Slickdeals, Amazon, Best Buy
//!   "looking for a good air fryer" → checks air fryer reviews + deals
//!   "I like sneakers" → alerts on sneaker drops and sales

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct DealWatchInstinct {
    /// Seconds between deal checks.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl DealWatchInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for DealWatchInstinct {
    fn name(&self) -> &str {
        "DealWatch"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // Rate-limit (cold-start guard)
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

        // Only check if user has shopping-related interests
        let has_shopping_interest = state.user_interests.iter().any(|i| {
            let lower = i.to_lowercase();
            lower.contains("shopping")
                || lower.contains("deals")
                || lower.contains("buy")
                || lower.contains("fashion")
                || lower.contains("sneaker")
                || lower.contains("tech gadget")
                || lower.contains("electronics")
        });

        // Always check — even without explicit shopping interest, the user may have
        // mentioned wanting specific items in conversation.
        let user = &state.config_user_name;

        let execute_msg = format!(
            "EXECUTE First use recall with query \"want to buy shopping wishlist looking for\" to check if \
             {user} has mentioned wanting to buy anything specific. \
             Also use recall with query \"shopping preferences budget brands\" for their shopping style. \
             If they have specific items they want, use web_search to search for deals on the MOST \
             wanted item (e.g., \"best deal [item] today site:slickdeals.net OR site:reddit.com/r/deals\"). \
             If no specific items, but they like shopping, search for \"best deals today\" filtered \
             to their preferred categories. \
             Only report if you find a GENUINELY good deal (30%+ off, lowest price, flash sale). \
             Share it in 2-3 sentences with the price, where, and why it's good. \
             If nothing notable, respond with just \"No noteworthy deals today.\" \
             After you're done, call browser_cleanup to free resources.",
        );

        let urgency = if has_shopping_interest { 0.5 } else { 0.3 };

        vec![UrgeSpec::new("DealWatch", &execute_msg, urgency)
            .with_cooldown("deal_watch:check")
            .with_context(serde_json::json!({
                "check_type": "deal_monitoring",
                "has_shopping_interest": has_shopping_interest,
            }))]
    }
}
