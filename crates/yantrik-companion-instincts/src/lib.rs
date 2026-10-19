//! Instinct system — drives proactive companion behavior.
//!
//! Each instinct evaluates the companion's state and produces urges
//! (things the companion should bring up with the user).

mod activity_reflector;
mod aftermath;
mod automation;
mod check_in;
mod cognitive_load;
mod conflict_alerting;
mod conversational_callback;
mod evening_reflection;
mod follow_up;
mod humor;
mod memory_weaver;
mod morning_brief;
mod pattern_surfacing;
mod question_asking;
mod reminder;
mod routine;
mod scheduler;
mod self_awareness;
mod silence_reveal;
mod smart_updates;
mod weather_watch;
mod email_watch;
mod news_watch;
mod interest_intelligence;
mod deal_watch;
mod activity_recommender;
mod connection_weaver;
mod context_bridge;
mod deep_dive;
mod golden_find;
mod growth_mirror;
mod night_owl;
mod wonder_sense;
mod local_pulse;
mod tradition_keeper;
mod myth_buster;
mod cooking_companion;
mod second_brain;
mod health_pulse;
mod money_mind;
mod relationship_radar;
mod goal_keeper;
mod decision_lab;
mod skill_forge;
mod philosophy_companion;
mod devils_advocate;
mod dream_keeper;
mod pattern_breaker;
mod opportunity_scout;
mod open_loops_guardian;
#[cfg(test)]
mod test_util;

use yantrik_companion_core::config::InstinctSettings;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

// Re-export Instinct trait from core
pub use yantrik_companion_core::instincts::Instinct;

pub use activity_reflector::ActivityReflectorInstinct;
pub use aftermath::AftermathInstinct;
pub use automation::AutomationInstinct;
pub use conversational_callback::ConversationalCallbackInstinct;
pub use check_in::CheckInInstinct;
pub use cognitive_load::CognitiveLoadInstinct;
pub use conflict_alerting::ConflictAlertingInstinct;
pub use evening_reflection::EveningReflectionInstinct;
pub use follow_up::FollowUpInstinct;
pub use humor::HumorInstinct;
pub use memory_weaver::MemoryWeaverInstinct;
pub use morning_brief::MorningBriefInstinct;
pub use pattern_surfacing::PatternSurfacingInstinct;
pub use question_asking::QuestionAskingInstinct;
pub use reminder::ReminderInstinct;
pub use routine::RoutineInstinct;
pub use scheduler::SchedulerInstinct;
pub use self_awareness::SelfAwarenessInstinct;
pub use silence_reveal::SilenceRevealInstinct;
pub use smart_updates::SmartUpdatesInstinct;
pub use weather_watch::WeatherWatchInstinct;
pub use email_watch::EmailWatchInstinct;
pub use news_watch::NewsWatchInstinct;
pub use interest_intelligence::InterestIntelligenceInstinct;
pub use deal_watch::DealWatchInstinct;
pub use activity_recommender::ActivityRecommenderInstinct;
pub use connection_weaver::ConnectionWeaverInstinct;
pub use context_bridge::ContextBridgeInstinct;
pub use deep_dive::DeepDiveInstinct;
pub use golden_find::GoldenFindInstinct;
pub use growth_mirror::GrowthMirrorInstinct;
pub use night_owl::NightOwlInstinct;
pub use wonder_sense::WonderSenseInstinct;
pub use local_pulse::LocalPulseInstinct;
pub use tradition_keeper::TraditionKeeperInstinct;
pub use myth_buster::MythBusterInstinct;
pub use cooking_companion::CookingCompanionInstinct;
pub use second_brain::SecondBrainInstinct;
pub use health_pulse::HealthPulseInstinct;
pub use money_mind::MoneyMindInstinct;
pub use relationship_radar::RelationshipRadarInstinct;
pub use goal_keeper::GoalKeeperInstinct;
pub use decision_lab::DecisionLabInstinct;
pub use skill_forge::SkillForgeInstinct;
pub use philosophy_companion::PhilosophyCompanionInstinct;
pub use devils_advocate::DevilsAdvocateInstinct;
pub use dream_keeper::DreamKeeperInstinct;
pub use pattern_breaker::PatternBreakerInstinct;
pub use opportunity_scout::OpportunityScoutInstinct;
pub use open_loops_guardian::OpenLoopsGuardianInstinct;

