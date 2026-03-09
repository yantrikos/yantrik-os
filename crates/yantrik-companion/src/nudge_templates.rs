//! Nudge Templates — Tier 1 rule-based proactive message generation.
//!
//! Zero LLM cost. Pattern-matches against LifeEvents and PWG state to generate
//! contextual, human-friendly nudges. Each template defines:
//! - Trigger: which LifeEventKind(s) activate it
//! - Condition: additional checks (salience threshold, time window, etc.)
//! - Message: template with slots filled from event data
//! - Importance: how urgent this nudge is (affects delivery timing)
//!
//! Examples:
//! - Rain during commute → "It's going to rain around 8 AM. Carry an umbrella!"
//! - Oil price spike + fuel node → "Oil prices up due to {reason}. Might be a good time to fill up."
//! - Birthday in 3 days → "Hey, {person}'s birthday is in 3 days. Started thinking about a gift?"
//! - Concert by liked artist → "There's a {artist} concert at {venue} on {date}. Tickets from {price}."

use crate::graph_bridge::{LifeEvent, LifeEventKind};

// ── Nudge Output ────────────────────────────────────────────────────

/// A generated nudge ready for delivery to the user.
#[derive(Debug, Clone)]
pub struct Nudge {
    /// The nudge message text.
    pub message: String,
    /// Category for UI grouping.
    pub category: NudgeCategory,
    /// Importance (0.0–1.0). Higher = more urgent delivery.
    pub importance: f64,
    /// Why this nudge was generated (for transparency/explainability).
    pub reasoning: String,
    /// Source event that triggered this nudge.
    pub source_event_kind: String,
    /// Whether this nudge is actionable (user can do something about it).
    pub actionable: bool,
    /// Optional action suggestion.
    pub suggested_action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NudgeCategory {
    Weather,
    Calendar,
    Finance,
    Relationship,
    Discovery,
    Health,
    General,
}

impl NudgeCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Weather => "weather",
            Self::Calendar => "calendar",
            Self::Finance => "finance",
            Self::Relationship => "relationship",
            Self::Discovery => "discovery",
            Self::Health => "health",
            Self::General => "general",
        }
    }
}

// ── Template Registry ───────────────────────────────────────────────

