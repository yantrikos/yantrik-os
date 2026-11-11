//! Instinct trait — the interface all companion instincts implement.

use crate::types::{CompanionState, UrgeSpec};

/// Trait for companion instincts.
pub trait Instinct: Send + Sync {
    fn name(&self) -> &str;

    /// Periodic evaluation during background cognition tick.
    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec>;

    /// Whether this instinct's urges send the person's interests or place to the web (an urge that
    /// tells the model to search, in any model tier's wording). Declared here, once per instinct,
    /// and held back while background research is off (`background_research::gate`): the gate never
    /// reads an urge's text.
    fn searches_web(&self) -> bool {
        false
    }

    /// Lightweight check on every user interaction.
    fn on_interaction(&self, _state: &CompanionState, _user_text: &str) -> Vec<UrgeSpec> {
        vec![]
    }
}
