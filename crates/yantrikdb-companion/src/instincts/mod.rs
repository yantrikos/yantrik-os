//! Instinct system — drives proactive companion behavior.
//!
//! Each instinct evaluates the companion's state and produces urges
//! (things the companion should bring up with the user).

mod bond_milestone;
mod check_in;
mod conflict_alerting;
mod emotional_awareness;
mod follow_up;
mod humor;
mod pattern_surfacing;
mod reminder;
mod self_awareness;

use crate::config::InstinctSettings;
use crate::types::{CompanionState, UrgeSpec};

pub use bond_milestone::BondMilestoneInstinct;
pub use check_in::CheckInInstinct;
pub use conflict_alerting::ConflictAlertingInstinct;
pub use emotional_awareness::EmotionalAwarenessInstinct;
pub use follow_up::FollowUpInstinct;
pub use humor::HumorInstinct;
pub use pattern_surfacing::PatternSurfacingInstinct;
pub use reminder::ReminderInstinct;
pub use self_awareness::SelfAwarenessInstinct;

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

/// Load instincts based on configuration.
pub fn load_instincts(settings: &InstinctSettings) -> Vec<Box<dyn Instinct>> {
    let mut instincts: Vec<Box<dyn Instinct>> = Vec::new();

    if settings.check_in_enabled {
        instincts.push(Box::new(CheckInInstinct::new(settings.check_in_hours)));
    }
    if settings.emotional_awareness_enabled {
        instincts.push(Box::new(EmotionalAwarenessInstinct));
    }
    if settings.follow_up_enabled {
        instincts.push(Box::new(FollowUpInstinct));
    }
    if settings.reminder_enabled {
        instincts.push(Box::new(ReminderInstinct));
    }
    if settings.pattern_surfacing_enabled {
        instincts.push(Box::new(PatternSurfacingInstinct));
    }
    if settings.conflict_alerting_enabled {
        instincts.push(Box::new(ConflictAlertingInstinct::new(
            settings.conflict_alert_threshold,
        )));
    }

    // Soul instincts — bond-awareness, self-awareness, humor
    instincts.push(Box::new(BondMilestoneInstinct));
    instincts.push(Box::new(SelfAwarenessInstinct));
    instincts.push(Box::new(HumorInstinct));

    instincts
}
