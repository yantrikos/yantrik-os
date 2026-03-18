//! Notes service contract — markdown note CRUD, search, tagging.

use serde::{Deserialize, Serialize};
use crate::email::ServiceError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSummary {
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub modified_at: String,
    pub created_at: String,
    pub pinned: bool,
    pub tags: Vec<String>,
    pub word_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteContent {
    pub id: String,
    pub title: String,
    pub body: String,
    pub modified_at: String,
    pub created_at: String,
    pub pinned: bool,
    pub tags: Vec<String>,
}

/// Notes service operations.
pub trait NotesService: Send + Sync {
    fn list(&self, folder: Option<&str>) -> Result<Vec<NoteSummary>, ServiceError>;
    fn get(&self, note_id: &str) -> Result<NoteContent, ServiceError>;
    fn create(&self, title: &str, body: &str, tags: Vec<String>) -> Result<NoteContent, ServiceError>;
    fn update(&self, note_id: &str, title: &str, body: &str) -> Result<(), ServiceError>;
    fn delete(&self, note_id: &str) -> Result<(), ServiceError>;
    fn set_pinned(&self, note_id: &str, pinned: bool) -> Result<(), ServiceError>;
    fn set_tags(&self, note_id: &str, tags: Vec<String>) -> Result<(), ServiceError>;
    fn search(&self, query: &str) -> Result<Vec<NoteSummary>, ServiceError>;
}
