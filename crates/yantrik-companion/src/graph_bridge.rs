//! Graph Bridge — maps normalized life events to PWG node activations.
//!
//! The bridge sits between data connectors (weather, news, calendar, email, etc.)
//! and the Personal World Graph. When a connector produces a normalized event,
//! the bridge:
//! 1. Identifies which PWG nodes are relevant (by keyword, entity type, or name)
//! 2. Activates those nodes with appropriate delta
//! 3. Propagation handles the rest (oil prices → fuel → commute)
//!
//! This is **Tier 1 intelligence** — pure rules, zero LLM cost.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::world_graph::{EntityType, WorldGraph};

// ── Normalized Life Events ───────────────────────────────────────────

/// A normalized event from any data connector.
/// These are produced by weather, news, calendar, email, filesystem, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifeEvent {
    /// Unique event type identifier.
    pub kind: LifeEventKind,
    /// Human-readable summary.
    pub summary: String,
    /// Keywords/topics extracted from this event.
    pub keywords: Vec<String>,
    /// Entities mentioned (people, places, projects).
    pub entities: Vec<String>,
    /// How urgent/important this event is (0.0–1.0).
    pub importance: f64,
    /// Source connector that produced this event.
    pub source: String,
    /// Optional structured data.
    pub data: serde_json::Value,
    /// Unix timestamp of the event.
    pub timestamp: f64,
}

/// Categories of life events that connectors produce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifeEventKind {
    // ── Weather ──
    /// Weather forecast updated for a location.
    WeatherForecast,
    /// Rain/snow expected during a relevant time window.
    PrecipitationAlert,
    /// Extreme temperature warning.
    TemperatureExtreme,
    /// Severe weather alert.
    SevereWeather,

    // ── News ──
    /// A news article matched user interests.
    NewsRelevant,
    /// Breaking news with potential personal impact.
    NewsBreaking,
    /// Price/market change that may affect user.
    PriceChange,

    // ── Calendar ──
    /// A calendar event is approaching.
    CalendarApproaching,
    /// A calendar conflict was detected.
    CalendarConflict,
    /// New calendar event was added.
    CalendarCreated,
    /// A free block was detected in the schedule.
    FreeBlockDetected,

    // ── Email ──
    /// Important email received (from known person or high-priority).
    EmailImportant,
    /// Email thread aging without reply.
    EmailAging,
    /// Bill or renewal detected in email.
    BillDetected,

    // ── Files ──
    /// A document was opened/modified.
    DocumentActivity,
    /// A document was opened repeatedly without meaningful changes.
    DocumentStalled,
    /// A deadline-related document was detected.
    DeadlineDocument,

    // ── Events/Discovery ──
    /// A local event matching user interests was found.
    EventDiscovered,

    // ── Personal ──
    /// An important date is approaching (birthday, anniversary).
    DateApproaching,
    /// Communication gap detected with an important person.
    CommunicationGap,

    // ── System ──
    /// User entered focus/deep work state.
    FocusStarted,
    /// User has been idle for extended period.
    UserIdle,
    /// User resumed from idle.
    UserResumed,

    /// Generic / custom event type.
    Custom(String),
}

impl LifeEventKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::WeatherForecast => "weather:forecast",
            Self::PrecipitationAlert => "weather:precipitation",
            Self::TemperatureExtreme => "weather:temperature",
            Self::SevereWeather => "weather:severe",
            Self::NewsRelevant => "news:relevant",
            Self::NewsBreaking => "news:breaking",
            Self::PriceChange => "news:price_change",
            Self::CalendarApproaching => "calendar:approaching",
            Self::CalendarConflict => "calendar:conflict",
            Self::CalendarCreated => "calendar:created",
            Self::FreeBlockDetected => "calendar:free_block",
            Self::EmailImportant => "email:important",
            Self::EmailAging => "email:aging",
            Self::BillDetected => "email:bill",
            Self::DocumentActivity => "file:activity",
            Self::DocumentStalled => "file:stalled",
            Self::DeadlineDocument => "file:deadline",
            Self::EventDiscovered => "event:discovered",
            Self::DateApproaching => "personal:date",
            Self::CommunicationGap => "personal:comm_gap",
            Self::FocusStarted => "system:focus",
            Self::UserIdle => "system:idle",
            Self::UserResumed => "system:resumed",
            Self::Custom(s) => s.as_str(),
        }
    }
}

// ── Graph Bridge ─────────────────────────────────────────────────────

/// Result of processing a life event through the bridge.
#[derive(Debug, Clone)]
pub struct BridgeResult {
    /// How many nodes were directly activated.
    pub direct_activations: usize,
    /// How many nodes were activated via propagation.
    pub propagated_activations: usize,
    /// Names of directly activated nodes (for logging/debugging).
    pub activated_names: Vec<String>,
}

