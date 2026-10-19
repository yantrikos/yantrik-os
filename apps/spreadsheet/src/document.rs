//! The open file: its sheet, its history, where it lives, and reading and writing it.
//!
//! Saving writes a temporary file beside the target and renames it over, so a failed write
//! never leaves half a sheet where the old one was, and then reads the file back: what `save`
//! reports is what is on the disk, not what was meant to be.
//!
//! What is saved is what was typed, formulas included (`=SUM(A1:A5)`), the way LibreOffice's
//! "save cell formulas" does it. CSV has no other place for them, and saving only the values
//! would mean a sheet saved and reopened had lost every formula in it. Excel and LibreOffice
//! both read a leading `=` in a CSV cell as a formula again.

use std::path::{Path, PathBuf};

use crate::address::Cell;
use crate::csv;
use crate::history::{Edit, History};
use crate::sheet::Sheet;

/// The largest file `open` reads. Far more text than [`crate::sheet::MAX_CELLS`] cells hold.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct Document {
    pub sheet: Sheet,
    pub history: History,
    pub path: Option<PathBuf>,
}

/// What an open read.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    pub path: PathBuf,
    pub file_rows: usize,
    pub file_cols: usize,
}

/// What a save wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub path: PathBuf,
    pub bytes: usize,
    pub rows: usize,
    pub cols: usize,
    /// Whether a file was already at the path and is now replaced.
    pub replaced: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SaveError {
    /// A file is already there, and this save was not one that replaces files.
    Exists(PathBuf),
    /// Anything else, in a sentence.
    Failed(String),
}

impl Document {
    pub fn new() -> Self {
        Self { sheet: Sheet::default(), history: History::clean(), path: None }
    }

    pub fn dirty(&self) -> bool {
        self.history.dirty()
    }

    /// The file's name, or "Untitled".
    pub fn title(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    /// Put `text` in a cell as one undoable step. Returns the text it replaced.
    pub fn set(&mut self, cell: Cell, text: &str) -> Result<String, String> {
        let before = self.sheet.set(cell, text)?;
        self.history.record(Edit { cell, before: before.clone(), after: text.to_string() });
        Ok(before)
    }

    /// Replace this document with the CSV at `path`. Nothing changes when it cannot be read.
    pub fn open(&mut self, path: &Path) -> Result<Opened, String> {
        let shown = path.display();
        let bytes = read_regular(path)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("{shown} is not UTF-8 text, so it is not a CSV file this sheet can read"))?;
        let rows = csv::parse(&text).map_err(|e| format!("{shown} could not be read as CSV: {e}"))?;
        let file_rows = rows.len();
        let file_cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let sheet = Sheet::from_rows(&rows).map_err(|e| format!("{shown} is too large to open: {e}"))?;
        self.sheet = sheet;
        self.history = History::clean();
        self.path = Some(path.to_path_buf());
        Ok(Opened { path: path.to_path_buf(), file_rows, file_cols })
    }

    /// Write the sheet to `path`. A file already there is replaced only when `replace` is set.
    pub fn save(&mut self, path: &Path, replace: bool) -> Result<Saved, SaveError> {
        let shown = path.display();
        if path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() != Some("csv") {
            return Err(SaveError::Failed(format!("{shown} does not end in .csv; this sheet saves CSV files only")));
        }
        let existing = match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => return Err(SaveError::Failed(format!("{shown} is a folder"))),
            Ok(m) if m.file_type().is_symlink() => {
                return Err(SaveError::Failed(format!("{shown} is a link; save to the file it points at instead")))
            }
            Ok(m) => Some(m.permissions()),
            Err(_) => None,
        };
        let existed = existing.is_some();
        if existed && !replace {
            return Err(SaveError::Exists(path.to_path_buf()));
        }
        let dir = match path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d,
            _ => Path::new("."),
        };
        if !dir.is_dir() {
            return Err(SaveError::Failed(format!("the folder {} does not exist", dir.display())));
        }

        let rows = self.sheet.used_rows();
        let text = csv::write(&rows);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
        let failed = |what: &str, e: std::io::Error| SaveError::Failed(format!("{shown} was not saved: {what} {}", io_words(&e)));
        write_new(&temp, text.as_bytes()).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            failed("writing it failed,", e)
        })?;
        // A replaced file keeps who may read it.
        if let Some(permissions) = existing {
            let _ = std::fs::set_permissions(&temp, permissions);
        }
        if let Err(e) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(failed("putting it in place failed,", e));
        }
        let back = std::fs::read(path).map_err(|e| failed("it could not be read back,", e))?;
        if back != text.as_bytes() {
            return Err(SaveError::Failed(format!(
                "{shown} was written but reads back different from the sheet; it is not saved"
            )));
        }
        self.path = Some(path.to_path_buf());
        self.history.mark_saved();
        Ok(Saved {
            path: path.to_path_buf(),
            bytes: back.len(),
            rows: rows.len(),
            cols: rows.first().map_or(0, Vec::len),
            replaced: existed,
        })
    }
}

/// The bytes of the regular file at `path`, at most [`MAX_FILE_BYTES`] of them.
///
/// Opened without blocking and checked on the open handle, so a FIFO, a device, or a link to
/// one is refused before a byte is read: `fs::read` of a FIFO waited forever on the window's
/// thread, and `/dev/zero` (length 0) passed the size check and read without end (security
/// review of #727). The read itself stops one byte past the limit, for a file that grows.
fn read_regular(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let shown = path.display();
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| format!("{shown} could not be opened: {}", io_words(&e)))?;
    let meta = file.metadata().map_err(|e| format!("{shown} could not be opened: {}", io_words(&e)))?;
    if meta.is_dir() {
        return Err(format!("{shown} is a folder, not a CSV file"));
    }
    if !meta.is_file() {
        return Err(format!("{shown} is not a regular file (a device, a pipe or a socket), so it is not opened"));
    }
    let too_big = || {
        format!(
            "{shown} is more than the {} MB this sheet opens",
            MAX_FILE_BYTES / (1024 * 1024)
        )
    };
    if meta.len() > MAX_FILE_BYTES {
        return Err(too_big());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{shown} could not be read: {}", io_words(&e)))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(too_big());
    }
    Ok(bytes)
}

/// A new file, created fresh: never through a link left at the name. It is made `0o644`, less
/// the umask, the mode of any new document: this user writes it and others may read it. A
/// file being replaced is given the old one's mode after this (`Document::save`).
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
    }
    let mut f = options.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// An IO error in words a person reads.
fn io_words(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "there is no such file".into(),
        std::io::ErrorKind::PermissionDenied => "permission was refused".into(),
        _ => e.to_string(),
    }
}

/// A path as a person or a mind wrote it: `~/` is the home, and a relative path is from `base`.
pub fn resolve(text: &str, base: &Path) -> PathBuf {
    let text = text.trim();
    let path = match text.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest),
        None if text == "~" => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()),
        None => PathBuf::from(text),
    };
    if path.is_absolute() { path } else { base.join(path) }
}
