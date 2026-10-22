//! The disk an editor's store writes through, as a seam: the real one, and one that fails a
//! chosen step so every way a save can fail is a test (design/notes-gate-2026-10-21.md §3).
//!
//! An atomic replace is five steps after a check: create a temp file, write it, sync it, rename
//! it over the destination, sync the directory. Each failure means something different for what
//! is on disk, and [`Step::durability`] says which. A create is the same five steps, with the temp
//! file linked at the destination instead of renamed over it, so it can never replace anything.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::Durability;

/// One step of an atomic replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The destination is not a file this replace may take the place of (a link).
    Check,
    CreateTemp,
    Write,
    SyncFile,
    /// The new bytes take the destination's name: a rename over it, or a link at it (a create).
    Rename,
    SyncDir,
}

impl Step {
    /// What a failure at this step leaves on disk.
    pub fn durability(self) -> Durability {
        match self {
            Step::Check | Step::CreateTemp | Step::Write | Step::SyncFile | Step::Rename => Durability::NotWritten,
            Step::SyncDir => Durability::Unknown,
        }
    }
}

/// A failed disk operation, with the step it failed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskError {
    pub step: Step,
    pub error: String,
}

impl std::fmt::Display for DiskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?})", self.error, self.step)
    }
}

/// What an editor's store needs from the disk.
pub trait Disk: Send + Sync {
    /// Replaces (or creates) `path` with `bytes`, whole or not at all.
    fn write_atomically(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError>;
    /// Creates `path` with `bytes`, whole or not at all, and never over anything already there:
    /// a file, a link or anything else at `path` fails the publishing step and is left as it was.
    /// For a document saved for the first time, an export, a record that must not be replaced.
    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError>;
    /// The file's bytes, or `None` when it is absent.
    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>>;
    fn remove(&self, path: &Path) -> std::io::Result<()>;

    /// [`Disk::write_atomically`], handing back what the new bytes replaced, taken at the same
    /// instant they went in. A store that checked the file before writing learns whether
    /// another program wrote it between that check and this replace, and can keep what that
    /// program wrote instead of losing it. `Err` means nothing was replaced; `Ok` with
    /// `unsure` set means the new bytes are in place but their durability is not known
    /// (`Step::SyncDir`), and what they replaced is still handed back. A disk that cannot tell
    /// what it replaced answers [`Displaced::Unknown`].
    fn exchange(&self, path: &Path, bytes: &[u8]) -> Result<Exchanged, DiskError> {
        match self.write_atomically(path, bytes) {
            Ok(()) => Ok(Exchanged { displaced: Displaced::Unknown, unsure: None }),
            Err(e) if e.step == Step::SyncDir => Ok(Exchanged { displaced: Displaced::Unknown, unsure: Some(e) }),
            Err(e) => Err(e),
        }
    }
}

/// A replace that put the new bytes in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchanged {
    /// What the new bytes took the place of.
    pub displaced: Displaced,
    /// Set when the directory could not be synced after: the new bytes' durability is unknown.
    pub unsure: Option<DiskError>,
}

/// What a replace took the place of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Displaced {
    /// Nothing was at the path: the file was created.
    Nothing,
    /// These bytes were at the path at the instant the new ones replaced them.
    Bytes(Vec<u8>),
    /// A file was replaced, and is kept unread at this path (beside the original, under a
    /// hidden temp name), because it could not be read back safely: too large, or unreadable.
    Unread(PathBuf),
    /// The disk cannot tell (a filesystem without an atomic exchange).
    Unknown,
}

/// Temp files in this process, numbered, so two writes never share one.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

/// The real disk.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealDisk;

impl RealDisk {
    /// A temp file beside `path`, made by this call and by nothing else (`create_new`), private.
    /// For a replace (`wear_mode`) it then carries the destination's own mode; a create has no
    /// destination to take one from, and stays private.
    pub(super) fn create_temp(path: &Path, wear_mode: bool) -> std::io::Result<(PathBuf, std::fs::File)> {
        RealDisk::create_temp_as(path, wear_mode, "tmp")
    }

    /// [`RealDisk::create_temp`], named `.<file>.<pid>.<n>.<suffix>`.
    pub(super) fn create_temp_as(path: &Path, wear_mode: bool, suffix: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
        let parent = path.parent().ok_or_else(|| std::io::Error::other("no parent directory"))?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(".{name}.{}.{n}.{suffix}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(&temp)?;
        // A note made private stays private: the replacement wears the original's mode.
        if let Some(meta) = wear_mode.then(|| std::fs::metadata(path).ok()).flatten() {
            if let Err(e) = file.set_permissions(meta.permissions()) {
                drop(file);
                let _ = std::fs::remove_file(&temp);
                return Err(e);
            }
        }
        Ok((temp, file))
    }
}

impl Disk for RealDisk {
    fn write_atomically(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        let fail = |step: Step| move |e: std::io::Error| DiskError { step, error: e.to_string() };
        // A link is not replaced: a rename would swap the link for a file and leave the note it
        // pointed at stale, and writing through it would follow a path someone else chose.
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(DiskError { step: Step::Check, error: "the file is a link, and is not replaced".into() });
        }
        let (temp, mut file) = RealDisk::create_temp(path, true).map_err(fail(Step::CreateTemp))?;
        let result = (|| {
            file.write_all(bytes).map_err(fail(Step::Write))?;
            file.sync_all().map_err(fail(Step::SyncFile))?;
            std::fs::rename(&temp, path).map_err(fail(Step::Rename))?;
            let parent = path.parent().unwrap_or(Path::new("."));
            std::fs::File::open(parent).and_then(|d| d.sync_all()).map_err(fail(Step::SyncDir))
        })();
        // Only a temp this call made, and only while it is still a temp (not yet renamed).
        if matches!(&result, Err(e) if e.step != Step::SyncDir) {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }

    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        let fail = |step: Step| move |e: std::io::Error| DiskError { step, error: e.to_string() };
        let (temp, mut file) = RealDisk::create_temp(path, false).map_err(fail(Step::CreateTemp))?;
        let result = (|| {
            file.write_all(bytes).map_err(fail(Step::Write))?;
            file.sync_all().map_err(fail(Step::SyncFile))?;
            // A link fails with "File exists" when anything is at `path`: the check and the
            // publish are one step, so a file made meanwhile by someone else is never replaced.
            std::fs::hard_link(&temp, path).map_err(fail(Step::Rename))
        })();
        // The temp name is ours alone in every outcome: after the link it is a second name for
        // the new file, before it the only one.
        let _ = std::fs::remove_file(&temp);
        result?;
        let parent = path.parent().unwrap_or(Path::new("."));
        std::fs::File::open(parent).and_then(|d| d.sync_all()).map_err(fail(Step::SyncDir))
    }

    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    fn exchange(&self, path: &Path, bytes: &[u8]) -> Result<Exchanged, DiskError> {
        super::exchange::exchange(path, bytes)
    }
}

