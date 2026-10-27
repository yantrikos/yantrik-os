//! What the document's draft holds: the app's own schema, written through the framework's draft
//! lane (`fx.save_draft`) and read back from `Boot::drafts` at the next start.
//!
//! A draft holds the document's text, the file it is a draft of, and that file's text as the
//! document last read or wrote it, so a recovered document is unsaved against the right baseline
//! and a moved file can still be looked for.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::limits::validate;

/// The schema's version.
pub const VERSION: u32 = 1;

/// The document's draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocDraft {
    pub version: u32,
    /// The document's file, if it has one.
    pub path: Option<PathBuf>,
    pub text: String,
    /// The file's text as the document last read or wrote it.
    pub on_disk: Option<String>,
    /// When the draft was made, in milliseconds: the newest is the one a start opens.
    pub at: u64,
}

impl DocDraft {
    pub fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// A draft read back, if it is one this app wrote and both its texts are documents.
    pub fn parse(bytes: &[u8]) -> Result<DocDraft, String> {
        let draft: DocDraft = serde_json::from_slice(bytes).map_err(|e| format!("it could not be read: {e}"))?;
        if draft.version != VERSION {
            return Err(format!("it was written by another version of yDoc ({})", draft.version));
        }
        validate(&draft.text)?;
        if let Some(on_disk) = &draft.on_disk {
            validate(on_disk)?;
        }
        if draft.path.as_ref().is_some_and(|p| !p.is_absolute()) {
            return Err("its file is not an absolute path".into());
        }
        Ok(draft)
    }
}

/// Milliseconds since the epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_round_trips_and_a_bad_one_is_refused() {
        let d = DocDraft {
            version: VERSION,
            path: Some("/home/p/a.md".into()),
            text: "draft 🦀".into(),
            on_disk: Some("disk".into()),
            at: 2,
        };
        assert_eq!(DocDraft::parse(&d.bytes()).unwrap(), d);
        assert!(DocDraft::parse(b"{not json").is_err());
        let binary = DocDraft { text: "\u{1}".into(), ..d.clone() };
        assert!(DocDraft::parse(&binary.bytes()).is_err());
        let relative = DocDraft { path: Some("a.md".into()), ..d.clone() };
        assert!(DocDraft::parse(&relative.bytes()).is_err());
        let future = DocDraft { version: 9, ..d };
        assert!(DocDraft::parse(&future.bytes()).unwrap_err().contains("another version"));
    }
}
