//! Instinct system — drives proactive companion behavior.
//!
//! Each instinct evaluates the companion's state and produces urges
//! (things the companion should bring up with the user).

mod activity_reflector;
mod automation;
mod bond_milestone;
mod check_in;
mod cognitive_load;
mod conflict_alerting;
mod emotional_awareness;
mod follow_up;
mod humor;
mod memory_weaver;
mod morning_brief;
mod pattern_surfacing;
mod predictive_workflow;
mod reminder;
mod routine;
mod scheduler;
mod self_awareness;
mod serendipity;
mod smart_updates;
mod weather_watch;

use crate::config::InstinctSettings;
use crate::types::{CompanionState, UrgeSpec};

pub use activity_reflector::ActivityReflectorInstinct;
pub use automation::AutomationInstinct;
pub use bond_milestone::BondMilestoneInstinct;
pub use check_in::CheckInInstinct;
pub use cognitive_load::CognitiveLoadInstinct;
pub use conflict_alerting::ConflictAlertingInstinct;
pub use emotional_awareness::EmotionalAwarenessInstinct;
pub use follow_up::FollowUpInstinct;
pub use humor::HumorInstinct;
pub use memory_weaver::MemoryWeaverInstinct;
pub use morning_brief::MorningBriefInstinct;
pub use pattern_surfacing::PatternSurfacingInstinct;
pub use predictive_workflow::PredictiveWorkflowInstinct;
pub use reminder::ReminderInstinct;
pub use routine::RoutineInstinct;
pub use scheduler::SchedulerInstinct;
pub use self_awareness::SelfAwarenessInstinct;
pub use serendipity::SerendipityInstinct;
pub use smart_updates::SmartUpdatesInstinct;
pub use weather_watch::WeatherWatchInstinct;

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

    if settings.memory_weaver_enabled {
        instincts.push(Box::new(MemoryWeaverInstinct::new(
            settings.memory_weaver_idle_minutes,
            settings.memory_weaver_min_memories,
        )));
    }

    // Scheduler instinct — converts due scheduled tasks into urges
    instincts.push(Box::new(SchedulerInstinct));

    // Automation instinct — converts fired automations into executable urges
    instincts.push(Box::new(AutomationInstinct));

    // Soul instincts — bond-awareness, self-awareness, humor
    instincts.push(Box::new(BondMilestoneInstinct));
    instincts.push(Box::new(SelfAwarenessInstinct));
    instincts.push(Box::new(HumorInstinct));

    // V15: Proactive intelligence instincts
    instincts.push(Box::new(MorningBriefInstinct::new()));
    instincts.push(Box::new(WeatherWatchInstinct::new()));
    instincts.push(Box::new(ActivityReflectorInstinct));
    instincts.push(Box::new(SerendipityInstinct));

    // Phase 2: Proactive intelligence
    if settings.predictive_workflow_enabled {
        instincts.push(Box::new(PredictiveWorkflowInstinct::new()));
    }
    if settings.routine_enabled {
        instincts.push(Box::new(RoutineInstinct::new()));
    }
    if settings.cognitive_load_enabled {
        instincts.push(Box::new(CognitiveLoadInstinct::new()));
    }
    if settings.smart_updates_enabled {
        instincts.push(Box::new(SmartUpdatesInstinct::new()));
    }

    instincts
}
