//! Notification service contract — toast/notification delivery.

use serde::{Deserialize, Serialize};
use crate::email::ServiceError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub title: String,
    pub body: String,
    pub icon: Option<String>,
    pub urgency: Urgency,
    pub source_app: String,
    pub timestamp: String,
    pub actions: Vec<NotificationAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

/// Notification service operations (shell-provided, consumed by apps).
pub trait NotificationService: Send + Sync {
    fn notify(&self, notification: Notification) -> Result<String, ServiceError>;
    fn dismiss(&self, notification_id: &str) -> Result<(), ServiceError>;
    fn list_recent(&self, limit: u32) -> Result<Vec<Notification>, ServiceError>;
    fn clear_all(&self) -> Result<(), ServiceError>;
}
