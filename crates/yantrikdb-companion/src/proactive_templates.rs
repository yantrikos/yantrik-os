//! Proactive template engine — pre-written templates with data slots and bond variants.
//!
//! Templates are instinct-specific message patterns with `{slot}` placeholders
//! filled at runtime from data (weather, system stats, patterns, etc.).
//! Each template can have bond-level suffixes that add personality.
//!
//! No LLM required — the 3B model is too slow for real-time composition during
//! the 60s think cycle. Templates keep proactive messages feeling natural.

use std::collections::HashMap;

use crate::bond::BondLevel;

/// A single proactive message template.
#[derive(Debug, Clone)]
pub struct ProactiveTemplate {
    /// Which instinct this template serves.
    pub instinct: &'static str,
    /// Minimum bond level to use this template.
    pub min_bond: BondLevel,
    /// Template text with `{slot}` placeholders.
    /// e.g. "Good morning, {user}. {weather_summary}"
    pub text: &'static str,
    /// Bond-level suffix variants. Appended after slot substitution.
    /// Higher bond levels get more personality.
    pub bond_suffixes: &'static [(BondLevel, &'static str)],
}

impl ProactiveTemplate {
    /// Fill template slots from a data map and append bond-appropriate suffix.
    pub fn render(&self, data: &HashMap<String, String>, bond: BondLevel) -> String {
        let mut result = self.text.to_string();

        // Replace all {slot} placeholders
        for (key, value) in data {
            let placeholder = format!("{{{}}}", key);
            result = result.replace(&placeholder, value);
        }

        // Strip any unfilled placeholders (graceful degradation)
        result = strip_unfilled_placeholders(&result);

        // Append bond-level suffix (pick the highest that doesn't exceed current bond)
        let suffix = best_suffix(self.bond_suffixes, bond);
        if !suffix.is_empty() {
            // Replace slots in suffix too
            let mut s = suffix.to_string();
            for (key, value) in data {
                let placeholder = format!("{{{}}}", key);
                s = s.replace(&placeholder, value);
            }
            s = strip_unfilled_placeholders(&s);
            if !s.is_empty() {
                result.push(' ');
                result.push_str(&s);
            }
        }

        result.trim().to_string()
    }
}

/// Pick the best suffix: highest bond level that is <= the current level.
fn best_suffix<'a>(suffixes: &'a [(BondLevel, &'a str)], bond: BondLevel) -> &'a str {
    let mut best: Option<(BondLevel, &str)> = None;
    for &(level, text) in suffixes {
        if level <= bond {
            match best {
                Some((prev, _)) if level > prev => best = Some((level, text)),
                None => best = Some((level, text)),
                _ => {}
            }
        }
    }
    best.map(|(_, t)| t).unwrap_or("")
}

/// Remove unfilled `{placeholder}` tokens from text.
fn strip_unfilled_placeholders(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' {
            // Scan ahead for closing brace
            let mut placeholder = String::new();
            let mut found_close = false;
            for inner in chars.by_ref() {
                if inner == '}' {
                    found_close = true;
                    break;
                }
                placeholder.push(inner);
            }
            if !found_close {
                // Wasn't a placeholder, keep the original text
                result.push('{');
                result.push_str(&placeholder);
            }
            // If found_close, we drop the entire {placeholder}
        } else {
            result.push(ch);
        }
    }
    // Clean up double spaces from removed placeholders
    while result.contains("  ") {
        result = result.replace("  ", " ");
    }
    result
}

// ─── Template Engine ────────────────────────────────────────────────────────

/// The template engine selects and renders templates for proactive delivery.
pub struct TemplateEngine {
    /// Tracks last-used variant index per instinct to rotate variety.
    variant_index: HashMap<String, usize>,
}

impl TemplateEngine {
    pub fn new() -> Self {
        Self {
            variant_index: HashMap::new(),
        }
    }

