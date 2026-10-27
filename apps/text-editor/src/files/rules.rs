//! What the Text Editor decides about its files: what a document may hold, and its words. The
//! file layer itself is the framework's (`yantrik_app::documents::files`).

use std::path::Path;

use yantrik_app::documents::files::FileRules;

use crate::text::limits::{validate, MAX_BYTES};

/// What the editor says when a tab's file no longer holds what the tab last read or wrote.
pub const CHANGED_ON_DISK: &str = "File changed on disk. Your draft is intact; use Save As to keep both versions.";

/// What the editor says when a Save As path is taken.
const ALREADY_THERE: &str = "That file already exists. Choose another name; nothing was overwritten. Save \
                                 As with overwrite=true replaces it.";

/// The Text Editor's limits and sentences.
pub struct TextRules;

impl FileRules for TextRules {
    fn max_bytes(&self) -> usize {
        MAX_BYTES
    }

    fn validate(&self, text: &str) -> Result<(), String> {
        validate(text)
    }

    fn reader(&self) -> fn(&Path) -> Result<String, String> {
        super::read::read_text
    }

    fn cannot_read(&self, _: &Path, e: &std::io::Error) -> String {
        format!("Cannot read: {e}")
    }

    fn not_regular(&self) -> String {
        "Only regular text files can be opened.".into()
    }

    fn too_big(&self, _: &Path, _: u64) -> String {
        "File exceeds the 1 MiB editing limit. It was not loaded.".into()
    }

    fn not_utf8(&self) -> String {
        "File is not valid UTF-8; no lossy conversion was performed.".into()
    }

    fn cannot_open(&self, _: &Path, e: &std::io::Error) -> String {
        format!("Cannot open: {e}")
    }

    fn relative(&self, _: &Path) -> String {
        "Use an absolute file path.".into()
    }

    fn cannot_folder(&self, e: &std::io::Error) -> String {
        format!("Cannot open folder: {e}")
    }

    fn open_elsewhere(&self, tab: &str) -> String {
        format!("That file is open in another tab ({tab}); close it there, or choose another name. Nothing was written.")
    }

    fn already_there(&self, _: &Path) -> String {
        ALREADY_THERE.into()
    }

    fn changed_on_disk(&self, _: &Path) -> String {
        CHANGED_ON_DISK.into()
    }

    fn linked(&self, _: &Path) -> String {
        "This path is linked or is not a regular file. Use Save As.".into()
    }

    fn read_only(&self, _: &Path) -> String {
        "This file is read-only. Use Save As.".into()
    }

    fn save_failed(&self, e: &dyn std::fmt::Display) -> String {
        format!("Save failed: {e}. Your draft is still open.")
    }
}