#[cfg(any(test, feature = "testing"))]
pub use faulty::FaultyDisk;

/// A disk that fails a chosen step, for an app's tests (the `testing` feature): never shipped.
#[cfg(any(test, feature = "testing"))]
mod faulty {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::{Disk, DiskError, RealDisk, Step};

    /// Fails a chosen step for paths that match, then behaves.
    #[derive(Debug, Default)]
    pub struct FaultyDisk {
        faults: Mutex<Vec<Fault>>,
        /// Paths the fault hit, in order, so a test can assert which write failed.
        pub hits: Mutex<Vec<PathBuf>>,
    }

    #[derive(Debug, Clone)]
    struct Fault {
        step: Step,
        /// A path ends with this to be hit (`".meta"`, `"pending.json"`, or `""` for every path).
        suffix: String,
        /// How many more times it fails; `None` for always.
        times: Option<usize>,
    }

    impl FaultyDisk {
        pub fn new() -> FaultyDisk {
            FaultyDisk::default()
        }

        /// Fails `step` for paths ending with `suffix`, `times` times (`None`: until healed).
        pub fn fail(&self, step: Step, suffix: &str, times: Option<usize>) {
            self.faults.lock().unwrap_or_else(|p| p.into_inner()).push(Fault { step, suffix: suffix.into(), times });
        }

        pub fn heal(&self) {
            self.faults.lock().unwrap_or_else(|p| p.into_inner()).clear();
        }

        fn fault_for(&self, path: &Path) -> Option<Step> {
            let mut faults = self.faults.lock().unwrap_or_else(|p| p.into_inner());
            let text = path.to_string_lossy();
            let at = faults.iter().position(|f| text.ends_with(&f.suffix) && f.times != Some(0))?;
            let step = faults[at].step;
            if let Some(n) = faults[at].times.as_mut() {
                *n -= 1;
            }
            self.hits.lock().unwrap_or_else(|p| p.into_inner()).push(path.to_path_buf());
            Some(step)
        }
    }

    impl Disk for FaultyDisk {
        fn write_atomically(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
            match self.fault_for(path) {
                // A failure before the rename leaves the destination as it was.
                Some(step @ (Step::Check | Step::CreateTemp | Step::Write | Step::SyncFile | Step::Rename)) => {
                    Err(DiskError { step, error: format!("injected failure at {step:?}") })
                }
                // A failure after the rename: the bytes are there, their durability is not known.
                Some(Step::SyncDir) => {
                    RealDisk.write_atomically(path, bytes)?;
                    Err(DiskError { step: Step::SyncDir, error: "injected failure at SyncDir".into() })
                }
                None => RealDisk.write_atomically(path, bytes),
            }
        }

        fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
            match self.fault_for(path) {
                // A failure before the link leaves nothing at the destination.
                Some(step @ (Step::Check | Step::CreateTemp | Step::Write | Step::SyncFile | Step::Rename)) => {
                    Err(DiskError { step, error: format!("injected failure at {step:?}") })
                }
                Some(Step::SyncDir) => {
                    RealDisk.create_new(path, bytes)?;
                    Err(DiskError { step: Step::SyncDir, error: "injected failure at SyncDir".into() })
                }
                None => RealDisk.create_new(path, bytes),
            }
        }

        fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
            RealDisk.read(path)
        }

        fn remove(&self, path: &Path) -> std::io::Result<()> {
            RealDisk.remove(path)
        }

        fn exchange(&self, path: &Path, bytes: &[u8]) -> Result<super::Exchanged, DiskError> {
            match self.fault_for(path) {
                Some(step @ (Step::Check | Step::CreateTemp | Step::Write | Step::SyncFile | Step::Rename)) => {
                    Err(DiskError { step, error: format!("injected failure at {step:?}") })
                }
                // In place, and what it replaced handed back, but the directory sync failed.
                Some(Step::SyncDir) => {
                    let done = RealDisk.exchange(path, bytes)?;
                    let injected = DiskError { step: Step::SyncDir, error: "injected failure at SyncDir".into() };
                    Ok(super::Exchanged { displaced: done.displaced, unsure: Some(injected) })
                }
                None => RealDisk.exchange(path, bytes),
            }
        }
    }
}
