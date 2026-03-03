//! Proactive conversation engine — delivers urge-based messages without the LLM.
//!
//! When an urge reaches sufficient urgency, the engine composes a message
//! from instinct-specific templates and pushes it to the user via the
//! proactive message channel. No LLM required.

use rusqlite::Connection;

use crate::config::ProactiveConfig;
use crate::types::{ProactiveMessage, Urge};
use crate::urges::UrgeQueue;

/// Engine that converts high-urgency urges into proactive messages.
pub struct ProactiveEngine {
    config: ProactiveConfig,
    last_delivery_ts: f64,
    user_name: String,
}

impl ProactiveEngine {
    pub fn new(config: ProactiveConfig, user_name: &str) -> Self {
        Self {
            config,
            last_delivery_ts: 0.0,
            user_name: user_name.to_string(),
        }
    }

    /// Check if any pending urge qualifies for proactive delivery.
    ///
    /// Called during each think cycle (~60s). Returns a message if
    /// an urge exceeds the urgency threshold and cooldown has elapsed.
    pub fn check(
        &mut self,
        urge_queue: &UrgeQueue,
        conn: &Connection,
    ) -> Option<ProactiveMessage> {
        if !self.config.enabled {
            return None;
        }

        let now = now_ts();

        // Respect cooldown between proactive messages
        let cooldown_secs = self.config.cooldown_minutes as f64 * 60.0;
        if now - self.last_delivery_ts < cooldown_secs {
            return None;
        }

        // Peek at top pending urge
        let pending = urge_queue.get_pending(conn, 1);
        let urge = pending.first()?;

        // Must exceed urgency threshold
        if urge.urgency < self.config.delivery_threshold {
            return None;
        }

        // Must have a suggested message (instinct should populate this)
        if urge.suggested_message.is_empty() && urge.reason.is_empty() {
            return None;
        }

        // Pop it (marks as delivered in the urge queue)
        let delivered = urge_queue.pop_for_interaction(conn, 1);
        let urge = delivered.into_iter().next()?;

        let text = self.compose_message(&urge);
        self.last_delivery_ts = now;

        tracing::info!(
            instinct = urge.instinct_name,
            urgency = urge.urgency,
            "Proactive message delivered"
        );

        Some(ProactiveMessage {
            text,
            urge_ids: vec![urge.urge_id],
            generated_at: now,
        })
    }

    /// Compose a user-facing message from an urge using instinct templates.
    fn compose_message(&self, urge: &Urge) -> String {
        let user = &self.user_name;
        let reason = &urge.reason;
        let msg = &urge.suggested_message;

        match urge.instinct_name.as_str() {
            "check_in" => {
                if msg.is_empty() {
                    format!("Hey {}. {}", user, reason)
                } else {
                    msg.clone()
                }
            }
            "reminder" => {
                if msg.is_empty() {
                    format!("Reminder: {}", reason)
                } else {
                    msg.clone()
                }
            }
            "follow_up" => {
                if msg.is_empty() {
                    format!("By the way \u{2014} {}", reason)
                } else {
                    format!("By the way \u{2014} {}", msg)
                }
            }
            "emotional_awareness" => {
                if msg.is_empty() {
                    format!("I noticed {}.", reason)
                } else {
                    format!("I noticed {}. {}", reason, msg)
                }
            }
            "pattern_surfacing" => {
                format!("I've been noticing something: {}", reason)
            }
            "conflict_alerting" => {
                format!("Something seems off: {}", reason)
            }
            "bond_milestone" => {
                if msg.is_empty() {
                    reason.clone()
                } else {
                    msg.clone()
                }
            }
            "self_awareness" => reason.clone(),
            "humor" => {
                if msg.is_empty() {
                    reason.clone()
                } else {
                    msg.clone()
                }
            }
            _ => {
                // Unknown instinct — use whatever text is available
                if !msg.is_empty() {
                    msg.clone()
                } else {
                    reason.clone()
                }
            }
        }
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