    /// Select and render the best template for an instinct.
    ///
    /// Returns `None` if no template matches the instinct + bond level.
    pub fn render(
        &mut self,
        instinct: &str,
        data: &HashMap<String, String>,
        bond: BondLevel,
    ) -> Option<String> {
        let templates = get_templates(instinct);
        if templates.is_empty() {
            return None;
        }

        // Filter to templates the current bond level qualifies for
        let eligible: Vec<&ProactiveTemplate> =
            templates.iter().filter(|t| bond >= t.min_bond).collect();
        if eligible.is_empty() {
            return None;
        }

        // Rotate through variants
        let idx = self.variant_index.entry(instinct.to_string()).or_insert(0);
        let template = eligible[*idx % eligible.len()];
        *idx = idx.wrapping_add(1);

        let rendered = template.render(data, bond);
        if rendered.trim().is_empty() {
            return None;
        }
        Some(rendered)
    }
}

// ─── Template Definitions ───────────────────────────────────────────────────

/// Get templates for a given instinct name (lowercase).
pub fn get_templates(instinct: &str) -> &'static [ProactiveTemplate] {
    match instinct {
        "morning_brief" => MORNING_BRIEF_TEMPLATES,
        "weather_watch" => WEATHER_WATCH_TEMPLATES,
        "check_in" => CHECK_IN_TEMPLATES,
        "resource_guardian" => RESOURCE_GUARDIAN_TEMPLATES,
        "focus_flow" => FOCUS_FLOW_TEMPLATES,
        "error_companion" => ERROR_COMPANION_TEMPLATES,
        "reminder" | "scheduler" => SCHEDULER_TEMPLATES,
        "pattern_surfacing" | "activity_reflector" => PATTERN_TEMPLATES,
        "bond_milestone" | "bondmilestone" => BOND_MILESTONE_TEMPLATES,
        _ => &[],
    }
}

// ── Morning Brief ───────────────────────────────────────────────────────────

static MORNING_BRIEF_TEMPLATES: &[ProactiveTemplate] = &[
    // Day 1–3: System + weather only
    ProactiveTemplate {
        instinct: "morning_brief",
        min_bond: BondLevel::Stranger,
        text: "Good morning, {user}. {weather_line} System is running smoothly{system_note}.",
        bond_suffixes: &[
            (BondLevel::Friend, "Have a good one."),
            (BondLevel::Confidant, "Let me know if you need anything."),
            (BondLevel::PartnerInCrime, "Let's get it."),
        ],
    },
    // Week 1+: adds schedule
    ProactiveTemplate {
        instinct: "morning_brief",
        min_bond: BondLevel::Acquaintance,
        text: "Good morning, {user}. {weather_line} {schedule_line} {system_note}",
        bond_suffixes: &[
            (BondLevel::Friend, "Ready when you are."),
            (BondLevel::PartnerInCrime, "Let's make today count."),
        ],
    },
    // Week 2+: adds patterns
    ProactiveTemplate {
        instinct: "morning_brief",
        min_bond: BondLevel::Friend,
        text: "Morning, {user}. {weather_line} {schedule_line} {pattern_note} {system_note}",
        bond_suffixes: &[
            (BondLevel::Confidant, "I've got your back today."),
            (BondLevel::PartnerInCrime, "Same team, let's go."),
        ],
    },
];

// ── Weather Watch ───────────────────────────────────────────────────────────

static WEATHER_WATCH_TEMPLATES: &[ProactiveTemplate] = &[
    // Action-first: notable weather
    ProactiveTemplate {
        instinct: "weather_watch",
        min_bond: BondLevel::Stranger,
        text: "Heads up \u{2014} {weather_alert}.",
        bond_suffixes: &[
            (BondLevel::Friend, "Plan accordingly."),
            (BondLevel::PartnerInCrime, "Don't say I didn't warn you."),
        ],
    },
    // Temperature swing
    ProactiveTemplate {
        instinct: "weather_watch",
        min_bond: BondLevel::Acquaintance,
        text: "Weather shift: {weather_detail}.",
        bond_suffixes: &[
            (BondLevel::Friend, "Might want to dress for it."),
            (BondLevel::Confidant, "Knowing you, you'll forget a jacket."),
        ],
    },
];

// ── Check-in ────────────────────────────────────────────────────────────────

