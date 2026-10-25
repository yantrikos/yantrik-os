//! Where the library's file is written and read back from: the real disk, or under a test a disk
//! that can be told to refuse, so a failed write is tested without depending on who runs the
//! tests (a read-only directory does not stop root).

use std::path::Path;
use std::sync::Arc;

use crate::store::{file, Library};

/// The library's file, as the write lane reaches it. Blocking: from a job only.
pub trait Disk: Send + Sync {
    /// The whole file replaced by `bytes`, or left exactly as it was, with the reason in words.
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String>;
    /// What the file holds now.
    fn read_back(&self, path: &Path) -> Result<Library, String>;
}

/// The disk: Foundation's atomic writer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Real;

impl Disk for Real {
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        file::write(path, bytes)
    }

    fn read_back(&self, path: &Path) -> Result<Library, String> {
        file::read_back(path)
    }
}

/// The disk an app at these paths writes to: the real one, or a test's.
pub fn for_app(paths: &yantrik_app::Paths) -> Arc<dyn Disk> {
    #[cfg(test)]
    if let Some(disk) = testing::installed(paths) {
        return disk;
    }
    let _ = paths;
    Arc::new(Real)
}

#[cfg(test)]
pub mod testing {
    //! A disk that writes for real until it is told to refuse.

    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    use super::{Disk, Real};
    use crate::store::Library;

    /// The sentence a refused write gives, as an OS error would.
    pub const REFUSED: &str = "No space left on device (os error 28)";

    #[derive(Default)]
    pub struct Faulty {
        refuse: AtomicBool,
        panic: AtomicBool,
        fail_read: AtomicBool,
        panic_read: AtomicBool,
        writes: AtomicUsize,
    }

    impl Faulty {
        pub fn refuse(&self, on: bool) {
            self.refuse.store(on, Ordering::SeqCst);
        }

        /// The next write panics, as a job that ends without a result.
        pub fn panic_next(&self) {
            self.panic.store(true, Ordering::SeqCst);
        }

        /// Reading the file back fails, as an I/O error would.
        pub fn fail_reads(&self, on: bool) {
            self.fail_read.store(on, Ordering::SeqCst);
        }

        /// The next read back panics, as a job that ends without a result.
        pub fn panic_next_read(&self) {
            self.panic_read.store(true, Ordering::SeqCst);
        }

        /// Writes that reached the disk.
        pub fn writes(&self) -> usize {
            self.writes.load(Ordering::SeqCst)
        }
    }

    impl Disk for Faulty {
        fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
            if self.panic.swap(false, Ordering::SeqCst) {
                panic!("the test's disk gave way");
            }
            if self.refuse.load(Ordering::SeqCst) {
                return Err(format!("{}: {REFUSED}", path.display()));
            }
            Real.write(path, bytes)?;
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn read_back(&self, path: &Path) -> Result<Library, String> {
            if self.panic_read.swap(false, Ordering::SeqCst) {
                panic!("the test's disk gave way on a read");
            }
            if self.fail_read.load(Ordering::SeqCst) {
                return Err(format!("{}: Input/output error (os error 5)", path.display()));
            }
            Real.read_back(path)
        }
    }

    fn installed_disks() -> &'static Mutex<HashMap<PathBuf, Arc<Faulty>>> {
        static DISKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Faulty>>>> = OnceLock::new();
        DISKS.get_or_init(Default::default)
    }

    /// The disk the app at `paths` will write to, from its next start.
    pub fn install(paths: &yantrik_app::Paths) -> Arc<Faulty> {
        let disk = Arc::new(Faulty::default());
        installed_disks()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(paths.data_dir().to_path_buf(), disk.clone());
        disk
    }

    pub(super) fn installed(paths: &yantrik_app::Paths) -> Option<Arc<dyn Disk>> {
        installed_disks()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(paths.data_dir())
            .map(|d| d.clone() as Arc<dyn Disk>)
    }
}
