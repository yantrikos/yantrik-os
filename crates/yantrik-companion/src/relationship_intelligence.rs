//! Relationship Intelligence — per-person communication patterns and social memory.
//!
//! Builds a social memory layer on top of the world model's Person entities:
//! - Communication frequency per person
//! - Preferred channel (email, chat, call)
//! - Tone patterns (formal, casual, brief)
//! - Response time norms
//! - Relationship type (colleague, friend, family, acquaintance)
//! - Shared context / inside references
//!
//! Generates insights:
//! - "You usually send Alex concise updates"
//! - "You haven't replied to your sister in 9 days"
//! - "This person prefers calendar invites over email threads"
//!
//! Not surveillance — useful social memory that respects boundaries.

use std::collections::HashMap;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Person Profile ──────────────────────────────────────────────────────────

/// Social profile for a person in the user's network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonProfile {
    /// Unique identifier (name-based hash or explicit).
    pub person_id: String,
    /// Display name.
    pub name: String,
    /// Relationship type.
    pub relationship: RelationshipType,
    /// Preferred communication channel.
    pub preferred_channel: CommunicationChannel,
    /// Typical tone when communicating with this person.
    pub tone: CommunicationTone,
    /// Average response time from user to this person (hours).
    pub avg_response_hours: f64,
    /// Total communications tracked.
    pub total_communications: u32,
    /// Last communication timestamp.
    pub last_contact_at: Option<f64>,
    /// Shared context / notes.
    pub shared_context: Vec<String>,
    /// Relationship health score (0.0-1.0).
    pub health_score: f64,
    pub created_at: f64,
    pub updated_at: f64,
}

/// Type of relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationshipType {
    Family,
    CloseFriend,
    Friend,
    Colleague,
    Professional,
    Acquaintance,
    Unknown,
}

impl RelationshipType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Family => "family",
            Self::CloseFriend => "close_friend",
            Self::Friend => "friend",
            Self::Colleague => "colleague",
            Self::Professional => "professional",
            Self::Acquaintance => "acquaintance",
            Self::Unknown => "unknown",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "family" => Self::Family,
            "close_friend" => Self::CloseFriend,
            "friend" => Self::Friend,
            "colleague" => Self::Colleague,
            "professional" => Self::Professional,
            "acquaintance" => Self::Acquaintance,
            _ => Self::Unknown,
        }
    }

    /// Expected contact frequency in days.
    pub fn expected_contact_days(&self) -> f64 {
        match self {
            Self::Family => 7.0,
            Self::CloseFriend => 14.0,
            Self::Friend => 30.0,
            Self::Colleague => 5.0,
            Self::Professional => 30.0,
            Self::Acquaintance => 90.0,
            Self::Unknown => 90.0,
        }
    }
}

/// Preferred communication channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommunicationChannel {
    Email,
    Chat,
    WhatsApp,
    Phone,
    InPerson,
    Calendar,
    Unknown,
}

impl CommunicationChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Chat => "chat",
            Self::WhatsApp => "whatsapp",
            Self::Phone => "phone",
            Self::InPerson => "in_person",
            Self::Calendar => "calendar",
            Self::Unknown => "unknown",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "email" => Self::Email,
            "chat" => Self::Chat,
            "whatsapp" => Self::WhatsApp,
            "phone" => Self::Phone,
            "in_person" => Self::InPerson,
            "calendar" => Self::Calendar,
            _ => Self::Unknown,
        }
    }
}

/// Communication tone pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommunicationTone {
    /// "Hey! Quick update..."
    Casual,
    /// "Hi Alex, I wanted to let you know..."
    Warm,
    /// "Dear Mr. Smith, Please find attached..."
    Formal,
    /// "Update: done. Next: X."
    Brief,
    /// Not enough data yet.
    Unknown,
}

impl CommunicationTone {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Casual => "casual",
            Self::Warm => "warm",
            Self::Formal => "formal",
            Self::Brief => "brief",
            Self::Unknown => "unknown",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "casual" => Self::Casual,
            "warm" => Self::Warm,
            "formal" => Self::Formal,
            "brief" => Self::Brief,
            _ => Self::Unknown,
        }
    }
}