static CHECK_IN_TEMPLATES: &[ProactiveTemplate] = &[
    ProactiveTemplate {
        instinct: "check_in",
        min_bond: BondLevel::Stranger,
        text: "Hey {user}. It's been a while \u{2014} I'm here if you need anything.",
        bond_suffixes: &[],
    },
    ProactiveTemplate {
        instinct: "check_in",
        min_bond: BondLevel::Friend,
        text: "Hey {user}, haven't heard from you in a bit. Everything good?",
        bond_suffixes: &[
            (BondLevel::Confidant, "Not being needy, just checking."),
            (BondLevel::PartnerInCrime, "Don't ghost me."),
        ],
    },
    ProactiveTemplate {
        instinct: "check_in",
        min_bond: BondLevel::Confidant,
        text: "{user}, you've been quiet. Just making sure you're alright.",
        bond_suffixes: &[
            (BondLevel::PartnerInCrime, "You know I'll bug you until you respond."),
        ],
    },
];

// ── Resource Guardian ───────────────────────────────────────────────────────

static RESOURCE_GUARDIAN_TEMPLATES: &[ProactiveTemplate] = &[
    // Action-first: "I did something" pattern
    ProactiveTemplate {
        instinct: "resource_guardian",
        min_bond: BondLevel::Stranger,
        text: "Noticed {resource_type} is high ({resource_value}). {top_process}",
        bond_suffixes: &[
            (BondLevel::Friend, "Want me to look into it?"),
            (BondLevel::PartnerInCrime, "Shall I kill it?"),
        ],
    },
    ProactiveTemplate {
        instinct: "resource_guardian",
        min_bond: BondLevel::Acquaintance,
        text: "{resource_type} usage spiked to {resource_value}. {top_process}",
        bond_suffixes: &[
            (BondLevel::Friend, "I can help clean things up."),
            (BondLevel::Confidant, "This happens a lot, by the way."),
        ],
    },
    // Battery specific
    ProactiveTemplate {
        instinct: "resource_guardian",
        min_bond: BondLevel::Stranger,
        text: "Battery at {battery_pct}%{charge_status}.",
        bond_suffixes: &[
            (BondLevel::Friend, "Might want to plug in soon."),
            (BondLevel::PartnerInCrime, "We're living dangerously."),
        ],
    },
];

// ── Focus Flow ──────────────────────────────────────────────────────────────

static FOCUS_FLOW_TEMPLATES: &[ProactiveTemplate] = &[
    // Deep work detected
    ProactiveTemplate {
        instinct: "focus_flow",
        min_bond: BondLevel::Acquaintance,
        text: "You've been focused for {focus_duration}. Nice deep work session.",
        bond_suffixes: &[
            (BondLevel::Friend, "I'll keep things quiet."),
            (BondLevel::Confidant, "I silenced notifications for you."),
            (BondLevel::PartnerInCrime, "Zone mode activated. I'll guard the door."),
        ],
    },
    // Break reminder
    ProactiveTemplate {
        instinct: "focus_flow",
        min_bond: BondLevel::Acquaintance,
        text: "You've been going for {focus_duration} straight. A short break might help.",
        bond_suffixes: &[
            (BondLevel::Friend, "Even machines need cooldown."),
            (BondLevel::Confidant, "Your eyes will thank you."),
            (BondLevel::PartnerInCrime, "Get up. Stretch. That's an order."),
        ],
    },
];

// ── Error Companion ─────────────────────────────────────────────────────────

static ERROR_COMPANION_TEMPLATES: &[ProactiveTemplate] = &[
    // Action-first: I noticed + I recall
    ProactiveTemplate {
        instinct: "error_companion",
        min_bond: BondLevel::Stranger,
        text: "Caught an error in {error_source}: {error_summary}.",
        bond_suffixes: &[
            (BondLevel::Friend, "I've seen this pattern before \u{2014} want me to pull up what worked last time?"),
            (BondLevel::Confidant, "Last time this happened, {past_fix}. Want to try that again?"),
        ],
    },
    ProactiveTemplate {
        instinct: "error_companion",
        min_bond: BondLevel::Acquaintance,
        text: "{error_source} just threw: {error_summary}. {error_context}",
        bond_suffixes: &[
            (BondLevel::Friend, "Need a hand?"),
            (BondLevel::PartnerInCrime, "Here we go again."),
        ],
    },
];

// ── Scheduler / Reminder ────────────────────────────────────────────────────

