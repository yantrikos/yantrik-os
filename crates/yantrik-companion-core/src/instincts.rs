//! Instinct trait — the interface all companion instincts implement.

use crate::types::{CompanionState, UrgeSpec};

/// Trait for companion instincts.
pub trait Instinct: Send + Sync {
    fn name(&self) -> &str;

    /// Periodic evaluation during background cognition tick.
    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec>;

    /// Lightweight check on every user interaction.
    fn on_interaction(&self, _state: &CompanionState, _user_text: &str) -> Vec<UrgeSpec> {
        vec![]
    }
}