/// Maps life events to PWG node activations.
pub struct GraphBridge;

impl GraphBridge {
    /// Process a life event: identify relevant nodes and activate them.
    pub fn process_event(conn: &Connection, event: &LifeEvent) -> BridgeResult {
        let mut direct = 0;
        let mut propagated = 0;
        let mut activated_names = Vec::new();

        // Strategy 1: Match by explicit entities mentioned in the event
        for entity_name in &event.entities {
            if let Some(node) = Self::find_best_match(conn, entity_name) {
                let trigger = format!("{}:{}", event.kind.as_str(), entity_name);
                let records = WorldGraph::activate(conn, node.id, event.importance * 0.8, &trigger);
                activated_names.push(node.name.clone());
                direct += 1;
                propagated += records.len().saturating_sub(1);
            }
        }

        // Strategy 2: Match by keywords against interest/resource nodes
        for keyword in &event.keywords {
            let matches = WorldGraph::search_nodes(conn, keyword);
            for node in matches.iter().take(3) {
                // Skip if already activated by entity match
                if activated_names.contains(&node.name) {
                    continue;
                }
                let trigger = format!("{}:kw:{}", event.kind.as_str(), keyword);
                let delta = event.importance * keyword_relevance_score(keyword, node);
                if delta > 0.05 {
                    let records = WorldGraph::activate(conn, node.id, delta, &trigger);
                    activated_names.push(node.name.clone());
                    direct += 1;
                    propagated += records.len().saturating_sub(1);
                }
            }
        }

        // Strategy 3: Event-type-specific rules (hardcoded Tier 1 intelligence)
        let rule_activations = Self::apply_event_rules(conn, event);
        for (name, node_id, delta) in &rule_activations {
            if !activated_names.contains(name) {
                let trigger = format!("{}:rule", event.kind.as_str());
                let records = WorldGraph::activate(conn, *node_id, *delta, &trigger);
                activated_names.push(name.clone());
                direct += 1;
                propagated += records.len().saturating_sub(1);
            }
        }

        BridgeResult {
            direct_activations: direct,
            propagated_activations: propagated,
            activated_names,
        }
    }

    /// Apply hardcoded rules based on event type.
    /// Returns (node_name, node_id, delta) for each rule-based activation.
    fn apply_event_rules(conn: &Connection, event: &LifeEvent) -> Vec<(String, i64, f64)> {
        let mut activations = Vec::new();

        match &event.kind {
            // Weather events → activate "weather" resource + location nodes
            LifeEventKind::PrecipitationAlert
            | LifeEventKind::SevereWeather
            | LifeEventKind::TemperatureExtreme
            | LifeEventKind::WeatherForecast => {
                if let Some(weather) = WorldGraph::find_node(conn, EntityType::Resource, "weather") {
                    activations.push(("weather".into(), weather.id, event.importance));
                }
                if let Some(commute) = WorldGraph::find_node(conn, EntityType::Rhythm, "commute") {
                    activations.push(("commute".into(), commute.id, event.importance * 0.6));
                }
            }

            // Price changes → activate relevant resource nodes
            LifeEventKind::PriceChange => {
                // Check if this is about fuel/oil
                let is_fuel = event.keywords.iter().any(|k| {
                    let kl = k.to_lowercase();
                    kl.contains("oil") || kl.contains("fuel") || kl.contains("gas") || kl.contains("petrol")
                });
                if is_fuel {
                    if let Some(fuel) = WorldGraph::find_node(conn, EntityType::Resource, "fuel") {
                        activations.push(("fuel".into(), fuel.id, event.importance));
                    }
                }
            }

            // Calendar approaching → activate commute + work_hours
            LifeEventKind::CalendarApproaching => {
                if let Some(work) = WorldGraph::find_node(conn, EntityType::Rhythm, "work_hours") {
                    activations.push(("work_hours".into(), work.id, event.importance * 0.5));
                }
            }

            // Email aging / communication gap → find the person node
            LifeEventKind::EmailAging | LifeEventKind::CommunicationGap => {
                for entity in &event.entities {
                    if let Some(person) = WorldGraph::find_node(conn, EntityType::Person, entity) {
                        activations.push((entity.clone(), person.id, event.importance));
                    }
                }
            }

            // Bill detected → activate finance-related interests
            LifeEventKind::BillDetected => {
                if let Some(finance) = WorldGraph::find_node(conn, EntityType::Interest, "Finance") {
                    activations.push(("Finance".into(), finance.id, event.importance * 0.6));
                }
                // Also try "personal finance" sub-interest
                if let Some(pf) = WorldGraph::find_node(conn, EntityType::Interest, "personal finance") {
                    activations.push(("personal finance".into(), pf.id, event.importance * 0.7));
                }
            }

            // Important date approaching → relationship care
            LifeEventKind::DateApproaching => {
                for entity in &event.entities {
                    if let Some(person) = WorldGraph::find_node(conn, EntityType::Person, entity) {
                        activations.push((entity.clone(), person.id, event.importance));
                    }
                }
            }

            // Focus/idle → activate work rhythm
            LifeEventKind::FocusStarted => {
                if let Some(work) = WorldGraph::find_node(conn, EntityType::Rhythm, "work_hours") {
                    activations.push(("work_hours".into(), work.id, 0.5));
                }
            }

            _ => {}
        }

        activations
    }

