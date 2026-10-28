//! What ySheets decides about its files: what a sheet file may hold, and its words. The file layer
//! itself is the framework's (`yantrik_app::documents::files`), shared with the editors.
//!
//! The sentences that have no path of their own to name read as the end of "<path> ...", which
//! `open_sheet` puts in front of them.

use std::path::Path;

use yantrik_app::documents::files::FileRules;

use crate::csv;
use super::MAX_FILE_BYTES;

/// The rules for a sheet's CSV file: it must read as CSV, whole, within the sheet's limits.
pub struct SheetRules;

fn io_words(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "there is no such file".into(),
        std::io::ErrorKind::PermissionDenied => "permission was refused".into(),
        _ => e.to_string(),
    }
}

impl FileRules for SheetRules {
    fn max_bytes(&self) -> usize {
        MAX_FILE_BYTES as usize
    }

    fn validate(&self, text: &str) -> Result<(), String> {
        csv::parse(text).map(|_| ()).map_err(|e| format!("could not be read as CSV: {e}"))
    }

    fn reader(&self) -> fn(&Path) -> Result<String, String> {
        super::read::read_text
    }

    fn cannot_read(&self, _path: &Path, e: &std::io::Error) -> String {
        format!("could not be opened: {}", io_words(e))
    }

    fn not_regular(&self) -> String {
        "is not a regular file (a folder, a device, a pipe or a socket), so it is not opened".into()
    }

    fn too_big(&self, _path: &Path, _len: u64) -> String {
        format!("is more than the {} MB this sheet opens", MAX_FILE_BYTES / (1024 * 1024))
    }

    fn not_utf8(&self) -> String {
        "is not UTF-8 text, so it is not a CSV file this sheet can read".into()
    }

    fn cannot_open(&self, _path: &Path, e: &std::io::Error) -> String {
        format!("could not be opened: {}", io_words(e))
    }

    fn relative(&self, target: &Path) -> String {
        format!("`{}` is not an absolute path; give one starting with / or ~/", target.display())
    }

    fn cannot_folder(&self, e: &std::io::Error) -> String {
        match e.kind() {
            std::io::ErrorKind::NotFound => "That folder does not exist, so nothing was written.".into(),
            _ => format!("Cannot write into that folder: {e}"),
        }
    }

    fn open_elsewhere(&self, name: &str) -> String {
        // ySheets holds one sheet, so a save is never onto another's file.
        format!("That file is open in {name}; nothing was written.")
    }

    fn already_there(&self, path: &Path) -> String {
        format!("{} is already there, so nothing was written. Replacing it is save_over.", path.display())
    }

    fn changed_on_disk(&self, path: &Path) -> String {
        format!(
            "{} changed on disk since it was opened. Nothing was written and your changes are still here. Keep both versions by saving to \
             another path (`save` with a path, or Save As in the window).",
            path.display()
        )
    }

    fn linked(&self, path: &Path) -> String {
        format!(
            "{} is a link or is not a regular file, so nothing was written; save to the file it points at instead.",
            path.display()
        )
    }

    fn read_only(&self, path: &Path) -> String {
        format!("{} is read-only, so nothing was written. Save to another path (`save` with a path, or Save As in the window).", path.display())
    }

    fn write_into_moved(&self, now: &Path) -> String {
        format!("`save_over` with the path {} writes your changes into it.", now.display())
    }

    fn write_here_or_elsewhere(&self) -> String {
        "`save_over` writes it here again, or `save` with a path puts it wherever you choose.".into()
    }

    fn save_failed(&self, e: &dyn std::fmt::Display) -> String {
        format!("Save failed: {e}. Your sheet is still open and unchanged.")
    }
}
