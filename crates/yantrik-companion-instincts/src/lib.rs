//! Instinct system — drives proactive companion behavior.
//!
//! Each instinct evaluates the companion's state and produces urges
//! (things the companion should bring up with the user).

mod activity_reflector;
mod aftermath;
mod automation;
mod bond_milestone;
mod check_in;
mod cognitive_load;
mod conflict_alerting;
mod conversational_callback;
mod emotional_awareness;
mod evening_reflection;
mod follow_up;
mod humor;
mod memory_weaver;
mod morning_brief;
mod pattern_surfacing;
mod predictive_workflow;
mod question_asking;
mod reminder;
mod routine;
mod scheduler;
mod self_awareness;
mod serendipity;
mod silence_reveal;
mod smart_updates;
mod weather_watch;
mod email_watch;
mod news_watch;
mod curiosity;
mod trend_watch;
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
mod legacy_builder;
mod identity_thread;
mod myth_buster;
mod cooking_companion;
mod second_brain;
mod health_pulse;
mod money_mind;
mod relationship_radar;
mod goal_keeper;
mod decision_lab;
mod skill_forge;
mod time_capture;
mod mentor_match;
mod debrief_partner;
mod philosophy_companion;
mod devils_advocate;
mod energy_map;
mod future_self;
mod dream_keeper;
mod cultural_radar;
mod pattern_breaker;
mod opportunity_scout;
mod open_loops_guardian;

use yantrik_companion_core::config::InstinctSettings;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

// Re-export Instinct trait from core
pub use yantrik_companion_core::instincts::Instinct;

pub use activity_reflector::ActivityReflectorInstinct;
pub use aftermath::AftermathInstinct;
pub use automation::AutomationInstinct;
pub use bond_milestone::BondMilestoneInstinct;
pub use conversational_callback::ConversationalCallbackInstinct;
pub use check_in::CheckInInstinct;
pub use cognitive_load::CognitiveLoadInstinct;
pub use conflict_alerting::ConflictAlertingInstinct;
pub use emotional_awareness::EmotionalAwarenessInstinct;
pub use evening_reflection::EveningReflectionInstinct;
pub use follow_up::FollowUpInstinct;
pub use humor::HumorInstinct;
pub use memory_weaver::MemoryWeaverInstinct;
pub use morning_brief::MorningBriefInstinct;
pub use pattern_surfacing::PatternSurfacingInstinct;
pub use predictive_workflow::PredictiveWorkflowInstinct;
pub use question_asking::QuestionAskingInstinct;
pub use reminder::ReminderInstinct;
pub use routine::RoutineInstinct;
pub use scheduler::SchedulerInstinct;
pub use self_awareness::SelfAwarenessInstinct;
pub use serendipity::SerendipityInstinct;
pub use silence_reveal::SilenceRevealInstinct;
pub use smart_updates::SmartUpdatesInstinct;
pub use weather_watch::WeatherWatchInstinct;
pub use email_watch::EmailWatchInstinct;
pub use news_watch::NewsWatchInstinct;
pub use curiosity::CuriosityInstinct;
pub use trend_watch::TrendWatchInstinct;
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
pub use legacy_builder::LegacyBuilderInstinct;
pub use identity_thread::IdentityThreadInstinct;
pub use myth_buster::MythBusterInstinct;
pub use cooking_companion::CookingCompanionInstinct;
pub use second_brain::SecondBrainInstinct;
pub use health_pulse::HealthPulseInstinct;
pub use money_mind::MoneyMindInstinct;
pub use relationship_radar::RelationshipRadarInstinct;
pub use goal_keeper::GoalKeeperInstinct;
pub use decision_lab::DecisionLabInstinct;
pub use skill_forge::SkillForgeInstinct;
pub use time_capture::TimeCaptureInstinct;
pub use mentor_match::MentorMatchInstinct;
pub use debrief_partner::DebriefPartnerInstinct;
pub use philosophy_companion::PhilosophyCompanionInstinct;
pub use devils_advocate::DevilsAdvocateInstinct;
pub use energy_map::EnergyMapInstinct;
pub use future_self::FutureSelfInstinct;
pub use dream_keeper::DreamKeeperInstinct;
pub use cultural_radar::CulturalRadarInstinct;
pub use pattern_breaker::PatternBreakerInstinct;
pub use opportunity_scout::OpportunityScoutInstinct;
pub use open_loops_guardian::OpenLoopsGuardianInstinct;

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
    instincts.push(Box::new(HumorInstinct::new()));

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

    if settings.email_watch_enabled {
        instincts.push(Box::new(EmailWatchInstinct::new(settings.email_poll_minutes)));
    }

    // Browser-based proactive intelligence
    if settings.news_watch_enabled {
        instincts.push(Box::new(NewsWatchInstinct::new(settings.news_watch_interval_minutes)));
    }
    if settings.trend_watch_enabled {
        instincts.push(Box::new(TrendWatchInstinct::new(settings.trend_watch_interval_minutes)));
    }
    if settings.curiosity_enabled {
        instincts.push(Box::new(CuriosityInstinct::new(
            settings.curiosity_idle_minutes,
            settings.curiosity_interval_hours,
        )));
    }

    // Human-first intelligence instincts (interest-aware, location-aware)
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
    if settings.golden_find_enabled {
        instincts.push(Box::new(GoldenFindInstinct::new(
            settings.golden_find_interval_hours,
        )));
    }
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
    if settings.legacy_builder_enabled {
        instincts.push(Box::new(LegacyBuilderInstinct::new(
            settings.legacy_builder_interval_hours,
        )));
    }
    if settings.identity_thread_enabled {
        instincts.push(Box::new(IdentityThreadInstinct::new(
            settings.identity_thread_interval_hours,
        )));
    }
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
    if settings.skill_forge_enabled {
        instincts.push(Box::new(SkillForgeInstinct::new(
            settings.skill_forge_interval_hours,
        )));
    }

    // Alive Mind instincts (batch 2)
    if settings.time_capture_enabled {
        instincts.push(Box::new(TimeCaptureInstinct::new(
            settings.time_capture_interval_hours,
        )));
    }
    if settings.mentor_match_enabled {
        instincts.push(Box::new(MentorMatchInstinct::new(
            settings.mentor_match_interval_hours,
        )));
    }
    if settings.debrief_partner_enabled {
        instincts.push(Box::new(DebriefPartnerInstinct::new(
            settings.debrief_partner_interval_hours,
        )));
    }
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
    if settings.energy_map_enabled {
        instincts.push(Box::new(EnergyMapInstinct::new(
            settings.energy_map_interval_hours,
        )));
    }
    if settings.future_self_enabled {
        instincts.push(Box::new(FutureSelfInstinct::new(
            settings.future_self_interval_hours,
        )));
    }
    if settings.dream_keeper_enabled {
        instincts.push(Box::new(DreamKeeperInstinct::new(
            settings.dream_keeper_interval_hours,
        )));
    }
    if settings.cultural_radar_enabled {
        instincts.push(Box::new(CulturalRadarInstinct::new(
            settings.cultural_radar_interval_hours,
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
