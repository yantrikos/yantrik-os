//! Personal World Graph (PWG) — a living graph of the user's life.
//!
//! The PWG is the core data structure that makes proactive intelligence possible.
//! Unlike the flat world model (commitments, preferences, routines), the graph
//! captures **relationships between entities** and enables **salience propagation**:
//!
//! When news about oil prices arrives, activation spreads:
//!   Interest:finance → Resource:fuel → Commitment:commute → Person:self
//!
//! This allows the system to connect dots across domains without LLM inference.
//!
//! ## Entity Types
//! - **Person**: people in the user's life with relationship metadata
//! - **Commitment**: deadlines, promises, obligations
//! - **Project**: active workstreams with momentum tracking
//! - **Resource**: money, energy, subscriptions, possessions
//! - **Rhythm**: repeating patterns (wake time, work blocks, exercise)
//! - **Risk**: potential problems (missed deadline, burnout, overspend)
//! - **Preference**: user preferences with domain scope
//! - **Interest**: user interests that drive content scanning
//! - **Place**: locations (home, work, favorite spots)
//!
//! ## Salience System
//! Each node has a salience score (0.0–1.0) that:
//! - **Activates** when relevant events occur
//! - **Propagates** through edges to related nodes
//! - **Decays** over time (configurable half-life, default 4 hours)
//!
//! The result: at any moment, `get_salient()` returns what currently matters
//! in the user's life — without any LLM reasoning.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Entity Types ─────────────────────────────────────────────────────

/// The type of entity in the Personal World Graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityType {
    Person,
    Commitment,
    Project,
    Resource,
    Rhythm,
    Risk,
    Preference,
    Interest,
    Place,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Commitment => "commitment",
            Self::Project => "project",
            Self::Resource => "resource",
            Self::Rhythm => "rhythm",
            Self::Risk => "risk",
            Self::Preference => "preference",
            Self::Interest => "interest",
            Self::Place => "place",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "person" => Some(Self::Person),
            "commitment" => Some(Self::Commitment),
            "project" => Some(Self::Project),
            "resource" => Some(Self::Resource),
            "rhythm" => Some(Self::Rhythm),
            "risk" => Some(Self::Risk),
            "preference" => Some(Self::Preference),
            "interest" => Some(Self::Interest),
            "place" => Some(Self::Place),
            _ => None,
        }
    }
}

/// Relationship type between two entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationType {
    /// Entity belongs to another (file belongs to project).
    BelongsTo,
    /// General association.
    RelatedTo,
    /// Entity is important for another (person important for commitment).
    ImportantFor,
    /// Entity depends on another (project depends on resource).
    DependsOn,
    /// Entity impacts another (news impacts resource).
    Impacts,
    /// Communication relationship (person communicates with person).
    CommunicatesWith,
    /// Entity is located at place.
    LocatedAt,
    /// Parent-child for interest hierarchy (tech → AI, tech → startups).
    ChildOf,
}

impl RelationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BelongsTo => "belongs_to",
            Self::RelatedTo => "related_to",
            Self::ImportantFor => "important_for",
            Self::DependsOn => "depends_on",
            Self::Impacts => "impacts",
            Self::CommunicatesWith => "communicates_with",
            Self::LocatedAt => "located_at",
            Self::ChildOf => "child_of",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "belongs_to" => Some(Self::BelongsTo),
            "related_to" => Some(Self::RelatedTo),
            "important_for" => Some(Self::ImportantFor),
            "depends_on" => Some(Self::DependsOn),
            "impacts" => Some(Self::Impacts),
            "communicates_with" => Some(Self::CommunicatesWith),
            "located_at" => Some(Self::LocatedAt),
            "child_of" => Some(Self::ChildOf),
            _ => None,
        }
    }

    /// Default propagation factor for this relationship type.
    /// Higher = activation spreads more strongly through this edge.
    pub fn default_propagation(&self) -> f64 {
        match self {
            Self::Impacts => 0.7,         // strong: news impacts resources
            Self::ImportantFor => 0.6,     // strong: person important for commitment
            Self::DependsOn => 0.5,        // moderate: project depends on resource
            Self::BelongsTo => 0.4,        // moderate: file belongs to project
            Self::CommunicatesWith => 0.3, // weaker: person-to-person
            Self::RelatedTo => 0.3,        // general association
            Self::ChildOf => 0.5,          // parent-child interests
            Self::LocatedAt => 0.2,        // weak: place association
        }
    }
}

// ── Graph Node ───────────────────────────────────────────────────────

/// A node in the Personal World Graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: i64,
    pub entity_type: EntityType,
    /// Display name (e.g., "Sarah Chen", "Q3 Board Deck", "Technology").
    pub name: String,
    /// Structured metadata as JSON (varies by entity type).
    pub metadata: serde_json::Value,
    /// Current salience score (0.0–1.0). Higher = more currently relevant.
    pub salience: f64,
    /// When this node was last activated (Unix timestamp).
    pub last_activated: f64,
    /// Confidence in this entity's existence/relevance (0.0–1.0).
    pub confidence: f64,
    /// Where this entity was discovered from.
    pub provenance: String,
    /// Search keywords for matching against events/content.
    pub keywords: Vec<String>,
    pub created_at: f64,
    pub updated_at: f64,
}

