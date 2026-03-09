//! Living Workspace Orchestration — intent-based contextual workspaces.
//!
//! Instead of launching individual apps, Yantrik assembles contextual workspaces
//! by intent. A workspace configures:
//! - Relevant files and documents
//! - People involved (from world model)
//! - Pending commitments (from commitment engine)
//! - Communication threads
//! - Proactive settings (mode override)
//! - Companion mode (focus, social, sleep)
//!
//! Examples:
//! - "board meeting prep" → docs, participant summaries, pending commitments
//! - "deep work block" → minimize interruptions, relevant files, focus mode
//! - "travel day" → offline bundles, maps, itinerary
//! - "weekly review" → commitment summary, calendar recap, pending items
//!
//! Workspaces can be:
//! - User-created (explicit)
//! - System-suggested (from calendar, routine detection)
//! - Auto-assembled (from recent activity patterns)

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Workspace Definition ────────────────────────────────────────────────────

/// A contextual workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub description: String,
    /// What kind of workspace.
    pub kind: WorkspaceKind,
    /// Relevant file paths or document references.
    pub files: Vec<String>,
    /// People involved (names or IDs from world model).
    pub people: Vec<String>,
    /// Commitment IDs relevant to this workspace.
    pub commitment_ids: Vec<String>,
    /// Communication thread references (email threads, chat IDs).
    pub threads: Vec<String>,
    /// Companion mode override when this workspace is active.
    pub mode_override: Option<CompanionMode>,
    /// Tags for matching (calendar keywords, routine patterns).
    pub tags: Vec<String>,
    /// How this workspace was created.
    pub source: WorkspaceSource,
    /// Is this workspace currently active?
    pub active: bool,
    pub created_at: f64,
    pub last_activated_at: Option<f64>,
    pub activation_count: u32,
}

/// How a workspace was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceSource {
    /// User explicitly created it.
    UserCreated,
    /// System suggested based on calendar/routine.
    SystemSuggested,
    /// Auto-assembled from recent activity.
    AutoAssembled,
}

impl WorkspaceSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UserCreated => "user_created",
            Self::SystemSuggested => "system_suggested",
            Self::AutoAssembled => "auto_assembled",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "user_created" => Self::UserCreated,
            "system_suggested" => Self::SystemSuggested,
            "auto_assembled" => Self::AutoAssembled,
            _ => Self::UserCreated,
        }
    }
}

/// Types of workspaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceKind {
    /// Meeting preparation.
    MeetingPrep,
    /// Deep work / focus session.
    DeepWork,
    /// Travel planning / execution.
    Travel,
    /// Review (weekly, monthly, etc.).
    Review,
    /// Project-specific workspace.
    Project,
    /// General-purpose custom workspace.
    Custom,
}

impl WorkspaceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MeetingPrep => "meeting_prep",
            Self::DeepWork => "deep_work",
            Self::Travel => "travel",
            Self::Review => "review",
            Self::Project => "project",
            Self::Custom => "custom",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "meeting_prep" => Self::MeetingPrep,
            "deep_work" => Self::DeepWork,
            "travel" => Self::Travel,
            "review" => Self::Review,
            "project" => Self::Project,
            _ => Self::Custom,
        }
    }
}

// ── Companion Modes ─────────────────────────────────────────────────────────

/// Companion operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompanionMode {
    /// Minimal interruptions, task-oriented, only critical alerts.
    Focus,
    /// Chatty, proactive suggestions enabled, full personality.
    Social,
    /// Only critical system alerts, no proactive behavior.
    Sleep,
}

impl CompanionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Social => "social",
            Self::Sleep => "sleep",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "focus" => Self::Focus,
            "social" => Self::Social,
            "sleep" => Self::Sleep,
            _ => Self::Social,
        }
    }

    /// Proactive delivery threshold override.
    pub fn proactive_threshold(&self) -> f64 {
        match self {
            Self::Focus => 0.9, // Only very urgent
            Self::Social => 0.3, // Low bar, chatty
            Self::Sleep => 1.5,  // Effectively blocks all proactive
        }
    }

    /// Personality warmth multiplier (affects template selection).
    pub fn warmth_multiplier(&self) -> f64 {
        match self {
            Self::Focus => 0.5, // Terse, efficient
            Self::Social => 1.2, // Warm, friendly
            Self::Sleep => 0.3,  // Minimal
        }
    }

    /// Which delivery channels are allowed.
    pub fn allowed_channels(&self) -> Vec<&'static str> {
        match self {
            Self::Focus => vec!["lock_screen", "badge"],
            Self::Social => vec!["whisper_card", "badge", "ambient", "lock_screen"],
            Self::Sleep => vec!["lock_screen"], // Critical only
        }
    }
}

