//! Question Asking instinct — builds the relationship by asking genuine questions.
//!
//! The only instinct that PULLS information instead of pushing it.
//! Graduated by bond level from casual to deep.

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

use std::sync::Mutex;

/// Questions graduated by bond level.
const STRANGER_QUESTIONS: &[&str] = &[
    "What kind of work keeps you busy most days?",
    "Are you a morning person or a night owl?",
    "Do you have a go-to way to unwind after a long day?",
];

const ACQUAINTANCE_QUESTIONS: &[&str] = &[
    "What have you been reading or watching lately?",
    "Do you prefer working with music on or in silence?",
    "What's something you've learned recently that surprised you?",
    "Is there a tool or app you couldn't live without?",
    "What's the most interesting problem you've worked on recently?",
];

const FRIEND_QUESTIONS: &[&str] = &[
    "What's been on your mind lately outside of work?",
    "If you had a free weekend with zero obligations, what would you do?",
    "What's a skill you've been wanting to pick up?",
    "Is there something about your workflow you wish was different?",
    "What got you into this line of work originally?",
    "What's a project you're secretly proud of?",
];

const CONFIDANT_QUESTIONS: &[&str] = &[
    "What's something you've been putting off that you wish you'd just do?",
    "What does a really good day look like for you?",
    "Is there anything you'd change about how we work together?",
    "What matters most to you right now in life?",
    "What's something most people don't know about you?",
];

pub struct QuestionAskingInstinct {
    /// Track which questions have been asked (index into each level's array)
    asked_indices: Mutex<Vec<(BondLevel, usize)>>,
    /// Last time a question was asked
    last_asked_ts: Mutex<f64>,
}

impl QuestionAskingInstinct {
    pub fn new() -> Self {
        Self {
            asked_indices: Mutex::new(Vec::new()),
            last_asked_ts: Mutex::new(0.0),
        }
    }

    fn get_question(&self, bond_level: BondLevel) -> Option<&'static str> {
        let asked = self.asked_indices.lock().ok()?;

        // Get the question pool for current bond level
        let pool = match bond_level {
            BondLevel::Stranger => STRANGER_QUESTIONS,
            BondLevel::Acquaintance => ACQUAINTANCE_QUESTIONS,
            BondLevel::Friend => FRIEND_QUESTIONS,
            BondLevel::Confidant | BondLevel::PartnerInCrime => CONFIDANT_QUESTIONS,
        };

        // Find first unasked question at this level
        let asked_at_level: Vec<usize> = asked
            .iter()
            .filter(|(lvl, _)| *lvl == bond_level)
            .map(|(_, idx)| *idx)
            .collect();

        for i in 0..pool.len() {
            if !asked_at_level.contains(&i) {
                return Some(pool[i]);
            }
        }

        // All asked — wrap around
        let idx = asked_at_level.len() % pool.len();
        Some(pool[idx])
    }
}

impl Instinct for QuestionAskingInstinct {
    fn name(&self) -> &str {
        "QuestionAsking"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only at Acquaintance+
        if state.bond_level < BondLevel::Acquaintance {
            return vec![];
        }

        // Cooldown: 8 hours between questions
        let cooldown_secs = 8.0 * 3600.0;
        if let Ok(last) = self.last_asked_ts.lock() {
            if state.current_ts - *last < cooldown_secs {
                return vec![];
            }
        }

        // Don't ask during active conversation (user sent message recently)
        if state.idle_seconds < 300.0 && state.conversation_turn_count > 0 {
            return vec![];
        }

        // Only during reasonable hours (9 AM - 9 PM)
        if state.current_hour < 9 || state.current_hour > 21 {
            return vec![];
        }

        let question = match self.get_question(state.bond_level) {
            Some(q) => q,
            None => return vec![],
        };

        let urgency = match state.bond_level {
            BondLevel::Acquaintance => 0.35,
            BondLevel::Friend => 0.4,
            BondLevel::Confidant => 0.45,
            BondLevel::PartnerInCrime => 0.5,
            _ => 0.3,
        };

        // Record that we asked
        if let Ok(mut last) = self.last_asked_ts.lock() {
            *last = state.current_ts;
        }
        if let Ok(mut asked) = self.asked_indices.lock() {
            let pool = match state.bond_level {
                BondLevel::Stranger => STRANGER_QUESTIONS,
                BondLevel::Acquaintance => ACQUAINTANCE_QUESTIONS,
                BondLevel::Friend => FRIEND_QUESTIONS,
                _ => CONFIDANT_QUESTIONS,
            };
            if let Some(idx) = pool.iter().position(|q| *q == question) {
                asked.push((state.bond_level, idx));
            }
        }

        vec![
            UrgeSpec::new(
                self.name(),
                &format!(
                    "EXECUTE Ask this question naturally in conversation, \
                     adapting the phrasing to feel spontaneous (not scripted): \
                     \"{}\". You can rephrase it, add a brief lead-in, or connect \
                     it to something you know about them. Keep it casual — 1-2 sentences max.",
                    question
                ),
                urgency,
            )
            .with_cooldown("question_asking:periodic"),
        ]
    }
}
