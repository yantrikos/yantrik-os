//! Another program's version a save replaced, kept beside the file instead of lost.
//!
//! A save replaces the file by an atomic exchange (`Disk::exchange`), so the bytes it took the
//! place of come back at the same instant ours went in. When they are not what the document knew was
//! there, another program wrote the file between the save's check and its replace, and those bytes
//! are kept: as a file of their own beside the original, never over anything (`create_new`), and
//! private (mode 0600) since they may be.
//!
//! The exchange reads back at most 64 MiB. A larger file it took the place of is left unread under
//! a hidden temporary name beside the original (`Displaced::Unread`); it is given a visible name
//! here when it can be, and the notice says where it is either way. Nothing here ever deletes it.
//!
//! A name beside the original that cannot be made (taken by anything, a link that leads nowhere
//! included; too long for the folder) is passed for the next, then for short names, then for the
//! editor's own state folder: another program's version is dropped only if none can be written.

use std::path::{Path, PathBuf};

use crate::documents::disk::{Disk, Step};

/// Where another program's version is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kept {
    /// Kept as this file, beside the original.
    Beside(PathBuf),
    /// No name beside the original could be made (the folder refused it): kept as this file in
    /// the app's own state folder instead.
    Elsewhere(PathBuf),
    /// Too large to read back, or unreadable: kept where the exchange left it, at this path.
    Unread(PathBuf),
    /// It could not be written anywhere, and why.
    Lost(String),
}

/// How many names of each kind are tried.
const NAMES: usize = 100;

/// The names a kept version is given, in the order they are tried: beside the original with its
/// stem (`notes (outside version).txt`, `… 2`), then short ones that fit any folder whatever the
/// stem's length (`outside version 1`), all in the original's folder.
fn names(original: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    let long = (1..=NAMES).map(move |n| beside(original, n));
    let short = (1..=NAMES).map(move |n| original.with_file_name(format!("outside version {n}")));
    long.chain(short)
}

/// Another program's `bytes`, which a save of `original` replaced, kept: beside it, created new so
/// nothing is ever replaced (a name taken by anything, a link that leads nowhere included, is
/// passed by), and failing that in `elsewhere`, the app's own folder. Never dropped while any
/// of those can be written.
pub fn keep_bytes(disk: &dyn Disk, original: &Path, bytes: &[u8], elsewhere: &Path) -> Kept {
    for path in names(original) {
        match disk.create_new(&path, bytes) {
            // Made, its directory sync aside: it is there to be found.
            Ok(()) => return Kept::Beside(path),
            Err(e) if e.step == Step::SyncDir => return Kept::Beside(path),
            // Taken, or a name the folder will not have (too long): the next one.
            Err(_) => continue,
        }
    }
    let _ = std::fs::create_dir_all(elsewhere);
    let mut last = String::new();
    for n in 1..=NAMES {
        let path = elsewhere.join(format!("outside-version-{}-{n}", super::now_ms()));
        match disk.create_new(&path, bytes) {
            Ok(()) => return Kept::Elsewhere(path),
            Err(e) if e.step == Step::SyncDir => return Kept::Elsewhere(path),
            Err(e) => last = e.error,
        }
    }
    Kept::Lost(last)
}

/// A file the exchange left unread at `temp`, given a visible name beside `original` when it can
/// be (a second name, then the hidden one removed), and left where it is when not: it is kept
/// either way.
pub fn keep_unread(original: &Path, temp: &Path) -> Kept {
    for path in names(original) {
        if std::fs::hard_link(temp, &path).is_ok() {
            let _ = std::fs::remove_file(temp);
            return Kept::Beside(path);
        }
    }
    Kept::Unread(temp.to_path_buf())
}

/// `notes (outside version).txt`, then `notes (outside version 2).txt`, beside `notes.txt`.
fn beside(original: &Path, n: usize) -> PathBuf {
    let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = original.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let tag = if n == 1 { "outside version".to_string() } else { format!("outside version {n}") };
    original.with_file_name(format!("{stem} ({tag}){ext}"))
}

/// The sentence that says where another program's version of `file` is.
pub fn said(kept: &Kept, file: &Path) -> String {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match kept {
        Kept::Beside(path) | Kept::Elsewhere(path) => format!(
            "Another program wrote {name} while it was being saved; its version is kept as {}.",
            path.display()
        ),
        Kept::Unread(path) => format!(
            "Another program put a file at {name} while it was being saved, too large to check (over 64 MiB); it is kept unread at {}.",
            path.display()
        ),
        Kept::Lost(why) => format!(
            "Another program wrote {name} while it was being saved, and its version could not be kept ({why})."
        ),
    }
}

/// The same, as an agent reads it about a document it is not shown: no name, no path.
pub fn said_hidden(kept: &Kept) -> String {
    match kept {
        Kept::Beside(_) | Kept::Elsewhere(_) | Kept::Unread(_) => {
            "Another program wrote (hidden) while it was being saved; its version is kept.".into()
        }
        Kept::Lost(_) => "Another program wrote (hidden) while it was being saved, and its version could not be kept.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_beside_the_original_keep_its_extension() {
        let p = Path::new("/d/notes.txt");
        assert_eq!(beside(p, 1), Path::new("/d/notes (outside version).txt"));
        assert_eq!(beside(p, 2), Path::new("/d/notes (outside version 2).txt"));
        assert_eq!(beside(Path::new("/d/Makefile"), 1), Path::new("/d/Makefile (outside version)"));
    }
}
