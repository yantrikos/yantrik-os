//! Everything the app does on the disk. Each function here runs in a job, never on the window's
//! thread, and returns what it found as plain data for the command that carries it back.

pub mod drafts;
pub mod names;
pub mod open;
pub mod picker;
pub mod read;
pub mod rules;
pub mod save;

// Keeping another program's version is the framework's.
pub use yantrik_app::documents::files::kept;

/// The largest file `open` reads. Far more text than [`crate::sheet::MAX_CELLS`] cells hold.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
