//! The shell's own file handling in the scripts folder (#1265, round 12).
//!
//! Earlier versions let a script write anything in the scripts folder, so an install can hold a link
//! or a crafted registry entry. The shell therefore never takes that folder as it finds it:
//!
//! - **Descriptor, no links.** Every read, write, removal and `chmod` of a script, of `registry.json`
//!   and of a run's temporary file goes through the folder's descriptor with `O_NOFOLLOW`; anything
//!   that is not a regular file is refused (`coder_dirfd::Dir`).
//! - **The registry's `filename` is untrusted.** It must be one plain component, equal to the
//!   sanitized script name plus its language's extension ([`checked_filename`]).
//! - **The listings never follow a link.** The files a run made are listed from the entry's own kind,
//!   to a depth of 8 and at most 2,000 entries ([`list_files`]); more are counted, not listed.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::coder_dirfd::Dir;

/// The deepest folder a listing enters, and how many entries it names.
const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 2_000;

/// The scripts folder, held by descriptor. Errors name no path: they reach the Mind.
pub struct Scripts {
    dir: Dir,
    path: PathBuf,
}

impl Scripts {
    /// The account's scripts folder, made when missing and opened without following a link at it.
    pub fn open() -> Result<Scripts, String> {
        let path = crate::coder_deps::scripts_dir().map_err(|_| "the scripts folder is not available: this account's home cannot be had".to_string())?;
        Scripts::open_at(path)
    }

    /// The scripts folder at `path` (see [`Scripts::open`]).
    pub(crate) fn open_at(path: PathBuf) -> Result<Scripts, String> {
        std::fs::create_dir_all(&path).map_err(|e| format!("the scripts folder could not be made: {}", e.kind()))?;
        let dir = Dir::open(&path).map_err(|_| "the scripts folder is a link or not a folder, so it was not used".to_string())?;
        Ok(Scripts { dir, path })
    }

    /// Where the folder is, for the sandbox and for the person; not for the Mind's answers.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The text of a plain file of the folder.
    pub fn read(&self, file: &str) -> Result<String, String> {
        self.dir.read_regular(plain(file)?).map_err(|_| format!("{file} is missing or is not a plain file"))
    }

    /// Write a plain file of the folder (`exclusive`: a new one).
    pub fn write(&self, file: &str, data: &str, mode: u32, exclusive: bool) -> Result<(), String> {
        self.dir.write_regular(plain(file)?, data.as_bytes(), mode, exclusive).map_err(|_| format!("{file} could not be written (it is a link, not a plain file, or not writable)"))
    }

    /// Whether `file` is a plain file (a link is not).
    pub fn is_plain_file(&self, file: &str) -> bool {
        plain(file).ok().and_then(|n| self.dir.mode_of(n).ok().flatten()).is_some_and(|m| m & libc::S_IFMT == libc::S_IFREG)
    }

    /// Remove a file of the folder (a link is removed itself).
    pub fn remove(&self, file: &str) {
        if let Ok(n) = plain(file) {
            let _ = self.dir.unlink(n);
        }
    }
}

/// `file` as one plain component: no `/`, no `..`, not empty.
fn plain(file: &str) -> Result<&OsStr, String> {
    let mut parts = Path::new(file).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(n)), None) if n == OsStr::new(file) => Ok(n),
        _ => Err("a script file name must be one plain name".to_string()),
    }
}

/// The registry's `filename` for the script `key` of language extension `ext`, or a refusal: it must
/// be exactly the sanitized name plus the extension, never a path.
pub fn checked_filename<'a>(key: &str, filename: &'a str, ext: &str) -> Result<&'a str, String> {
    match filename == format!("{key}{ext}") && plain(filename).is_ok() {
        true => Ok(filename),
        false => Err(format!("Error: the registry entry for '{key}' names a file that is not its own script file, so it was not used. Write the script again with script_write.")),
    }
}

/// The plain files below `dir`, never following a link (a link is neither listed nor entered), to a
/// depth of 8, and how many more than 2,000 there were.
pub fn list_files(dir: &Path) -> (Vec<PathBuf>, usize) {
    let mut out = (Vec::new(), 0);
    if let Ok(top) = Dir::open(dir) {
        walk(&top, dir, 0, &mut out);
    }
    out
}

fn walk(dir: &Dir, at: &Path, depth: usize, out: &mut (Vec<PathBuf>, usize)) {
    let Ok(mut names) = dir.names() else { return };
    names.sort();
    for n in names {
        match dir.mode_of(&n).ok().flatten().map(|m| m & libc::S_IFMT) {
            Some(libc::S_IFREG) if out.0.len() < MAX_ENTRIES => out.0.push(at.join(&n)),
            Some(libc::S_IFREG) => out.1 += 1,
            Some(libc::S_IFDIR) if depth < MAX_DEPTH => {
                if let Ok(child) = dir.child(&n) {
                    walk(&child, &at.join(&n), depth + 1, out);
                }
            }
            _ => {}
        }
    }
}
