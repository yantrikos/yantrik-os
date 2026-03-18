//! Default instinct-to-category and instinct-to-time-sensitivity mappings.
//!
//! Used by `UrgeSpec::new()` to assign sensible defaults without requiring
//! each instinct to manually specify category and sensitivity.

use crate::types::{AutonomyTier, InstinctCategory, TimeSensitivity};

/// Centralized default category for an instinct name.
pub fn default_category(instinct_name: &str) -> InstinctCategory {
    match instinct_name {
        "scheduler" | "Reminder" | "Automation" | "morning_brief" | "GoalKeeper"
        | "PredictiveWorkflow" | "SmartUpdates" | "EmailWatch" | "OpportunityScout"
        | "open_loops_guardian" => {
            InstinctCategory::Anticipatory
        }
        "check_in" | "CheckIn" | "HealthPulse" | "EnergyMap" | "NightOwl" | "CognitiveLoad" => {
            InstinctCategory::Wellbeing
        }
        "Humor" | "ConnectionWeaver" | "ContextBridge" | "RelationshipRadar"
        | "ConversationalCallback" | "BondMilestone" | "QuestionAsking" => {
            InstinctCategory::Social
        }
        "SkillForge" | "MentorMatch" | "FutureSelf" | "GrowthMirror"
        | "DecisionLab" | "MoneyMind" | "DealWatch" => InstinctCategory::Growth,
        "WorldSense" | "TrendWatch" | "LocalPulse" | "GoldenFind" | "Curiosity"
        | "CulturalRadar" | "weather_watch" | "WeatherWatch" | "ActivityRecommender"
        | "InterestIntelligence" => InstinctCategory::Awareness,
        "EmotionalAwareness" | "Aftermath" | "EveningReflection" | "SilenceReveal"
        | "DreamKeeper" | "FollowUp" => InstinctCategory::Emotional,
        _ => InstinctCategory::Meta,
    }
}

/// Centralized default time sensitivity for an instinct name.
pub fn default_time_sensitivity(instinct_name: &str) -> TimeSensitivity {
    match instinct_name {
        // Tier 0: Immediate
        "scheduler" | "Reminder" | "Automation" => TimeSensitivity::Immediate,
        // Tier 1: Today
        "morning_brief" | "GoalKeeper" | "NightOwl" | "EmailWatch"
        | "EmotionalAwareness" | "Aftermath" | "EveningReflection"
        | "BondMilestone" | "CognitiveLoad" | "PredictiveWorkflow"
        | "weather_watch" | "WeatherWatch" | "ConflictAlerting" | "Cortex"
        | "open_loops_guardian" => TimeSensitivity::Today,
        // Tier 2: Soon
        "HealthPulse" | "check_in" | "CheckIn" | "GrowthMirror" | "DecisionLab"
        | "MoneyMind" | "DealWatch" | "WorldSense" | "RelationshipRadar"
        | "SilenceReveal" | "FollowUp" | "SmartUpdates" | "OpportunityScout"
        | "InterestIntelligence" | "Routine" => TimeSensitivity::Soon,
        // Tier 3: Ambient
        _ => TimeSensitivity::Ambient,
    }
}

/// Centralized default autonomy tier for an instinct name.
///
/// Most instincts produce suggestions (NotifySuggestion). Background maintenance
/// instincts run silently. Actions that affect the outside world require permission.
pub fn default_autonomy(instinct_name: &str) -> AutonomyTier {
    match instinct_name {
        // Silent: internal maintenance, no user impact
        "MemoryWeaver" | "PatternBreaker" | "Cortex" | "CK5" | "ConflictAlerting" => {
            AutonomyTier::SilentBackground
        }
        // Interrupt: security, critical system issues
        "SecurityAlert" | "SystemCritical" => AutonomyTier::InterruptNow,
        // Ask permission: actions that affect external systems
        "Automation" | "scheduler" => AutonomyTier::AskPermission,
        // Default: show as suggestion
        _ => AutonomyTier::NotifySuggestion,
    }
}
