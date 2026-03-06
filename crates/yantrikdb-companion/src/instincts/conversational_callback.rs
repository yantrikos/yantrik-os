//! Conversational Callback instinct — references past conversations naturally.
//!
//! Detects when current context echoes something discussed before and
//! makes natural callbacks: "Remember when you were debugging that..."

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct ConversationalCallbackInstinct;

impl Instinct for ConversationalCallbackInstinct {
    fn name(&self) -> &str {
        "ConversationalCallback"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only at Acquaintance+
        if state.bond_level < BondLevel::Acquaintance {
            return vec![];
        }

        // Need some memory to callback to
        if state.memory_count < 5 {
            return vec![];
        }

        // Only during reasonable hours
        if state.current_hour < 8 || state.current_hour > 22 {
            return vec![];
        }

        // Don't interrupt active conversations
        if state.idle_seconds < 300.0 && state.conversation_turn_count > 0 {
            return vec![];
        }

        // Need some idle time (user isn't in the middle of something)
        if state.idle_seconds < 600.0 {
            return vec![];
        }

        let urgency = match state.bond_level {
            BondLevel::Acquaintance => 0.3,
            BondLevel::Friend => 0.4,
            BondLevel::Confidant => 0.45,
            BondLevel::PartnerInCrime => 0.5,
            _ => 0.25,
        };

        // Use recent events as context triggers for callbacks
        let recent_context: Vec<&str> = state
            .recent_events
            .iter()
            .filter(|(_, ts, _)| state.current_ts - ts < 3600.0) // Last hour
            .map(|(desc, _, _)| desc.as_str())
            .collect();

        let context_hint = if recent_context.is_empty() {
            "No specific recent context — recall something interesting from past conversations.".to_string()
        } else {
            format!(
                "Recent activity: [{}]. Look for connections to past conversations.",
                recent_context.join("; ")
            )
        };

        vec![
            UrgeSpec::new(
                self.name(),
                &format!(
                    "EXECUTE Search your memory for a past conversation or event that connects \
                     to the current moment. {}. Make a brief, natural callback — \
                     'Remember when...' or 'This reminds me of...' or 'Didn't you mention...'. \
                     Keep it to 1-2 sentences. If nothing connects naturally, don't force it — \
                     just say nothing (return empty).",
                    context_hint
                ),
                urgency,
            )
            .with_cooldown("callback:periodic"),
        ]
    }
}
