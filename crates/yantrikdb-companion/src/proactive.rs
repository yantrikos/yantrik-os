//! Proactive conversation engine — delivers urge-based messages without the LLM.
//!
//! When an urge reaches sufficient urgency, the engine composes a message
//! from instinct-specific templates and pushes it to the user via the
//! proactive message channel. No LLM required.
//!
//! V15: Now uses the `proactive_templates` engine first (bond-aware templates
//! with data slots), falling back to the legacy `compose_message` for instincts
//! that don't have templates yet.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::bond::BondLevel;
use crate::config::ProactiveConfig;
use crate::proactive_templates::TemplateEngine;
use crate::types::{ProactiveMessage, Urge};
use crate::urges::UrgeQueue;

/// Engine that converts high-urgency urges into proactive messages.
///
/// V15 frequency governor: cooldown scales with bond level, and a question
/// budget ensures we don't ask too many questions (2:1 statement-to-question ratio).
pub struct ProactiveEngine {
    config: ProactiveConfig,
    last_delivery_ts: f64,
    user_name: String,
    templates: TemplateEngine,
    bond_level: BondLevel,
    /// Rolling count of statements delivered (resets every 24h).
    statements_today: u32,
    /// Rolling count of questions delivered (resets every 24h).
    questions_today: u32,
    /// Timestamp of last daily reset.
    daily_reset_ts: f64,
}

impl ProactiveEngine {
    pub fn new(config: ProactiveConfig, user_name: &str) -> Self {
        Self {
            config,
            last_delivery_ts: 0.0,
            user_name: user_name.to_string(),
            templates: TemplateEngine::new(),
            bond_level: BondLevel::Stranger,
            statements_today: 0,
            questions_today: 0,
            daily_reset_ts: 0.0,
        }
    }

    /// Update the bond level used for template rendering and frequency gating.
    pub fn set_bond_level(&mut self, level: BondLevel) {
        self.bond_level = level;
    }

    /// Bond-based cooldown in seconds.
    ///
    /// Stranger: 60 min, Acquaintance: 45 min, Friend: 30 min,
    /// Confidant: 20 min, Partner-in-Crime: 10 min.
    fn effective_cooldown_secs(&self) -> f64 {
        let bond_cooldown: f64 = match self.bond_level {
            BondLevel::Stranger => 60.0 * 60.0,
            BondLevel::Acquaintance => 45.0 * 60.0,
            BondLevel::Friend => 30.0 * 60.0,
            BondLevel::Confidant => 20.0 * 60.0,
            BondLevel::PartnerInCrime => 10.0 * 60.0,
        };
        // Config cooldown is a floor — never go below configured minimum
        let config_cooldown = self.config.cooldown_minutes as f64 * 60.0;
        bond_cooldown.max(config_cooldown)
    }

    /// Check if sending a question is within budget (2:1 statement-to-question ratio).
    fn question_budget_ok(&self, is_question: bool) -> bool {
        if !is_question {
            return true;
        }
        // Allow at least 1 question even with 0 statements
        if self.questions_today == 0 {
            return true;
        }
        // 2:1 ratio — need at least 2 statements per question
        self.statements_today >= self.questions_today * 2
    }

