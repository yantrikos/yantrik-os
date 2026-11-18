//! What the sheet's draft holds: the app's own schema, written through the framework's draft lane
//! (`fx.save_draft`) and read back from `Boot::drafts` at the next start.
//!
//! A draft holds the typed text of the used block, the file it is a draft of, and that file's text
//! as the sheet last read or wrote it, so a recovered sheet is unsaved against the right baseline
//! and a moved file can still be looked for. What is typed is untrusted until it has been through
//! the same limits a file is held to.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::rules::SheetRules;
use crate::formula::budget::MAX_TEXT;
use crate::sheet::{MAX_CELLS, MAX_SHEET_COLS};
use yantrik_app::documents::files::FileRules as _;

/// The schema's version.
pub const VERSION: u32 = 1;

/// The sheet's draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetDraft {
    pub version: u32,
    /// The sheet's file, if it has one.
    pub path: Option<PathBuf>,
    /// The typed text of the used block, row by row.
    pub rows: Vec<Vec<String>>,
    /// The file's text as the sheet last read or wrote it. `None` for a sheet never written.
    pub on_disk: Option<String>,
    /// The selected cell, as A1 names it.
    pub active: String,
    /// The cell whose text was still in the formula bar, not entered, when the draft was made at a
    /// logout. Its text is in `rows` (in the draft only, never in a file), and the notice at the
    /// next start says so.
    #[serde(default)]
    pub typing: Option<String>,
    /// When the draft was made, in milliseconds.
    pub at: u64,
}

impl SheetDraft {
    pub fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// A draft read back, if it is one this app wrote and holds a sheet within the limits a file
    /// is held to.
    pub fn parse(bytes: &[u8]) -> Result<SheetDraft, String> {
        let draft: SheetDraft = serde_json::from_slice(bytes).map_err(|e| format!("it could not be read: {e}"))?;
        if draft.version != VERSION {
            return Err(format!("it was written by another version of ySheets ({})", draft.version));
        }
        let cells: usize = draft.rows.iter().map(Vec::len).sum();
        if cells > MAX_CELLS || draft.rows.iter().any(|r| r.len() > MAX_SHEET_COLS) {
            return Err("it holds more than one sheet holds".into());
        }
        if draft.rows.iter().flatten().any(|t| t.chars().count() > MAX_TEXT) {
            return Err("a cell in it is longer than a cell holds".into());
        }
        if let Some(on_disk) = &draft.on_disk {
            SheetRules.validate(on_disk)?;
        }
        if draft.path.as_ref().is_some_and(|p| !p.is_absolute()) {
            return Err("its file is not an absolute path".into());
        }
        Ok(draft)
    }
}

/// Milliseconds since the epoch.
pub use yantrik_app::documents::files::now_ms;

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> SheetDraft {
        SheetDraft {
            version: VERSION,
            path: Some("/home/p/a.csv".into()),
            rows: vec![vec!["1".into(), "=A1+1".into()]],
            on_disk: Some("1,2\r\n".into()),
            active: "B1".into(),
            typing: None,
            at: 2,
        }
    }

    #[test]
    fn a_draft_round_trips_and_a_bad_one_is_refused() {
        let d = draft();
        assert_eq!(SheetDraft::parse(&d.bytes()).unwrap(), d);
        assert!(SheetDraft::parse(b"{not json").is_err());
        let relative = SheetDraft { path: Some("a.csv".into()), ..d.clone() };
        assert!(SheetDraft::parse(&relative.bytes()).is_err());
        let not_csv = SheetDraft { on_disk: Some("a,\"never closed\n".into()), ..d.clone() };
        assert!(SheetDraft::parse(&not_csv.bytes()).is_err());
        let future = SheetDraft { version: 9, ..d };
        assert!(SheetDraft::parse(&future.bytes()).unwrap_err().contains("another version"));
    }

    #[test]
    fn a_draft_past_the_sheets_limits_is_refused_whole() {
        let wide = SheetDraft { rows: vec![vec![String::new(); MAX_SHEET_COLS + 1]], ..draft() };
        assert!(SheetDraft::parse(&wide.bytes()).is_err());
        let long = SheetDraft { rows: vec![vec!["x".repeat(MAX_TEXT + 1)]], ..draft() };
        assert!(SheetDraft::parse(&long.bytes()).is_err());
        let tall = SheetDraft { rows: vec![vec![String::new()]; MAX_CELLS + 1], ..draft() };
        assert!(SheetDraft::parse(&tall.bytes()).is_err());
    }
}