/// Load instincts based on configuration.
pub fn load_instincts(settings: &InstinctSettings) -> Vec<Box<dyn Instinct>> {
    let mut instincts: Vec<Box<dyn Instinct>> = Vec::new();

    if settings.check_in_enabled {
        instincts.push(Box::new(CheckInInstinct::new(settings.check_in_hours)));
    }
    // emotional_awareness is retired (#755): it announced "I noticed emotional tone shifted
    // negative" — a diagnosis of the person, which the companion never makes.
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

    // Soul instincts — self-awareness, humor. bond_milestone is retired (#755): a level change
    // is never announced as a celebration of the relationship.
    instincts.push(Box::new(SelfAwarenessInstinct));
    instincts.push(Box::new(HumorInstinct::new()));

    // V15: Proactive intelligence instincts
    instincts.push(Box::new(MorningBriefInstinct::new()));
    instincts.push(Box::new(WeatherWatchInstinct::new()));
    instincts.push(Box::new(ActivityReflectorInstinct));

    // Phase 2: Proactive intelligence
    // A merged instinct's old switch is not read: the target's own switch decides, so turning
    // the target off always turns it off (#755: "off" must mean off).
    // predictive_workflow is merged into routine (#755): one routine line per window, not one
    // "you usually…" per hour.
    if settings.routine_enabled {
        instincts.push(Box::new(RoutineInstinct::new()));
    }
    if settings.cognitive_load_enabled {
        instincts.push(Box::new(CognitiveLoadInstinct::new()));
    }
    if settings.smart_updates_enabled {
        instincts.push(Box::new(SmartUpdatesInstinct::new()));
    }

    if settings.email_watch_enabled {
        instincts.push(Box::new(EmailWatchInstinct::new(settings.email_poll_minutes)));
    }

    // Browser-based proactive intelligence
    // trend_watch is merged into news_watch (#755), whose cultural lens covers trends.
    if settings.news_watch_enabled {
        instincts.push(Box::new(NewsWatchInstinct::new(settings.news_watch_interval_minutes)));
    }

    // Human-first intelligence instincts (interest-aware, location-aware)
    // curiosity is merged into interest_intelligence (#755): the same interest research, but
    // never started by the person's absence.
    if settings.interest_intelligence_enabled {
        instincts.push(Box::new(InterestIntelligenceInstinct::new(
            settings.interest_intelligence_interval_hours,
        )));
    }
    if settings.deal_watch_enabled {
        instincts.push(Box::new(DealWatchInstinct::new(
            settings.deal_watch_interval_hours,
        )));
    }
    if settings.activity_recommender_enabled {
        instincts.push(Box::new(ActivityRecommenderInstinct::new(
            settings.activity_recommender_interval_hours,
        )));
    }
    if settings.connection_weaver_enabled {
        instincts.push(Box::new(ConnectionWeaverInstinct::new(
            settings.connection_weaver_interval_hours,
        )));
    }
    if settings.context_bridge_enabled {
        instincts.push(Box::new(ContextBridgeInstinct::new(
            settings.context_bridge_interval_hours,
        )));
    }
    if settings.deep_dive_enabled {
        instincts.push(Box::new(DeepDiveInstinct::new(
            settings.deep_dive_interval_hours,
        )));
    }
    if settings.wonder_sense_enabled {
        instincts.push(Box::new(WonderSenseInstinct::new(
            settings.wonder_sense_interval_hours,
        )));
    }
    // cultural_radar is merged into golden_find (#755): both found the one thing matching taste.
    if settings.golden_find_enabled {
        instincts.push(Box::new(GoldenFindInstinct::new(
            settings.golden_find_interval_hours,
        )));
    }
    // legacy_builder is merged into growth_mirror (#755): one evidence-based progress line.
    if settings.growth_mirror_enabled {
        instincts.push(Box::new(GrowthMirrorInstinct::new(
            settings.growth_mirror_interval_hours,
        )));
    }
    if settings.local_pulse_enabled {
        instincts.push(Box::new(LocalPulseInstinct::new(
            settings.local_pulse_interval_hours,
        )));
    }
    if settings.tradition_keeper_enabled {
        instincts.push(Box::new(TraditionKeeperInstinct::new(
            settings.tradition_keeper_interval_hours,
        )));
    }
    if settings.night_owl_enabled {
        instincts.push(Box::new(NightOwlInstinct::new(
            settings.night_owl_interval_hours,
        )));
    }
    // identity_thread is retired (#755): reading someone's identity back to them from their
    // private words is intimacy through surveillance.
    if settings.myth_buster_enabled {
        instincts.push(Box::new(MythBusterInstinct::new(
            settings.myth_buster_interval_hours,
        )));
    }
    if settings.cooking_companion_enabled {
        instincts.push(Box::new(CookingCompanionInstinct::new(
            settings.cooking_companion_interval_hours,
        )));
    }
    if settings.second_brain_enabled {
        instincts.push(Box::new(SecondBrainInstinct::new(
            settings.second_brain_interval_hours,
        )));
    }

    // Alive Mind instincts
    if settings.health_pulse_enabled {
        instincts.push(Box::new(HealthPulseInstinct::new(
            settings.health_pulse_interval_hours,
        )));
    }
    if settings.money_mind_enabled {
        instincts.push(Box::new(MoneyMindInstinct::new(
            settings.money_mind_interval_hours,
        )));
    }
    if settings.relationship_radar_enabled {
        instincts.push(Box::new(RelationshipRadarInstinct::new(
            settings.relationship_radar_interval_hours,
        )));
    }
    // future_self is merged into goal_keeper (#755): one place for stated intentions.
    if settings.goal_keeper_enabled {
        instincts.push(Box::new(GoalKeeperInstinct::new(
            settings.goal_keeper_interval_hours,
        )));
    }
    if settings.decision_lab_enabled {
        instincts.push(Box::new(DecisionLabInstinct::new(
            settings.decision_lab_interval_hours,
        )));
    }
    // mentor_match is merged into skill_forge (#755): both answered "what unblocks the thing
    // you're learning".
    if settings.skill_forge_enabled {
        instincts.push(Box::new(SkillForgeInstinct::new(
            settings.skill_forge_interval_hours,
        )));
    }

    // Alive Mind instincts (batch 2)
    // time_capture is retired and debrief_partner merged into aftermath (#755).
    if settings.philosophy_companion_enabled {
        instincts.push(Box::new(PhilosophyCompanionInstinct::new(
            settings.philosophy_companion_interval_hours,
        )));
    }

    // Alive Mind instincts (batch 3)
    if settings.devils_advocate_enabled {
        instincts.push(Box::new(DevilsAdvocateInstinct::new(
            settings.devils_advocate_interval_hours,
        )));
    }
    // energy_map is retired (#755): it commented on the person's hours and energy.
    if settings.dream_keeper_enabled {
        instincts.push(Box::new(DreamKeeperInstinct::new(
            settings.dream_keeper_interval_hours,
        )));
    }
    if settings.pattern_breaker_enabled {
        instincts.push(Box::new(PatternBreakerInstinct::new(
            settings.pattern_breaker_interval_hours,
        )));
    }
    if settings.opportunity_scout_enabled {
        instincts.push(Box::new(OpportunityScoutInstinct::new(
            settings.opportunity_scout_interval_hours,
        )));
    }

    // Open Loops Guardian (always on — monitors commitments and attention items)
    instincts.push(Box::new(OpenLoopsGuardianInstinct::new(
        settings.open_loops_threshold.unwrap_or(5),
    )));

    // Natural Communication instincts (always loaded — bond-gated internally)
    instincts.push(Box::new(AftermathInstinct));
    instincts.push(Box::new(QuestionAskingInstinct::new()));
    instincts.push(Box::new(EveningReflectionInstinct::new()));
    instincts.push(Box::new(ConversationalCallbackInstinct));
    instincts.push(Box::new(SilenceRevealInstinct::new()));

    instincts
}