    /// Reset daily counters if a new day has started.
    fn maybe_reset_daily(&mut self, now: f64) {
        if now - self.daily_reset_ts > 86400.0 {
            self.statements_today = 0;
            self.questions_today = 0;
            self.daily_reset_ts = now;
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
            tracing::info!("Proactive disabled");
            return None;
        }

        let now = now_ts();
        self.maybe_reset_daily(now);

        // V15 frequency governor: bond-based cooldown
        let cooldown_secs = self.effective_cooldown_secs();
        let elapsed = now - self.last_delivery_ts;
        if elapsed < cooldown_secs {
            tracing::info!(
                elapsed_secs = elapsed as u64,
                cooldown_secs = cooldown_secs as u64,
                bond = self.bond_level.name(),
                "Proactive cooldown active (bond-scaled)"
            );
            return None;
        }

        tracing::info!(
            elapsed_secs = elapsed as u64,
            bond = self.bond_level.name(),
            "Proactive cooldown expired, checking urges"
        );

        // Peek at top pending urge
        let pending = urge_queue.get_pending(conn, 1);
        let urge = match pending.first() {
            Some(u) => u,
            None => {
                tracing::info!("Proactive check: no pending urges");
                return None;
            }
        };

        // Must exceed urgency threshold
        if urge.urgency < self.config.delivery_threshold {
            tracing::info!(
                urgency = urge.urgency,
                threshold = self.config.delivery_threshold,
                "Proactive check: urgency below threshold"
            );
            return None;
        }

        // Must have a suggested message (instinct should populate this)
        if urge.suggested_message.is_empty() && urge.reason.is_empty() {
            tracing::info!(
                instinct = urge.instinct_name,
                "Proactive check: no message text"
            );
            return None;
        }

        // Pop it (marks as delivered in the urge queue)
        let delivered = urge_queue.pop_for_interaction(conn, 1);
        let urge = delivered.into_iter().next()?;

        let text = self.compose_message(&urge);

        // Skip delivery if compose returned empty (e.g. humor hint without concrete text)
        if text.is_empty() {
            tracing::info!(
                instinct = urge.instinct_name,
                "Proactive skipped — no composable message"
            );
            return None;
        }

        // V15: Question budget — check if this message is a question
        let is_question = text.ends_with('?');
        if !self.question_budget_ok(is_question) {
            tracing::info!(
                statements = self.statements_today,
                questions = self.questions_today,
                "Proactive skipped — question budget exceeded (2:1 ratio)"
            );
            return None;
        }

        self.last_delivery_ts = now;

        // Track statement/question counts
        if is_question {
            self.questions_today += 1;
        } else {
            self.statements_today += 1;
        }

        tracing::info!(
            instinct = urge.instinct_name,
            urgency = urge.urgency,
            is_question,
            statements = self.statements_today,
            questions = self.questions_today,
            "Proactive message delivered"
        );

        Some(ProactiveMessage {
            text,
            urge_ids: vec![urge.urge_id],
            generated_at: now,
        })
    }

    /// Compose a user-facing message from an urge.
    ///
    /// Tries V15 template engine first (bond-aware, data-slot templates).
    /// Falls back to legacy hardcoded patterns for instincts without templates.
    fn compose_message(&mut self, urge: &Urge) -> String {
        let instinct = urge.instinct_name.to_lowercase();

        // Build data slots from the urge's context + standard fields
        let data = self.build_data_slots(urge);

        // Try template engine first
        if let Some(rendered) = self.templates.render(&instinct, &data, self.bond_level) {
            return rendered;
        }

        // Legacy fallback for instincts without templates
        self.compose_legacy(urge)
    }

    /// Build the data slot map from an urge for template rendering.
    fn build_data_slots(&self, urge: &Urge) -> HashMap<String, String> {
        let mut data = HashMap::new();

        // Standard slots available to all templates
        data.insert("user".into(), self.user_name.clone());
        data.insert("reason".into(), urge.reason.clone());
        if !urge.suggested_message.is_empty() {
            data.insert("message".into(), urge.suggested_message.clone());
        }

        // Extract slots from urge context JSON
        if let Some(obj) = urge.context.as_object() {
            for (key, val) in obj {
                let s = match val {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => val.to_string(),
                };
                if !s.is_empty() && s != "null" {
                    data.insert(key.clone(), s);
                }
            }
        }

        data
    }

    /// Legacy message composition (pre-V15).
    fn compose_legacy(&self, urge: &Urge) -> String {
        let user = &self.user_name;
        let reason = &urge.reason;
        let msg = &urge.suggested_message;

        let instinct = urge.instinct_name.to_lowercase();
        match instinct.as_str() {
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
                // Memory conflicts are internal housekeeping, not user-facing
                return String::new();
            }
            "bond_milestone" | "bondmilestone" => {
                if msg.is_empty() {
                    reason.clone()
                } else {
                    msg.clone()
                }
            }
            "scheduler" => {
                if msg.is_empty() {
                    format!("Scheduled: {}", reason)
                } else {
                    msg.clone()
                }
            }
            "self_awareness" | "selfawareness" => reason.clone(),
            "humor" => {
                // Humor urges are tone hints for conversations, not standalone messages.
                // Only deliver if the instinct provided a concrete suggested_message.
                if msg.is_empty() {
                    return String::new(); // Skip — raw hint, not user-facing
                }
                msg.clone()
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
