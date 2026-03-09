//! Stewardship Loop — the background runtime that makes Yantrik a living companion.
//!
//! Periodically polls all connectors, feeds LifeEvents through the GraphBridge
//! into the PWG, checks for salient nodes, and generates proactive nudges.
//!
//! Schedule:
//! - Calendar: every 15 min
//! - News:     every 30 min
//! - Weather:  every 60 min
//! - Events:   every 24 hours
//! - PWG decay + salience check: every 5 min
//! - Morning brief: once daily at configured time
//!
//! Architecture:
//! ```
//! ┌─────────────────────────────────────────┐
//! │         Stewardship Loop (thread)        │
//! │                                          │
//! │  ┌──────────┐  ┌──────────┐  ┌────────┐ │
//! │  │ Weather  │  │  News    │  │Calendar│ │
//! │  │Connector │  │ Scanner  │  │Analyzer│ │
//! │  └────┬─────┘  └────┬─────┘  └───┬────┘ │
//! │       │              │             │      │
//! │       ▼              ▼             ▼      │
//! │  ┌──────────────────────────────────────┐│
//! │  │     LifeEvent normalization           ││
//! │  └──────────────┬───────────────────────┘│
//! │                 ▼                         │
//! │  ┌──────────────────────────────────────┐│
//! │  │     GraphBridge → PWG activation      ││
//! │  └──────────────┬───────────────────────┘│
//! │                 ▼                         │
//! │  ┌──────────────────────────────────────┐│
//! │  │  Nudge Templates → proactive message  ││
//! │  └──────────────────────────────────────┘│
//! └─────────────────────────────────────────┘
//! ```

use rusqlite::Connection;

use crate::connectors::weather::{WeatherConfig, scan_weather};
use crate::connectors::news::{self, NewsScannerConfig};
use crate::connectors::calendar::{CalendarConfig, CalendarEvent, analyze_calendar};
use crate::connectors::events::{EventDiscoveryConfig, scan_events};
use crate::graph_bridge::{GraphBridge, LifeEvent};
use crate::nudge_templates::{Nudge, compose_morning_brief, generate_nudges};
use crate::world_graph::WorldGraph;

// ── Configuration ───────────────────────────────────────────────────

/// Stewardship loop configuration.
#[derive(Debug, Clone)]
pub struct StewardshipConfig {
    /// Weather poll interval in seconds.
    pub weather_interval_secs: u64,
    /// News poll interval in seconds.
    pub news_interval_secs: u64,
    /// Calendar check interval in seconds.
    pub calendar_interval_secs: u64,
    /// Event discovery interval in seconds.
    pub events_interval_secs: u64,
    /// PWG decay + salience check interval in seconds.
    pub salience_check_interval_secs: u64,
    /// Minimum salience to trigger a proactive nudge.
    pub nudge_salience_threshold: f64,
    /// Maximum nudges per hour (anti-spam).
    pub max_nudges_per_hour: u32,
    /// User's name for morning brief.
    pub user_name: String,
    /// Hour to deliver morning brief (0-23).
    pub morning_brief_hour: u8,
    /// Weather config.
    pub weather: WeatherConfig,
    /// Calendar config.
    pub calendar: CalendarConfig,
    /// Event discovery config.
    pub events: EventDiscoveryConfig,
    /// News scanner config.
    pub news: NewsScannerConfig,
}

impl Default for StewardshipConfig {
    fn default() -> Self {
        Self {
            weather_interval_secs: 3600,      // 1 hour
            news_interval_secs: 1800,         // 30 min
            calendar_interval_secs: 900,      // 15 min
            events_interval_secs: 86400,      // 24 hours
            salience_check_interval_secs: 300, // 5 min
            nudge_salience_threshold: 0.4,
            max_nudges_per_hour: 5,
            user_name: "there".into(),
            morning_brief_hour: 7,
            weather: WeatherConfig::default(),
            calendar: CalendarConfig::default(),
            events: EventDiscoveryConfig::default(),
            news: NewsScannerConfig::default(),
        }
    }
}

// ── Stewardship State ───────────────────────────────────────────────

/// Tracks timing and state for the stewardship loop.
pub struct StewardshipState {
    pub config: StewardshipConfig,
    /// Last poll timestamps.
    last_weather_poll: f64,
    last_news_poll: f64,
    last_calendar_poll: f64,
    last_events_poll: f64,
    last_salience_check: f64,
    last_morning_brief_day: u32, // day-of-year of last brief
    /// Nudge rate limiting.
    nudges_this_hour: u32,
    nudge_hour_start: f64,
    /// Accumulated nudges waiting for delivery.
    pending_nudges: Vec<Nudge>,
    /// Calendar events (refreshed periodically).
    calendar_events: Vec<CalendarEvent>,
}

impl StewardshipState {
    pub fn new(config: StewardshipConfig) -> Self {
        Self {
            config,
            last_weather_poll: 0.0,
            last_news_poll: 0.0,
            last_calendar_poll: 0.0,
            last_events_poll: 0.0,
            last_salience_check: 0.0,
            last_morning_brief_day: 0,
            nudges_this_hour: 0,
            nudge_hour_start: 0.0,
            pending_nudges: Vec::new(),
            calendar_events: Vec::new(),
        }
    }

