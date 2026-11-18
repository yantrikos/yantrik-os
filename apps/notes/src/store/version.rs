//! The vault's format, written down: `.vault-version` (design/notes-gate-2026-10-21.md §3,
//! "Interrupted migration").
//!
//! Notes has written format 2 since its first commit (`.recovery-v2`, `.trash-v2`) and has never
//! migrated a vault, so nothing here migrates. The marker is groundwork: the first build that
//! changes the format has a version to read, and this build already refuses a vault newer than
//! it knows instead of writing format 2 into it.
//!
//! The marker is written once, on a vault without it (new, or made by a build before the marker),
//! through the store's create path, so it is whole or absent and never replaces anything: two
//! copies making it at once end with one valid file. It is read at every load and never
//! rewritten. A vault whose marker cannot be written, or says nothing this build can read, is
//! opened read-only with the reason. Its name starts with a dot and does not end `.md`, so it is
//! never a note.

use super::{read, Vault};

/// The marker's name in the vault.
pub const MARKER: &str = ".vault-version";
/// The format this build reads and writes.
pub const FORMAT: u32 = 2;
/// The most a marker may hold: a number and a line break, with room to spare.
const MARKER_LIMIT: usize = 64;

/// What the vault's marker says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Format {
    /// This build's format.
    Current,
    /// No marker yet.
    Absent,
    /// A later format than this build knows.
    Newer(u32),
    /// A marker this build cannot read (not a number, an older number no build wrote, a link, too
    /// large), and why.
    Unreadable(String),
}

/// Said when a start is refused for a vault a newer Notes wrote.
pub fn newer_at_start(found: u32) -> String {
    format!(
        "This library was written by a newer Notes (format {found}); this Notes reads format {FORMAT}. \
         It was not opened, and nothing in it was changed."
    )
}

impl Vault {
    fn marker(&self) -> std::path::PathBuf {
        self.dir.join(MARKER)
    }

    /// What the marker says, read and never written.
    pub fn format(&self) -> Format {
        match read(&self.marker(), MARKER_LIMIT) {
            Ok(None) => Format::Absent,
            Ok(Some(text)) => match text.trim().parse::<u32>() {
                Ok(FORMAT) => Format::Current,
                Ok(n) if n > FORMAT => Format::Newer(n),
                Ok(n) => Format::Unreadable(format!("it says format {n}, which no Notes wrote")),
                Err(_) => Format::Unreadable("it does not hold a format number".into()),
            },
            Err(e) => Format::Unreadable(e),
        }
    }

    /// The marker, written once if the vault has none. `Ok` when this build may write the vault;
    /// otherwise the sentence every change is refused with while it is open read-only.
    pub fn mark(&self) -> Result<(), String> {
        let unmade = match self.format() {
            Format::Absent => match self.disk.create_new(&self.marker(), format!("{FORMAT}\n").as_bytes()) {
                Ok(()) => return Ok(()),
                Err(e) => e.error,
            },
            other => return writable(other, None),
        };
        // The create never replaces: another copy may have made it first, or it could not be
        // made at all. What is there now decides; a marker made is never made again.
        writable(self.format(), Some(unmade))
    }
}

/// Whether a vault whose marker says `format` may be written; `unmade` is why a marker this
/// load tried to make was not.
fn writable(format: Format, unmade: Option<String>) -> Result<(), String> {
    match format {
        Format::Current => Ok(()),
        Format::Absent => Err(format!(
            "This library is open read-only: Notes could not write its version file ({}). Make the \
             folder writable, then Refresh.",
            unmade.unwrap_or_default()
        )),
        Format::Newer(n) => Err(format!(
            "This library is open read-only: a newer Notes (format {n}) has written it, and this \
             Notes writes format {FORMAT}. Nothing in it was changed."
        )),
        Format::Unreadable(why) => Err(format!(
            "This library is open read-only: its version file cannot be read ({why}). Nothing in \
             it was changed."
        )),
    }
}
