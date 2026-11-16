//! Judging every entry of one walk (#1247, review R11-M3).
//!
//! [`super::is_protected`] asks the filesystem where each listed place really is, and where a Mind's
//! workshops are: some hundreds of microseconds an entry in a home that holds the usual places, so a
//! walk of 20,000 files took seconds where `du` takes tens of milliseconds. A walk reads the same
//! home from its first entry to its last, so [`Walk::new`] asks once, and [`Walk::is_protected`]
//! judges an entry by its text and that answer.
//!
//! An entry that is a link is judged by [`super::is_protected`] whole, which follows it, so what a link
//! leads to is protected as it was. An entry that is not a link stands where its path says, under a
//! folder the walk was started on, and that folder is judged once by the whole rule.

use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::{copy_marker, from_env, in_a_listed_place_or_named, lowered, resolved, under_prefix, workshops, PROTECTED};

/// What one walk needs to know about the session's home, asked once.
pub struct Walk {
    /// The folder the walk started on is a protected place: everything under it is.
    root_protected: bool,
    /// Where each place that is not at its written path really is, in lower-case parts: the listed
    /// places in the home, and under the XDG base directories when those are moved.
    places: Vec<Vec<String>>,
    /// The workshop roots (the person's, and one under `XDG_DATA_HOME`): where each really is, and the
    /// device and inode of every folder that is one (a second mount of it).
    workshops: Vec<(PathBuf, Vec<(u64, u64)>)>,
}

impl Walk {
    /// A walk that starts on `root`.
    pub fn new(root: &Path) -> Walk {
        let (home, data, config, state) = (from_env("HOME"), from_env("XDG_DATA_HOME"), from_env("XDG_CONFIG_HOME"), from_env("XDG_STATE_HOME"));
        let mut places = Vec::new();
        let mut keep = |at: PathBuf| {
            places.push(lowered(&at));
            if let Some(real) = resolved(&at) {
                places.push(lowered(&real));
            }
        };
        if let Some(home) = &home {
            for place in PROTECTED {
                let first = place.split('/').next().unwrap_or(place);
                if home.join(first).symlink_metadata().is_ok() {
                    keep(home.join(place));
                }
            }
        }
        if let Some(base) = &data {
            under_prefix(".local/share/").for_each(|place| keep(base.join(place)));
        }
        if let Some(base) = &state {
            keep(base.join("yantrik"));
        }
        if let Some(base) = &config {
            under_prefix(".config/").for_each(|place| keep(base.join(place)));
        }
        let roots = home.iter().map(|home| workshops::base(home)).chain(data.iter().map(|base| base.join("yantrik/workshops")));
        let workshops = roots.map(|at| workshops::Root::at(&at)).map(|root| (root.real().to_path_buf(), root.folder_ids())).collect();
        Walk { root_protected: super::is_protected(root), places, workshops }
    }

    /// Whether `path` (an entry found under the folder the walk started on, with its `meta` from
    /// `symlink_metadata`) is a protected place or file.
    pub fn is_protected(&self, path: &Path, meta: &Metadata) -> bool {
        if self.root_protected {
            return true;
        }
        if meta.file_type().is_symlink() {
            return super::is_protected(path);
        }
        self.by_text(path, 0) || (meta.is_dir() && self.workshops.iter().any(|(_, ids)| ids.contains(&(meta.dev(), meta.ino()))))
    }

    fn by_text(&self, path: &Path, depth: u8) -> bool {
        in_a_listed_place_or_named(path)
            || (depth < 4
                && path
                    .file_name()
                    .and_then(|name| copy_marker::encoded_path(&name.to_string_lossy()))
                    .is_some_and(|original| self.by_text(&original, depth + 1)))
            || {
                let parts = lowered(path);
                self.places.iter().any(|want| parts.len() >= want.len() && parts[..want.len()] == want[..])
            }
            || self.workshops.iter().any(|(real, _)| path.starts_with(real))
    }
}
