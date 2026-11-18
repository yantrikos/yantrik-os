//! Every write the vault makes, through the disk seam, with the store's own refusals first. A
//! vault open read-only refuses each of them here, so no path into the vault writes past it.

use std::path::Path;

use yantrik_app::documents::disk::{DiskError, Exchanged, Step};

use super::Vault;

impl Vault {
    /// `text` at `path`, whole or not at all. With `replace` the file there is replaced, unless it
    /// is linked or read-only; without it, nothing already at `path` is ever replaced.
    pub(super) fn put(&self, path: &Path, text: &str, replace: bool) -> Result<(), DiskError> {
        if let Some(why) = &self.read_only {
            return Err(DiskError { step: Step::Check, error: why.clone() });
        }
        if replace {
            replaceable(path)?;
            self.disk.write_atomically(path, text.as_bytes())
        } else {
            self.disk.create_new(path, text.as_bytes())
        }
    }

    /// `text` in place of the file at `path`, refused as a replacing `put` is, handing back what
    /// it took the place of at that instant (`Disk::exchange`).
    pub(super) fn exchange(&self, path: &Path, text: &str) -> Result<Exchanged, DiskError> {
        if let Some(why) = &self.read_only {
            return Err(DiskError { step: Step::Check, error: why.clone() });
        }
        replaceable(path)?;
        self.disk.exchange(path, text.as_bytes())
    }

    /// Removes a file; one already gone is removed.
    pub(super) fn remove(&self, path: &Path) -> Result<(), String> {
        if let Some(why) = &self.read_only {
            return Err(why.clone());
        }
        self.disk.remove(path).map_err(|e| e.to_string())
    }

    /// The directory's entries made durable: after a removal.
    pub(super) fn sync_dir(&self) -> Result<(), String> {
        std::fs::File::open(&self.dir)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }
}

/// Whether the file at `path`, if any, is one a save may replace. A link is never replaced (it
/// would leave the note it points at stale), nor a file with a second name (a hard link someone
/// made), nor one the person made read-only. Nothing at `path` is fine: the replace creates it.
fn replaceable(path: &Path) -> Result<(), DiskError> {
    use std::os::unix::fs::MetadataExt as _;
    match std::fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() || m.nlink() != 1 || m.mode() & 0o222 == 0 => Err(DiskError {
            step: Step::Check,
            error: "Destination is linked or read-only".into(),
        }),
        _ => Ok(()),
    }
}