#[cfg(test)]
mod load_tests {
    use super::*;

    const MERGE_TARGETS: &[&str] = &[
        "routine",
        "WorldSense",
        "InterestIntelligence",
        "GoldenFind",
        "GrowthMirror",
        "GoalKeeper",
        "SkillForge",
    ];

    fn loaded(settings: &InstinctSettings) -> Vec<String> {
        load_instincts(settings).iter().map(|i| i.name().to_string()).collect()
    }

    #[test]
    fn turning_off_a_merge_target_turns_it_off() {
        // The merged instincts' old switches default to on; they must not keep a target running
        // after the person turned it off.
        let mut s = InstinctSettings::default();
        s.routine_enabled = false;
        s.news_watch_enabled = false;
        s.interest_intelligence_enabled = false;
        s.golden_find_enabled = false;
        s.growth_mirror_enabled = false;
        s.goal_keeper_enabled = false;
        s.skill_forge_enabled = false;
        let names = loaded(&s);
        for gone in MERGE_TARGETS {
            assert!(!names.iter().any(|n| n == gone), "{gone} still loaded: {names:?}");
        }
    }

    #[test]
    fn defaults_load_the_merge_targets() {
        let names = loaded(&InstinctSettings::default());
        for kept in MERGE_TARGETS {
            assert!(names.iter().any(|n| n == kept), "{kept} missing: {names:?}");
        }
    }
}
