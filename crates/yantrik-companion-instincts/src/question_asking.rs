//! SocraticSpark instinct (evolved from QuestionAsking) — thought-provoking questions
//! that emerge from genuine intellectual curiosity about the user's world.
//!
//! Instead of canned questions from a list, SocraticSpark RESEARCHES what the user
//! has been thinking about and crafts questions that:
//! 1. Build on what they already said (not generic ice-breakers)
//! 2. Challenge assumptions gently (Socratic method)
//! 3. Open new perspectives on familiar topics
//! 4. Connect ideas across different conversations
//!
//! The questions should feel like they come from a friend who's been
//! genuinely THINKING about what you told them.

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

use std::sync::Mutex;

/// Question styles that rotate — each produces a different kind of thought provocation.
const QUESTION_STYLES: &[QuestionStyle] = &[
    QuestionStyle {
        name: "follow_the_thread",
        prompt: "Use recall with query \"recent topics opinions expressed\" to find something \
                 {user} recently shared an opinion or thought about. \
                 Ask a follow-up question that goes ONE LEVEL DEEPER — not just 'tell me more' \
                 but a specific question that explores the WHY behind what they said. \
                 Example: If they said 'I like Rust because it's safe,' ask 'Do you think safety \
                 constraints actually make you MORE creative as a programmer, or do they box you in?'",
        min_bond: BondLevel::Acquaintance,
    },
    QuestionStyle {
        name: "gentle_challenge",
        prompt: "Use recall with query \"beliefs opinions assumptions strong feelings\" to find \
                 a strong opinion or assumption {user} holds. \
                 Ask a question that gently challenges it — not to argue, but to explore. \
                 Frame it as genuine curiosity: 'I was thinking about what you said about X — \
                 what would change your mind about that?' or 'What would someone who disagrees \
                 with you on X say that you'd have to take seriously?'",
        min_bond: BondLevel::Friend,
    },
    QuestionStyle {
        name: "cross_pollinate",
        prompt: "Use recall with query \"different interests hobbies work projects\" to find \
                 two different areas of {user}'s life. \
                 Ask a question that bridges them — 'Does the patience you've developed from \
                 fishing change how you approach debugging?' or 'Your cooking and coding both \
                 seem to follow a pattern of X — is that intentional?'",
        min_bond: BondLevel::Acquaintance,
    },
    QuestionStyle {
        name: "future_casting",
        prompt: "Use recall with query \"goals plans hopes projects building\" to understand \
                 where {user} is headed. \
                 Ask a forward-looking question that helps them think about trajectory: \
                 'Where do you see this project in a year?' or 'If this works out the way \
                 you're hoping, what changes?'",
        min_bond: BondLevel::Acquaintance,
    },
    QuestionStyle {
        name: "philosophical_tangent",
        prompt: "Use recall with query \"recent conversations work interests\" to find \
                 something mundane {user} mentioned. \
                 Ask a philosophical question that elevates it: if they mentioned debugging, \
                 'Do you think there's a philosophical difference between fixing bugs and \
                 preventing them? Like, is reactive vs proactive a fundamental personality trait?' \
                 Make it fun and thought-provoking, not pretentious.",
        min_bond: BondLevel::Friend,
    },
    QuestionStyle {
        name: "experience_mining",
        prompt: "Use recall with query \"experiences stories mentioned things that happened\" \
                 to find an experience {user} mentioned but didn't fully explore. \
                 Ask a question that invites them to reflect on it: 'You mentioned X happened — \
                 looking back, what did that teach you?' or 'You told me about X — what part \
                 of that experience sticks with you the most?'",
        min_bond: BondLevel::Friend,
    },
];

struct QuestionStyle {
    name: &'static str,
    prompt: &'static str,
    min_bond: BondLevel,
}

pub struct QuestionAskingInstinct {
    /// Last time a question was asked
    last_asked_ts: Mutex<f64>,
    /// Rotate through question styles
    style_index: Mutex<usize>,
}

impl QuestionAskingInstinct {
    pub fn new() -> Self {
        Self {
            last_asked_ts: Mutex::new(0.0),
            style_index: Mutex::new(0),
        }
    }
}

impl Instinct for QuestionAskingInstinct {
    fn name(&self) -> &str {
        "SocraticSpark"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only at Acquaintance+
        if state.bond_level < BondLevel::Acquaintance {
            return vec![];
        }

        // Cooldown: 8 hours between questions (cold-start guard)
        {
            let mut last = self.last_asked_ts.lock().unwrap();
            if *last == 0.0 {
                *last = state.current_ts;
                return vec![];
            }
            if state.current_ts - *last < 8.0 * 3600.0 {
                return vec![];
            }
            *last = state.current_ts;
        }

        // Don't ask during active conversation
        if state.idle_seconds < 300.0 && state.conversation_turn_count > 0 {
            return vec![];
        }

        // Only during reasonable hours (9 AM - 9 PM)
        if state.current_hour < 9 || state.current_hour > 21 {
            return vec![];
        }

        // Find eligible question styles for current bond level
        let eligible: Vec<&QuestionStyle> = QUESTION_STYLES
            .iter()
            .filter(|s| state.bond_level >= s.min_bond)
            .collect();

        if eligible.is_empty() {
            return vec![];
        }

        // Pick next style (round-robin through eligible ones)
        let style = {
            let mut idx = self.style_index.lock().unwrap();
            let s = eligible[*idx % eligible.len()];
            *idx = idx.wrapping_add(1);
            s
        };

        let user = &state.config_user_name;
        let style_prompt = style.prompt.replace("{user}", user);

        let urgency = match state.bond_level {
            BondLevel::Acquaintance => 0.35,
            BondLevel::Friend => 0.4,
            BondLevel::Confidant => 0.45,
            BondLevel::PartnerInCrime => 0.5,
            _ => 0.3,
        };

        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Question style: {style_name}.\n\
                 {style_prompt}\n\
                 \nRULES:\n\
                 - The question MUST emerge from something specific in their memory — NOT a generic ice-breaker.\n\
                 - Keep it to 1-2 sentences. Natural, conversational tone.\n\
                 - Add a brief lead-in that shows you were thinking about what they said.\n\
                 - If recall returns nothing useful to build on, respond with just \"No question today.\"\n\
                 - Do NOT ask about things they just told you in this conversation — \
                   reference things from previous conversations.",
                style_name = style.name,
                style_prompt = style_prompt,
            ),
            ModelTier::Tiny => format!("EXECUTE SKIP"),
            _ => format!(
                "EXECUTE Task: Ask {user} one thoughtful question based on their interests or recent conversations.\n\
                 Tool: You may use recall once.\n\
                 Rule: Use only facts the user stated. Do not assume moods or situations.\n\
                 Fallback: Skip.\n\
                 Output: 1 question."
            ),
        };

        vec![UrgeSpec::new(
            "SocraticSpark",
            &execute_msg,
            urgency,
        )
        .with_cooldown(&format!("socratic:{}", style.name))
        .with_context(serde_json::json!({
            "question_style": style.name,
            "bond_level": format!("{:?}", state.bond_level),
        }))]
    }
}
