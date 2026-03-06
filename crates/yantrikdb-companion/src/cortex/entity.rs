//! Entity resolution — canonical cross-system identities.
//!
//! Maps raw identifiers from different systems to canonical entity IDs.
//! Uses 4-tier matching: exact → fuzzy → pattern → LLM (disabled by default).

use rusqlite::Connection;

use super::pulse::Pulse;
use super::schema;

// ── Core Types ───────────────────────────────────────────────────────

/// Systems that produce entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemSource {
    Jira,
    Git,
    Email,
    Calendar,
    FileSystem,
    Browser,
    Memory,
}

impl SystemSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jira => "jira",
            Self::Git => "git",
            Self::Email => "email",
            Self::Calendar => "calendar",
            Self::FileSystem => "filesystem",
            Self::Browser => "browser",
            Self::Memory => "memory",
        }
    }
}

/// Types of entities the cortex tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityType {
    Person,
    Ticket,
    File,
    Meeting,
    Project,
    Commit,
    Email,
    Branch,
    Interest,
    Event,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Ticket => "ticket",
            Self::File => "file",
            Self::Meeting => "meeting",
            Self::Project => "project",
            Self::Commit => "commit",
            Self::Email => "email",
            Self::Branch => "branch",
            Self::Interest => "interest",
            Self::Event => "event",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "person" => Some(Self::Person),
            "ticket" => Some(Self::Ticket),
            "file" => Some(Self::File),
            "meeting" => Some(Self::Meeting),
            "project" => Some(Self::Project),
            "commit" => Some(Self::Commit),
            "email" => Some(Self::Email),
            "branch" => Some(Self::Branch),
            "interest" => Some(Self::Interest),
            "event" => Some(Self::Event),
            _ => None,
        }
    }
}

/// A canonical entity with cross-system identity.
#[derive(Debug, Clone)]
pub struct CanonicalEntity {
    pub id: String,
    pub display_name: String,
    pub entity_type: EntityType,
    pub aliases: Vec<(SystemSource, String)>,
    pub relevance: f64,
}

/// A resolved entity reference from a pulse.
#[derive(Debug, Clone)]
pub struct ResolvedEntity {
    pub entity_id: String,
    pub role: String, // "actor", "target", "context"
}

// ── Entity Resolver ──────────────────────────────────────────────────

/// Resolves raw entity references to canonical entity IDs.
///
/// 4-tier strategy:
/// 1. **Exact match** — entity hint already IS the canonical ID (e.g., "ticket:YOS-142")
/// 2. **Alias lookup** — find entity by system-specific identifier
/// 3. **Fuzzy name match** — normalized name comparison
/// 4. **Create new** — if no match found, create a new entity
pub struct EntityResolver;

impl EntityResolver {
    pub fn new() -> Self {
        Self
    }

    /// Resolve all entity references from a pulse.
    pub fn resolve_pulse_entities(
        &self,
        conn: &Connection,
        pulse: &Pulse,
    ) -> Vec<ResolvedEntity> {
        pulse
            .entity_refs
            .iter()
            .filter_map(|(hint, role)| {
                self.resolve_single(conn, hint, &pulse.source)
                    .map(|entity_id| ResolvedEntity {
                        entity_id,
                        role: role.clone(),
                    })
            })
            .collect()
    }

    /// Resolve a single entity hint to a canonical entity ID.
    fn resolve_single(
        &self,
        conn: &Connection,
        hint: &str,
        source: &SystemSource,
    ) -> Option<String> {
        // Parse hint format: "type:identifier"
        let (entity_type, identifier) = parse_entity_hint(hint)?;

        // Tier 1: Direct canonical ID lookup
        let canonical_id = format!("{}:{}", entity_type.as_str(), normalize_id(identifier));
        if entity_exists(conn, &canonical_id) {
            // Boost relevance on re-encounter
            schema::boost_relevance(conn, &canonical_id, 0.05);
            return Some(canonical_id);
        }

        // Tier 2: Alias lookup
        if let Some(found) = schema::find_entity_by_alias(conn, *source, identifier) {
            schema::boost_relevance(conn, &found, 0.05);
            return Some(found);
        }

        // Tier 3: Fuzzy name match (for person entities)
        if entity_type == EntityType::Person {
            if let Some(found) = schema::find_entity_by_name(conn, EntityType::Person, identifier) {
                schema::boost_relevance(conn, &found, 0.05);
                return Some(found);
            }
        }

        // Tier 4: Create new entity
        let display_name = prettify_identifier(identifier, entity_type);
        let _ = schema::upsert_entity(
            conn,
            &canonical_id,
            &display_name,
            entity_type,
            *source,
            identifier,
        );
        Some(canonical_id)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Parse "type:identifier" hint into (EntityType, identifier).
fn parse_entity_hint(hint: &str) -> Option<(EntityType, &str)> {
    let colon = hint.find(':')?;
    let type_str = &hint[..colon];
    let identifier = &hint[colon + 1..];
    if identifier.is_empty() {
        return None;
    }
    let entity_type = EntityType::from_str(type_str)?;
    Some((entity_type, identifier))
}

/// Check if an entity exists in the database.
fn entity_exists(conn: &Connection, id: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM cortex_entities WHERE id = ?1",
        rusqlite::params![id],
        |_| Ok(()),
    )
    .is_ok()
}

/// Normalize an identifier for use as canonical ID component.
fn normalize_id(id: &str) -> String {
    id.trim()
        .to_lowercase()
        .replace(' ', "-")
        .replace(['<', '>', '"', '\'', '(', ')'], "")
}

/// Create a human-readable display name from an identifier.
fn prettify_identifier(identifier: &str, entity_type: EntityType) -> String {
    match entity_type {
        EntityType::Ticket => identifier.to_uppercase(),
        EntityType::Person => {
            // "sarah-chen" → "Sarah Chen", "sarah@co.com" → "sarah@co.com"
            if identifier.contains('@') {
                identifier.to_string()
            } else {
                identifier
                    .split('-')
                    .map(|word| {
                        let mut chars = word.chars();
                        match chars.next() {
                            None => String::new(),
                            Some(c) => {
                                c.to_uppercase().to_string() + &chars.collect::<String>()
                            }
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        }
        EntityType::File => {
            // Show just the filename, not full path
            identifier
                .rsplit('/')
                .next()
                .unwrap_or(identifier)
                .to_string()
        }
        _ => identifier.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_entity_hint() {
        let (t, id) = parse_entity_hint("ticket:YOS-142").unwrap();
        assert_eq!(t, EntityType::Ticket);
        assert_eq!(id, "YOS-142");

        let (t, id) = parse_entity_hint("person:sarah@co.com").unwrap();
        assert_eq!(t, EntityType::Person);
        assert_eq!(id, "sarah@co.com");

        assert!(parse_entity_hint("invalid").is_none());
        assert!(parse_entity_hint("ticket:").is_none());
    }

    #[test]
    fn test_prettify() {
        assert_eq!(prettify_identifier("YOS-142", EntityType::Ticket), "YOS-142");
        assert_eq!(prettify_identifier("sarah-chen", EntityType::Person), "Sarah Chen");
        assert_eq!(prettify_identifier("sarah@co.com", EntityType::Person), "sarah@co.com");
        assert_eq!(prettify_identifier("src/auth/mod.rs", EntityType::File), "mod.rs");
    }

    #[test]
    fn test_normalize_id() {
        assert_eq!(normalize_id("Sarah Chen"), "sarah-chen");
        assert_eq!(normalize_id("YOS-142"), "yos-142");
    }
}
