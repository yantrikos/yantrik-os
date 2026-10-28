//! What yPresent decides about its files: what a deck may hold, and its words. The file layer
//! itself is the framework's (`yantrik_app::documents::files`), shared with the editors.

use std::path::Path;

use yantrik_app::documents::files::FileRules;

use crate::deck::{parse, LoadError, MAX_FILE_BYTES};

/// The rules for a `.ydeck`: it must be a deck, whole, by the format's own parser.
pub struct DeckRules;

/// The rules for a text export: any text, up to what a deck of the largest size could write.
pub struct ExportRules;

/// The most an export may hold. A deck is at most [`MAX_FILE_BYTES`] of JSON, and its Markdown
/// is never longer than that, so this is room, not a limit anyone meets.
const EXPORT_MAX_BYTES: usize = 16 * 1024 * 1024;

impl FileRules for DeckRules {
    fn max_bytes(&self) -> usize {
        MAX_FILE_BYTES as usize
    }

    fn validate(&self, text: &str) -> Result<(), String> {
        parse(text).map(|_| ()).map_err(|e| e.to_string())
    }

    fn reader(&self) -> fn(&Path) -> Result<String, String> {
        super::read::read_text
    }

    fn cannot_read(&self, _path: &Path, e: &std::io::Error) -> String {
        LoadError::Unreadable(e.to_string()).to_string()
    }

    fn not_regular(&self) -> String {
        LoadError::Unreadable("it is not a regular file".into()).to_string()
    }

    fn too_big(&self, _path: &Path, len: u64) -> String {
        LoadError::TooLarge(len).to_string()
    }

    fn not_utf8(&self) -> String {
        LoadError::Unreadable("stream did not contain valid UTF-8".into()).to_string()
    }

    fn cannot_open(&self, _path: &Path, e: &std::io::Error) -> String {
        LoadError::Unreadable(e.to_string()).to_string()
    }

    fn relative(&self, target: &Path) -> String {
        format!("Use an absolute file path; `{}` is relative to a directory this app does not have.", target.display())
    }

    fn cannot_folder(&self, e: &std::io::Error) -> String {
        format!("Cannot write into that folder: {e}")
    }

    fn open_elsewhere(&self, name: &str) -> String {
        // yPresent holds one deck, so a save is never onto another's file.
        format!("That file is open in {name}; nothing was written.")
    }

    fn already_there(&self, path: &Path) -> String {
        format!("{} already exists. Choose another name; nothing was overwritten. Save As with overwrite=true replaces it.", path.display())
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

/// An export is a copy: every sentence is the deck's, and only what the text may be differs.
impl FileRules for ExportRules {
    fn max_bytes(&self) -> usize {
        EXPORT_MAX_BYTES
    }

    fn validate(&self, text: &str) -> Result<(), String> {
        if text.len() > EXPORT_MAX_BYTES {
            return Err(format!("The export is {} bytes, past the {EXPORT_MAX_BYTES}-byte limit.", text.len()));
        }
        Ok(())
    }

    fn reader(&self) -> fn(&Path) -> Result<String, String> {
        DeckRules.reader()
    }

    fn cannot_read(&self, path: &Path, e: &std::io::Error) -> String {
        DeckRules.cannot_read(path, e)
    }

    fn not_regular(&self) -> String {
        DeckRules.not_regular()
    }

    fn too_big(&self, path: &Path, len: u64) -> String {
        DeckRules.too_big(path, len)
    }

    fn not_utf8(&self) -> String {
        DeckRules.not_utf8()
    }

    fn cannot_open(&self, path: &Path, e: &std::io::Error) -> String {
        DeckRules.cannot_open(path, e)
    }

    fn relative(&self, target: &Path) -> String {
        DeckRules.relative(target)
    }

    fn cannot_folder(&self, e: &std::io::Error) -> String {
        DeckRules.cannot_folder(e)
    }

    fn open_elsewhere(&self, name: &str) -> String {
        DeckRules.open_elsewhere(name)
    }

    fn already_there(&self, path: &Path) -> String {
        DeckRules.already_there(path)
    }

    fn changed_on_disk(&self, path: &Path) -> String {
        DeckRules.changed_on_disk(path)
    }

    fn linked(&self, path: &Path) -> String {
        DeckRules.linked(path)
    }

    fn read_only(&self, path: &Path) -> String {
        DeckRules.read_only(path)
    }

    fn save_failed(&self, e: &dyn std::fmt::Display) -> String {
        format!("Export failed: {e}.")
    }
}