static SCHEDULER_TEMPLATES: &[ProactiveTemplate] = &[
    ProactiveTemplate {
        instinct: "scheduler",
        min_bond: BondLevel::Stranger,
        text: "Reminder: {task_label}.",
        bond_suffixes: &[
            (BondLevel::Friend, "Just a nudge."),
        ],
    },
    ProactiveTemplate {
        instinct: "scheduler",
        min_bond: BondLevel::Friend,
        text: "Hey {user} \u{2014} {task_label}.",
        bond_suffixes: &[
            (BondLevel::Confidant, "You asked me to remind you."),
            (BondLevel::PartnerInCrime, "Don't pretend you forgot."),
        ],
    },
];

// ── Pattern / Activity Reflector ────────────────────────────────────────────

static PATTERN_TEMPLATES: &[ProactiveTemplate] = &[
    ProactiveTemplate {
        instinct: "pattern_surfacing",
        min_bond: BondLevel::Friend,
        text: "I've been noticing something: {pattern_description}.",
        bond_suffixes: &[
            (BondLevel::Confidant, "Just an observation, no judgement."),
            (BondLevel::PartnerInCrime, "Take it or leave it."),
        ],
    },
    ProactiveTemplate {
        instinct: "activity_reflector",
        min_bond: BondLevel::Friend,
        text: "Over the past few days: {activity_summary}.",
        bond_suffixes: &[
            (BondLevel::Confidant, "Thought you'd want to know."),
        ],
    },
];

// ── Bond Milestone ──────────────────────────────────────────────────────────

static BOND_MILESTONE_TEMPLATES: &[ProactiveTemplate] = &[
    ProactiveTemplate {
        instinct: "bond_milestone",
        min_bond: BondLevel::Acquaintance,
        text: "{milestone_message}",
        bond_suffixes: &[],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_render() {
        let tmpl = &MORNING_BRIEF_TEMPLATES[0];
        let mut data = HashMap::new();
        data.insert("user".into(), "Sync".into());
        data.insert("weather_line".into(), "Clear skies, 72\u{00b0}F.".into());
        data.insert("system_note".into(), "".into());

        let result = tmpl.render(&data, BondLevel::Stranger);
        assert!(result.contains("Good morning, Sync"));
        assert!(result.contains("Clear skies"));
        // No suffix at Stranger level for this template
        assert!(!result.contains("Have a good one"));
    }

    #[test]
    fn test_bond_suffix() {
        let tmpl = &MORNING_BRIEF_TEMPLATES[0];
        let mut data = HashMap::new();
        data.insert("user".into(), "Sync".into());
        data.insert("weather_line".into(), "Rain expected.".into());
        data.insert("system_note".into(), "".into());

        let result = tmpl.render(&data, BondLevel::Friend);
        assert!(result.contains("Have a good one"));
    }

    #[test]
    fn test_unfilled_placeholders_stripped() {
        let tmpl = &MORNING_BRIEF_TEMPLATES[2];
        let mut data = HashMap::new();
        data.insert("user".into(), "Sync".into());
        data.insert("weather_line".into(), "Sunny.".into());
        // Missing: schedule_line, pattern_note, system_note

        let result = tmpl.render(&data, BondLevel::Friend);
        assert!(!result.contains("{schedule_line}"));
        assert!(!result.contains("{pattern_note}"));
        assert!(result.contains("Sunny."));
    }

    #[test]
    fn test_engine_rotation() {
        let mut engine = TemplateEngine::new();
        let mut data = HashMap::new();
        data.insert("user".into(), "Sync".into());
        data.insert("weather_line".into(), "Clear.".into());
        data.insert("system_note".into(), "".into());

        let r1 = engine.render("morning_brief", &data, BondLevel::Stranger);
        let r2 = engine.render("morning_brief", &data, BondLevel::Stranger);
        // At Stranger level, only one template qualifies (min_bond: Stranger)
        // so r1 and r2 should be the same
        assert!(r1.is_some());
        assert!(r2.is_some());
    }

    #[test]
    fn test_no_templates_for_unknown() {
        let mut engine = TemplateEngine::new();
        let data = HashMap::new();
        let result = engine.render("nonexistent", &data, BondLevel::Friend);
        assert!(result.is_none());
    }
}
