//! What yDoc decides about its files: what a document may hold, and its words. The file layer
//! itself is the framework's (`yantrik_app::documents::files`), shared with the Text Editor.

use std::path::Path;

use yantrik_app::documents::files::FileRules;

use crate::limits::{validate, MAX_BYTES};

/// yDoc's limits and sentences.
pub struct DocRules;

impl FileRules for DocRules {
    fn max_bytes(&self) -> usize {
        MAX_BYTES
    }

    fn validate(&self, text: &str) -> Result<(), String> {
        validate(text)
    }

    fn reader(&self) -> fn(&Path) -> Result<String, String> {
        super::read::read_text
    }

    fn cannot_read(&self, path: &Path, e: &std::io::Error) -> String {
        format!("Cannot read {}: {e}", path.display())
    }

    fn not_regular(&self) -> String {
        "Only a regular file can be opened as a document.".into()
    }

    fn too_big(&self, path: &Path, len: u64) -> String {
        format!("{} is {len} bytes, past the 1 MiB editing limit. It was not loaded.", path.display())
    }

    fn not_utf8(&self) -> String {
        "That file is not valid UTF-8, and no lossy conversion was made.".into()
    }

    fn cannot_open(&self, path: &Path, e: &std::io::Error) -> String {
        format!("Cannot open {}: {e}", path.display())
    }

    fn relative(&self, target: &Path) -> String {
        format!(
            "Use an absolute file path; `{}` is relative to a directory this app does not have.",
            target.display()
        )
    }

    fn cannot_folder(&self, e: &std::io::Error) -> String {
        format!("Cannot write into that folder: {e}")
    }

    fn open_elsewhere(&self, name: &str) -> String {
        // yDoc holds one document, so a save is never onto another's file.
        format!("That file is open in {name}; nothing was written.")
    }

    fn already_there(&self, path: &Path) -> String {
        format!(
            "{} already exists. Choose another name; nothing was overwritten. Save As with overwrite=true replaces it.",
            path.display()
        )
    }

    fn changed_on_disk(&self, path: &Path) -> String {
        format!(
            "{} changed on disk since it was opened. Nothing was written; your draft is intact. Use Save As to keep \
             both versions.",
            path.display()
        )
    }

    fn linked(&self, path: &Path) -> String {
        format!("{} is a link or is not a regular file, so nothing was written. Save As to another path.", path.display())
    }

    fn read_only(&self, path: &Path) -> String {
        format!("{} is read-only, so nothing was written. Save As to another path.", path.display())
    }

    fn save_failed(&self, e: &dyn std::fmt::Display) -> String {
        format!("Save failed: {e}. Your draft is still open and unchanged.")
    }
}