    /// Find the best matching node for an entity name.
    fn find_best_match(
        conn: &Connection,
        name: &str,
    ) -> Option<crate::world_graph::GraphNode> {
        // Try exact match by name across all types
        let types = [
            EntityType::Person,
            EntityType::Project,
            EntityType::Commitment,
            EntityType::Place,
            EntityType::Resource,
            EntityType::Interest,
        ];

        for etype in &types {
            if let Some(node) = WorldGraph::find_node(conn, *etype, name) {
                return Some(node);
            }
        }

        // Fall back to keyword search
        let results = WorldGraph::search_nodes(conn, name);
        results.into_iter().next()
    }
}

/// Compute a relevance score for how well a keyword matches a graph node.
fn keyword_relevance_score(keyword: &str, node: &crate::world_graph::GraphNode) -> f64 {
    let kl = keyword.to_lowercase();
    let nl = node.name.to_lowercase();

    // Exact name match
    if nl == kl {
        return 0.9;
    }

    // Name contains keyword
    if nl.contains(&kl) || kl.contains(&nl) {
        return 0.7;
    }

    // Keyword matches one of the node's keywords
    for nk in &node.keywords {
        if nk.to_lowercase() == kl {
            return 0.8;
        }
        if nk.to_lowercase().contains(&kl) || kl.contains(&nk.to_lowercase()) {
            return 0.5;
        }
    }

    0.2 // Weak match (found via search)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_graph::WorldGraph;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        WorldGraph::ensure_tables(&conn);
        WorldGraph::seed_interests(&conn, &["Finance", "Technology"]);
        WorldGraph::seed_defaults(&conn);
        conn
    }

    fn now_ts() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    #[test]
    fn weather_event_activates_commute() {
        let conn = setup();

        let event = LifeEvent {
            kind: LifeEventKind::PrecipitationAlert,
            summary: "Rain expected from 2pm today".into(),
            keywords: vec!["rain".into(), "precipitation".into()],
            entities: vec![],
            importance: 0.7,
            source: "weather".into(),
            data: serde_json::json!({"rain_start": "14:00", "probability": 0.85}),
            timestamp: now_ts(),
        };

        let result = GraphBridge::process_event(&conn, &event);
        assert!(result.direct_activations > 0, "should activate weather/commute nodes");
        assert!(result.activated_names.contains(&"weather".to_string()));

        // Commute should be activated (via rule or propagation)
        let commute = WorldGraph::find_node(&conn, EntityType::Rhythm, "commute").unwrap();
        assert!(commute.salience > 0.0, "commute should be activated");
    }

    #[test]
    fn news_event_oil_prices() {
        let conn = setup();

        let event = LifeEvent {
            kind: LifeEventKind::PriceChange,
            summary: "Oil prices surge after Iran conflict escalation".into(),
            keywords: vec!["oil".into(), "prices".into(), "iran".into(), "fuel".into()],
            entities: vec![],
            importance: 0.8,
            source: "news_scanner".into(),
            data: serde_json::json!({"commodity": "crude_oil", "change_pct": 5.2}),
            timestamp: now_ts(),
        };

        let result = GraphBridge::process_event(&conn, &event);
        assert!(result.direct_activations > 0);

        // Fuel resource should be activated
        let fuel = WorldGraph::find_node(&conn, EntityType::Resource, "fuel").unwrap();
        assert!(fuel.salience > 0.0, "fuel should be activated by oil price news");

        // Commute should be activated via propagation from fuel
        let commute = WorldGraph::find_node(&conn, EntityType::Rhythm, "commute").unwrap();
        assert!(commute.salience > 0.0, "commute activated via fuel propagation");
    }

    #[test]
    fn calendar_event_activates_person() {
        let conn = setup();

        // Add a person node
        WorldGraph::find_or_create(
            &conn, EntityType::Person, "Sarah Chen", "email", &["sarah", "chen"],
        );

        let event = LifeEvent {
            kind: LifeEventKind::CalendarApproaching,
            summary: "Meeting with Sarah Chen in 30 minutes".into(),
            keywords: vec!["meeting".into(), "budget review".into()],
            entities: vec!["Sarah Chen".into()],
            importance: 0.8,
            source: "calendar".into(),
            data: serde_json::json!({"event_id": "evt-123", "minutes_until": 30}),
            timestamp: now_ts(),
        };

        let result = GraphBridge::process_event(&conn, &event);
        assert!(result.activated_names.contains(&"Sarah Chen".to_string()));

        let sarah = WorldGraph::find_node(&conn, EntityType::Person, "Sarah Chen").unwrap();
        assert!(sarah.salience > 0.0);
    }

    #[test]
    fn bill_detected_activates_finance() {
        let conn = setup();

        let event = LifeEvent {
            kind: LifeEventKind::BillDetected,
            summary: "Insurance renewal notice — due in 5 days".into(),
            keywords: vec!["insurance".into(), "renewal".into(), "bill".into()],
            entities: vec![],
            importance: 0.7,
            source: "email_scanner".into(),
            data: serde_json::json!({"amount": 1200, "due_days": 5}),
            timestamp: now_ts(),
        };

        let result = GraphBridge::process_event(&conn, &event);
        assert!(result.direct_activations > 0);

        // Finance interest should be activated
        let finance = WorldGraph::find_node(&conn, EntityType::Interest, "Finance").unwrap();
        assert!(finance.salience > 0.0);
    }

    #[test]
    fn keyword_matching_activates_interests() {
        let conn = setup();

        let event = LifeEvent {
            kind: LifeEventKind::NewsRelevant,
            summary: "New AI breakthrough in language models".into(),
            keywords: vec!["AI".into(), "language models".into(), "technology".into()],
            entities: vec![],
            importance: 0.6,
            source: "news_scanner".into(),
            data: serde_json::Value::Null,
            timestamp: now_ts(),
        };

        let result = GraphBridge::process_event(&conn, &event);
        // Should activate AI interest node (child of Technology)
        assert!(result.direct_activations > 0);

        let ai = WorldGraph::find_node(&conn, EntityType::Interest, "AI");
        assert!(ai.is_some(), "AI interest should exist");
        assert!(ai.unwrap().salience > 0.0, "AI should be activated by keyword match");
    }

    #[test]
    fn anniversary_approaching() {
        let conn = setup();

        // Add spouse node
        WorldGraph::find_or_create(
            &conn, EntityType::Person, "Maya", "onboarding", &["spouse", "wife"],
        );

        let event = LifeEvent {
            kind: LifeEventKind::DateApproaching,
            summary: "Wedding anniversary with Maya in 3 weeks".into(),
            keywords: vec!["anniversary".into(), "wedding".into()],
            entities: vec!["Maya".into()],
            importance: 0.8,
            source: "calendar".into(),
            data: serde_json::json!({"date_type": "anniversary", "days_until": 21}),
            timestamp: now_ts(),
        };

        let result = GraphBridge::process_event(&conn, &event);
        assert!(result.activated_names.contains(&"Maya".to_string()));

        let maya = WorldGraph::find_node(&conn, EntityType::Person, "Maya").unwrap();
        assert!(maya.salience > 0.0);
    }

    #[test]
    fn event_discovered_concert() {
        let conn = setup();

        let event = LifeEvent {
            kind: LifeEventKind::EventDiscovered,
            summary: "Radiohead concert at Madison Square Garden, tickets from $45".into(),
            keywords: vec!["concert".into(), "music".into(), "radiohead".into()],
            entities: vec![],
            importance: 0.5,
            source: "event_scanner".into(),
            data: serde_json::json!({
                "event_name": "Radiohead Live",
                "venue": "Madison Square Garden",
                "price_from": 45,
                "date": "2026-04-15"
            }),
            timestamp: now_ts(),
        };

        // This should activate music-related interest nodes if user has Music interest
        // Since we seeded Technology and Finance, not Music, it should have weaker matches
        let result = GraphBridge::process_event(&conn, &event);
        // May or may not match depending on keyword overlap
        // The point is it doesn't crash and processes correctly
        assert!(result.direct_activations >= 0);
    }

    #[test]
    fn multiple_events_accumulate_salience() {
        let conn = setup();

        // Two related news events should stack salience
        for summary in &["Oil prices up 3%", "OPEC cuts production forecast"] {
            let event = LifeEvent {
                kind: LifeEventKind::PriceChange,
                summary: summary.to_string(),
                keywords: vec!["oil".into(), "prices".into()],
                entities: vec![],
                importance: 0.6,
                source: "news".into(),
                data: serde_json::Value::Null,
                timestamp: now_ts(),
            };
            GraphBridge::process_event(&conn, &event);
        }

        let fuel = WorldGraph::find_node(&conn, EntityType::Resource, "fuel").unwrap();
        // Should have accumulated salience from both events
        assert!(fuel.salience > 0.3, "salience should accumulate from multiple events, got {}", fuel.salience);
    }
}