    /// Run a single stewardship tick. Call this periodically (e.g., every 60 seconds).
    /// Returns any nudges generated during this tick.
    pub fn tick(&mut self, conn: &Connection) -> Vec<Nudge> {
        let now = now_ts();
        let mut all_events: Vec<LifeEvent> = Vec::new();

        // Reset hourly nudge counter
        if now - self.nudge_hour_start > 3600.0 {
            self.nudges_this_hour = 0;
            self.nudge_hour_start = now;
        }

        // 1. Weather poll
        if now - self.last_weather_poll >= self.config.weather_interval_secs as f64 {
            let weather_events = scan_weather(&self.config.weather);
            tracing::debug!(count = weather_events.len(), "Weather scan produced events");
            all_events.extend(weather_events);
            self.last_weather_poll = now;
        }

        // 2. News poll
        if now - self.last_news_poll >= self.config.news_interval_secs as f64 {
            let scan_result = news::scan_feeds(conn, &self.config.news);
            tracing::debug!(
                feeds = scan_result.feeds_fetched,
                relevant = scan_result.articles_relevant,
                "News scan complete"
            );
            all_events.extend(scan_result.events);
            self.last_news_poll = now;
        }

        // 3. Calendar check
        if now - self.last_calendar_poll >= self.config.calendar_interval_secs as f64 {
            let cal_events = analyze_calendar(&self.calendar_events, &self.config.calendar);
            tracing::debug!(count = cal_events.len(), "Calendar analysis produced events");
            all_events.extend(cal_events);
            self.last_calendar_poll = now;
        }

        // 4. Event discovery (daily)
        if now - self.last_events_poll >= self.config.events_interval_secs as f64 {
            let disc_events = scan_events(&self.config.events, conn);
            tracing::debug!(count = disc_events.len(), "Event discovery produced events");
            all_events.extend(disc_events);
            self.last_events_poll = now;
        }

        // 5. Feed all events through the graph bridge
        for event in &all_events {
            let result = GraphBridge::process_event(conn, event);
            if result.direct_activations > 0 {
                tracing::debug!(
                    event_kind = event.kind.as_str(),
                    direct = result.direct_activations,
                    propagated = result.propagated_activations,
                    nodes = ?result.activated_names,
                    "PWG activated"
                );
            }
        }

        // 6. Generate nudges from events
        let mut nudges: Vec<Nudge> = all_events.iter().flat_map(generate_nudges).collect();

        // 7. Salience check — find PWG nodes that crossed threshold
        if now - self.last_salience_check >= self.config.salience_check_interval_secs as f64 {
            WorldGraph::decay_all(conn, None);
            let salient = WorldGraph::get_salient(conn, self.config.nudge_salience_threshold, 5);
            for node in &salient {
                tracing::debug!(
                    node = %node.name,
                    salience = node.salience,
                    "High-salience PWG node"
                );
            }
            self.last_salience_check = now;
        }

        // 8. Morning brief check
        let current_hour = ((now % 86400.0) / 3600.0) as u8;
        let current_day = (now / 86400.0) as u32;
        if current_hour == self.config.morning_brief_hour && current_day != self.last_morning_brief_day {
            let brief = compose_morning_brief(&all_events, &self.config.user_name);
            nudges.push(Nudge {
                message: brief,
                category: crate::nudge_templates::NudgeCategory::General,
                importance: 0.8,
                reasoning: "Daily morning brief".into(),
                source_event_kind: "morning_brief".into(),
                actionable: false,
                suggested_action: None,
            });
            self.last_morning_brief_day = current_day;
        }

        // 9. Rate limit nudges
        let remaining_quota = self.config.max_nudges_per_hour.saturating_sub(self.nudges_this_hour) as usize;
        nudges.sort_by(|a, b| b.importance.partial_cmp(&a.importance).unwrap_or(std::cmp::Ordering::Equal));
        nudges.truncate(remaining_quota);
        self.nudges_this_hour += nudges.len() as u32;

        nudges
    }

    /// Update the calendar events cache (call when new data arrives from Google Calendar).
    pub fn update_calendar_events(&mut self, events: Vec<CalendarEvent>) {
        self.calendar_events = events;
    }

    /// Get pending nudges and clear the queue.
    pub fn drain_pending(&mut self) -> Vec<Nudge> {
        std::mem::take(&mut self.pending_nudges)
    }

    /// Force a specific connector to poll immediately.
    pub fn force_poll(&mut self, connector: &str) {
        match connector {
            "weather" => self.last_weather_poll = 0.0,
            "news" => self.last_news_poll = 0.0,
            "calendar" => self.last_calendar_poll = 0.0,
            "events" => self.last_events_poll = 0.0,
            _ => {}
        }
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_bridge::LifeEventKind;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        WorldGraph::ensure_tables(&conn);
        WorldGraph::seed_interests(&conn, &["Technology", "Finance"]);
        WorldGraph::seed_defaults(&conn);
        conn
    }

