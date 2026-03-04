//! Notification store — persistent notification history.
//!
//! Captures every D-Bus notification (via SystemObserver events) into a
//! capped ring buffer. Powers the Notification Center (screen 9).
//!
//! Notifications are persisted to `~/.yantrik/notifications.json` so they
//! survive reboots. The store sorts entries by app-name for grouped display,
//! inserting synthetic "group header" rows that the UI renders differently.

use std::cell::RefCell;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

/// A single notification entry (also the on-disk representation).
#[derive(Clone, Serialize, Deserialize)]
pub struct NotificationEntry {
    pub id: u64,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub urgency: u8,
    pub timestamp: f64,
    pub read: bool,
}

/// In-memory notification store, kept on main thread (Rc<RefCell>).
pub struct NotificationStore {
    entries: Vec<NotificationEntry>,
    counter: u64,
}

/// Shared handle to the notification store.
pub type SharedStore = Rc<RefCell<NotificationStore>>;

/// Path to the persistence file.
fn persist_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let dir = std::path::PathBuf::from(home).join(".yantrik");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("notifications.json")
}

impl NotificationStore {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            counter: 0,
        }
    }

    /// Load from disk, falling back to empty on any error.
    pub fn load() -> Self {
        let path = persist_path();
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<Vec<NotificationEntry>>(&json) {
                Ok(entries) => {
                    let counter = entries.iter().map(|e| e.id).max().unwrap_or(0);
                    tracing::info!(
                        count = entries.len(),
                        path = %path.display(),
                        "Loaded notifications from disk"
                    );
                    Self { entries, counter }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to parse notifications.json, starting fresh");
                    Self::new()
                }
            },
            Err(_) => Self::new(),
        }
    }

    /// Save current entries to disk.
    fn persist(&self) {
        let path = persist_path();
        match serde_json::to_string(&self.entries) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!(error = %e, "Failed to write notifications.json");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to serialize notifications");
            }
        }
    }

    /// Add a new notification.
    pub fn push(&mut self, app: String, summary: String, body: String, urgency: u8) {
        self.counter += 1;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        self.entries.push(NotificationEntry {
            id: self.counter,
            app,
            summary,
            body,
            urgency,
            timestamp,
            read: false,
        });
        // Cap at 200 entries (oldest removed first)
        if self.entries.len() > 200 {
            self.entries.remove(0);
        }
        self.persist();
    }

    /// Get all entries grouped by app-name (sorted alphabetically by app,
    /// newest-first within each group).
    pub fn entries_grouped(&self) -> Vec<&NotificationEntry> {
        // Collect entries into groups keyed by app name
        let mut groups: std::collections::BTreeMap<String, Vec<&NotificationEntry>> =
            std::collections::BTreeMap::new();
        for e in &self.entries {
            groups
                .entry(e.app.to_lowercase())
                .or_default()
                .push(e);
        }
        // Within each group, sort newest first
        let mut result = Vec::new();
        for (_key, mut group) in groups {
            group.sort_by(|a, b| b.timestamp.partial_cmp(&a.timestamp).unwrap());
            result.extend(group);
        }
        result
    }

    /// Get all entries, newest first (ungrouped — kept for backward compat).
    pub fn entries_newest_first(&self) -> impl Iterator<Item = &NotificationEntry> {
        self.entries.iter().rev()
    }

    /// Count of unread notifications.
    pub fn unread_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.read).count()
    }

    /// Mark all as read.
    pub fn mark_all_read(&mut self) {
        for e in &mut self.entries {
            e.read = true;
        }
        self.persist();
    }

    /// Mark a specific notification as read by ID.
    pub fn mark_read(&mut self, id: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.read = true;
        }
        self.persist();
    }

    /// Clear all notifications for a specific app name.
    pub fn clear_group(&mut self, app_name: &str) {
        let lower = app_name.to_lowercase();
        self.entries.retain(|e| e.app.to_lowercase() != lower);
        self.persist();
    }

    /// Clear all notifications.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.persist();
    }
}

/// Convert a NotificationEntry to a Slint NotificationData struct.
pub fn to_slint_data(entry: &NotificationEntry, now: f64) -> crate::NotificationData {
    crate::NotificationData {
        id: entry.id.to_string().into(),
        app_name: entry.app.clone().into(),
        summary: entry.summary.clone().into(),
        body: entry.body.clone().into(),
        urgency: entry.urgency as i32,
        time_ago: crate::bridge::format_time_ago(now - entry.timestamp).into(),
        is_read: entry.read,
        is_group_header: false,
        group_name: entry.app.clone().into(),
        group_count: 0,
    }
}

/// Sync the full notification list to the UI, grouped by app-name with
/// synthetic group-header rows inserted before each group.
pub fn sync_to_ui(store: &NotificationStore, ui_weak: &slint::Weak<crate::App>) {
    if let Some(ui) = ui_weak.upgrade() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let grouped = store.entries_grouped();

        let mut items: Vec<crate::NotificationData> = Vec::new();
        let mut current_app: Option<String> = None;

        for entry in &grouped {
            let app_lower = entry.app.to_lowercase();
            if current_app.as_ref() != Some(&app_lower) {
                // Count notifications in this group
                let group_count = grouped
                    .iter()
                    .filter(|e| e.app.to_lowercase() == app_lower)
                    .count();

                // Insert group header
                items.push(crate::NotificationData {
                    id: slint::SharedString::default(),
                    app_name: entry.app.clone().into(),
                    summary: entry.app.clone().into(),
                    body: format!("{} notification{}", group_count, if group_count == 1 { "" } else { "s" }).into(),
                    urgency: 0,
                    time_ago: slint::SharedString::default(),
                    is_read: true,
                    is_group_header: true,
                    group_name: entry.app.clone().into(),
                    group_count: group_count as i32,
                });
                current_app = Some(app_lower);
            }
            items.push(to_slint_data(entry, now));
        }

        ui.set_notification_unread_count(store.unread_count() as i32);
        ui.set_notification_list(slint::ModelRc::new(slint::VecModel::from(items)));
    }
}
