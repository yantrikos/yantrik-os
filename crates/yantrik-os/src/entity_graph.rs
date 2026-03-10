//! Unified Entity Graph — cross-app object model with relations and FTS5 search.
//!
//! Every meaningful object in Yantrik (email thread, calendar event, note, file,
//! task, decision, person) is represented as a `UniversalObject` in a shared
//! SQLite graph. Relations between objects form a queryable entity graph.
//!
//! This is the foundation for cross-app intelligence: global search, "Open in...",
//! "Convert to...", and workflow chaining (email → calendar → notes → tasks).
//!
//! Architecture:
//! - **Objects** have a kind, source app, searchable text, and flexible JSON metadata
//! - **Relations** are typed edges between objects (References, CreatedFrom, AttachedTo, etc.)
//! - **FTS5** provides instant full-text search across all objects
//! - **EntityGraph** wraps a SQLite connection with CRUD + search methods

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────────────────────────────────────
// Object Model
// ────────────────────────────────────────────────────────────────────────────

/// The unified object types in the entity graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectKind {
    Person,
    Thread,
    Event,
    Note,
    File,
    Task,
    Decision,
    Spreadsheet,
    Document,
    Presentation,
    Snippet,
}

impl ObjectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Thread => "thread",
            Self::Event => "event",
            Self::Note => "note",
            Self::File => "file",
            Self::Task => "task",
            Self::Decision => "decision",
            Self::Spreadsheet => "spreadsheet",
            Self::Document => "document",
            Self::Presentation => "presentation",
            Self::Snippet => "snippet",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "person" => Some(Self::Person),
            "thread" => Some(Self::Thread),
            "event" => Some(Self::Event),
            "note" => Some(Self::Note),
            "file" => Some(Self::File),
            "task" => Some(Self::Task),
            "decision" => Some(Self::Decision),
            "spreadsheet" => Some(Self::Spreadsheet),
            "document" => Some(Self::Document),
            "presentation" => Some(Self::Presentation),
            "snippet" => Some(Self::Snippet),
            _ => None,
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Person => "P",
            Self::Thread => "@",
            Self::Event => "▦",
            Self::Note => "✎",
            Self::File => "F",
            Self::Task => "☐",
            Self::Decision => "◆",
            Self::Spreadsheet => "YS",
            Self::Document => "YD",
            Self::Presentation => "YP",
            Self::Snippet => "<>",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Person => "Person",
            Self::Thread => "Email",
            Self::Event => "Event",
            Self::Note => "Note",
            Self::File => "File",
            Self::Task => "Task",
            Self::Decision => "Decision",
            Self::Spreadsheet => "Sheet",
            Self::Document => "Document",
            Self::Presentation => "Slides",
            Self::Snippet => "Snippet",
        }
    }
}

/// A universal object in the entity graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalObject {
    pub id: String,
    pub kind: ObjectKind,
    pub title: String,
    pub summary: String,
    pub source_app: String,
    pub source_id: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub metadata: serde_json::Value,
    pub searchable_text: String,
}

impl UniversalObject {
    /// Create a new object with a generated UUID.
    pub fn new(
        kind: ObjectKind,
        title: impl Into<String>,
        source_app: impl Into<String>,
        source_id: impl Into<String>,
    ) -> Self {
        let now = now_ts();
        Self {
            id: generate_id(),
            kind,
            title: title.into(),
            summary: String::new(),
            source_app: source_app.into(),
            source_id: source_id.into(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            searchable_text: String::new(),
        }
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    pub fn with_searchable_text(mut self, text: impl Into<String>) -> Self {
        self.searchable_text = text.into();
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Relations
// ────────────────────────────────────────────────────────────────────────────

/// Typed relationships between objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationKind {
    /// A references B (email mentions a file).
    References,
    /// A was created from B (note created from email).
    CreatedFrom,
    /// A is attached to B.
    AttachedTo,
    /// Person is attendee of Event.
    Attendee,
    /// Task is assigned to Person.
    AssignedTo,
    /// Generic association.
    RelatedTo,
    /// A is a follow-up to B.
    FollowUp,
    /// A mentions Person B.
    Mentions,
    /// A contains B (folder contains file, notebook contains note).
    Contains,
}

impl RelationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::References => "references",
            Self::CreatedFrom => "created_from",
            Self::AttachedTo => "attached_to",
            Self::Attendee => "attendee",
            Self::AssignedTo => "assigned_to",
            Self::RelatedTo => "related_to",
            Self::FollowUp => "follow_up",
            Self::Mentions => "mentions",
            Self::Contains => "contains",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "references" => Some(Self::References),
            "created_from" => Some(Self::CreatedFrom),
            "attached_to" => Some(Self::AttachedTo),
            "attendee" => Some(Self::Attendee),
            "assigned_to" => Some(Self::AssignedTo),
            "related_to" => Some(Self::RelatedTo),
            "follow_up" => Some(Self::FollowUp),
            "mentions" => Some(Self::Mentions),
            "contains" => Some(Self::Contains),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::References => "references",
            Self::CreatedFrom => "created from",
            Self::AttachedTo => "attached to",
            Self::Attendee => "attendee of",
            Self::AssignedTo => "assigned to",
            Self::RelatedTo => "related to",
            Self::FollowUp => "follow-up to",
            Self::Mentions => "mentions",
            Self::Contains => "contains",
        }
    }
}

