//! Which apps the shell has launched, and whose windows they show: the account `describe shell`'s
//! `windows`, the dock's running marks and `launcher.running` are read from.
//!
//! One record per app id, keyed to the pid that owns the window. Our apps are single-instance: a
//! second copy started while one is open asks the first to show itself and exits, so a live
//! record is never taken over by a second copy, and that copy's exit never removes it.
//!
//! An app of the framework (`crate::framework`) is one instance with a window per display (#963):
//! it holds the person's windows and a Mind's together, so a launch pid cannot say whose windows
//! are open (#972 S1). For such an app the instance's own report decides
//! (`yantrik_ipc_contracts::mind_view::window_owners`):
//! - a launch on a Mind's display never writes the person's record;
//! - the person's record lists the app only while the report has a window of the person's open;
//! - when the launch that holds the record exits while the instance still has the person's window
//!   open, the record moves to the instance rather than going.
//!
//! Every other program keeps the launch-pid account, less what is drawing in Mind View.

use std::collections::{HashMap, HashSet};

use yantrik_ipc_contracts::mind_view::window_owners::Report;

use crate::framework;

/// Where a launch was drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    /// The person's desktop.
    Person,
    /// A Mind's display (Mind View).
    Mind,
}

/// One app the shell launched and has not yet reaped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// The logical id the shell launched it under, e.g. `notes`.
    pub app_id: String,
    pub pid: u32,
    /// The binary that was spawned, e.g. `yantrik-notes` or `chromium`.
    pub binary: String,
    /// Unix seconds when it was launched.
    pub since_unix: u64,
}

/// The shell's account of what it launched.
#[derive(Debug, Default)]
pub struct Seats {
    apps: HashMap<String, Record>,
}

impl Seats {
    /// The shell started `record` on `seat`. Says whether the record is now this pid's.
    pub fn launched(&mut self, record: Record, seat: Seat, alive: impl Fn(u32) -> bool) -> bool {
        if seat == Seat::Mind && framework::by_program(&record.binary).is_some() {
            // The Mind's window, which is not the person's to list.
            return false;
        }
        if let Some(open) = self.apps.get(&record.app_id) {
            if open.pid != record.pid && alive(open.pid) {
                return false;
            }
        }
        self.apps.insert(record.app_id.clone(), record);
        true
    }

    /// The process `pid`, launched as `app_id`, has exited. `report` is an app of the framework's
    /// own report of its windows, by its program.
    pub fn exited(&mut self, app_id: &str, pid: u32, report: impl Fn(&str) -> Option<Report>, alive: impl Fn(u32) -> bool) {
        let Some(open) = self.apps.get_mut(app_id).filter(|open| open.pid == pid) else { return };
        let instance = framework::by_program(&open.binary)
            .and(report(&open.binary))
            .filter(|r| r.person > 0 && r.pid != pid && alive(r.pid));
        match instance {
            // A second copy handed the person's launch over and left: the window is the instance's.
            Some(r) => open.pid = r.pid,
            None => {
                self.apps.remove(app_id);
            }
        }
    }

    /// Every record whose process is alive, by id.
    pub fn records(&self, alive: impl Fn(u32) -> bool) -> Vec<Record> {
        let mut apps: Vec<Record> = self.apps.values().filter(|a| alive(a.pid)).cloned().collect();
        apps.sort_by(|a, b| a.app_id.cmp(&b.app_id));
        apps
    }

    /// The apps with a window on the person's desktop. An app of the framework that reports its
    /// windows is listed while it has one of the person's; every other live record is listed
    /// unless it is drawing in Mind View (`in_mind_view`, the pids launched there).
    pub fn persons(
        &self,
        in_mind_view: &HashSet<u32>,
        alive: impl Fn(u32) -> bool,
        report: impl Fn(&str) -> Option<Report>,
    ) -> Vec<Record> {
        let mut out = self.records(alive);
        out.retain(|a| match framework::by_program(&a.binary).and(report(&a.binary)) {
            Some(r) => r.person > 0,
            None => !in_mind_view.contains(&a.pid),
        });
        out
    }
}

/// The report the instance of the framework app `app` (default profile) keeps in `dir`, the
/// session's socket directory: a file of this account's, small, and naming a live process. Anything
/// else is no report, and the shell falls back to the launch pids.
pub fn read_report(dir: &std::path::Path, app: &framework::FrameworkApp, alive: impl Fn(u32) -> bool) -> Option<Report> {
    use yantrik_ipc_contracts::mind_view::window_owners::{file_name, MAX_BYTES};
    let path = dir.join(file_name(app.id));
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() as usize > MAX_BYTES || !ours(&meta) {
        return None;
    }
    let report = Report::parse(&std::fs::read_to_string(&path).ok()?)?;
    alive(report.pid).then_some(report)
}

#[cfg(unix)]
fn ours(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid cannot fail and touches no memory.
    meta.uid() == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn ours(_meta: &std::fs::Metadata) -> bool {
    true
}

#[cfg(test)]
#[path = "seats_tests.rs"]
mod tests;
