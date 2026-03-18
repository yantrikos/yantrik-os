//! Personality evolution data types — communication style, opinions, shared references.

use serde::{Deserialize, Serialize};

/// Communication style parameters that evolve over time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationStyle {
    /// 1.0 = formal, 0.0 = casual. Decays toward bond-level target.
    pub formality: f64,
    /// Target % of responses with humor.
    pub humor_ratio: f64,
    /// How opinionated to be (0.0 = neutral, 1.0 = very opinionated).
    pub opinion_strength: f64,
    /// How often to ask questions back.
    pub question_ratio: f64,
}

impl Default for CommunicationStyle {
    fn default() -> Self {
        Self {
            formality: 0.8,
            humor_ratio: 0.0,
            opinion_strength: 0.0,
            question_ratio: 0.3,
        }
    }
}

/// An opinion the companion has formed about a topic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opinion {
    pub topic: String,
    pub stance: String,
    pub confidence: f64,
    pub evidence_count: i64,
}

/// A shared reference (inside joke, callback) between companion and user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedReference {
    pub ref_id: String,
    pub reference_text: String,
    pub origin_context: String,
    pub times_used: i64,
}