/// Try all templates against a LifeEvent and return matching nudges.
/// Most events produce 0 or 1 nudge; some may produce multiple.
pub fn generate_nudges(event: &LifeEvent) -> Vec<Nudge> {
    let mut nudges = Vec::new();

    match &event.kind {
        LifeEventKind::PrecipitationAlert => {
            if let Some(n) = nudge_rain_alert(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::TemperatureExtreme => {
            if let Some(n) = nudge_temperature_extreme(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::SevereWeather => {
            if let Some(n) = nudge_severe_weather(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::PriceChange => {
            if let Some(n) = nudge_price_change(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::CalendarApproaching => {
            if let Some(n) = nudge_calendar_approaching(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::CalendarConflict => {
            if let Some(n) = nudge_calendar_conflict(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::DateApproaching => {
            if let Some(n) = nudge_personal_date(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::EventDiscovered => {
            if let Some(n) = nudge_event_discovered(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::NewsRelevant => {
            if let Some(n) = nudge_relevant_news(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::FreeBlockDetected => {
            if let Some(n) = nudge_free_block(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::CommunicationGap => {
            if let Some(n) = nudge_communication_gap(event) {
                nudges.push(n);
            }
        }
        LifeEventKind::BillDetected => {
            if let Some(n) = nudge_bill_detected(event) {
                nudges.push(n);
            }
        }
        _ => {} // No template for this event type yet
    }

    nudges
}

// ── Individual Templates ────────────────────────────────────────────

fn nudge_rain_alert(event: &LifeEvent) -> Option<Nudge> {
    let location = event.data["location"].as_str().unwrap_or("your area");
    let probability = event.data["max_probability"].as_f64().unwrap_or(0.0);
    let rain_hours: Vec<String> = event.data["rain_hours"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let time_context = if !rain_hours.is_empty() {
        format!("around {}", rain_hours.join(" and "))
    } else {
        "today".into()
    };

    let message = if probability > 80.0 {
        format!(
            "Rain is very likely {} at {}. Carry an umbrella!",
            time_context, location
        )
    } else {
        format!(
            "There's a {}% chance of rain {} at {}. Might want to grab an umbrella.",
            probability as u32, time_context, location
        )
    };

    Some(Nudge {
        message,
        category: NudgeCategory::Weather,
        importance: if probability > 80.0 { 0.7 } else { 0.5 },
        reasoning: format!("Rain expected during commute hours ({}% probability)", probability as u32),
        source_event_kind: event.kind.as_str().into(),
        actionable: true,
        suggested_action: Some("Carry an umbrella or raincoat".into()),
    })
}

fn nudge_temperature_extreme(event: &LifeEvent) -> Option<Nudge> {
    let temp_type = event.data["type"].as_str().unwrap_or("unknown");
    let temp = event.data["temperature"].as_f64().unwrap_or(0.0);
    let location = event.data["location"].as_str().unwrap_or("your area");

    let (message, action) = match temp_type {
        "heat" => (
            format!(
                "It's going to be very hot today at {} — {:.0}°C. Stay hydrated and try to stay indoors during peak hours.",
                location, temp
            ),
            "Drink plenty of water, avoid direct sun between 12-3 PM".to_string(),
        ),
        "cold" => (
            format!(
                "Bundle up! It's going to be cold at {} today — low of {:.0}°C. Watch for icy patches.",
                location, temp
            ),
            "Wear warm layers, be careful on icy roads".to_string(),
        ),
        _ => return None,
    };

    Some(Nudge {
        message,
        category: NudgeCategory::Weather,
        importance: 0.6,
        reasoning: format!("Temperature extreme: {:.0}°C ({})", temp, temp_type),
        source_event_kind: event.kind.as_str().into(),
        actionable: true,
        suggested_action: Some(action),
    })
}

fn nudge_severe_weather(event: &LifeEvent) -> Option<Nudge> {
    let desc = event.data["description"].as_str().unwrap_or("severe weather");
    let location = event.data["location"].as_str().unwrap_or("your area");

    Some(Nudge {
        message: format!(
            "⚠ Severe weather alert for {}: {}. Consider postponing outdoor plans.",
            location, desc
        ),
        category: NudgeCategory::Weather,
        importance: 0.9,
        reasoning: format!("Severe weather: {} at {}", desc, location),
        source_event_kind: event.kind.as_str().into(),
        actionable: true,
        suggested_action: Some("Stay indoors if possible, secure outdoor items".into()),
    })
}

fn nudge_price_change(event: &LifeEvent) -> Option<Nudge> {
    // Check if this is about fuel/oil
    let is_fuel = event.keywords.iter().any(|k| {
        let lower = k.to_lowercase();
        lower.contains("oil") || lower.contains("fuel") || lower.contains("gas") || lower.contains("petrol")
    });

    if is_fuel {
        let summary = &event.summary;
        return Some(Nudge {
            message: format!(
                "I noticed something that might affect you: {}. Might be a good time to top up the car before prices change.",
                summary
            ),
            category: NudgeCategory::Finance,
            importance: 0.5,
            reasoning: "Price change in fuel/energy sector detected, user has commute pattern".into(),
            source_event_kind: event.kind.as_str().into(),
            actionable: true,
            suggested_action: Some("Consider filling up the car soon".into()),
        });
    }

    // Generic price change
    Some(Nudge {
        message: format!("Price update that may interest you: {}", event.summary),
        category: NudgeCategory::Finance,
        importance: 0.3,
        reasoning: "Price change matched user interests".into(),
        source_event_kind: event.kind.as_str().into(),
        actionable: false,
        suggested_action: None,
    })
}

fn nudge_calendar_approaching(event: &LifeEvent) -> Option<Nudge> {
    let title = event.data["event_title"].as_str().unwrap_or("an event");
    let minutes = event.data["minutes_until"].as_u64().unwrap_or(0);
    let location = event.data["location"].as_str();

    let mut message = format!("Heads up — \"{}\" starts in {} minutes", title, minutes);
    if let Some(loc) = location {
        if !loc.is_empty() {
            message.push_str(&format!(" at {}", loc));
        }
    }
    message.push('.');

    // Add travel time hint for physical locations
    let action = location.and_then(|loc| {
        if !loc.is_empty() && !loc.to_lowercase().contains("zoom") && !loc.to_lowercase().contains("meet") {
            Some(format!("Head to {} — leave with buffer for travel", loc))
        } else {
            None
        }
    });

    Some(Nudge {
        message,
        category: NudgeCategory::Calendar,
        importance: if minutes <= 10 { 0.9 } else { 0.6 },
        reasoning: format!("Calendar event in {} minutes", minutes),
        source_event_kind: event.kind.as_str().into(),
        actionable: action.is_some(),
        suggested_action: action,
    })
}

fn nudge_calendar_conflict(event: &LifeEvent) -> Option<Nudge> {
    let event_a = event.data["event_a"]["title"].as_str().unwrap_or("Event A");
    let event_b = event.data["event_b"]["title"].as_str().unwrap_or("Event B");
    let overlap = event.data["overlap_minutes"].as_u64().unwrap_or(0);

    Some(Nudge {
        message: format!(
            "Schedule conflict: \"{}\" and \"{}\" overlap by {} minutes. You might want to reschedule one.",
            event_a, event_b, overlap
        ),
        category: NudgeCategory::Calendar,
        importance: 0.7,
        reasoning: format!("{} min overlap between calendar events", overlap),
        source_event_kind: event.kind.as_str().into(),
        actionable: true,
        suggested_action: Some("Reschedule or decline one of the conflicting events".into()),
    })
}

fn nudge_personal_date(event: &LifeEvent) -> Option<Nudge> {
    let title = event.data["event_title"].as_str().unwrap_or("A special date");
    let days = event.data["days_away"].as_u64().unwrap_or(0);

    let person = event.entities.first().map(|s| s.as_str()).unwrap_or("someone");

    let message = if days <= 1 {
        format!(
            "Hey, {} is tomorrow! Have you got everything sorted?",
            title
        )
    } else if days <= 3 {
        format!(
            "{} is in {} days. Have you started planning or thought about gift ideas? I can help if you give me a budget and preferences to narrow it down.",
            title, days
        )
    } else {
        format!(
            "Just a heads up — {} is coming up in {} days. Good time to start thinking about it.",
            title, days
        )
    };

    Some(Nudge {
        message,
        category: NudgeCategory::Relationship,
        importance: if days <= 1 { 0.9 } else if days <= 3 { 0.7 } else { 0.4 },
        reasoning: format!("{}'s special date in {} days", person, days),
        source_event_kind: event.kind.as_str().into(),
        actionable: true,
        suggested_action: if days <= 3 {
            Some("Consider gift ideas, make dinner reservations, or plan an activity".into())
        } else {
            None
        },
    })
}

fn nudge_event_discovered(event: &LifeEvent) -> Option<Nudge> {
    let title = event.data["title"].as_str().unwrap_or("an event");
    let venue = event.data["venue"].as_str().unwrap_or("");
    let date = event.data["date"].as_str().unwrap_or("");
    let price = event.data["price"].as_str().unwrap_or("");
    let url = event.data["url"].as_str().unwrap_or("");

    let mut message = format!("I found something you might like: \"{}\"", title);
    if !venue.is_empty() {
        message.push_str(&format!(" at {}", venue));
    }
    if !date.is_empty() {
        message.push_str(&format!(" on {}", date));
    }
    message.push('.');
    if !price.is_empty() {
        message.push_str(&format!(" Tickets from {}.", price));
    }
    if !url.is_empty() {
        message.push_str(&format!(" More info: {}", url));
    }

    Some(Nudge {
        message,
        category: NudgeCategory::Discovery,
        importance: event.importance * 0.8,
        reasoning: "Event matches your interests".into(),
        source_event_kind: event.kind.as_str().into(),
        actionable: true,
        suggested_action: Some("Check it out and book tickets if interested".into()),
    })
}

fn nudge_relevant_news(event: &LifeEvent) -> Option<Nudge> {
    // Only nudge for high-importance news
    if event.importance < 0.5 {
        return None;
    }

    Some(Nudge {
        message: format!(
            "I was checking the news and thought you'd find this interesting: {}",
            event.summary
        ),
        category: NudgeCategory::General,
        importance: event.importance * 0.6,
        reasoning: "News article matched user interests with high relevance".into(),
        source_event_kind: event.kind.as_str().into(),
        actionable: false,
        suggested_action: None,
    })
}

fn nudge_free_block(event: &LifeEvent) -> Option<Nudge> {
    let duration = event.data["duration_minutes"].as_u64().unwrap_or(0);
    if duration < 60 {
        return None; // Only nudge for 1h+ blocks
    }

    let next_event = event.data["next_event"].as_str();
    let hours = duration / 60;

    let message = if let Some(next) = next_event {
        format!(
            "You have about {}h free before \"{}\". Good window for focused work or a break.",
            hours, next
        )
    } else {
        format!(
            "You have about {}h free for the rest of the work day. Good time for deep work.",
            hours
        )
    };

    Some(Nudge {
        message,
        category: NudgeCategory::Calendar,
        importance: 0.3,
        reasoning: format!("{}min free block detected in schedule", duration),
        source_event_kind: event.kind.as_str().into(),
        actionable: false,
        suggested_action: None,
    })
}

fn nudge_communication_gap(event: &LifeEvent) -> Option<Nudge> {
    let person = event.entities.first().map(|s| s.as_str()).unwrap_or("someone");

    Some(Nudge {
        message: format!(
            "It's been a while since you connected with {}. Maybe drop them a quick message?",
            person
        ),
        category: NudgeCategory::Relationship,
        importance: 0.3,
        reasoning: format!("No communication with {} for extended period", person),
        source_event_kind: event.kind.as_str().into(),
        actionable: true,
        suggested_action: Some(format!("Send a quick hello to {}", person)),
    })
}

fn nudge_bill_detected(event: &LifeEvent) -> Option<Nudge> {
    Some(Nudge {
        message: format!(
            "Looks like there's a bill or payment to handle: {}",
            event.summary
        ),
        category: NudgeCategory::Finance,
        importance: 0.7,
        reasoning: "Bill or renewal detected in email".into(),
        source_event_kind: event.kind.as_str().into(),
        actionable: true,
        suggested_action: Some("Review and pay or set a reminder".into()),
    })
}

// ── Morning Brief Composition ───────────────────────────────────────

/// Compose a morning brief from multiple LifeEvents.
/// Groups by category, prioritizes, and produces a cohesive summary.
pub fn compose_morning_brief(
    events: &[LifeEvent],
    user_name: &str,
) -> String {
    let all_nudges: Vec<Nudge> = events.iter().flat_map(generate_nudges).collect();

    if all_nudges.is_empty() {
        return format!("Good morning, {}! Looks like a quiet day ahead. Enjoy it!", user_name);
    }

    let mut sections: Vec<String> = Vec::new();

    // Group by category
    let weather: Vec<&Nudge> = all_nudges.iter().filter(|n| n.category == NudgeCategory::Weather).collect();
    let calendar: Vec<&Nudge> = all_nudges.iter().filter(|n| n.category == NudgeCategory::Calendar).collect();
    let relationships: Vec<&Nudge> = all_nudges.iter().filter(|n| n.category == NudgeCategory::Relationship).collect();
    let finance: Vec<&Nudge> = all_nudges.iter().filter(|n| n.category == NudgeCategory::Finance).collect();
    let discoveries: Vec<&Nudge> = all_nudges.iter().filter(|n| n.category == NudgeCategory::Discovery).collect();
    let general: Vec<&Nudge> = all_nudges.iter().filter(|n| n.category == NudgeCategory::General).collect();

    // Weather first — most immediately useful
    if !weather.is_empty() {
        let weather_lines: Vec<&str> = weather.iter().map(|n| n.message.as_str()).collect();
        sections.push(format!("🌤 Weather\n{}", weather_lines.join("\n")));
    }

    // Calendar — today's schedule
    if !calendar.is_empty() {
        let cal_lines: Vec<&str> = calendar.iter().map(|n| n.message.as_str()).collect();
        sections.push(format!("📅 Today's Schedule\n{}", cal_lines.join("\n")));
    }

    // Relationships — personal touch
    if !relationships.is_empty() {
        let rel_lines: Vec<&str> = relationships.iter().map(|n| n.message.as_str()).collect();
        sections.push(format!("💬 People\n{}", rel_lines.join("\n")));
    }

    // Finance — actionable
    if !finance.is_empty() {
        let fin_lines: Vec<&str> = finance.iter().map(|n| n.message.as_str()).collect();
        sections.push(format!("💰 Finance\n{}", fin_lines.join("\n")));
    }

    // Discoveries — fun stuff
    if !discoveries.is_empty() {
        let disc_lines: Vec<&str> = discoveries.iter().take(3).map(|n| n.message.as_str()).collect();
        sections.push(format!("✨ Interesting Finds\n{}", disc_lines.join("\n")));
    }

    // General news
    if !general.is_empty() {
        let gen_lines: Vec<&str> = general.iter().take(2).map(|n| n.message.as_str()).collect();
        sections.push(format!("📰 Worth Knowing\n{}", gen_lines.join("\n")));
    }

    format!(
        "Good morning, {}! Here's what's on your radar today:\n\n{}",
        user_name,
        sections.join("\n\n")
    )
}

// ── Structured Morning Brief ────────────────────────────────────────

/// A structured section for the morning brief card UI.
/// Each section maps to a collapsible row in the MorningBriefCard.
#[derive(Debug, Clone)]
pub struct BriefSection {
    /// Emoji icon for the section header.
    pub icon: String,
    /// Section label (e.g., "Weather", "Today's Schedule").
    pub label: String,
    /// Section body content.
    pub content: String,
    /// Whether the section should start expanded.
    pub expanded: bool,
    /// Optional action ID for tapping the section.
    pub action_id: String,
}

/// Result of composing a structured morning brief.
#[derive(Debug, Clone)]
pub struct StructuredBrief {
    /// Greeting line (e.g., "Good morning, Pranab!").
    pub greeting: String,
    /// Collapsible sections, ordered by priority.
    pub sections: Vec<BriefSection>,
}

/// Compose a structured morning brief from LifeEvents.
/// Returns typed sections for the desktop card UI instead of flat text.
pub fn compose_structured_brief(
    events: &[LifeEvent],
    user_name: &str,
) -> StructuredBrief {
    let all_nudges: Vec<Nudge> = events.iter().flat_map(generate_nudges).collect();

    let greeting = format!("Good morning, {}!", user_name);

    if all_nudges.is_empty() {
        return StructuredBrief {
            greeting,
            sections: vec![BriefSection {
                icon: "☀".into(),
                label: "Today".into(),
                content: "Looks like a quiet day ahead. Enjoy it!".into(),
                expanded: true,
                action_id: String::new(),
            }],
        };
    }

    let mut sections = Vec::new();

    // Weather — most immediately useful, always expanded
    let weather: Vec<&Nudge> = all_nudges.iter()
        .filter(|n| n.category == NudgeCategory::Weather)
        .collect();
    if !weather.is_empty() {
        let content = weather.iter().map(|n| n.message.as_str()).collect::<Vec<_>>().join("\n");
        sections.push(BriefSection {
            icon: "🌤".into(),
            label: "Weather".into(),
            content,
            expanded: true,
            action_id: "navigate:weather".into(),
        });
    }

    // Calendar — today's schedule
    let calendar: Vec<&Nudge> = all_nudges.iter()
        .filter(|n| n.category == NudgeCategory::Calendar)
        .collect();
    if !calendar.is_empty() {
        let content = calendar.iter().map(|n| n.message.as_str()).collect::<Vec<_>>().join("\n");
        sections.push(BriefSection {
            icon: "📅".into(),
            label: "Today's Schedule".into(),
            content,
            expanded: true,
            action_id: "navigate:calendar".into(),
        });
    }

    // Relationships — personal touch
    let relationships: Vec<&Nudge> = all_nudges.iter()
        .filter(|n| n.category == NudgeCategory::Relationship)
        .collect();
    if !relationships.is_empty() {
        let content = relationships.iter().map(|n| n.message.as_str()).collect::<Vec<_>>().join("\n");
        sections.push(BriefSection {
            icon: "💬".into(),
            label: "People".into(),
            content,
            expanded: false,
            action_id: String::new(),
        });
    }

    // Finance — actionable
    let finance: Vec<&Nudge> = all_nudges.iter()
        .filter(|n| n.category == NudgeCategory::Finance)
        .collect();
    if !finance.is_empty() {
        let content = finance.iter().map(|n| n.message.as_str()).collect::<Vec<_>>().join("\n");
        sections.push(BriefSection {
            icon: "💰".into(),
            label: "Finance".into(),
            content,
            expanded: false,
            action_id: String::new(),
        });
    }

    // Discoveries — fun stuff
    let discoveries: Vec<&Nudge> = all_nudges.iter()
        .filter(|n| n.category == NudgeCategory::Discovery)
        .collect();
    if !discoveries.is_empty() {
        let content = discoveries.iter().take(3).map(|n| n.message.as_str()).collect::<Vec<_>>().join("\n");
        sections.push(BriefSection {
            icon: "✨".into(),
            label: "Interesting Finds".into(),
            content,
            expanded: false,
            action_id: String::new(),
        });
    }

    // Health
    let health: Vec<&Nudge> = all_nudges.iter()
        .filter(|n| n.category == NudgeCategory::Health)
        .collect();
    if !health.is_empty() {
        let content = health.iter().map(|n| n.message.as_str()).collect::<Vec<_>>().join("\n");
        sections.push(BriefSection {
            icon: "🫀".into(),
            label: "Health".into(),
            content,
            expanded: false,
            action_id: String::new(),
        });
    }

    // General news
    let general: Vec<&Nudge> = all_nudges.iter()
        .filter(|n| n.category == NudgeCategory::General)
        .collect();
    if !general.is_empty() {
        let content = general.iter().take(2).map(|n| n.message.as_str()).collect::<Vec<_>>().join("\n");
        sections.push(BriefSection {
            icon: "📰".into(),
            label: "Worth Knowing".into(),
            content,
            expanded: false,
            action_id: String::new(),
        });
    }

    StructuredBrief { greeting, sections }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn now_ts() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    #[test]
    fn rain_alert_nudge() {
        let event = LifeEvent {
            kind: LifeEventKind::PrecipitationAlert,
            summary: "Rain expected".into(),
            keywords: vec!["rain".into()],
            entities: vec!["home".into()],
            importance: 0.7,
            source: "weather".into(),
            data: serde_json::json!({
                "location": "home",
                "rain_hours": ["8:00", "9:00"],
                "max_probability": 85.0,
            }),
            timestamp: now_ts(),
        };

        let nudges = generate_nudges(&event);
        assert_eq!(nudges.len(), 1);

        let nudge = &nudges[0];
        assert!(nudge.message.contains("umbrella"));
        assert!(nudge.message.contains("8:00"));
        assert_eq!(nudge.category, NudgeCategory::Weather);
        assert!(nudge.actionable);
    }

    #[test]
    fn fuel_price_nudge() {
        let event = LifeEvent {
            kind: LifeEventKind::PriceChange,
            summary: "Crude oil prices surge 5% after Iran tensions".into(),
            keywords: vec!["oil".into(), "crude".into(), "fuel".into(), "prices".into()],
            entities: vec![],
            importance: 0.6,
            source: "news".into(),
            data: serde_json::json!({}),
            timestamp: now_ts(),
        };

        let nudges = generate_nudges(&event);
        assert_eq!(nudges.len(), 1);

        let nudge = &nudges[0];
        assert!(nudge.message.contains("top up the car"));
        assert_eq!(nudge.category, NudgeCategory::Finance);
    }

    #[test]
    fn birthday_nudge_3_days() {
        let event = LifeEvent {
            kind: LifeEventKind::DateApproaching,
            summary: "Mom's Birthday in 3 days".into(),
            keywords: vec!["birthday".into()],
            entities: vec!["Mom".into()],
            importance: 0.7,
            source: "calendar".into(),
            data: serde_json::json!({
                "event_title": "Mom's Birthday",
                "days_away": 3,
            }),
            timestamp: now_ts(),
        };

        let nudges = generate_nudges(&event);
        assert_eq!(nudges.len(), 1);

        let nudge = &nudges[0];
        assert!(nudge.message.contains("Birthday"));
        assert!(nudge.message.contains("gift"));
        assert!(nudge.message.contains("3 days"));
        assert_eq!(nudge.category, NudgeCategory::Relationship);
    }

    #[test]
    fn birthday_nudge_tomorrow() {
        let event = LifeEvent {
            kind: LifeEventKind::DateApproaching,
            summary: "Mom's Birthday tomorrow".into(),
            keywords: vec!["birthday".into()],
            entities: vec!["Mom".into()],
            importance: 0.9,
            source: "calendar".into(),
            data: serde_json::json!({
                "event_title": "Mom's Birthday",
                "days_away": 1,
            }),
            timestamp: now_ts(),
        };

        let nudges = generate_nudges(&event);
        let nudge = &nudges[0];
        assert!(nudge.message.contains("tomorrow"));
        assert!(nudge.importance >= 0.9);
    }

    #[test]
    fn event_discovered_nudge() {
        let event = LifeEvent {
            kind: LifeEventKind::EventDiscovered,
            summary: "Concert found".into(),
            keywords: vec!["music".into(), "concert".into()],
            entities: vec!["Coldplay Concert".into()],
            importance: 0.7,
            source: "events".into(),
            data: serde_json::json!({
                "title": "Coldplay World Tour",
                "venue": "Wembley Stadium",
                "date": "April 15, 2026",
                "price": "$85",
                "url": "https://example.com/coldplay",
            }),
            timestamp: now_ts(),
        };

        let nudges = generate_nudges(&event);
        assert_eq!(nudges.len(), 1);

        let nudge = &nudges[0];
        assert!(nudge.message.contains("Coldplay"));
        assert!(nudge.message.contains("Wembley"));
        assert!(nudge.message.contains("$85"));
        assert_eq!(nudge.category, NudgeCategory::Discovery);
    }

    #[test]
    fn calendar_approaching_nudge() {
        let event = LifeEvent {
            kind: LifeEventKind::CalendarApproaching,
            summary: "Meeting in 15 min".into(),
            keywords: vec!["calendar".into()],
            entities: vec![],
            importance: 0.8,
            source: "calendar".into(),
            data: serde_json::json!({
                "event_title": "Sprint Planning",
                "minutes_until": 15,
                "location": "Conference Room B",
            }),
            timestamp: now_ts(),
        };

        let nudges = generate_nudges(&event);
        assert_eq!(nudges.len(), 1);
        assert!(nudges[0].message.contains("Sprint Planning"));
        assert!(nudges[0].message.contains("15 minutes"));
        assert!(nudges[0].message.contains("Conference Room B"));
    }

    #[test]
    fn conflict_nudge() {
        let event = LifeEvent {
            kind: LifeEventKind::CalendarConflict,
            summary: "Conflict detected".into(),
            keywords: vec!["calendar".into(), "conflict".into()],
            entities: vec![],
            importance: 0.8,
            source: "calendar".into(),
            data: serde_json::json!({
                "event_a": { "title": "Team Standup" },
                "event_b": { "title": "1:1 with Manager" },
                "overlap_minutes": 30,
            }),
            timestamp: now_ts(),
        };

        let nudges = generate_nudges(&event);
        assert_eq!(nudges.len(), 1);
        assert!(nudges[0].message.contains("Team Standup"));
        assert!(nudges[0].message.contains("1:1 with Manager"));
        assert!(nudges[0].message.contains("30 minutes"));
    }

    #[test]
    fn low_importance_news_not_nudged() {
        let event = LifeEvent {
            kind: LifeEventKind::NewsRelevant,
            summary: "Minor tech update".into(),
            keywords: vec!["tech".into()],
            entities: vec![],
            importance: 0.3, // below 0.5 threshold
            source: "news".into(),
            data: serde_json::json!({}),
            timestamp: now_ts(),
        };

        let nudges = generate_nudges(&event);
        assert!(nudges.is_empty(), "Low-importance news should not generate nudge");
    }

    #[test]
    fn morning_brief_composition() {
        let events = vec![
            LifeEvent {
                kind: LifeEventKind::PrecipitationAlert,
                summary: "Rain".into(),
                keywords: vec![],
                entities: vec![],
                importance: 0.7,
                source: "weather".into(),
                data: serde_json::json!({
                    "location": "home",
                    "rain_hours": ["8:00"],
                    "max_probability": 90.0,
                }),
                timestamp: now_ts(),
            },
            LifeEvent {
                kind: LifeEventKind::DateApproaching,
                summary: "Anniversary".into(),
                keywords: vec![],
                entities: vec!["Partner".into()],
                importance: 0.8,
                source: "calendar".into(),
                data: serde_json::json!({
                    "event_title": "Wedding Anniversary",
                    "days_away": 2,
                }),
                timestamp: now_ts(),
            },
            LifeEvent {
                kind: LifeEventKind::EventDiscovered,
                summary: "Concert".into(),
                keywords: vec![],
                entities: vec![],
                importance: 0.6,
                source: "events".into(),
                data: serde_json::json!({
                    "title": "Jazz Night",
                    "venue": "Blue Note",
                    "date": "March 20",
                    "price": "$25",
                    "url": "",
                }),
                timestamp: now_ts(),
            },
        ];

        let brief = compose_morning_brief(&events, "Alex");
        assert!(brief.contains("Good morning, Alex!"));
        assert!(brief.contains("Weather"));
        assert!(brief.contains("umbrella"));
        assert!(brief.contains("People"));
        assert!(brief.contains("Anniversary"));
        assert!(brief.contains("Interesting Finds"));
        assert!(brief.contains("Jazz Night"));
    }

    #[test]
    fn empty_morning_brief() {
        let brief = compose_morning_brief(&[], "Alex");
        assert!(brief.contains("Good morning, Alex!"));
        assert!(brief.contains("quiet day"));
    }

    #[test]
    fn severe_weather_high_importance() {
        let event = LifeEvent {
            kind: LifeEventKind::SevereWeather,
            summary: "Thunderstorm alert".into(),
            keywords: vec!["severe".into()],
            entities: vec![],
            importance: 0.9,
            source: "weather".into(),
            data: serde_json::json!({
                "location": "home",
                "description": "Thunderstorm with hail",
            }),
            timestamp: now_ts(),
        };

        let nudges = generate_nudges(&event);
        assert_eq!(nudges.len(), 1);
        assert!(nudges[0].importance >= 0.9);
        assert!(nudges[0].message.contains("Thunderstorm"));
    }

    #[test]
    fn structured_brief_with_events() {
        let events = vec![
            LifeEvent {
                kind: LifeEventKind::PrecipitationAlert,
                summary: "Rain".into(),
                keywords: vec!["rain".into()],
                entities: vec!["home".into()],
                importance: 0.7,
                source: "weather".into(),
                data: serde_json::json!({
                    "location": "home",
                    "rain_hours": ["8:00", "9:00"],
                    "max_probability": 85.0,
                }),
                timestamp: now_ts(),
            },
            LifeEvent {
                kind: LifeEventKind::EventDiscovered,
                summary: "Concert".into(),
                keywords: vec![],
                entities: vec![],
                importance: 0.6,
                source: "events".into(),
                data: serde_json::json!({
                    "title": "Jazz Night",
                    "venue": "Blue Note",
                    "date": "March 20",
                    "price": "$25",
                    "url": "",
                }),
                timestamp: now_ts(),
            },
        ];

        let brief = compose_structured_brief(&events, "Alex");
        assert_eq!(brief.greeting, "Good morning, Alex!");
        assert!(brief.sections.len() >= 2);

        // Weather section should be first and expanded
        let weather = &brief.sections[0];
        assert_eq!(weather.label, "Weather");
        assert!(weather.expanded);
        assert!(weather.content.contains("umbrella"));
        assert_eq!(weather.action_id, "navigate:weather");

        // Discovery section should exist
        let discovery = brief.sections.iter().find(|s| s.label == "Interesting Finds");
        assert!(discovery.is_some());
        assert!(discovery.unwrap().content.contains("Jazz Night"));
    }

    #[test]
    fn structured_brief_empty_events() {
        let brief = compose_structured_brief(&[], "Pranab");
        assert_eq!(brief.greeting, "Good morning, Pranab!");
        assert_eq!(brief.sections.len(), 1);
        assert!(brief.sections[0].content.contains("quiet day"));
    }
}