    #[test]
    fn stewardship_state_creation() {
        let config = StewardshipConfig::default();
        let state = StewardshipState::new(config);
        assert_eq!(state.last_weather_poll, 0.0);
        assert_eq!(state.nudges_this_hour, 0);
        assert!(state.pending_nudges.is_empty());
    }

    #[test]
    fn tick_with_no_connectors_configured() {
        let conn = setup_db();
        let config = StewardshipConfig {
            // No weather locations → weather scan returns empty
            weather: WeatherConfig::default(),
            // No event feeds → event scan returns empty
            events: EventDiscoveryConfig::default(),
            // No calendar events cached
            ..StewardshipConfig::default()
        };
        let mut state = StewardshipState::new(config);

        // First tick should run all polls (all timestamps are 0)
        let nudges = state.tick(&conn);
        // With no external data, we might get 0 nudges or morning brief
        assert!(nudges.len() <= 5);

        // Verify timestamps were updated
        assert!(state.last_weather_poll > 0.0);
        assert!(state.last_news_poll > 0.0);
        assert!(state.last_calendar_poll > 0.0);
    }

    #[test]
    fn rate_limiting() {
        let config = StewardshipConfig {
            max_nudges_per_hour: 3,
            ..StewardshipConfig::default()
        };
        let state = StewardshipState::new(config);
        assert_eq!(state.config.max_nudges_per_hour, 3);
    }

    #[test]
    fn force_poll_resets_timer() {
        let config = StewardshipConfig::default();
        let mut state = StewardshipState::new(config);
        state.last_weather_poll = 999999.0;
        state.force_poll("weather");
        assert_eq!(state.last_weather_poll, 0.0);
    }

    #[test]
    fn calendar_events_update() {
        let config = StewardshipConfig::default();
        let mut state = StewardshipState::new(config);
        assert!(state.calendar_events.is_empty());

        let events = vec![CalendarEvent {
            id: "test-1".into(),
            summary: "Test Meeting".into(),
            start_ts: now_ts() + 900.0,
            end_ts: now_ts() + 4500.0,
            all_day: false,
            location: String::new(),
            description: String::new(),
            attendees: vec![],
            recurring: false,
            source: "test".into(),
            status: crate::connectors::calendar::EventStatus::Confirmed,
        }];

        state.update_calendar_events(events);
        assert_eq!(state.calendar_events.len(), 1);
    }

    #[test]
    fn graph_bridge_integration() {
        let conn = setup_db();

        // Simulate a weather event flowing through the bridge
        let event = LifeEvent {
            kind: LifeEventKind::PrecipitationAlert,
            summary: "Rain expected at home".into(),
            keywords: vec!["rain".into(), "weather".into()],
            entities: vec!["home".into()],
            importance: 0.7,
            source: "weather".into(),
            data: serde_json::json!({
                "location": "home",
                "rain_hours": ["8:00"],
                "max_probability": 80.0,
            }),
            timestamp: now_ts(),
        };

        // Process through bridge
        let result = GraphBridge::process_event(&conn, &event);
        assert!(result.direct_activations > 0, "Should activate PWG nodes");

        // Generate nudge
        let nudges = generate_nudges(&event);
        assert!(!nudges.is_empty(), "Should generate rain nudge");
        assert!(nudges[0].message.contains("umbrella"));
    }

    #[test]
    fn end_to_end_oil_price_flow() {
        let conn = setup_db();

        // 1. News scanner detects oil price article → LifeEvent
        let event = LifeEvent {
            kind: LifeEventKind::PriceChange,
            summary: "Crude oil prices surge 8% after military strikes".into(),
            keywords: vec!["oil".into(), "crude".into(), "fuel".into(), "prices".into()],
            entities: vec![],
            importance: 0.7,
            source: "news:reuters".into(),
            data: serde_json::json!({}),
            timestamp: now_ts(),
        };

        // 2. Bridge processes event → activates PWG nodes
        let result = GraphBridge::process_event(&conn, &event);
        assert!(result.direct_activations > 0, "Should activate fuel-related nodes");

        // 3. Nudge template generates actionable message
        let nudges = generate_nudges(&event);
        assert!(!nudges.is_empty(), "Should generate price change nudge");
        let nudge = &nudges[0];
        assert!(nudge.message.contains("top up the car"), "Message: {}", nudge.message);
        assert!(nudge.actionable);

        // 4. PWG should have activated "fuel" node via propagation
        let salient = WorldGraph::get_salient(&conn, 0.1, 10);
        let fuel_activated = salient.iter().any(|n| n.name.to_lowercase().contains("fuel") || n.name.to_lowercase().contains("oil"));
        assert!(fuel_activated, "Fuel/oil PWG node should be activated. Salient: {:?}",
            salient.iter().map(|n| format!("{}={:.2}", n.name, n.salience)).collect::<Vec<_>>());
    }
}