/// An edge between two nodes in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: i64,
    pub source_id: i64,
    pub target_id: i64,
    pub relation: RelationType,
    /// Strength of this relationship (0.0–1.0).
    pub weight: f64,
    /// How strongly activation propagates through this edge.
    pub propagation_factor: f64,
    /// Confidence in this relationship.
    pub confidence: f64,
    pub metadata: serde_json::Value,
    pub created_at: f64,
}

/// An activation event — records why a node was activated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivationRecord {
    pub node_id: i64,
    /// What triggered this activation.
    pub trigger: String,
    /// How much salience was added.
    pub delta: f64,
    /// Was this a direct activation or propagated?
    pub propagated: bool,
    pub timestamp: f64,
}

// ── Personal World Graph Engine ──────────────────────────────────────

/// Default salience half-life in seconds (4 hours).
const DEFAULT_HALF_LIFE_SECS: f64 = 4.0 * 3600.0;

/// Maximum propagation depth (prevent infinite loops).
const MAX_PROPAGATION_DEPTH: u32 = 3;

/// Minimum salience delta to continue propagating.
const MIN_PROPAGATION_DELTA: f64 = 0.01;

/// The Personal World Graph engine.
pub struct WorldGraph;

impl WorldGraph {
    /// Create all PWG tables.
    pub fn ensure_tables(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pwg_nodes (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                entity_type     TEXT NOT NULL,
                name            TEXT NOT NULL,
                metadata        TEXT NOT NULL DEFAULT '{}',
                salience        REAL NOT NULL DEFAULT 0.0,
                last_activated  REAL NOT NULL DEFAULT 0.0,
                confidence      REAL NOT NULL DEFAULT 0.5,
                provenance      TEXT NOT NULL DEFAULT 'system',
                keywords        TEXT NOT NULL DEFAULT '[]',
                created_at      REAL NOT NULL,
                updated_at      REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_pwg_nodes_type ON pwg_nodes(entity_type);
            CREATE INDEX IF NOT EXISTS idx_pwg_nodes_salience ON pwg_nodes(salience DESC);
            CREATE INDEX IF NOT EXISTS idx_pwg_nodes_name ON pwg_nodes(name);

            CREATE TABLE IF NOT EXISTS pwg_edges (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                source_id           INTEGER NOT NULL REFERENCES pwg_nodes(id),
                target_id           INTEGER NOT NULL REFERENCES pwg_nodes(id),
                relation            TEXT NOT NULL,
                weight              REAL NOT NULL DEFAULT 0.5,
                propagation_factor  REAL NOT NULL DEFAULT 0.3,
                confidence          REAL NOT NULL DEFAULT 0.5,
                metadata            TEXT NOT NULL DEFAULT '{}',
                created_at          REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_pwg_edges_source ON pwg_edges(source_id);
            CREATE INDEX IF NOT EXISTS idx_pwg_edges_target ON pwg_edges(target_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_pwg_edges_unique
                ON pwg_edges(source_id, target_id, relation);

            CREATE TABLE IF NOT EXISTS pwg_activations (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                node_id     INTEGER NOT NULL REFERENCES pwg_nodes(id),
                trigger_src TEXT NOT NULL,
                delta       REAL NOT NULL,
                propagated  INTEGER NOT NULL DEFAULT 0,
                timestamp   REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_pwg_act_node ON pwg_activations(node_id);
            CREATE INDEX IF NOT EXISTS idx_pwg_act_time ON pwg_activations(timestamp DESC);",
        )
        .expect("failed to create PWG tables");
    }

    // ── Node CRUD ────────────────────────────────────────────────────

    /// Insert a new node, returning its ID.
    pub fn insert_node(conn: &Connection, node: &GraphNode) -> i64 {
        let keywords_json = serde_json::to_string(&node.keywords).unwrap_or_default();
        let meta_str = serde_json::to_string(&node.metadata).unwrap_or_default();
        conn.execute(
            "INSERT INTO pwg_nodes
             (entity_type, name, metadata, salience, last_activated, confidence,
              provenance, keywords, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                node.entity_type.as_str(),
                node.name,
                meta_str,
                node.salience,
                node.last_activated,
                node.confidence,
                node.provenance,
                keywords_json,
                node.created_at,
                node.updated_at,
            ],
        )
        .expect("failed to insert PWG node");
        conn.last_insert_rowid()
    }

    /// Find a node by type and name.
    pub fn find_node(conn: &Connection, entity_type: EntityType, name: &str) -> Option<GraphNode> {
        conn.query_row(
            "SELECT id, entity_type, name, metadata, salience, last_activated,
                    confidence, provenance, keywords, created_at, updated_at
             FROM pwg_nodes WHERE entity_type = ?1 AND name = ?2",
            params![entity_type.as_str(), name],
            |row| Self::row_to_node(row),
        )
        .ok()
    }

    /// Find a node by ID.
    pub fn get_node(conn: &Connection, id: i64) -> Option<GraphNode> {
        conn.query_row(
            "SELECT id, entity_type, name, metadata, salience, last_activated,
                    confidence, provenance, keywords, created_at, updated_at
             FROM pwg_nodes WHERE id = ?1",
            params![id],
            |row| Self::row_to_node(row),
        )
        .ok()
    }

    /// Find nodes by type.
    pub fn nodes_by_type(conn: &Connection, entity_type: EntityType) -> Vec<GraphNode> {
        conn.prepare(
            "SELECT id, entity_type, name, metadata, salience, last_activated,
                    confidence, provenance, keywords, created_at, updated_at
             FROM pwg_nodes WHERE entity_type = ?1
             ORDER BY salience DESC",
        )
        .and_then(|mut stmt| {
            stmt.query_map(params![entity_type.as_str()], |row| Self::row_to_node(row))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    }

    /// Find or create a node. Returns (id, was_created).
    pub fn find_or_create(
        conn: &Connection,
        entity_type: EntityType,
        name: &str,
        provenance: &str,
        keywords: &[&str],
    ) -> (i64, bool) {
        if let Some(existing) = Self::find_node(conn, entity_type, name) {
            return (existing.id, false);
        }
        let now = now_ts();
        let node = GraphNode {
            id: 0,
            entity_type,
            name: name.to_string(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            salience: 0.0,
            last_activated: 0.0,
            confidence: 0.5,
            provenance: provenance.to_string(),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            created_at: now,
            updated_at: now,
        };
        let id = Self::insert_node(conn, &node);
        (id, true)
    }

    /// Update node metadata.
    pub fn update_metadata(conn: &Connection, id: i64, metadata: &serde_json::Value) {
        let now = now_ts();
        let meta_str = serde_json::to_string(metadata).unwrap_or_default();
        let _ = conn.execute(
            "UPDATE pwg_nodes SET metadata = ?1, updated_at = ?2 WHERE id = ?3",
            params![meta_str, now, id],
        );
    }

    /// Search nodes by keyword match.
    pub fn search_nodes(conn: &Connection, query: &str) -> Vec<GraphNode> {
        let pattern = format!("%{}%", query.to_lowercase());
        conn.prepare(
            "SELECT id, entity_type, name, metadata, salience, last_activated,
                    confidence, provenance, keywords, created_at, updated_at
             FROM pwg_nodes
             WHERE LOWER(name) LIKE ?1 OR LOWER(keywords) LIKE ?1
             ORDER BY salience DESC
             LIMIT 20",
        )
        .and_then(|mut stmt| {
            stmt.query_map(params![pattern], |row| Self::row_to_node(row))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    }

    // ── Edge CRUD ────────────────────────────────────────────────────

    /// Insert or update an edge between two nodes.
    pub fn upsert_edge(conn: &Connection, edge: &GraphEdge) {
        let meta_str = serde_json::to_string(&edge.metadata).unwrap_or_default();
        let _ = conn.execute(
            "INSERT INTO pwg_edges
             (source_id, target_id, relation, weight, propagation_factor,
              confidence, metadata, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(source_id, target_id, relation) DO UPDATE SET
                weight = ?4, propagation_factor = ?5, confidence = ?6,
                metadata = ?7",
            params![
                edge.source_id,
                edge.target_id,
                edge.relation.as_str(),
                edge.weight,
                edge.propagation_factor,
                edge.confidence,
                meta_str,
                edge.created_at,
            ],
        );
    }

    /// Get all edges from a node (outgoing).
    pub fn edges_from(conn: &Connection, node_id: i64) -> Vec<GraphEdge> {
        Self::query_edges(conn, "WHERE source_id = ?1", params![node_id])
    }

    /// Get all edges to a node (incoming).
    pub fn edges_to(conn: &Connection, node_id: i64) -> Vec<GraphEdge> {
        Self::query_edges(conn, "WHERE target_id = ?1", params![node_id])
    }

    /// Get all edges for a node (both directions).
    pub fn edges_for(conn: &Connection, node_id: i64) -> Vec<GraphEdge> {
        Self::query_edges(
            conn,
            "WHERE source_id = ?1 OR target_id = ?1",
            params![node_id],
        )
    }

    /// Get related nodes (neighbors) with their edges.
    pub fn related_nodes(conn: &Connection, node_id: i64) -> Vec<(GraphNode, GraphEdge)> {
        let edges = Self::edges_for(conn, node_id);
        let mut result = Vec::new();
        for edge in edges {
            let other_id = if edge.source_id == node_id {
                edge.target_id
            } else {
                edge.source_id
            };
            if let Some(node) = Self::get_node(conn, other_id) {
                result.push((node, edge));
            }
        }
        result
    }

    // ── Salience & Activation ────────────────────────────────────────

    /// Activate a node: boost its salience and propagate to neighbors.
    /// Returns the list of all nodes that were activated (direct + propagated).
    pub fn activate(
        conn: &Connection,
        node_id: i64,
        delta: f64,
        trigger: &str,
    ) -> Vec<ActivationRecord> {
        let mut records = Vec::new();
        Self::activate_recursive(conn, node_id, delta, trigger, 0, &mut records);
        records
    }

    fn activate_recursive(
        conn: &Connection,
        node_id: i64,
        delta: f64,
        trigger: &str,
        depth: u32,
        records: &mut Vec<ActivationRecord>,
    ) {
        if depth > MAX_PROPAGATION_DEPTH || delta < MIN_PROPAGATION_DELTA {
            return;
        }

        // Avoid re-activating the same node in this cascade
        if records.iter().any(|r| r.node_id == node_id) {
            return;
        }

        let now = now_ts();
        let propagated = depth > 0;

        // Clamp salience to [0.0, 1.0]
        let _ = conn.execute(
            "UPDATE pwg_nodes
             SET salience = MIN(1.0, MAX(0.0, salience + ?1)),
                 last_activated = ?2,
                 updated_at = ?2
             WHERE id = ?3",
            params![delta, now, node_id],
        );

        // Record activation
        let _ = conn.execute(
            "INSERT INTO pwg_activations (node_id, trigger_src, delta, propagated, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![node_id, trigger, delta, propagated as i32, now],
        );

        records.push(ActivationRecord {
            node_id,
            trigger: trigger.to_string(),
            delta,
            propagated,
            timestamp: now,
        });

        // Propagate to neighbors
        let edges = Self::edges_for(conn, node_id);
        for edge in &edges {
            let neighbor_id = if edge.source_id == node_id {
                edge.target_id
            } else {
                edge.source_id
            };
            let prop_delta = delta * edge.propagation_factor * edge.weight;
            Self::activate_recursive(conn, neighbor_id, prop_delta, trigger, depth + 1, records);
        }
    }

    /// Decay all node salience based on time elapsed since last activation.
    /// Uses exponential decay with configurable half-life.
    pub fn decay_all(conn: &Connection, half_life_secs: Option<f64>) {
        let half_life = half_life_secs.unwrap_or(DEFAULT_HALF_LIFE_SECS);
        let decay_rate = (0.5_f64).ln() / half_life; // negative
        let now = now_ts();

        // SQLite doesn't have EXP(), so we compute decay in Rust
        let nodes: Vec<(i64, f64, f64)> = conn
            .prepare(
                "SELECT id, salience, last_activated FROM pwg_nodes
                 WHERE salience > 0.001 AND last_activated > 0",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?, row.get::<_, f64>(2)?))
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        for (id, salience, last_activated) in &nodes {
            let elapsed = now - last_activated;
            let factor = (decay_rate * elapsed).exp();
            let new_salience = (salience * factor).max(0.0);
            if new_salience < 0.001 {
                let _ = conn.execute(
                    "UPDATE pwg_nodes SET salience = 0.0 WHERE id = ?1",
                    params![id],
                );
            } else {
                let _ = conn.execute(
                    "UPDATE pwg_nodes SET salience = ?1 WHERE id = ?2",
                    params![new_salience, id],
                );
            }
        }
    }

    /// Get the most salient nodes (what currently matters).
    pub fn get_salient(conn: &Connection, threshold: f64, limit: usize) -> Vec<GraphNode> {
        conn.prepare(
            "SELECT id, entity_type, name, metadata, salience, last_activated,
                    confidence, provenance, keywords, created_at, updated_at
             FROM pwg_nodes
             WHERE salience >= ?1
             ORDER BY salience DESC
             LIMIT ?2",
        )
        .and_then(|mut stmt| {
            stmt.query_map(params![threshold, limit as i64], |row| Self::row_to_node(row))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    }

    /// Get the activation path between two nodes (for legibility/explainability).
    /// Returns the shortest path of edges connecting them, if any.
    pub fn activation_path(conn: &Connection, from_id: i64, to_id: i64) -> Vec<(GraphNode, GraphEdge)> {
        // BFS to find shortest path
        use std::collections::{HashMap, VecDeque};

        let mut visited: HashMap<i64, (i64, GraphEdge)> = HashMap::new(); // node_id → (parent_id, edge)
        let mut queue = VecDeque::new();
        queue.push_back(from_id);

        while let Some(current) = queue.pop_front() {
            if current == to_id {
                // Reconstruct path
                let mut path = Vec::new();
                let mut node_id = to_id;
                while node_id != from_id {
                    if let Some((parent, edge)) = visited.get(&node_id) {
                        if let Some(node) = Self::get_node(conn, node_id) {
                            path.push((node, edge.clone()));
                        }
                        node_id = *parent;
                    } else {
                        break;
                    }
                }
                path.reverse();
                return path;
            }

            let edges = Self::edges_for(conn, current);
            for edge in edges {
                let neighbor = if edge.source_id == current {
                    edge.target_id
                } else {
                    edge.source_id
                };
                if neighbor != from_id && !visited.contains_key(&neighbor) {
                    visited.insert(neighbor, (current, edge));
                    queue.push_back(neighbor);
                }
            }
        }

        Vec::new() // No path found
    }

    /// Get recent activation history for a node.
    pub fn activation_history(conn: &Connection, node_id: i64, limit: usize) -> Vec<ActivationRecord> {
        conn.prepare(
            "SELECT node_id, trigger_src, delta, propagated, timestamp
             FROM pwg_activations
             WHERE node_id = ?1
             ORDER BY timestamp DESC
             LIMIT ?2",
        )
        .and_then(|mut stmt| {
            stmt.query_map(params![node_id, limit as i64], |row| {
                Ok(ActivationRecord {
                    node_id: row.get(0)?,
                    trigger: row.get(1)?,
                    delta: row.get(2)?,
                    propagated: row.get::<_, i32>(3)? != 0,
                    timestamp: row.get(4)?,
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    }

    // ── Interest Seeding ─────────────────────────────────────────────

    /// Seed interest nodes from user's selected interest categories.
    /// Creates parent Interest nodes and child sub-interest nodes.
    pub fn seed_interests(conn: &Connection, interests: &[&str]) -> Vec<i64> {
        let interest_tree: &[(&str, &[&str])] = &[
            ("Technology", &["AI", "startups", "programming", "gadgets", "cybersecurity"]),
            ("Finance", &["stocks", "crypto", "real estate", "personal finance", "oil prices", "commodities"]),
            ("Sports", &["football", "cricket", "basketball", "tennis", "F1"]),
            ("Music", &["concerts", "new releases", "festivals", "artists"]),
            ("Politics", &["geopolitics", "elections", "policy", "international relations"]),
            ("Health", &["fitness", "nutrition", "mental health", "sleep", "medical"]),
            ("Science", &["space", "physics", "biology", "climate", "research"]),
            ("Gaming", &["PC gaming", "console", "esports", "game releases"]),
            ("Travel", &["flights", "destinations", "hotels", "visa", "travel deals"]),
            ("Business", &["entrepreneurship", "management", "markets", "industry trends"]),
            ("Entertainment", &["movies", "TV shows", "streaming", "books"]),
            ("Food", &["restaurants", "recipes", "cooking", "food trends"]),
            ("Cars", &["fuel prices", "maintenance", "new models", "EV"]),
        ];

        let now = now_ts();
        let mut created_ids = Vec::new();

        for interest_name in interests {
            // Create parent Interest node
            let (parent_id, _) = Self::find_or_create(
                conn,
                EntityType::Interest,
                interest_name,
                "onboarding",
                &[&interest_name.to_lowercase()],
            );

            // Set high initial salience for user-selected interests
            let _ = conn.execute(
                "UPDATE pwg_nodes SET salience = 0.8, confidence = 1.0, last_activated = ?1 WHERE id = ?2",
                params![now, parent_id],
            );
            created_ids.push(parent_id);

            // Find matching sub-interests from the tree
            if let Some((_, subs)) = interest_tree.iter().find(|(name, _)| {
                name.to_lowercase() == interest_name.to_lowercase()
            }) {
                for sub in *subs {
                    let (sub_id, _) = Self::find_or_create(
                        conn,
                        EntityType::Interest,
                        sub,
                        "onboarding",
                        &[&sub.to_lowercase()],
                    );

                    // Create parent→child edge
                    Self::upsert_edge(conn, &GraphEdge {
                        id: 0,
                        source_id: sub_id,
                        target_id: parent_id,
                        relation: RelationType::ChildOf,
                        weight: 0.8,
                        propagation_factor: RelationType::ChildOf.default_propagation(),
                        confidence: 1.0,
                        metadata: serde_json::Value::Null,
                        created_at: now,
                    });

                    // Set moderate initial salience for sub-interests
                    let _ = conn.execute(
                        "UPDATE pwg_nodes SET salience = 0.5, confidence = 0.8, last_activated = ?1 WHERE id = ?2",
                        params![now, sub_id],
                    );
                }
            }
        }

        created_ids
    }

    /// Seed essential resource nodes that most users need.
    pub fn seed_defaults(conn: &Connection) {
        let now = now_ts();
        let defaults: &[(&str, EntityType, &[&str])] = &[
            ("fuel", EntityType::Resource, &["gas", "petrol", "oil", "fuel prices"]),
            ("weather", EntityType::Resource, &["rain", "temperature", "forecast", "umbrella"]),
            ("commute", EntityType::Rhythm, &["traffic", "travel time", "route"]),
            ("work_hours", EntityType::Rhythm, &["office", "meetings", "focus"]),
            ("sleep", EntityType::Rhythm, &["bedtime", "wake up", "rest"]),
        ];

        for (name, etype, keywords) in defaults {
            Self::find_or_create(conn, *etype, name, "system", keywords);
        }

        // Create edges: fuel → impacts → commute
        if let (Some(fuel), Some(commute)) = (
            Self::find_node(conn, EntityType::Resource, "fuel"),
            Self::find_node(conn, EntityType::Rhythm, "commute"),
        ) {
            Self::upsert_edge(conn, &GraphEdge {
                id: 0,
                source_id: fuel.id,
                target_id: commute.id,
                relation: RelationType::Impacts,
                weight: 0.7,
                propagation_factor: RelationType::Impacts.default_propagation(),
                confidence: 0.9,
                metadata: serde_json::Value::Null,
                created_at: now,
            });
        }

        // Link oil prices interest to fuel resource if finance interest exists
        if let (Some(oil), Some(fuel)) = (
            Self::find_node(conn, EntityType::Interest, "oil prices"),
            Self::find_node(conn, EntityType::Resource, "fuel"),
        ) {
            Self::upsert_edge(conn, &GraphEdge {
                id: 0,
                source_id: oil.id,
                target_id: fuel.id,
                relation: RelationType::Impacts,
                weight: 0.8,
                propagation_factor: 0.6,
                confidence: 0.9,
                metadata: serde_json::Value::Null,
                created_at: now,
            });
        }
    }

    // ── Stats / Dashboard ────────────────────────────────────────────

    /// Get graph statistics.
    pub fn stats(conn: &Connection) -> GraphStats {
        let node_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM pwg_nodes", [], |r| r.get(0))
            .unwrap_or(0);
        let edge_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM pwg_edges", [], |r| r.get(0))
            .unwrap_or(0);
        let active_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pwg_nodes WHERE salience > 0.01",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let activation_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM pwg_activations", [], |r| r.get(0))
            .unwrap_or(0);

        GraphStats {
            node_count: node_count as usize,
            edge_count: edge_count as usize,
            active_nodes: active_count as usize,
            total_activations: activation_count as usize,
        }
    }

    /// Generate a summary of currently salient nodes for system prompt injection.
    pub fn salience_summary(conn: &Connection) -> String {
        let salient = Self::get_salient(conn, 0.1, 15);
        if salient.is_empty() {
            return String::new();
        }

        let mut out = String::from("## Currently Active in User's Life\n");
        for node in &salient {
            out.push_str(&format!(
                "- [{}] {} (salience: {:.2})\n",
                node.entity_type.as_str(),
                node.name,
                node.salience,
            ));
        }
        out
    }

    // ── Internal helpers ─────────────────────────────────────────────

    fn row_to_node(row: &rusqlite::Row) -> rusqlite::Result<GraphNode> {
        let meta_str: String = row.get(3)?;
        let keywords_str: String = row.get(8)?;
        Ok(GraphNode {
            id: row.get(0)?,
            entity_type: EntityType::from_str(&row.get::<_, String>(1).unwrap_or_default())
                .unwrap_or(EntityType::Resource),
            name: row.get(2)?,
            metadata: serde_json::from_str(&meta_str).unwrap_or(serde_json::Value::Null),
            salience: row.get(4)?,
            last_activated: row.get(5)?,
            confidence: row.get(6)?,
            provenance: row.get(7)?,
            keywords: serde_json::from_str(&keywords_str).unwrap_or_default(),
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }

    fn query_edges(
        conn: &Connection,
        where_clause: &str,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> Vec<GraphEdge> {
        let sql = format!(
            "SELECT id, source_id, target_id, relation, weight, propagation_factor,
                    confidence, metadata, created_at
             FROM pwg_edges {}",
            where_clause
        );
        conn.prepare(&sql)
            .and_then(|mut stmt| {
                stmt.query_map(params, |row| {
                    let meta_str: String = row.get(7)?;
                    Ok(GraphEdge {
                        id: row.get(0)?,
                        source_id: row.get(1)?,
                        target_id: row.get(2)?,
                        relation: RelationType::from_str(
                            &row.get::<_, String>(3).unwrap_or_default(),
                        )
                        .unwrap_or(RelationType::RelatedTo),
                        weight: row.get(4)?,
                        propagation_factor: row.get(5)?,
                        confidence: row.get(6)?,
                        metadata: serde_json::from_str(&meta_str)
                            .unwrap_or(serde_json::Value::Null),
                        created_at: row.get(8)?,
                    })
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub active_nodes: usize,
    pub total_activations: usize,
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        WorldGraph::ensure_tables(&conn);
        conn
    }

    #[test]
    fn insert_and_find_node() {
        let conn = setup();
        let now = now_ts();

        let node = GraphNode {
            id: 0,
            entity_type: EntityType::Person,
            name: "Sarah Chen".into(),
            metadata: serde_json::json!({"relationship": "colleague", "department": "engineering"}),
            salience: 0.0,
            last_activated: 0.0,
            confidence: 0.9,
            provenance: "email".into(),
            keywords: vec!["sarah".into(), "chen".into(), "engineering".into()],
            created_at: now,
            updated_at: now,
        };

        let id = WorldGraph::insert_node(&conn, &node);
        assert!(id > 0);

        let found = WorldGraph::find_node(&conn, EntityType::Person, "Sarah Chen");
        assert!(found.is_some());
        assert_eq!(found.unwrap().confidence, 0.9);
    }

    #[test]
    fn find_or_create() {
        let conn = setup();

        let (id1, created1) = WorldGraph::find_or_create(
            &conn, EntityType::Interest, "Technology", "onboarding", &["tech"],
        );
        assert!(created1);

        let (id2, created2) = WorldGraph::find_or_create(
            &conn, EntityType::Interest, "Technology", "onboarding", &["tech"],
        );
        assert!(!created2);
        assert_eq!(id1, id2);
    }

    #[test]
    fn edges_and_neighbors() {
        let conn = setup();
        let now = now_ts();

        // Create two nodes
        let (person_id, _) = WorldGraph::find_or_create(
            &conn, EntityType::Person, "Sarah", "test", &[],
        );
        let (project_id, _) = WorldGraph::find_or_create(
            &conn, EntityType::Project, "Q3 Deck", "test", &[],
        );

        // Create edge: Sarah is important for Q3 Deck
        WorldGraph::upsert_edge(&conn, &GraphEdge {
            id: 0,
            source_id: person_id,
            target_id: project_id,
            relation: RelationType::ImportantFor,
            weight: 0.8,
            propagation_factor: 0.6,
            confidence: 0.9,
            metadata: serde_json::Value::Null,
            created_at: now,
        });

        let edges = WorldGraph::edges_from(&conn, person_id);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target_id, project_id);

        let neighbors = WorldGraph::related_nodes(&conn, person_id);
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].0.name, "Q3 Deck");
    }

    #[test]
    fn activation_propagation() {
        let conn = setup();
        let now = now_ts();

        // Create chain: oil_prices → fuel → commute
        let (oil_id, _) = WorldGraph::find_or_create(
            &conn, EntityType::Interest, "oil prices", "test", &["oil"],
        );
        let (fuel_id, _) = WorldGraph::find_or_create(
            &conn, EntityType::Resource, "fuel", "test", &["gas"],
        );
        let (commute_id, _) = WorldGraph::find_or_create(
            &conn, EntityType::Rhythm, "commute", "test", &["travel"],
        );

        // oil_prices → impacts → fuel
        WorldGraph::upsert_edge(&conn, &GraphEdge {
            id: 0,
            source_id: oil_id,
            target_id: fuel_id,
            relation: RelationType::Impacts,
            weight: 0.8,
            propagation_factor: 0.7,
            confidence: 0.9,
            metadata: serde_json::Value::Null,
            created_at: now,
        });

        // fuel → impacts → commute
        WorldGraph::upsert_edge(&conn, &GraphEdge {
            id: 0,
            source_id: fuel_id,
            target_id: commute_id,
            relation: RelationType::Impacts,
            weight: 0.7,
            propagation_factor: 0.5,
            confidence: 0.9,
            metadata: serde_json::Value::Null,
            created_at: now,
        });

        // Activate oil_prices with news event
        let records = WorldGraph::activate(&conn, oil_id, 0.8, "news:iran_strike");

        // Should have activated all three nodes
        assert!(records.len() >= 2); // oil + fuel at minimum, possibly commute

        // Oil should have highest salience
        let oil = WorldGraph::get_node(&conn, oil_id).unwrap();
        assert!(oil.salience >= 0.7);

        // Fuel should have been activated via propagation
        let fuel = WorldGraph::get_node(&conn, fuel_id).unwrap();
        assert!(fuel.salience > 0.0, "fuel salience should be > 0, got {}", fuel.salience);

        // Commute might be activated (depends on propagation threshold)
        let commute = WorldGraph::get_node(&conn, commute_id).unwrap();
        // At depth 2: 0.8 * 0.7 * 0.8 = 0.448, then * 0.5 * 0.7 = 0.157 — above MIN threshold
        // Actually: depth 1 delta = 0.8 * 0.7 * 0.8 = 0.448
        //           depth 2 delta = 0.448 * 0.5 * 0.7 = 0.157
        assert!(commute.salience > 0.0, "commute should be activated via propagation");

        // Verify activation records
        let history = WorldGraph::activation_history(&conn, oil_id, 5);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].trigger, "news:iran_strike");
        assert!(!history[0].propagated);

        let fuel_history = WorldGraph::activation_history(&conn, fuel_id, 5);
        assert_eq!(fuel_history.len(), 1);
        assert!(fuel_history[0].propagated);
    }

    #[test]
    fn salience_decay() {
        let conn = setup();

        let (id, _) = WorldGraph::find_or_create(
            &conn, EntityType::Interest, "test", "test", &[],
        );

        // Set salience directly and fake last_activated to 8 hours ago (2 half-lives)
        let eight_hours_ago = now_ts() - 8.0 * 3600.0;
        let _ = conn.execute(
            "UPDATE pwg_nodes SET salience = 0.9, last_activated = ?1 WHERE id = ?2",
            params![eight_hours_ago, id],
        );

        let before = WorldGraph::get_node(&conn, id).unwrap();
        assert!(before.salience > 0.8, "pre-decay salience should be 0.9");

        // Decay
        WorldGraph::decay_all(&conn, None);

        let decayed = WorldGraph::get_node(&conn, id).unwrap();
        // After 2 half-lives, should be ~25% of original (0.9 * 0.25 ≈ 0.225)
        assert!(decayed.salience < 0.3, "salience should have decayed, got {}", decayed.salience);
        assert!(decayed.salience > 0.1, "shouldn't decay to zero yet, got {}", decayed.salience);
    }

    #[test]
    fn get_salient_nodes() {
        let conn = setup();

        // Create several nodes with different salience
        let (a, _) = WorldGraph::find_or_create(&conn, EntityType::Interest, "high", "test", &[]);
        let (b, _) = WorldGraph::find_or_create(&conn, EntityType::Interest, "medium", "test", &[]);
        let (_c, _) = WorldGraph::find_or_create(&conn, EntityType::Interest, "low", "test", &[]);

        WorldGraph::activate(&conn, a, 0.9, "test");
        WorldGraph::activate(&conn, b, 0.3, "test");
        // c stays at 0.0

        let salient = WorldGraph::get_salient(&conn, 0.2, 10);
        assert_eq!(salient.len(), 2);
        assert_eq!(salient[0].name, "high");
        assert_eq!(salient[1].name, "medium");
    }

    #[test]
    fn activation_path() {
        let conn = setup();
        let now = now_ts();

        let (a, _) = WorldGraph::find_or_create(&conn, EntityType::Interest, "A", "test", &[]);
        let (b, _) = WorldGraph::find_or_create(&conn, EntityType::Resource, "B", "test", &[]);
        let (c, _) = WorldGraph::find_or_create(&conn, EntityType::Rhythm, "C", "test", &[]);

        WorldGraph::upsert_edge(&conn, &GraphEdge {
            id: 0, source_id: a, target_id: b, relation: RelationType::Impacts,
            weight: 0.8, propagation_factor: 0.5, confidence: 0.9,
            metadata: serde_json::Value::Null, created_at: now,
        });
        WorldGraph::upsert_edge(&conn, &GraphEdge {
            id: 0, source_id: b, target_id: c, relation: RelationType::Impacts,
            weight: 0.7, propagation_factor: 0.5, confidence: 0.9,
            metadata: serde_json::Value::Null, created_at: now,
        });

        let path = WorldGraph::activation_path(&conn, a, c);
        assert_eq!(path.len(), 2); // B and C
        assert_eq!(path[0].0.name, "B");
        assert_eq!(path[1].0.name, "C");
    }

    #[test]
    fn seed_interests() {
        let conn = setup();

        let ids = WorldGraph::seed_interests(&conn, &["Technology", "Finance"]);
        assert_eq!(ids.len(), 2);

        // Should have parent + child nodes
        let tech_nodes = WorldGraph::nodes_by_type(&conn, EntityType::Interest);
        // Technology + 5 subs + Finance + 6 subs = 13
        assert!(tech_nodes.len() >= 10, "expected at least 10 interest nodes, got {}", tech_nodes.len());

        // Technology node should have high salience
        let tech = WorldGraph::find_node(&conn, EntityType::Interest, "Technology").unwrap();
        assert!(tech.salience >= 0.7);

        // AI should be a child of Technology
        let ai = WorldGraph::find_node(&conn, EntityType::Interest, "AI").unwrap();
        let ai_edges = WorldGraph::edges_from(&conn, ai.id);
        assert!(!ai_edges.is_empty());
    }

    #[test]
    fn seed_defaults_and_links() {
        let conn = setup();

        // First seed finance interests (creates oil prices sub-interest)
        WorldGraph::seed_interests(&conn, &["Finance"]);
        // Then seed defaults
        WorldGraph::seed_defaults(&conn);

        // Fuel and commute should exist
        let fuel = WorldGraph::find_node(&conn, EntityType::Resource, "fuel");
        assert!(fuel.is_some());

        let commute = WorldGraph::find_node(&conn, EntityType::Rhythm, "commute");
        assert!(commute.is_some());

        // fuel → commute edge should exist
        let fuel_edges = WorldGraph::edges_from(&conn, fuel.unwrap().id);
        assert!(!fuel_edges.is_empty(), "fuel should have outgoing edges");

        // oil prices → fuel edge should exist
        let oil = WorldGraph::find_node(&conn, EntityType::Interest, "oil prices");
        if let Some(oil_node) = oil {
            let oil_edges = WorldGraph::edges_from(&conn, oil_node.id);
            assert!(!oil_edges.is_empty(), "oil prices should link to fuel");
        }
    }

    #[test]
    fn search_nodes() {
        let conn = setup();
        WorldGraph::find_or_create(&conn, EntityType::Person, "Sarah Chen", "test", &["sarah", "engineering"]);
        WorldGraph::find_or_create(&conn, EntityType::Person, "Alex Park", "test", &["alex", "design"]);

        let results = WorldGraph::search_nodes(&conn, "sarah");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Sarah Chen");

        let results2 = WorldGraph::search_nodes(&conn, "engineering");
        assert_eq!(results2.len(), 1);
    }

    #[test]
    fn graph_stats() {
        let conn = setup();
        WorldGraph::seed_interests(&conn, &["Technology"]);
        WorldGraph::seed_defaults(&conn);

        let stats = WorldGraph::stats(&conn);
        assert!(stats.node_count > 0);
        assert!(stats.active_nodes > 0); // seeded interests have salience
    }

    #[test]
    fn full_scenario_iran_oil_commute() {
        let conn = setup();

        // Setup: user interested in finance, has a car, commutes to work
        WorldGraph::seed_interests(&conn, &["Finance"]);
        WorldGraph::seed_defaults(&conn);

        // Simulate: news about Iran striking oil infrastructure arrives
        // The news scanner would activate the "oil prices" interest node
        let oil = WorldGraph::find_node(&conn, EntityType::Interest, "oil prices");
        assert!(oil.is_some(), "oil prices interest should exist from Finance seeding");
        let oil_id = oil.unwrap().id;

        let records = WorldGraph::activate(&conn, oil_id, 0.9, "news:iran_oil_strike");

        // Verify propagation chain: oil prices → fuel → commute
        let fuel = WorldGraph::find_node(&conn, EntityType::Resource, "fuel").unwrap();
        let commute = WorldGraph::find_node(&conn, EntityType::Rhythm, "commute").unwrap();

        assert!(fuel.salience > 0.0, "fuel should be activated via propagation from oil prices");
        assert!(commute.salience > 0.0, "commute should be activated via fuel");

        // The system now knows: oil prices are salient → fuel is affected → commute is impacted
        // Day Composer can surface: "Oil prices may rise due to Iran conflict. Consider topping up."
        let salient = WorldGraph::get_salient(&conn, 0.05, 10);
        let salient_names: Vec<&str> = salient.iter().map(|n| n.name.as_str()).collect();
        assert!(salient_names.contains(&"oil prices"), "oil prices should be salient");

        // Activation path should explain the connection
        let path = WorldGraph::activation_path(&conn, oil_id, commute.id);
        assert!(!path.is_empty(), "should find path from oil prices to commute");
    }
}
