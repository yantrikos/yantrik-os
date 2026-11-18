//! The Markdown vault, with bounded reads and recoverable writes.
//!
//! One `.md` file per note, an optional `.meta` beside it, a write-ahead journal at
//! `.recovery-v2/pending.json` around every save, a Trash of JSON records in `.trash-v2`, and the
//! format's number in `.vault-version` (`version`). The format is the one Notes has always
//! written. Reads are the store's own (no link is followed, every size is bounded); every write
//! and removal goes through the framework's [`Disk`], so a test can make any step of any write
//! fail.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use yantrik_app::documents::disk::{Disk, RealDisk};

mod load;
mod read;
mod records;
mod resolve;
mod save;
mod theirs;
mod trash;
pub mod version;
mod write;

pub use load::{Interrupted, Loaded};
pub use read::read;
pub use resolve::{Reread, Resolved};
pub use save::{SaveFailed, SaveJob, Saved};
pub use theirs::{Alike, Theirs};

/// The most text one note may hold.
pub const LIMIT: usize = 256 * 1024;
/// The most text the whole library may hold.
pub const VAULT_LIMIT: usize = 32 * 1024 * 1024;
/// The most notes the library holds.
pub const MAX_NOTES: usize = 2000;
/// The most a `.meta` sidecar may hold.
pub const META_LIMIT: usize = 8192;
/// The most one journal or Trash record may hold: a note, its baseline and their sidecars.
const RECORD_LIMIT: usize = LIMIT * 12 + 32768;
/// The most lines one note may hold.
const LINE_LIMIT: usize = 10000;

/// Said when a note's file is not what this note was read from: the store never writes over it.
pub const CHANGED_ON_DISK: &str =
    "This note changed on disk. Your draft is protected; use Save a copy, then Refresh.";

/// The vault: where it is, the disk it writes through, and whether it may be written.
#[derive(Clone)]
pub struct Vault {
    dir: PathBuf,
    disk: Arc<dyn Disk>,
    /// Why nothing in the vault may be changed, when the last load found something stopping it
    /// (`version`): every write into the vault refuses with this sentence.
    read_only: Option<String>,
}

impl Vault {
    pub fn new(dir: PathBuf, disk: Arc<dyn Disk>) -> Vault {
        Vault { dir, disk, read_only: None }
    }

    /// The vault at `dir` on the real disk.
    pub fn real(dir: PathBuf) -> Vault {
        Vault::new(dir, Arc::new(RealDisk))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Why the vault is open read-only, if it is.
    pub fn read_only(&self) -> Option<&str> {
        self.read_only.as_deref()
    }

    /// This vault as a load found it: read-only for the reason given, or writable.
    pub fn with_access(&self, read_only: Option<String>) -> Vault {
        Vault { read_only, ..self.clone() }
    }

    /// A note's `.md` and `.meta`, for a valid file name only.
    fn files(&self, id: &str) -> Result<(PathBuf, PathBuf), String> {
        valid_id(id)?;
        let p = self.dir.join(id);
        Ok((p.clone(), p.with_extension("meta")))
    }

    fn journal(&self) -> PathBuf {
        self.dir.join(".recovery-v2/pending.json")
    }

    fn trash_record(&self, id: &str) -> PathBuf {
        self.dir.join(".trash-v2").join(format!("{id}.json"))
    }
}

/// Whether `s` may be a note: within the size and line limits, UTF-8 without control characters.
pub fn validate(s: &str) -> Result<(), String> {
    if s.len() > LIMIT || s.bytes().filter(|b| *b == b'\n').count() > LINE_LIMIT {
        return Err("A note can contain up to 256 KiB and 10,000 lines.".into());
    }
    if s.chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("Notes require UTF-8 text without control characters.".into());
    }
    Ok(())
}

/// A note's file name: one `.md` component, not hidden, no separators.
fn valid_id(id: &str) -> Result<(), String> {
    if id.ends_with(".md")
        && Path::new(id).components().count() == 1
        && !id.starts_with('.')
        && !id.contains(['/', '\\'])
    {
        Ok(())
    } else {
        Err("Invalid note filename".into())
    }
}

/// A path as a person types it: `~/` is the home directory.
pub fn expand_home(s: &str) -> PathBuf {
    if let Some(p) = s.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(p)
    } else {
        PathBuf::from(s)
    }
}
