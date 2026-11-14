//! `HOME` is process-wide and tests run in parallel. Every test that sets it holds [`Home`], which
//! takes the one crate-wide lock, points `HOME` at a folder of its own, and puts back what was
//! there when it is dropped. A test that sets `HOME` any other way races with the rest.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static HOME_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct Home {
    dir: PathBuf,
    made: bool,
    previous: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl Home {
    /// `HOME` at a new, empty folder of this process's temp dir, removed when dropped.
    pub(crate) fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("yantrik-home-{}-{tag}", std::process::id()));
        let lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp home");
        Self::set(dir, true, lock)
    }

    /// `HOME` at `dir` as it is (a path a test only names), left alone when dropped.
    pub(crate) fn at(dir: &Path) -> Self {
        let lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        Self::set(dir.to_path_buf(), false, lock)
    }

    fn set(dir: PathBuf, made: bool, lock: MutexGuard<'static, ()>) -> Self {
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", &dir);
        Home { dir, made, previous, _lock: lock }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        match &self.previous {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        if self.made {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}
