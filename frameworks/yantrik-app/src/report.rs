//! The instance's report of how many windows of the person's it has open, for the shell
//! (#978 step 1).
//!
//! An instance may run with no window at all: an agent's act started it, or the person closed
//! the last window while an agent was still using it. So the pid the shell launched cannot say
//! whether the person has a window. The instance can: it writes the count beside its socket every
//! time it changes (`yantrik_ipc_contracts::mind_view::window_owners`), and removes the file when
//! it ends. The shell lists the person's windows from it.
//!
//! Written whole to a temporary name and renamed over the report, so a reader never sees half of
//! one, in the session's private socket directory, readable by this account only.

use std::cell::RefCell;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::mind_view::window_owners::{file_name, Report};

/// Writes one instance's report.
pub struct Reporter {
    path: PathBuf,
    last: RefCell<Option<Report>>,
}

impl Reporter {
    /// The report of the instance `instance`, in the session's socket directory.
    pub fn for_instance(instance: &str) -> Reporter {
        Reporter::at(&yantrik_ipc_transport::server::socket_dir(), instance)
    }

    /// The report of `instance` in `dir`.
    pub fn at(dir: &Path, instance: &str) -> Reporter {
        Reporter { path: dir.join(file_name(instance)), last: RefCell::new(None) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The windows now: `person` of the person's. Nothing is written when nothing changed.
    pub fn write(&self, person: u32) {
        let report = Report { pid: std::process::id(), person };
        if self.last.borrow().as_ref() == Some(&report) {
            return;
        }
        match self.write_now(&report) {
            Ok(()) => *self.last.borrow_mut() = Some(report),
            Err(e) => tracing::warn!(path = %self.path.display(), error = %e, "the windows report was not written"),
        }
    }

    fn write_now(&self, report: &Report) -> std::io::Result<()> {
        let temporary = self.path.with_extension(format!("windows.{}", std::process::id()));
        let _ = std::fs::remove_file(&temporary);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(report.encode().as_bytes())?;
        drop(file);
        std::fs::rename(&temporary, &self.path)
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_is_written_whole_on_a_change_and_removed_at_the_end() {
        let dir = std::env::temp_dir().join(format!("yantrik-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let reporter = Reporter::at(&dir, "notes");
        reporter.write(1);
        let read = || Report::parse(&std::fs::read_to_string(reporter.path()).unwrap()).unwrap();
        assert_eq!(read(), Report { pid: std::process::id(), person: 1 });
        reporter.write(0);
        assert_eq!(read().person, 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(reporter.path()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let path = reporter.path().to_path_buf();
        drop(reporter);
        assert!(!path.exists(), "removed when the instance ends");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "no temporary left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
