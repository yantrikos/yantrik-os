//! The JSON a journal entry and a Trash record hold: the shape Notes has always written, so a
//! vault written by either build is read by the other.

use serde::{Deserialize, Serialize};

use crate::note::{Note, NoteText};

/// One note as a record. `baseline` and `baseline_meta` are what its file held when the record
/// was written (`None` for a note never saved); they are kept for the format and read by nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub text: String,
    pub meta: String,
    pub baseline: Option<String>,
    pub baseline_meta: Option<String>,
    pub modified: u64,
    pub trash: bool,
}

impl Record {
    pub fn of(id: &str, content: &NoteText, on_disk: Option<&NoteText>, modified: u64) -> Record {
        Record {
            id: id.into(),
            text: content.text.clone(),
            meta: content.meta.clone(),
            baseline: on_disk.map(|d| d.text.clone()),
            baseline_meta: on_disk.map(|d| d.meta.clone()),
            modified,
            trash: false,
        }
    }

    pub fn content(&self) -> NoteText {
        NoteText { text: self.text.clone(), meta: self.meta.clone() }
    }

    pub fn json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    /// The library's entry for a record in the Trash.
    pub fn into_trashed(self) -> Note {
        Note {
            content: self.content(),
            id: self.id,
            modified: self.modified,
            trash: true,
            stored: true,
        }
    }
}
