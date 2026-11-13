//! How often each instinct may fire, in the units its settings use (minutes or hours, as the
//! setting's name says). Defaults for `InstinctSettings`; kept apart so `config.rs` stays the
//! shape of the settings and not a column of one-line functions.

pub(crate) fn default_news_interval() -> f64 { 60.0 }
pub(crate) fn default_trend_interval() -> f64 { 45.0 }
pub(crate) fn default_curiosity_idle() -> f64 { 15.0 }
pub(crate) fn default_curiosity_interval() -> f64 { 4.0 }
pub(crate) fn default_interest_interval() -> f64 { 3.0 }
pub(crate) fn default_deal_interval() -> f64 { 6.0 }
pub(crate) fn default_activity_interval() -> f64 { 4.0 }
pub(crate) fn default_connection_weaver_interval() -> f64 { 5.0 }
pub(crate) fn default_context_bridge_interval() -> f64 { 8.0 }
pub(crate) fn default_deep_dive_interval() -> f64 { 6.0 }
pub(crate) fn default_wonder_sense_interval() -> f64 { 4.0 }
pub(crate) fn default_golden_find_interval() -> f64 { 6.0 }
pub(crate) fn default_growth_mirror_interval() -> f64 { 18.0 }
pub(crate) fn default_local_pulse_interval() -> f64 { 6.0 }
pub(crate) fn default_tradition_keeper_interval() -> f64 { 22.0 }
pub(crate) fn default_night_owl_interval() -> f64 { 2.5 }
pub(crate) fn default_legacy_builder_interval() -> f64 { 168.0 } // ~weekly
pub(crate) fn default_identity_thread_interval() -> f64 { 168.0 } // ~weekly
pub(crate) fn default_myth_buster_interval() -> f64 { 8.0 }
pub(crate) fn default_cooking_companion_interval() -> f64 { 6.0 }
pub(crate) fn default_second_brain_interval() -> f64 { 12.0 }
pub(crate) fn default_health_pulse_interval() -> f64 { 4.0 }
pub(crate) fn default_money_mind_interval() -> f64 { 12.0 }
pub(crate) fn default_relationship_radar_interval() -> f64 { 6.0 }
pub(crate) fn default_goal_keeper_interval() -> f64 { 6.0 }
pub(crate) fn default_decision_lab_interval() -> f64 { 8.0 }
pub(crate) fn default_skill_forge_interval() -> f64 { 12.0 }
pub(crate) fn default_time_capture_interval() -> f64 { 12.0 }
pub(crate) fn default_mentor_match_interval() -> f64 { 48.0 }
pub(crate) fn default_debrief_partner_interval() -> f64 { 6.0 }
pub(crate) fn default_philosophy_companion_interval() -> f64 { 48.0 }
pub(crate) fn default_devils_advocate_interval() -> f64 { 24.0 }
pub(crate) fn default_energy_map_interval() -> f64 { 48.0 }
pub(crate) fn default_future_self_interval() -> f64 { 72.0 }
pub(crate) fn default_dream_keeper_interval() -> f64 { 72.0 }
pub(crate) fn default_cultural_radar_interval() -> f64 { 24.0 }
pub(crate) fn default_pattern_breaker_interval() -> f64 { 72.0 }
pub(crate) fn default_opportunity_scout_interval() -> f64 { 48.0 }
