//! Which instincts stay when the Skill Store has skills with instincts of their own enabled.
//!
//! An instinct is either gated by a skill (it runs only while that skill is on) or part of the
//! companion itself (it always runs). This is the one place that says which are which.

use std::collections::HashSet;

use crate::instincts::Instinct;

/// Whether the instinct called `name` belongs to the companion and not to a skill. These are the
/// exact `name()` values of the instinct impls.
///
/// `commitment_prep` is here because it is not a skill's feature: it acts on what the person
/// promised in conversation, whichever skills they have on, and has its own switch and its own
/// consent (`commitment_store`). Gating it behind a skill would leave it off for anyone whose
/// Skill Store has any instinct enabled, which is nearly everyone.
pub fn is_core(name: &str) -> bool {
    matches!(
        name,
        "scheduler" | "automation" | "BondMilestone" | "SelfAwareness"
        | "morning_brief" | "activity_reflector" | "serendipity"
        | "Aftermath" | "SocraticSpark" | "EveningReflection"
        | "ConversationalCallback" | "SilenceReveal"
        | "check_in" | "emotional_awareness" | "follow_up"
        | "reminder" | "pattern_surfacing" | "conflict_alerting"
        | "MemoryWeaver"
        | "predictive_workflow" | "routine" | "cognitive_load" | "smart_updates"
        | "commitment_prep"
    )
}

/// Keep the core instincts and those a skill has enabled. An empty `enabled` keeps all: with no
/// skill naming an instinct there is nothing to filter by.
pub fn keep_enabled(instincts: &mut Vec<Box<dyn Instinct>>, enabled: &HashSet<String>) {
    if enabled.is_empty() {
        return;
    }
    instincts.retain(|inst| is_core(inst.name()) || enabled.contains(inst.name()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_companion_core::config::InstinctSettings;

    /// The set 520 had: the Email skill on, so only `EmailWatch` is named by a skill.
    #[test]
    fn commitment_prep_survives_the_skill_filter_in_the_instincts_the_shell_loads() {
        let mut loaded = crate::instincts::load_instincts(&InstinctSettings::default());
        assert!(loaded.iter().any(|i| i.name() == "commitment_prep"), "it is loaded at all");
        let before = loaded.len();
        keep_enabled(&mut loaded, &HashSet::from(["EmailWatch".to_string()]));
        assert!(loaded.len() < before, "the filter did filter");
        assert!(loaded.iter().any(|i| i.name() == "commitment_prep"), "commitment_prep was filtered out");
    }

    #[test]
    fn an_instinct_no_skill_names_and_that_is_not_core_is_dropped() {
        let mut loaded = crate::instincts::load_instincts(&InstinctSettings::default());
        keep_enabled(&mut loaded, &HashSet::from(["EmailWatch".to_string()]));
        assert!(loaded.iter().all(|i| is_core(i.name()) || i.name() == "EmailWatch"));
    }
}
