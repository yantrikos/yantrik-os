//! The file layer every document app shares: opening a file, saving one, following it when it
//! moves, and keeping what another program wrote. Each function runs in a job, never on the
//! window's thread, and returns plain data for the command that carries it back.
//!
//! What differs between apps is a parameter, not a copy: what a document may hold, and the
//! sentences an app says when a step is refused, are the app's [`FileRules`]. Everything else
//! (the order of the checks, what is read without following a link, how a replace is judged) is
//! one implementation, so a fix to it reaches every editor.

pub mod agent;
pub mod kept;
pub mod locate;
pub mod open;
pub mod read;
pub mod save;

use std::path::Path;

/// What one app decides about its files: its bound on a document and its words. The framework
/// decides the rest.
pub trait FileRules {
    /// The most bytes a document may hold; a file larger than this is never read whole.
    fn max_bytes(&self) -> usize;
    /// Whether `text` may be a document, with the sentence if not.
    fn validate(&self, text: &str) -> Result<(), String>;

    /// The app's own reader of a file, as a plain function: the search for a moved file reads with
    /// it, so it finds only what the app would open.
    fn reader(&self) -> fn(&Path) -> Result<String, String>;

    /// A file could not be opened for reading.
    fn cannot_read(&self, path: &Path, error: &std::io::Error) -> String;
    /// It is not a regular file.
    fn not_regular(&self) -> String;
    /// It is past [`FileRules::max_bytes`].
    fn too_big(&self, path: &Path, len: u64) -> String;
    /// It is not UTF-8.
    fn not_utf8(&self) -> String;
    /// Its path could not be resolved.
    fn cannot_open(&self, path: &Path, error: &std::io::Error) -> String;

    /// A save was given a relative path.
    fn relative(&self, target: &Path) -> String;
    /// The folder of a save could not be resolved.
    fn cannot_folder(&self, error: &std::io::Error) -> String;
    /// A save onto a file another document of the app holds (`name` as anyone may read it).
    fn open_elsewhere(&self, name: &str) -> String;
    /// A save onto a file that is there, without consent.
    fn already_there(&self, path: &Path) -> String;
    /// A save over a file that is no longer what the document last read or wrote.
    fn changed_on_disk(&self, path: &Path) -> String;
    /// A save onto a link, or onto what is not a regular file.
    fn linked(&self, path: &Path) -> String;
    /// A save onto a read-only file.
    fn read_only(&self, path: &Path) -> String;
    /// A step of a save failed.
    fn save_failed(&self, error: &dyn std::fmt::Display) -> String;
}

/// Milliseconds since the epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