// ── Communication Event ─────────────────────────────────────────────────────

/// A tracked communication event.
#[derive(Debug, Clone)]
pub struct CommunicationEvent {
    pub person_id: String,
    pub channel: CommunicationChannel,
    /// Was this incoming or outgoing?
    pub direction: Direction,
    /// Response time in hours (if this was a reply).
    pub response_hours: Option<f64>,
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Direction {
    Incoming,
    Outgoing,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
        }
    }
}

// ── Relationship Intelligence Engine ────────────────────────────────────────

pub struct RelationshipIntelligence;

impl RelationshipIntelligence {
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS person_profiles (
                person_id       TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                relationship    TEXT NOT NULL DEFAULT 'unknown',
                preferred_channel TEXT NOT NULL DEFAULT 'unknown',
                tone            TEXT NOT NULL DEFAULT 'unknown',
                avg_response_hours REAL NOT NULL DEFAULT 0.0,
                total_communications INTEGER NOT NULL DEFAULT 0,
                last_contact_at REAL,
                shared_context  TEXT NOT NULL DEFAULT '[]',
                health_score    REAL NOT NULL DEFAULT 0.5,
                created_at      REAL NOT NULL,
                updated_at      REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_pp_name ON person_profiles(name);
            CREATE INDEX IF NOT EXISTS idx_pp_relationship ON person_profiles(relationship);
            CREATE INDEX IF NOT EXISTS idx_pp_health ON person_profiles(health_score);

            CREATE TABLE IF NOT EXISTS communication_events (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                person_id   TEXT NOT NULL,
                channel     TEXT NOT NULL,
                direction   TEXT NOT NULL,
                response_hours REAL,
                timestamp   REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ce_person ON communication_events(person_id);
            CREATE INDEX IF NOT EXISTS idx_ce_time ON communication_events(timestamp);",
        )
        .expect("failed to create relationship intelligence tables");
    }

