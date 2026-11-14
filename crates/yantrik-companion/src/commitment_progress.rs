//! What a preparation has done so far, kept on the [`PreparedItem`] (#1184): one record per file
//! the promise names, the draft, and what is still to be tried.
//!
//! Only what the person would want is kept: the chosen paths, the draft, and words. No tool output,
//! no file listings. A retry reads this back, so what was already done (a file found, the draft
//! made) is not done twice. Nothing is opened for the person: they open what was found themselves.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::commitment_files::{FileKind, NamedFile, Role};

/// One file the promise names, and how far it got.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub label: String,
    /// `attach` for the thing being sent, `file` for one the promise points at.
    pub role: String,
    /// The one file chosen for it, once there is one.
    pub path: Option<String>,
    /// In the person's words: why no file was chosen.
    pub why: Option<String>,
    /// Nothing more will be tried for this file.
    pub settled: bool,
}

impl FileRecord {
    pub fn new(file: &NamedFile) -> Self {
        let role = if file.role == Role::Attach { "attach" } else { "file" };
        Self { label: file.label.clone(), role: role.into(), path: None, why: None, settled: false }
    }

    pub fn is_attachment(&self) -> bool {
        self.role == "attach"
    }

    pub fn kind(&self) -> Option<FileKind> {
        NamedFile { role: Role::Reference, label: self.label.clone() }.kind()
    }
}

/// The email draft made for the promise.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftRecord {
    pub to: String,
    pub subject: String,
    pub attached: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Progress {
    pub files: Vec<FileRecord>,
    pub draft: Option<DraftRecord>,
    /// In the person's words: why there is no draft, when there is not.
    pub draft_why: Option<String>,
    /// No more will be tried for the draft.
    pub draft_settled: bool,
    pub attempts: u32,
}

impl Progress {
    pub fn start(files: &[NamedFile]) -> Self {
        Self { files: files.iter().map(FileRecord::new).collect(), ..Self::default() }
    }

    /// Read back from an item's `data`.
    pub fn from_data(data: &Value) -> Option<Self> {
        serde_json::from_value(data.get("progress")?.clone()).ok()
    }

    /// Nothing is left to try.
    pub fn is_settled(&self) -> bool {
        self.files.iter().all(|f| f.settled) && (self.draft_settled || !self.files.iter().any(FileRecord::is_attachment))
    }

    pub fn attachment(&self) -> Option<&FileRecord> {
        self.files.iter().find(|f| f.is_attachment())
    }

    /// What did not happen, in the person's words.
    pub fn missing(&self) -> Vec<String> {
        let mut said: Vec<String> = self.files.iter().filter_map(|f| f.why.clone()).collect();
        said.extend(self.draft_why.clone());
        said
    }

    /// The `data` an item keeps.
    pub fn to_data(&self, commitment_id: i64, promisee: &str, deadline: f64, state: &str, next_try: Option<f64>) -> Value {
        let located: Vec<Value> = self
            .files
            .iter()
            .filter_map(|f| f.path.as_ref().map(|p| json!({ "label": f.label, "path": p, "role": f.role })))
            .collect();
        let attachment = self.attachment().and_then(|a| a.path.as_ref().map(|p| json!({ "label": a.label, "path": p })));
        json!({
            "commitment_id": commitment_id,
            "promisee": promisee,
            "deadline": deadline,
            "state": state,
            "next_try": next_try,
            "files": located,
            "attachment": attachment,
            "draft": self.draft.as_ref().map(|d| json!({
                "app": "email", "state": "draft", "to": d.to, "subject": d.subject, "attached": d.attached,
                "can_be_sent": false,
            })),
            "missing": self.missing(),
            "sent": false,
            "progress": self,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_round_trips_through_the_items_data_and_keeps_only_paths_and_words() {
        let files = [NamedFile { role: Role::Attach, label: "Q4 deck".into() }];
        let mut p = Progress::start(&files);
        p.files[0].path = Some("/h/Documents/Q4-deck.ydeck".into());
        let data = p.to_data(7, "Maya", 100.0, "retry", Some(5.0));
        let back = Progress::from_data(&data).unwrap();
        assert_eq!(back.files[0].path.as_deref(), Some("/h/Documents/Q4-deck.ydeck"));
        assert_eq!(data["files"][0]["path"], "/h/Documents/Q4-deck.ydeck");
        assert!(data.get("opened").is_none(), "nothing is opened for the person");
        assert_eq!(data["sent"], false);
        assert!(data.get("steps").is_none(), "no tool output is kept");
    }
}