// ── Workspace Manager ───────────────────────────────────────────────────────

pub struct WorkspaceManager;

impl WorkspaceManager {
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workspaces (
                id              TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                description     TEXT NOT NULL DEFAULT '',
                kind            TEXT NOT NULL DEFAULT 'custom',
                files           TEXT NOT NULL DEFAULT '[]',
                people          TEXT NOT NULL DEFAULT '[]',
                commitment_ids  TEXT NOT NULL DEFAULT '[]',
                threads         TEXT NOT NULL DEFAULT '[]',
                mode_override   TEXT,
                tags            TEXT NOT NULL DEFAULT '[]',
                source          TEXT NOT NULL DEFAULT 'user_created',
                active          INTEGER NOT NULL DEFAULT 0,
                created_at      REAL NOT NULL,
                last_activated_at REAL,
                activation_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_ws_active ON workspaces(active);
            CREATE INDEX IF NOT EXISTS idx_ws_kind ON workspaces(kind);
            CREATE INDEX IF NOT EXISTS idx_ws_activated ON workspaces(last_activated_at);",
        )
        .expect("failed to create workspaces table");
    }

    /// Create a new workspace.
    pub fn create(conn: &Connection, workspace: &Workspace) {
        let files = serde_json::to_string(&workspace.files).unwrap_or_default();
        let people = serde_json::to_string(&workspace.people).unwrap_or_default();
        let commitments = serde_json::to_string(&workspace.commitment_ids).unwrap_or_default();
        let threads = serde_json::to_string(&workspace.threads).unwrap_or_default();
        let tags = serde_json::to_string(&workspace.tags).unwrap_or_default();
        let mode = workspace.mode_override.map(|m| m.as_str().to_string());

        let _ = conn.execute(
            "INSERT OR REPLACE INTO workspaces
             (id, name, description, kind, files, people, commitment_ids, threads,
              mode_override, tags, source, active, created_at, last_activated_at, activation_count)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                workspace.id, workspace.name, workspace.description,
                workspace.kind.as_str(), files, people, commitments, threads,
                mode, tags, workspace.source.as_str(),
                workspace.active as i32, workspace.created_at,
                workspace.last_activated_at, workspace.activation_count,
            ],
        );
    }

    /// Get a workspace by ID.
    pub fn get(conn: &Connection, id: &str) -> Option<Workspace> {
        conn.query_row(
            "SELECT id, name, description, kind, files, people, commitment_ids, threads,
                    mode_override, tags, source, active, created_at, last_activated_at, activation_count
             FROM workspaces WHERE id = ?1",
            params![id],
            Self::row_to_workspace,
        ).ok()
    }

    /// Get the currently active workspace.
    pub fn active(conn: &Connection) -> Option<Workspace> {
        conn.query_row(
            "SELECT id, name, description, kind, files, people, commitment_ids, threads,
                    mode_override, tags, source, active, created_at, last_activated_at, activation_count
             FROM workspaces WHERE active = 1 LIMIT 1",
            [],
            Self::row_to_workspace,
        ).ok()
    }

    /// List all workspaces.
    pub fn list(conn: &Connection) -> Vec<Workspace> {
        let mut stmt = match conn.prepare(
            "SELECT id, name, description, kind, files, people, commitment_ids, threads,
                    mode_override, tags, source, active, created_at, last_activated_at, activation_count
             FROM workspaces ORDER BY last_activated_at DESC NULLS LAST",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map([], Self::row_to_workspace)
            .ok()
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Activate a workspace (deactivate all others first).
    pub fn activate(conn: &Connection, id: &str) {
        let now = now_ts();
        // Deactivate all
        let _ = conn.execute("UPDATE workspaces SET active = 0", []);
        // Activate the target
        let _ = conn.execute(
            "UPDATE workspaces SET active = 1, last_activated_at = ?1, activation_count = activation_count + 1
             WHERE id = ?2",
            params![now, id],
        );
    }

    /// Deactivate all workspaces.
    pub fn deactivate_all(conn: &Connection) {
        let _ = conn.execute("UPDATE workspaces SET active = 0", []);
    }

    /// Delete a workspace.
    pub fn delete(conn: &Connection, id: &str) {
        let _ = conn.execute("DELETE FROM workspaces WHERE id = ?1", params![id]);
    }

    /// Find workspaces matching keywords (for auto-suggestion from calendar events).
    pub fn find_matching(conn: &Connection, keywords: &[&str]) -> Vec<Workspace> {
        let all = Self::list(conn);
        all.into_iter()
            .filter(|ws| {
                let searchable = format!(
                    "{} {} {} {}",
                    ws.name.to_lowercase(),
                    ws.description.to_lowercase(),
                    ws.tags.join(" ").to_lowercase(),
                    ws.kind.as_str(),
                );
                keywords.iter().any(|kw| searchable.contains(&kw.to_lowercase()))
            })
            .collect()
    }

    /// Create a workspace template for a meeting.
    pub fn meeting_template(
        meeting_title: &str,
        participants: &[String],
        commitment_ids: &[String],
    ) -> Workspace {
        Workspace {
            id: format!("ws-meet-{}", &uuid7::uuid7().to_string()[..8]),
            name: format!("Meeting: {}", meeting_title),
            description: format!("Prep workspace for: {}", meeting_title),
            kind: WorkspaceKind::MeetingPrep,
            files: Vec::new(),
            people: participants.to_vec(),
            commitment_ids: commitment_ids.to_vec(),
            threads: Vec::new(),
            mode_override: Some(CompanionMode::Focus),
            tags: vec!["meeting".into(), meeting_title.to_lowercase()],
            source: WorkspaceSource::SystemSuggested,
            active: false,
            created_at: now_ts(),
            last_activated_at: None,
            activation_count: 0,
        }
    }

    /// Create a deep work template.
    pub fn deep_work_template(project_name: &str, files: &[String]) -> Workspace {
        Workspace {
            id: format!("ws-focus-{}", &uuid7::uuid7().to_string()[..8]),
            name: format!("Deep Work: {}", project_name),
            description: format!("Focus session for {}", project_name),
            kind: WorkspaceKind::DeepWork,
            files: files.to_vec(),
            people: Vec::new(),
            commitment_ids: Vec::new(),
            threads: Vec::new(),
            mode_override: Some(CompanionMode::Focus),
            tags: vec!["deep_work".into(), "focus".into(), project_name.to_lowercase()],
            source: WorkspaceSource::SystemSuggested,
            active: false,
            created_at: now_ts(),
            last_activated_at: None,
            activation_count: 0,
        }
    }

    /// Create a weekly review template.
    pub fn review_template(commitment_ids: &[String]) -> Workspace {
        Workspace {
            id: format!("ws-review-{}", &uuid7::uuid7().to_string()[..8]),
            name: "Weekly Review".into(),
            description: "Review commitments, calendar, and pending items".into(),
            kind: WorkspaceKind::Review,
            files: Vec::new(),
            people: Vec::new(),
            commitment_ids: commitment_ids.to_vec(),
            threads: Vec::new(),
            mode_override: Some(CompanionMode::Social),
            tags: vec!["review".into(), "weekly".into()],
            source: WorkspaceSource::SystemSuggested,
            active: false,
            created_at: now_ts(),
            last_activated_at: None,
            activation_count: 0,
        }
    }

    fn row_to_workspace(row: &rusqlite::Row<'_>) -> rusqlite::Result<Workspace> {
        let files: String = row.get(4)?;
        let people: String = row.get(5)?;
        let commitments: String = row.get(6)?;
        let threads: String = row.get(7)?;
        let mode: Option<String> = row.get(8)?;
        let tags: String = row.get(9)?;

        Ok(Workspace {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            kind: WorkspaceKind::from_str(&row.get::<_, String>(3)?),
            files: serde_json::from_str(&files).unwrap_or_default(),
            people: serde_json::from_str(&people).unwrap_or_default(),
            commitment_ids: serde_json::from_str(&commitments).unwrap_or_default(),
            threads: serde_json::from_str(&threads).unwrap_or_default(),
            mode_override: mode.map(|m| CompanionMode::from_str(&m)),
            tags: serde_json::from_str(&tags).unwrap_or_default(),
            source: WorkspaceSource::from_str(&row.get::<_, String>(10)?),
            active: row.get::<_, i32>(11)? != 0,
            created_at: row.get(12)?,
            last_activated_at: row.get(13)?,
            activation_count: row.get::<_, i32>(14)? as u32,
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
        WorkspaceManager::ensure_table(&conn);
        conn
    }

    #[test]
    fn create_and_get() {
        let conn = setup();
        let ws = Workspace {
            id: "ws1".into(),
            name: "Test Workspace".into(),
            description: "A test".into(),
            kind: WorkspaceKind::Custom,
            files: vec!["/tmp/file.txt".into()],
            people: vec!["Alice".into()],
            commitment_ids: vec!["c1".into()],
            threads: Vec::new(),
            mode_override: Some(CompanionMode::Focus),
            tags: vec!["test".into()],
            source: WorkspaceSource::UserCreated,
            active: false,
            created_at: now_ts(),
            last_activated_at: None,
            activation_count: 0,
        };

        WorkspaceManager::create(&conn, &ws);
        let loaded = WorkspaceManager::get(&conn, "ws1").unwrap();
        assert_eq!(loaded.name, "Test Workspace");
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(loaded.people, vec!["Alice"]);
        assert_eq!(loaded.mode_override, Some(CompanionMode::Focus));
    }

    #[test]
    fn activate_deactivate() {
        let conn = setup();
        let ws1 = Workspace {
            id: "ws1".into(), name: "WS1".into(), description: "".into(),
            kind: WorkspaceKind::Custom, files: vec![], people: vec![],
            commitment_ids: vec![], threads: vec![], mode_override: None,
            tags: vec![], source: WorkspaceSource::UserCreated,
            active: false, created_at: now_ts(), last_activated_at: None, activation_count: 0,
        };
        let ws2 = Workspace { id: "ws2".into(), name: "WS2".into(), ..ws1.clone() };

        WorkspaceManager::create(&conn, &ws1);
        WorkspaceManager::create(&conn, &ws2);

        WorkspaceManager::activate(&conn, "ws1");
        let active = WorkspaceManager::active(&conn).unwrap();
        assert_eq!(active.id, "ws1");
        assert_eq!(active.activation_count, 1);

        // Activating ws2 deactivates ws1
        WorkspaceManager::activate(&conn, "ws2");
        let active = WorkspaceManager::active(&conn).unwrap();
        assert_eq!(active.id, "ws2");

        // ws1 is no longer active
        let ws1_loaded = WorkspaceManager::get(&conn, "ws1").unwrap();
        assert!(!ws1_loaded.active);

        WorkspaceManager::deactivate_all(&conn);
        assert!(WorkspaceManager::active(&conn).is_none());
    }

    #[test]
    fn meeting_template_creation() {
        let ws = WorkspaceManager::meeting_template(
            "Q1 Board Review",
            &["Alice".into(), "Bob".into()],
            &["c1".into(), "c2".into()],
        );
        assert_eq!(ws.kind, WorkspaceKind::MeetingPrep);
        assert_eq!(ws.people.len(), 2);
        assert_eq!(ws.mode_override, Some(CompanionMode::Focus));
        assert!(ws.tags.contains(&"meeting".to_string()));
    }

    #[test]
    fn keyword_matching() {
        let conn = setup();

        let ws = Workspace {
            id: "ws1".into(), name: "Project Alpha Sprint".into(),
            description: "Sprint planning for Alpha".into(),
            kind: WorkspaceKind::Project, files: vec![], people: vec![],
            commitment_ids: vec![], threads: vec![], mode_override: None,
            tags: vec!["alpha".into(), "sprint".into()],
            source: WorkspaceSource::UserCreated,
            active: false, created_at: now_ts(), last_activated_at: None, activation_count: 0,
        };
        WorkspaceManager::create(&conn, &ws);

        let matches = WorkspaceManager::find_matching(&conn, &["alpha"]);
        assert_eq!(matches.len(), 1);

        let matches = WorkspaceManager::find_matching(&conn, &["beta"]);
        assert!(matches.is_empty());
    }

    #[test]
    fn companion_modes() {
        assert!(CompanionMode::Focus.proactive_threshold() > CompanionMode::Social.proactive_threshold());
        assert!(CompanionMode::Sleep.proactive_threshold() > CompanionMode::Focus.proactive_threshold());

        assert!(CompanionMode::Social.allowed_channels().len() > CompanionMode::Focus.allowed_channels().len());
        assert!(CompanionMode::Sleep.allowed_channels().len() < CompanionMode::Focus.allowed_channels().len());
    }

    #[test]
    fn delete_workspace() {
        let conn = setup();
        let ws = Workspace {
            id: "ws1".into(), name: "Temp".into(), description: "".into(),
            kind: WorkspaceKind::Custom, files: vec![], people: vec![],
            commitment_ids: vec![], threads: vec![], mode_override: None,
            tags: vec![], source: WorkspaceSource::UserCreated,
            active: false, created_at: now_ts(), last_activated_at: None, activation_count: 0,
        };
        WorkspaceManager::create(&conn, &ws);
        assert!(WorkspaceManager::get(&conn, "ws1").is_some());

        WorkspaceManager::delete(&conn, "ws1");
        assert!(WorkspaceManager::get(&conn, "ws1").is_none());
    }
}