/// An edge in the entity graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub id: i64,
    pub source_id: String,
    pub target_id: String,
    pub kind: RelationKind,
    pub created_at: f64,
    pub metadata: serde_json::Value,
}

// ────────────────────────────────────────────────────────────────────────────
// EntityGraph — SQLite-backed graph store
// ────────────────────────────────────────────────────────────────────────────

/// The unified entity graph. Thread-safe via external Arc<Mutex<>>.
pub struct EntityGraph {
    conn: rusqlite::Connection,
}

impl EntityGraph {
    /// Open or create an entity graph database.
    pub fn open(path: &str) -> Result<Self, rusqlite::Error> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -4000;",
        )?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS objects (
                 id              TEXT PRIMARY KEY,
                 kind            TEXT NOT NULL,
                 title           TEXT NOT NULL,
                 summary         TEXT NOT NULL DEFAULT '',
                 source_app      TEXT NOT NULL,
                 source_id       TEXT NOT NULL,
                 created_at      REAL NOT NULL,
                 updated_at      REAL NOT NULL,
                 metadata        TEXT NOT NULL DEFAULT '{}',
                 searchable_text TEXT NOT NULL DEFAULT ''
             );

             CREATE TABLE IF NOT EXISTS relations (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 source_id  TEXT NOT NULL,
                 target_id  TEXT NOT NULL,
                 kind       TEXT NOT NULL,
                 created_at REAL NOT NULL,
                 metadata   TEXT NOT NULL DEFAULT '{}',
                 FOREIGN KEY (source_id) REFERENCES objects(id) ON DELETE CASCADE,
                 FOREIGN KEY (target_id) REFERENCES objects(id) ON DELETE CASCADE
             );

             CREATE INDEX IF NOT EXISTS idx_obj_kind ON objects(kind);
             CREATE INDEX IF NOT EXISTS idx_obj_source ON objects(source_app, source_id);
             CREATE INDEX IF NOT EXISTS idx_obj_updated ON objects(updated_at DESC);
             CREATE INDEX IF NOT EXISTS idx_rel_source ON relations(source_id);
             CREATE INDEX IF NOT EXISTS idx_rel_target ON relations(target_id);
             CREATE INDEX IF NOT EXISTS idx_rel_kind ON relations(kind);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_rel_unique
                 ON relations(source_id, target_id, kind);",
        )?;

        // FTS5 for full-text search (content-less — we manage sync manually)
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS objects_fts USING fts5(
                 title, summary, searchable_text,
                 content='objects', content_rowid='rowid'
             );

             -- Triggers to keep FTS5 in sync with objects table
             CREATE TRIGGER IF NOT EXISTS objects_ai AFTER INSERT ON objects BEGIN
                 INSERT INTO objects_fts(rowid, title, summary, searchable_text)
                 VALUES (new.rowid, new.title, new.summary, new.searchable_text);
             END;

             CREATE TRIGGER IF NOT EXISTS objects_ad AFTER DELETE ON objects BEGIN
                 INSERT INTO objects_fts(objects_fts, rowid, title, summary, searchable_text)
                 VALUES ('delete', old.rowid, old.title, old.summary, old.searchable_text);
             END;

             CREATE TRIGGER IF NOT EXISTS objects_au AFTER UPDATE ON objects BEGIN
                 INSERT INTO objects_fts(objects_fts, rowid, title, summary, searchable_text)
                 VALUES ('delete', old.rowid, old.title, old.summary, old.searchable_text);
                 INSERT INTO objects_fts(rowid, title, summary, searchable_text)
                 VALUES (new.rowid, new.title, new.summary, new.searchable_text);
             END;",
        )?;

        tracing::info!("Entity graph opened at {path}");
        Ok(Self { conn })
    }

    /// Open an in-memory entity graph (for testing).
    pub fn in_memory() -> Result<Self, rusqlite::Error> {
        Self::open(":memory:")
    }

    // ── Object CRUD ──

    /// Insert or update an object. Uses source_app + source_id as the natural key.
    /// If an object with the same source already exists, updates it and returns the existing ID.
    pub fn upsert_object(&self, obj: &UniversalObject) -> Result<String, rusqlite::Error> {
        // Check if object already exists by source
        if let Some(existing) = self.find_by_source(&obj.source_app, &obj.source_id)? {
            // Update existing
            self.conn.execute(
                "UPDATE objects SET title = ?1, summary = ?2, updated_at = ?3,
                 metadata = ?4, searchable_text = ?5 WHERE id = ?6",
                rusqlite::params![
                    obj.title,
                    obj.summary,
                    now_ts(),
                    serde_json::to_string(&obj.metadata).unwrap_or_default(),
                    obj.searchable_text,
                    existing.id,
                ],
            )?;
            return Ok(existing.id);
        }

        // Insert new
        self.conn.execute(
            "INSERT INTO objects (id, kind, title, summary, source_app, source_id,
             created_at, updated_at, metadata, searchable_text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                obj.id,
                obj.kind.as_str(),
                obj.title,
                obj.summary,
                obj.source_app,
                obj.source_id,
                obj.created_at,
                obj.updated_at,
                serde_json::to_string(&obj.metadata).unwrap_or_default(),
                obj.searchable_text,
            ],
        )?;
        Ok(obj.id.clone())
    }

    /// Delete an object and all its relations.
    pub fn delete_object(&self, id: &str) -> Result<bool, rusqlite::Error> {
        let deleted = self
            .conn
            .execute("DELETE FROM objects WHERE id = ?1", rusqlite::params![id])?;
        if deleted > 0 {
            self.conn.execute(
                "DELETE FROM relations WHERE source_id = ?1 OR target_id = ?1",
                rusqlite::params![id],
            )?;
        }
        Ok(deleted > 0)
    }

    /// Get an object by its graph ID.
    pub fn get_object(&self, id: &str) -> Result<Option<UniversalObject>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, summary, source_app, source_id,
                    created_at, updated_at, metadata, searchable_text
             FROM objects WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id], row_to_object)?;
        match rows.next() {
            Some(Ok(obj)) => Ok(Some(obj)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    /// Find an object by its source app and source-specific ID.
    pub fn find_by_source(
        &self,
        app: &str,
        source_id: &str,
    ) -> Result<Option<UniversalObject>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, summary, source_app, source_id,
                    created_at, updated_at, metadata, searchable_text
             FROM objects WHERE source_app = ?1 AND source_id = ?2",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![app, source_id], row_to_object)?;
        match rows.next() {
            Some(Ok(obj)) => Ok(Some(obj)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    /// List objects by kind, ordered by updated_at DESC.
    pub fn objects_by_kind(
        &self,
        kind: ObjectKind,
        limit: usize,
    ) -> Result<Vec<UniversalObject>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, summary, source_app, source_id,
                    created_at, updated_at, metadata, searchable_text
             FROM objects WHERE kind = ?1
             ORDER BY updated_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![kind.as_str(), limit as i64], row_to_object)?;
        rows.collect()
    }

    /// List the most recently updated objects across all kinds.
    pub fn recent_objects(&self, limit: usize) -> Result<Vec<UniversalObject>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, summary, source_app, source_id,
                    created_at, updated_at, metadata, searchable_text
             FROM objects ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as i64], row_to_object)?;
        rows.collect()
    }

    /// Count objects by kind.
    pub fn count_by_kind(&self, kind: ObjectKind) -> u64 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM objects WHERE kind = ?1",
                rusqlite::params![kind.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64
    }

    /// Total object count.
    pub fn total_objects(&self) -> u64 {
        self.conn
            .query_row("SELECT COUNT(*) FROM objects", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64
    }

    // ── Relation CRUD ──

    /// Add a relation between two objects. Ignores duplicates (same source, target, kind).
    pub fn add_relation(
        &self,
        source_id: &str,
        target_id: &str,
        kind: RelationKind,
    ) -> Result<i64, rusqlite::Error> {
        self.conn.execute(
            "INSERT OR IGNORE INTO relations (source_id, target_id, kind, created_at, metadata)
             VALUES (?1, ?2, ?3, ?4, '{}')",
            rusqlite::params![source_id, target_id, kind.as_str(), now_ts()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Add a relation with metadata.
    pub fn add_relation_with_metadata(
        &self,
        source_id: &str,
        target_id: &str,
        kind: RelationKind,
        metadata: &serde_json::Value,
    ) -> Result<i64, rusqlite::Error> {
        self.conn.execute(
            "INSERT OR IGNORE INTO relations (source_id, target_id, kind, created_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                source_id,
                target_id,
                kind.as_str(),
                now_ts(),
                serde_json::to_string(metadata).unwrap_or_default(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Delete a specific relation.
    pub fn delete_relation(&self, id: i64) -> Result<bool, rusqlite::Error> {
        let deleted = self
            .conn
            .execute("DELETE FROM relations WHERE id = ?1", rusqlite::params![id])?;
        Ok(deleted > 0)
    }

    /// Get all relations for an object (both directions), with the related objects.
    pub fn get_relations(
        &self,
        object_id: &str,
    ) -> Result<Vec<(Relation, UniversalObject)>, rusqlite::Error> {
        let mut results = Vec::new();

        // Outgoing relations (this object → other)
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.source_id, r.target_id, r.kind, r.created_at, r.metadata,
                    o.id, o.kind, o.title, o.summary, o.source_app, o.source_id,
                    o.created_at, o.updated_at, o.metadata, o.searchable_text
             FROM relations r JOIN objects o ON o.id = r.target_id
             WHERE r.source_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![object_id], |row| {
            Ok((row_to_relation(row)?, row_to_object_offset(row, 6)?))
        })?;
        for r in rows {
            if let Ok(pair) = r {
                results.push(pair);
            }
        }

        // Incoming relations (other → this object)
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.source_id, r.target_id, r.kind, r.created_at, r.metadata,
                    o.id, o.kind, o.title, o.summary, o.source_app, o.source_id,
                    o.created_at, o.updated_at, o.metadata, o.searchable_text
             FROM relations r JOIN objects o ON o.id = r.source_id
             WHERE r.target_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![object_id], |row| {
            Ok((row_to_relation(row)?, row_to_object_offset(row, 6)?))
        })?;
        for r in rows {
            if let Ok(pair) = r {
                results.push(pair);
            }
        }

        Ok(results)
    }

    // ── Search ──

    /// Full-text search across all objects via FTS5.
    /// Returns objects ranked by relevance, limited to `limit` results.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UniversalObject>, rusqlite::Error> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Sanitize query for FTS5 (escape special chars, add prefix matching)
        let fts_query = sanitize_fts_query(query);

        let mut stmt = self.conn.prepare(
            "SELECT o.id, o.kind, o.title, o.summary, o.source_app, o.source_id,
                    o.created_at, o.updated_at, o.metadata, o.searchable_text
             FROM objects_fts f
             JOIN objects o ON o.rowid = f.rowid
             WHERE objects_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![fts_query, limit as i64], row_to_object)?;
        rows.collect()
    }

    /// Search objects filtered by kind.
    pub fn search_by_kind(
        &self,
        query: &str,
        kind: ObjectKind,
        limit: usize,
    ) -> Result<Vec<UniversalObject>, rusqlite::Error> {
        if query.trim().is_empty() {
            return self.objects_by_kind(kind, limit);
        }

        let fts_query = sanitize_fts_query(query);

        let mut stmt = self.conn.prepare(
            "SELECT o.id, o.kind, o.title, o.summary, o.source_app, o.source_id,
                    o.created_at, o.updated_at, o.metadata, o.searchable_text
             FROM objects_fts f
             JOIN objects o ON o.rowid = f.rowid
             WHERE objects_fts MATCH ?1 AND o.kind = ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![fts_query, kind.as_str(), limit as i64],
            row_to_object,
        )?;
        rows.collect()
    }

    // ── Stats ──

    /// Get a summary of object counts by kind.
    pub fn stats(&self) -> Vec<(String, u64)> {
        let mut stmt = match self.conn.prepare(
            "SELECT kind, COUNT(*) FROM objects GROUP BY kind ORDER BY COUNT(*) DESC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Compact: remove objects older than `before` timestamp that haven't been updated.
    pub fn compact(&self, before: f64) -> usize {
        self.conn
            .execute(
                "DELETE FROM objects WHERE updated_at < ?1",
                rusqlite::params![before],
            )
            .unwrap_or(0)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Row mappers
// ────────────────────────────────────────────────────────────────────────────

fn row_to_object(row: &rusqlite::Row) -> Result<UniversalObject, rusqlite::Error> {
    row_to_object_offset(row, 0)
}

fn row_to_object_offset(
    row: &rusqlite::Row,
    offset: usize,
) -> Result<UniversalObject, rusqlite::Error> {
    let kind_str: String = row.get(offset + 1)?;
    let metadata_str: String = row.get(offset + 8)?;
    Ok(UniversalObject {
        id: row.get(offset)?,
        kind: ObjectKind::from_str(&kind_str).unwrap_or(ObjectKind::File),
        title: row.get(offset + 2)?,
        summary: row.get(offset + 3)?,
        source_app: row.get(offset + 4)?,
        source_id: row.get(offset + 5)?,
        created_at: row.get(offset + 6)?,
        updated_at: row.get(offset + 7)?,
        metadata: serde_json::from_str(&metadata_str).unwrap_or(serde_json::Value::Null),
        searchable_text: row.get(offset + 9)?,
    })
}

fn row_to_relation(row: &rusqlite::Row) -> Result<Relation, rusqlite::Error> {
    let kind_str: String = row.get(3)?;
    let metadata_str: String = row.get(5)?;
    Ok(Relation {
        id: row.get(0)?,
        source_id: row.get(1)?,
        target_id: row.get(2)?,
        kind: RelationKind::from_str(&kind_str).unwrap_or(RelationKind::RelatedTo),
        created_at: row.get(4)?,
        metadata: serde_json::from_str(&metadata_str).unwrap_or(serde_json::Value::Null),
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────────

fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Generate a compact, time-sortable ID (timestamp + random suffix).
fn generate_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    // Simple random suffix using timestamp nanoseconds
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{:013x}-{:08x}", ts, nanos)
}

/// Sanitize a user query for FTS5 MATCH syntax.
/// Adds prefix matching (*) and escapes special FTS5 operators.
fn sanitize_fts_query(query: &str) -> String {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            // Remove FTS5 special chars
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
                .collect();
            if clean.is_empty() {
                return String::new();
            }
            // Add prefix match
            format!("\"{}\"*", clean)
        })
        .filter(|w| w.len() > 3) // skip empty results from cleaning
        .collect();

    if words.is_empty() {
        // Fallback: use original query quoted
        return format!("\"{}\"", query.replace('"', ""));
    }

    words.join(" ")
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_retrieve_object() {
        let graph = EntityGraph::in_memory().unwrap();
        let obj = UniversalObject::new(ObjectKind::Note, "My Note", "notes", "note-001")
            .with_summary("A test note")
            .with_searchable_text("This is the full content of my test note");

        let id = graph.upsert_object(&obj).unwrap();
        let retrieved = graph.get_object(&id).unwrap().unwrap();
        assert_eq!(retrieved.title, "My Note");
        assert_eq!(retrieved.kind, ObjectKind::Note);
        assert_eq!(retrieved.source_app, "notes");
    }

    #[test]
    fn upsert_updates_existing() {
        let graph = EntityGraph::in_memory().unwrap();
        let obj1 = UniversalObject::new(ObjectKind::Note, "Original", "notes", "note-001");
        let id1 = graph.upsert_object(&obj1).unwrap();

        let mut obj2 = UniversalObject::new(ObjectKind::Note, "Updated", "notes", "note-001");
        obj2.summary = "new summary".into();
        let id2 = graph.upsert_object(&obj2).unwrap();

        assert_eq!(id1, id2);
        let retrieved = graph.get_object(&id1).unwrap().unwrap();
        assert_eq!(retrieved.title, "Updated");
        assert_eq!(retrieved.summary, "new summary");
    }

    #[test]
    fn delete_object_and_relations() {
        let graph = EntityGraph::in_memory().unwrap();
        let obj1 = UniversalObject::new(ObjectKind::Note, "Note A", "notes", "a");
        let obj2 = UniversalObject::new(ObjectKind::Task, "Task B", "tasks", "b");
        let id1 = graph.upsert_object(&obj1).unwrap();
        let id2 = graph.upsert_object(&obj2).unwrap();

        graph
            .add_relation(&id1, &id2, RelationKind::References)
            .unwrap();
        assert!(graph.delete_object(&id1).unwrap());
        assert!(graph.get_object(&id1).unwrap().is_none());

        // Relations should also be deleted
        let rels = graph.get_relations(&id2).unwrap();
        assert!(rels.is_empty());
    }

    #[test]
    fn fts5_search() {
        let graph = EntityGraph::in_memory().unwrap();
        graph
            .upsert_object(
                &UniversalObject::new(ObjectKind::Note, "Meeting Notes", "notes", "n1")
                    .with_searchable_text("Discussed quarterly revenue targets and hiring plan"),
            )
            .unwrap();
        graph
            .upsert_object(
                &UniversalObject::new(ObjectKind::Thread, "Re: Budget", "email", "e1")
                    .with_searchable_text("Please review the attached budget spreadsheet"),
            )
            .unwrap();
        graph
            .upsert_object(
                &UniversalObject::new(ObjectKind::Event, "Q1 Review", "calendar", "c1")
                    .with_searchable_text("Quarterly review meeting with leadership team"),
            )
            .unwrap();

        let results = graph.search("quarterly", 10).unwrap();
        assert_eq!(results.len(), 2);

        let results = graph.search("budget", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ObjectKind::Thread);
    }

    #[test]
    fn relations_bidirectional() {
        let graph = EntityGraph::in_memory().unwrap();
        let id1 = graph
            .upsert_object(&UniversalObject::new(
                ObjectKind::Thread,
                "Email",
                "email",
                "e1",
            ))
            .unwrap();
        let id2 = graph
            .upsert_object(&UniversalObject::new(
                ObjectKind::Event,
                "Meeting",
                "calendar",
                "c1",
            ))
            .unwrap();
        let id3 = graph
            .upsert_object(&UniversalObject::new(
                ObjectKind::Note,
                "Notes",
                "notes",
                "n1",
            ))
            .unwrap();

        graph
            .add_relation(&id1, &id2, RelationKind::CreatedFrom)
            .unwrap();
        graph
            .add_relation(&id2, &id3, RelationKind::References)
            .unwrap();

        // Email should see Meeting relation
        let rels = graph.get_relations(&id1).unwrap();
        assert_eq!(rels.len(), 1);

        // Meeting should see both Email and Notes relations
        let rels = graph.get_relations(&id2).unwrap();
        assert_eq!(rels.len(), 2);
    }

    #[test]
    fn search_by_kind() {
        let graph = EntityGraph::in_memory().unwrap();
        graph
            .upsert_object(
                &UniversalObject::new(ObjectKind::Note, "Project Alpha", "notes", "n1")
                    .with_searchable_text("Alpha project details"),
            )
            .unwrap();
        graph
            .upsert_object(
                &UniversalObject::new(ObjectKind::Thread, "Re: Alpha", "email", "e1")
                    .with_searchable_text("Alpha project discussion"),
            )
            .unwrap();

        let notes = graph.search_by_kind("alpha", ObjectKind::Note, 10).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].kind, ObjectKind::Note);
    }

    #[test]
    fn stats() {
        let graph = EntityGraph::in_memory().unwrap();
        for i in 0..3 {
            graph
                .upsert_object(&UniversalObject::new(
                    ObjectKind::Note,
                    format!("Note {i}"),
                    "notes",
                    format!("n{i}"),
                ))
                .unwrap();
        }
        graph
            .upsert_object(&UniversalObject::new(
                ObjectKind::Thread,
                "Email",
                "email",
                "e1",
            ))
            .unwrap();

        assert_eq!(graph.total_objects(), 4);
        assert_eq!(graph.count_by_kind(ObjectKind::Note), 3);

        let stats = graph.stats();
        assert!(stats.iter().any(|(k, c)| k == "note" && *c == 3));
    }
}
