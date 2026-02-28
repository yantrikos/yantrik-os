//! Notification store — persistent in-memory notification history.
//!
//! Captures every D-Bus notification (via SystemObserver events) into a
//! capped ring buffer. Powers the Notification Center (screen 9).

use std::cell::RefCell;
use std::rc::Rc;

/// A single notification entry.
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

impl NotificationStore {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            counter: 0,
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
    }

    /// Get all entries, newest first.
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
    }

    /// Mark a specific notification as read by ID.
    pub fn mark_read(&mut self, id: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.read = true;
        }
    }

    /// Clear all notifications.
    pub fn clear(&mut self) {
        self.entries.clear();
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
    }
}

/// Sync the full notification list to the UI.
pub fn sync_to_ui(store: &NotificationStore, ui_weak: &slint::Weak<crate::App>) {
    if let Some(ui) = ui_weak.upgrade() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let items: Vec<crate::NotificationData> = store
            .entries_newest_first()
            .map(|e| to_slint_data(e, now))
            .collect();
        ui.set_notification_unread_count(store.unread_count() as i32);
        ui.set_notification_list(slint::ModelRc::new(slint::VecModel::from(items)));
    }
}
