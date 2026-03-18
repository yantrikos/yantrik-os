//! Email service contract — IMAP/SMTP operations.

use serde::{Deserialize, Serialize};

/// An email message summary (for list views).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailSummary {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub snippet: String,
    pub date: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub folder: String,
    pub thread_id: Option<String>,
}

/// Full email detail (for reading view).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailDetail {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_html: String,
    pub body_text: String,
    pub date: String,
    pub attachments: Vec<EmailAttachment>,
    pub thread_messages: Vec<EmailThreadEntry>,
}

/// An email attachment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAttachment {
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

/// A message within a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailThreadEntry {
    pub id: String,
    pub from: String,
    pub date: String,
    pub snippet: String,
}

/// An email folder (IMAP mailbox).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailFolder {
    pub name: String,
    pub unread_count: i32,
    pub total_count: i32,
}

/// Compose/send request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeRequest {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub reply_to_id: Option<String>,
    pub signature: Option<String>,
}

/// Email service operations.
pub trait EmailService: Send + Sync {
    fn list_folders(&self, account_id: &str) -> Result<Vec<EmailFolder>, ServiceError>;
    fn list_messages(&self, account_id: &str, folder: &str, page: u32, per_page: u32) -> Result<Vec<EmailSummary>, ServiceError>;
    fn get_message(&self, account_id: &str, message_id: &str) -> Result<EmailDetail, ServiceError>;
    fn send_message(&self, account_id: &str, compose: ComposeRequest) -> Result<(), ServiceError>;
    fn mark_read(&self, account_id: &str, message_id: &str, read: bool) -> Result<(), ServiceError>;
    fn mark_starred(&self, account_id: &str, message_id: &str, starred: bool) -> Result<(), ServiceError>;
    fn move_message(&self, account_id: &str, message_id: &str, target_folder: &str) -> Result<(), ServiceError>;
    fn delete_message(&self, account_id: &str, message_id: &str) -> Result<(), ServiceError>;
    fn search(&self, account_id: &str, query: &str) -> Result<Vec<EmailSummary>, ServiceError>;
}

/// Shared error type for all services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceError {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}