    /// Create or update a person profile.
    pub fn upsert_person(conn: &Connection, profile: &PersonProfile) {
        let context = serde_json::to_string(&profile.shared_context).unwrap_or_default();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO person_profiles
             (person_id, name, relationship, preferred_channel, tone,
              avg_response_hours, total_communications, last_contact_at,
              shared_context, health_score, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                profile.person_id, profile.name, profile.relationship.as_str(),
                profile.preferred_channel.as_str(), profile.tone.as_str(),
                profile.avg_response_hours, profile.total_communications,
                profile.last_contact_at, context, profile.health_score,
                profile.created_at, profile.updated_at,
            ],
        );
    }

    /// Get a person profile by ID.
    pub fn get_person(conn: &Connection, person_id: &str) -> Option<PersonProfile> {
        conn.query_row(
            "SELECT person_id, name, relationship, preferred_channel, tone,
                    avg_response_hours, total_communications, last_contact_at,
                    shared_context, health_score, created_at, updated_at
             FROM person_profiles WHERE person_id = ?1",
            params![person_id],
            Self::row_to_profile,
        ).ok()
    }

    /// Find person by name (case-insensitive partial match).
    pub fn find_by_name(conn: &Connection, name: &str) -> Vec<PersonProfile> {
        let pattern = format!("%{}%", name.to_lowercase());
        let mut stmt = match conn.prepare(
            "SELECT person_id, name, relationship, preferred_channel, tone,
                    avg_response_hours, total_communications, last_contact_at,
                    shared_context, health_score, created_at, updated_at
             FROM person_profiles WHERE LOWER(name) LIKE ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![pattern], Self::row_to_profile)
            .ok()
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Record a communication event.
    pub fn record_communication(conn: &Connection, event: &CommunicationEvent) {
        let _ = conn.execute(
            "INSERT INTO communication_events (person_id, channel, direction, response_hours, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.person_id, event.channel.as_str(),
                event.direction.as_str(), event.response_hours,
                event.timestamp,
            ],
        );

        // Update person profile
        Self::update_from_event(conn, event);

        // Feed brain: record entity event for pattern break detection
        let entity_id = format!("contact:{}", event.person_id);
        crate::brain_loop::record_entity_event(conn, &entity_id, event.channel.as_str());

        // Feed brain: record response time metric for baseline deviation
        if let Some(hours) = event.response_hours {
            crate::brain_loop::record_metric(
                conn, &entity_id, "response_hours", hours,
            );
        }
    }

    /// Update person profile from a communication event.
    fn update_from_event(conn: &Connection, event: &CommunicationEvent) {
        let now = event.timestamp;

        // Update total communications and last contact
        let _ = conn.execute(
            "UPDATE person_profiles SET
                total_communications = total_communications + 1,
                last_contact_at = MAX(COALESCE(last_contact_at, 0), ?1),
                updated_at = ?1
             WHERE person_id = ?2",
            params![now, event.person_id],
        );

        // Update average response time if this was a reply
        if let Some(hours) = event.response_hours {
            if let Some(profile) = Self::get_person(conn, &event.person_id) {
                let n = profile.total_communications.max(1) as f64;
                // Exponential moving average
                let alpha = 2.0 / (n + 1.0);
                let new_avg = alpha * hours + (1.0 - alpha) * profile.avg_response_hours;
                let _ = conn.execute(
                    "UPDATE person_profiles SET avg_response_hours = ?1, updated_at = ?2
                     WHERE person_id = ?3",
                    params![new_avg, now, event.person_id],
                );
            }
        }

        // Update channel preference (most used channel)
        Self::update_preferred_channel(conn, &event.person_id);

        // Update health score
        Self::update_health(conn, &event.person_id);
    }

    /// Update preferred channel based on most frequent channel.
    fn update_preferred_channel(conn: &Connection, person_id: &str) {
        if let Ok(channel) = conn.query_row(
            "SELECT channel FROM communication_events
             WHERE person_id = ?1
             GROUP BY channel ORDER BY COUNT(*) DESC LIMIT 1",
            params![person_id],
            |row| row.get::<_, String>(0),
        ) {
            let _ = conn.execute(
                "UPDATE person_profiles SET preferred_channel = ?1 WHERE person_id = ?2",
                params![channel, person_id],
            );
        }
    }

    /// Update relationship health score.
    fn update_health(conn: &Connection, person_id: &str) {
        let profile = match Self::get_person(conn, person_id) {
            Some(p) => p,
            None => return,
        };

        let now = now_ts();
        let days_since = profile.last_contact_at
            .map(|t| (now - t) / 86400.0)
            .unwrap_or(999.0);

        let expected_days = profile.relationship.expected_contact_days();

        // Health: 1.0 if recently contacted, decays toward 0 as days_since grows
        let ratio = days_since / expected_days;
        let health = if ratio <= 1.0 {
            1.0
        } else if ratio <= 2.0 {
            1.0 - (ratio - 1.0) * 0.3 // 70% at 2x expected
        } else if ratio <= 4.0 {
            0.7 - (ratio - 2.0) * 0.15 // 40% at 4x expected
        } else {
            (0.4 - (ratio - 4.0) * 0.05).max(0.05)
        };

        let _ = conn.execute(
            "UPDATE person_profiles SET health_score = ?1, updated_at = ?2 WHERE person_id = ?3",
            params![health, now, person_id],
        );
    }

    /// Get people with low relationship health (need attention).
    pub fn needing_attention(conn: &Connection, threshold: f64) -> Vec<PersonProfile> {
        let mut stmt = match conn.prepare(
            "SELECT person_id, name, relationship, preferred_channel, tone,
                    avg_response_hours, total_communications, last_contact_at,
                    shared_context, health_score, created_at, updated_at
             FROM person_profiles
             WHERE health_score < ?1 AND relationship NOT IN ('unknown', 'acquaintance')
             ORDER BY health_score ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![threshold], Self::row_to_profile)
            .ok()
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Generate a relationship insight for a person.
    pub fn insight_for(conn: &Connection, person_id: &str) -> Option<String> {
        let profile = Self::get_person(conn, person_id)?;
        let now = now_ts();

        let days_since = profile.last_contact_at
            .map(|t| (now - t) / 86400.0)
            .unwrap_or(999.0);
        let expected = profile.relationship.expected_contact_days();

        if days_since > expected * 1.5 {
            Some(format!(
                "You haven't been in touch with {} in {:.0} days (usually every {:.0} days)",
                profile.name, days_since, expected,
            ))
        } else if profile.avg_response_hours > 48.0 && profile.total_communications >= 5 {
            Some(format!(
                "Your average response time to {} is {:.0} hours",
                profile.name, profile.avg_response_hours,
            ))
        } else {
            None
        }
    }

    /// Get all people with their profiles.
    pub fn all_people(conn: &Connection) -> Vec<PersonProfile> {
        let mut stmt = match conn.prepare(
            "SELECT person_id, name, relationship, preferred_channel, tone,
                    avg_response_hours, total_communications, last_contact_at,
                    shared_context, health_score, created_at, updated_at
             FROM person_profiles ORDER BY last_contact_at DESC NULLS LAST",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map([], Self::row_to_profile)
            .ok()
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Add shared context to a person.
    pub fn add_shared_context(conn: &Connection, person_id: &str, context: &str) {
        if let Some(mut profile) = Self::get_person(conn, person_id) {
            if !profile.shared_context.contains(&context.to_string()) {
                profile.shared_context.push(context.to_string());
                // Keep last 20 shared contexts
                if profile.shared_context.len() > 20 {
                    profile.shared_context.remove(0);
                }
                let json = serde_json::to_string(&profile.shared_context).unwrap_or_default();
                let _ = conn.execute(
                    "UPDATE person_profiles SET shared_context = ?1, updated_at = ?2 WHERE person_id = ?3",
                    params![json, now_ts(), person_id],
                );
            }
        }
    }

    fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<PersonProfile> {
        let context_json: String = row.get(8)?;
        Ok(PersonProfile {
            person_id: row.get(0)?,
            name: row.get(1)?,
            relationship: RelationshipType::from_str(&row.get::<_, String>(2)?),
            preferred_channel: CommunicationChannel::from_str(&row.get::<_, String>(3)?),
            tone: CommunicationTone::from_str(&row.get::<_, String>(4)?),
            avg_response_hours: row.get(5)?,
            total_communications: row.get::<_, i32>(6)? as u32,
            last_contact_at: row.get(7)?,
            shared_context: serde_json::from_str(&context_json).unwrap_or_default(),
            health_score: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        RelationshipIntelligence::ensure_table(&conn);
        conn
    }

    fn make_profile(id: &str, name: &str, rel: RelationshipType) -> PersonProfile {
        let now = now_ts();
        PersonProfile {
            person_id: id.into(),
            name: name.into(),
            relationship: rel,
            preferred_channel: CommunicationChannel::Email,
            tone: CommunicationTone::Unknown,
            avg_response_hours: 0.0,
            total_communications: 0,
            last_contact_at: Some(now),
            shared_context: Vec::new(),
            health_score: 0.5,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn create_and_get_person() {
        let conn = setup();
        let profile = make_profile("p1", "Alice", RelationshipType::Colleague);
        RelationshipIntelligence::upsert_person(&conn, &profile);

        let loaded = RelationshipIntelligence::get_person(&conn, "p1").unwrap();
        assert_eq!(loaded.name, "Alice");
        assert_eq!(loaded.relationship, RelationshipType::Colleague);
    }

    #[test]
    fn communication_tracking() {
        let conn = setup();
        let profile = make_profile("p1", "Alice", RelationshipType::Colleague);
        RelationshipIntelligence::upsert_person(&conn, &profile);

        // Record 3 email communications
        for i in 0..3 {
            RelationshipIntelligence::record_communication(&conn, &CommunicationEvent {
                person_id: "p1".into(),
                channel: CommunicationChannel::Email,
                direction: Direction::Outgoing,
                response_hours: Some(2.0 + i as f64),
                timestamp: now_ts(),
            });
        }

        let loaded = RelationshipIntelligence::get_person(&conn, "p1").unwrap();
        assert_eq!(loaded.total_communications, 3);
        assert!(loaded.avg_response_hours > 0.0);
        assert_eq!(loaded.preferred_channel, CommunicationChannel::Email);
    }

    #[test]
    fn find_by_name_partial() {
        let conn = setup();
        RelationshipIntelligence::upsert_person(&conn,
            &make_profile("p1", "Alice Smith", RelationshipType::Friend));
        RelationshipIntelligence::upsert_person(&conn,
            &make_profile("p2", "Bob Jones", RelationshipType::Colleague));

        let results = RelationshipIntelligence::find_by_name(&conn, "alice");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Alice Smith");

        let results = RelationshipIntelligence::find_by_name(&conn, "jones");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn health_decays_with_no_contact() {
        let conn = setup();

        // Create person with old last contact
        let old_time = now_ts() - 30.0 * 86400.0; // 30 days ago
        let mut profile = make_profile("p1", "Alice", RelationshipType::Colleague);
        profile.last_contact_at = Some(old_time);
        RelationshipIntelligence::upsert_person(&conn, &profile);

        // Trigger health update
        RelationshipIntelligence::update_health(&conn, "p1");

        let loaded = RelationshipIntelligence::get_person(&conn, "p1").unwrap();
        // Colleague expected every 5 days, 30 days = 6x overdue
        assert!(loaded.health_score < 0.5,
            "Health should be low for 30-day no-contact colleague: {}", loaded.health_score);
    }

    #[test]
    fn needing_attention() {
        let conn = setup();

        // Active colleague (recent contact)
        let active = make_profile("p1", "Active Alice", RelationshipType::Colleague);
        RelationshipIntelligence::upsert_person(&conn, &active);

        // Neglected family member (old contact)
        let mut neglected = make_profile("p2", "Neglected Nancy", RelationshipType::Family);
        neglected.last_contact_at = Some(now_ts() - 60.0 * 86400.0);
        neglected.health_score = 0.2;
        RelationshipIntelligence::upsert_person(&conn, &neglected);

        let needs_attention = RelationshipIntelligence::needing_attention(&conn, 0.4);
        assert_eq!(needs_attention.len(), 1);
        assert_eq!(needs_attention[0].name, "Neglected Nancy");
    }

    #[test]
    fn shared_context() {
        let conn = setup();
        let profile = make_profile("p1", "Alice", RelationshipType::Friend);
        RelationshipIntelligence::upsert_person(&conn, &profile);

        RelationshipIntelligence::add_shared_context(&conn, "p1", "Working on Project Alpha together");
        RelationshipIntelligence::add_shared_context(&conn, "p1", "Met at RustConf 2025");

        let loaded = RelationshipIntelligence::get_person(&conn, "p1").unwrap();
        assert_eq!(loaded.shared_context.len(), 2);
        assert!(loaded.shared_context.contains(&"Met at RustConf 2025".to_string()));
    }

    #[test]
    fn insight_generation() {
        let conn = setup();

        // Person with old contact
        let mut profile = make_profile("p1", "Mom", RelationshipType::Family);
        profile.last_contact_at = Some(now_ts() - 15.0 * 86400.0); // 15 days, family expects 7
        RelationshipIntelligence::upsert_person(&conn, &profile);

        let insight = RelationshipIntelligence::insight_for(&conn, "p1");
        assert!(insight.is_some());
        assert!(insight.unwrap().contains("Mom"));
    }

    #[test]
    fn channel_preference_updates() {
        let conn = setup();
        let profile = make_profile("p1", "Alice", RelationshipType::Colleague);
        RelationshipIntelligence::upsert_person(&conn, &profile);

        // 3 WhatsApp messages, 1 email
        for _ in 0..3 {
            RelationshipIntelligence::record_communication(&conn, &CommunicationEvent {
                person_id: "p1".into(),
                channel: CommunicationChannel::WhatsApp,
                direction: Direction::Outgoing,
                response_hours: None,
                timestamp: now_ts(),
            });
        }
        RelationshipIntelligence::record_communication(&conn, &CommunicationEvent {
            person_id: "p1".into(),
            channel: CommunicationChannel::Email,
            direction: Direction::Outgoing,
            response_hours: None,
            timestamp: now_ts(),
        });

        let loaded = RelationshipIntelligence::get_person(&conn, "p1").unwrap();
        assert_eq!(loaded.preferred_channel, CommunicationChannel::WhatsApp);
    }
}
