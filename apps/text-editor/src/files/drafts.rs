//! What one tab's draft holds: the editor's own schema, written through the framework's draft lane
//! (`fx.save_draft`) and read back from `Boot::drafts` at the next start.
//!
//! A draft holds the tab's text, the file it is a draft of, and that file's text as the tab last
//! read or wrote it, so a recovered tab is unsaved against the right baseline and a moved file can
//! still be looked for. Who opened the tab is never written: a recovered tab is the person's, and
//! an opener read back from a draft could be forged (security review of #620).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::text::limits::validate;

/// The schema's version.
pub const VERSION: u32 = 1;

/// One tab's draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabDraft {
    pub version: u32,
    /// The tab's file, if it has one.
    pub path: Option<PathBuf>,
    pub text: String,
    /// The file's text as the tab last read or wrote it.
    pub on_disk: Option<String>,
    /// When the tab was first opened, in milliseconds: restored tabs keep their order.
    pub opened_at: u64,
    /// When the draft was made, in milliseconds.
    pub at: u64,
}

impl TabDraft {
    pub fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// A draft read back, if it is one this editor wrote and both its texts are documents.
    pub fn parse(bytes: &[u8]) -> Result<TabDraft, String> {
        let draft: TabDraft = serde_json::from_slice(bytes).map_err(|e| format!("it could not be read: {e}"))?;
        if draft.version != VERSION {
            return Err(format!("it was written by another version of the editor ({})", draft.version));
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
        let d = TabDraft {
            version: VERSION,
            path: Some("/home/p/a.txt".into()),
            text: "draft 🦀".into(),
            on_disk: Some("disk".into()),
            opened_at: 1,
            at: 2,
        };
        assert_eq!(TabDraft::parse(&d.bytes()).unwrap(), d);
        assert!(TabDraft::parse(b"{not json").is_err());
        let binary = TabDraft { text: "\u{1}".into(), ..d.clone() };
        assert!(TabDraft::parse(&binary.bytes()).is_err());
        let relative = TabDraft { path: Some("a.txt".into()), ..d.clone() };
        assert!(TabDraft::parse(&relative.bytes()).is_err());
        let future = TabDraft { version: 9, ..d };
        assert!(TabDraft::parse(&future.bytes()).unwrap_err().contains("another version"));
    }
}
