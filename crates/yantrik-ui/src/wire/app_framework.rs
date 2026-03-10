//! Shared App Framework — persistent state, background jobs, app policies.
//!
//! Provides cross-cutting infrastructure consumed by all app wire modules:
//! - **AppState**: Key-value persistent storage per app (replaces ad-hoc .meta files)
//! - **BackgroundJobManager**: Submit async work, get notified via event bus
//! - **AppPolicy**: Per-app behavior configuration (sync, notifications, retention)

use std::sync::{Arc, Mutex};

use serde::{de::DeserializeOwned, Serialize};

// ────────────────────────────────────────────────────────────────────────────
// AppState — persistent KV store per app
// ────────────────────────────────────────────────────────────────────────────

/// Persistent key-value store shared across all apps.
/// Each app namespaces its keys by app name.
/// Thread-safe via internal Mutex.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Mutex<AppStateInner>>,
}

struct AppStateInner {
    conn: rusqlite::Connection,
}

impl AppState {
    /// Open or create the app state database.
    pub fn open(path: &str) -> Result<Self, rusqlite::Error> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;

             CREATE TABLE IF NOT EXISTS app_state (
                 app        TEXT NOT NULL,
                 key        TEXT NOT NULL,
                 value      TEXT NOT NULL,
                 updated_at REAL NOT NULL,
                 PRIMARY KEY (app, key)
             );",
        )?;
        tracing::info!("AppState opened at {path}");
        Ok(Self {
            inner: Arc::new(Mutex::new(AppStateInner { conn })),
        })
    }

    /// Open an in-memory store (for testing).
    pub fn in_memory() -> Result<Self, rusqlite::Error> {
        Self::open(":memory:")
    }

    /// Get a string value.
    pub fn get(&self, app: &str, key: &str) -> Option<String> {
        let inner = self.inner.lock().ok()?;
        inner
            .conn
            .query_row(
                "SELECT value FROM app_state WHERE app = ?1 AND key = ?2",
                rusqlite::params![app, key],
                |row| row.get(0),
            )
            .ok()
    }

    /// Set a string value.
    pub fn set(&self, app: &str, key: &str, value: &str) {
        if let Ok(inner) = self.inner.lock() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();
            let _ = inner.conn.execute(
                "INSERT OR REPLACE INTO app_state (app, key, value, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![app, key, value, now],
            );
        }
    }

    /// Get a JSON-deserialized value.
    pub fn get_json<T: DeserializeOwned>(&self, app: &str, key: &str) -> Option<T> {
        let raw = self.get(app, key)?;
        serde_json::from_str(&raw).ok()
    }

    /// Set a JSON-serialized value.
    pub fn set_json<T: Serialize>(&self, app: &str, key: &str, value: &T) {
        if let Ok(json) = serde_json::to_string(value) {
            self.set(app, key, &json);
        }
    }

    /// Delete a key.
    pub fn delete(&self, app: &str, key: &str) {
        if let Ok(inner) = self.inner.lock() {
            let _ = inner.conn.execute(
                "DELETE FROM app_state WHERE app = ?1 AND key = ?2",
                rusqlite::params![app, key],
            );
        }
    }

    /// List all keys for an app.
    pub fn keys(&self, app: &str) -> Vec<String> {
        let inner = match self.inner.lock() {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        let mut stmt = match inner
            .conn
            .prepare("SELECT key FROM app_state WHERE app = ?1 ORDER BY key")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(rusqlite::params![app], |row| row.get(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    /// Delete all keys for an app.
    pub fn clear_app(&self, app: &str) {
        if let Ok(inner) = self.inner.lock() {
            let _ = inner.conn.execute(
                "DELETE FROM app_state WHERE app = ?1",
                rusqlite::params![app],
            );
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// BackgroundJobManager
// ────────────────────────────────────────────────────────────────────────────

/// A background job to execute on the worker thread.
pub struct BackgroundJob {
    pub id: String,
    pub app: String,
    pub description: String,
    pub task: Box<dyn FnOnce() -> String + Send>,
}

/// Manages a queue of background jobs executed on a dedicated worker thread.
/// Results are reported via the event bus.
#[derive(Clone)]
pub struct BackgroundJobManager {
    sender: crossbeam_channel::Sender<BackgroundJob>,
}

impl BackgroundJobManager {
    /// Create a new job manager with a worker thread.
    /// The `on_complete` callback is invoked with (job_id, app, result) when a job finishes.
    pub fn start(
        on_complete: impl Fn(String, String, String) + Send + 'static,
    ) -> Self {
        let (tx, rx) = crossbeam_channel::bounded::<BackgroundJob>(64);

        std::thread::Builder::new()
            .name("bg-jobs".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let id = job.id.clone();
                    let app = job.app.clone();
                    tracing::debug!(job_id = %id, app = %app, desc = %job.description, "Running background job");
                    let result = (job.task)();
                    on_complete(id, app, result);
                }
            })
            .expect("Failed to spawn background job worker");

        Self { sender: tx }
    }

    /// Submit a job for background execution.
    pub fn submit(&self, job: BackgroundJob) {
        if let Err(e) = self.sender.try_send(job) {
            tracing::warn!("Background job queue full or disconnected: {e}");
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// AppPolicy
// ────────────────────────────────────────────────────────────────────────────

/// Per-app behavior policy, stored in AppState.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct AppPolicy {
    /// Allow the app to auto-sync data in the background.
    pub auto_sync_enabled: bool,
    /// Allow background data fetching.
    pub background_fetch: bool,
    /// Notification level: "all", "important", "none".
    pub notification_level: String,
    /// Auto-cleanup data older than this many days (0 = never).
    pub data_retention_days: i32,
}

impl Default for AppPolicy {
    fn default() -> Self {
        Self {
            auto_sync_enabled: true,
            background_fetch: true,
            notification_level: "all".into(),
            data_retention_days: 0,
        }
    }
}

impl AppPolicy {
    /// Load policy for an app from AppState, or return defaults.
    pub fn load(state: &AppState, app: &str) -> Self {
        state
            .get_json::<Self>(app, "policy")
            .unwrap_or_default()
    }

    /// Save policy to AppState.
    pub fn save(&self, state: &AppState, app: &str) {
        state.set_json(app, "policy", self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_state_basic_crud() {
        let state = AppState::in_memory().unwrap();
        assert!(state.get("notes", "last_folder").is_none());

        state.set("notes", "last_folder", "favorites");
        assert_eq!(state.get("notes", "last_folder").unwrap(), "favorites");

        state.delete("notes", "last_folder");
        assert!(state.get("notes", "last_folder").is_none());
    }

    #[test]
    fn app_state_json() {
        let state = AppState::in_memory().unwrap();
        let tags = vec!["rust", "yantrik", "os"];
        state.set_json("notes", "recent_tags", &tags);

        let loaded: Vec<String> = state.get_json("notes", "recent_tags").unwrap();
        assert_eq!(loaded, vec!["rust", "yantrik", "os"]);
    }

    #[test]
    fn app_state_keys() {
        let state = AppState::in_memory().unwrap();
        state.set("email", "draft_count", "3");
        state.set("email", "last_sync", "2026-03-10");
        state.set("calendar", "view", "week");

        let email_keys = state.keys("email");
        assert_eq!(email_keys.len(), 2);
        assert!(email_keys.contains(&"draft_count".to_string()));
    }

    #[test]
    fn app_policy_load_save() {
        let state = AppState::in_memory().unwrap();
        let mut policy = AppPolicy::default();
        policy.notification_level = "important".into();
        policy.data_retention_days = 30;
        policy.save(&state, "email");

        let loaded = AppPolicy::load(&state, "email");
        assert_eq!(loaded.notification_level, "important");
        assert_eq!(loaded.data_retention_days, 30);
    }
}
