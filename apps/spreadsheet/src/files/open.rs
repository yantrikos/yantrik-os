//! Opening a sheet: the framework's opener, held to the format, and the sheet built from what was
//! read. Each function runs in a job, so the parse and the first working out are off the window's
//! thread.

use std::path::{Path, PathBuf};

use yantrik_app::documents::files::{agent, open};
pub use yantrik_app::documents::files::open::{described, Opened};

use super::rules::SheetRules;
use crate::csv;
use crate::sheet::Sheet;

/// A file read, and the sheet it makes, worked out.
#[derive(Debug, Clone)]
pub struct OpenedSheet {
    pub file: Opened,
    pub sheet: Sheet,
    /// How many rows and columns the file itself had, before the sheet padded them to a grid.
    pub file_rows: usize,
    pub file_cols: usize,
    /// The fingerprint of the file's text, made in the job: hashing a large file is not for the
    /// window's thread.
    pub baseline: yantrik_app::documents::Fingerprint,
    /// The cells as the window shows them, when a job made them (`update::opening`).
    pub cells: Option<Vec<Vec<crate::SpreadsheetCell>>>,
}

/// The sheet a file's text makes. The text has been read as CSV already; this is the same read,
/// kept, and the grid built and worked out.
pub fn sheet_of(file: Opened) -> Result<OpenedSheet, String> {
    let shown = file.path.display().to_string();
    let rows = csv::parse(&file.text).map_err(|e| format!("{shown} could not be read as CSV: {e}"))?;
    let file_rows = rows.len();
    let file_cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let sheet = Sheet::from_rows(&rows).map_err(|e| format!("{shown} is too large to open: {e}"))?;
    let baseline = yantrik_app::documents::Fingerprint::of_bytes(file.text.as_bytes());
    Ok(OpenedSheet { file, sheet, file_rows, file_cols, baseline, cells: None })
}

/// Opens the sheet at `path`. An agent may name only a file in the person's home, outside its
/// protected places, and never reaches one through a link that leads out of it; the person may
/// open any CSV they can read. A refusal names the path first.
pub fn open_sheet(path: &Path, agent: bool) -> Result<OpenedSheet, String> {
    open_sheet_in(&PathBuf::from(std::env::var_os("HOME").unwrap_or_default()), path, agent)
}

/// Whether `path` may be opened by this caller: the agent rule alone, which reads the disk (so it
/// runs in a job). The surface asks it before anything else is said about the open.
pub fn may_open(path: &Path, agent: bool) -> Result<(), String> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    agent::may_read_in(&home, agent, path).map_err(|e| said(path, e))
}

/// [`open_sheet`] for this home: the rule itself, with the home passed in so a test does not have
/// to set the process's environment.
pub fn open_sheet_in(home: &Path, path: &Path, agent: bool) -> Result<OpenedSheet, String> {
    agent::may_read_in(home, agent, path).map_err(|e| said(path, e))?;
    let file = open::open(&SheetRules, path, false).map_err(|e| said(path, e))?;
    sheet_of(file)
}

/// A file read again, after another program changed it, or found where it moved to.
pub fn read_sheet(path: &Path) -> Result<OpenedSheet, String> {
    let text = super::read::read_text(path).map_err(|e| said(path, e))?;
    sheet_of(described(path.to_path_buf(), text))
}

/// A reason from the file layer as a sentence about `path`. The framework's own refusals of an
/// agent (outside the home, protected) already name it.
fn said(path: &Path, reason: String) -> String {
    if reason.contains(&path.display().to_string()) {
        reason
    } else {
        format!("{} {reason}", path.display())
    }
}
