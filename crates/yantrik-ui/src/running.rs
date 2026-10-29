//! Which apps the shell has launched, and whether they are still alive.
//!
//! The taskbar and `describe shell` used to answer "what is open" by shelling out to
//! `wlrctl toplevel list` and guessing an app id from each window's title. That was wrong twice
//! over: `wlrctl` is not always installed (a fresh image had no window list at all, so
//! `describe shell` said "0 windows open" while three apps were running), and even when it is,
//! our Slint windows do not carry a distinct Wayland app id, so the title heuristic collapsed
//! every Yantrik window onto one id.
//!
//! But the shell does not need to ask the compositor about its own children — it *started* them.
//! It knows the id it launched, the pid it got back, and, through the reaper that already waits
//! on every child, the moment it exits. That is a more reliable account of what is open than any
//! query, and it needs neither a subprocess nor a Wayland protocol. This module is that account.
//!
//! It tracks only windowed apps — the separate processes the dock spawns. The dock's other
//! entries (files, settings, memory…) are *screens of the shell itself*, not windows, and are
//! already reported by the shell's `screen` field.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use yantrik_ipc_contracts::mind_view::window_owners::Report;
use yantrik_shell_core::seats::Seats;

/// One app the shell launched and has not yet reaped: its id (the one `open_app` accepts, e.g.
/// `notes`), pid, the binary that was spawned, and when, in Unix seconds.
pub use yantrik_shell_core::seats::Record as RunningApp;

/// A launch that started and then died before it could show a window.
#[derive(Clone, Debug)]
pub struct LaunchFailure {
    pub app_id: String,
    /// The binary that was spawned.
    pub binary: String,
    /// How the process ended, as the OS reported it.
    pub status: String,
    /// How long it survived, in milliseconds.
    pub lived_ms: u64,
    pub at_unix: u64,
}

fn failures() -> &'static Mutex<HashMap<String, LaunchFailure>> {
    static FAILED: OnceLock<Mutex<HashMap<String, LaunchFailure>>> = OnceLock::new();
    FAILED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How long a process must survive before we stop calling it a failed launch.
///
/// An app that exits inside this window never showed the user anything. Chromium, launched
/// without DISPLAY on a Wayland-only session, printed "Missing X server" and was gone in under
/// a second — while `open_app` had already answered `accepted: true`. Acceptance is not arrival.
pub const LAUNCH_GRACE_MS: u64 = 3_000;

/// Record that a launch died before it could become a window.
pub fn mark_launch_failed(app_id: &str, binary: &str, status: &str, lived_ms: u64) {
    if let Ok(mut map) = failures().lock() {
        map.insert(
            app_id.to_string(),
            LaunchFailure {
                app_id: app_id.to_string(),
                binary: binary.to_string(),
                status: status.to_string(),
                lived_ms,
                at_unix: now_unix(),
            },
        );
    }
}

/// Clear any past failure for an id, because it has just been launched again.
pub fn clear_launch_failure(app_id: &str) {
    if let Ok(mut map) = failures().lock() {
        map.remove(app_id);
    }
}

/// The last failed launch for `app_id`, if it failed rather than opened.
pub fn last_launch_failure(app_id: &str) -> Option<LaunchFailure> {
    failures().lock().ok().and_then(|m| m.get(app_id).cloned())
}

/// Every launch that has failed and not since succeeded, newest first.
pub fn launch_failures() -> Vec<LaunchFailure> {
    let mut out: Vec<LaunchFailure> =
        failures().lock().map(|m| m.values().cloned().collect()).unwrap_or_default();
    out.sort_by(|a, b| b.at_unix.cmp(&a.at_unix));
    out
}

/// The account of what the shell launched, and whose windows they show
/// (`yantrik_shell_core::seats`): an app of the framework may run with no window at all, started
/// on demand for an agent, so its own report says whether the person has one, not the pid the
/// shell launched.
fn registry() -> &'static Mutex<Seats> {
    static RUNNING: OnceLock<Mutex<Seats>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(Seats::default()))
}

/// An app of the framework's own report of its windows, by its program.
fn report_for(binary: &str) -> Option<Report> {
    let app = yantrik_shell_core::framework::by_program(binary)?;
    yantrik_shell_core::seats::read_report(&yantrik_ipc_transport::server::socket_dir(), app, pid_alive)
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Record that the shell has started `app_id` as pid `pid`. Says whether the record is now this
/// pid's.
///
/// One entry per id, and the entry belongs to the process that owns the window. Our apps are
/// single-instance: start `yantrik-notes` while Notes is open and the second copy asks the first
/// to show itself, then exits within milliseconds. Replacing the record with that copy's pid lost
/// the window twice over — the shell reported the app as launched a moment ago, and when the copy
/// exited, `mark_exited` matched and removed Notes from "what is open" while its window was still
/// on screen.
///
/// So a live record is left alone, and the caller is told, because a second copy exiting at once
/// is a handover rather than the failed launch it looks like.
pub fn mark_launched(app_id: &str, pid: u32, binary: &str) -> bool {
    let record = RunningApp { app_id: app_id.to_string(), pid, binary: binary.to_string(), since_unix: now_unix() };
    registry().lock().map(|mut seats| seats.launched(record, pid_alive)).unwrap_or(false)
}

/// Record that pid `pid`, launched as `app_id`, has exited.
///
/// Removes the entry only if the pid still matches. If the app was relaunched, the record now
/// holds a newer pid, and the old reaper firing must not evict the live window.
pub fn mark_exited(app_id: &str, pid: u32) {
    if let Ok(mut seats) = registry().lock() {
        seats.exited(app_id, pid, report_for, pid_alive);
    }
}

/// Whether pid is still a live process.
///
/// A reaper thread removes an app the instant it exits, but a hard kill can leave a moment where
/// the record still stands and the process is gone; checking `/proc` closes that gap so a caller
/// is never told a dead app is running.
#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    // No /proc off Linux; trust the reaper. This path is development-only.
    true
}

/// Every app the shell has open right now, sorted by id for a stable read.
pub fn running() -> Vec<RunningApp> {
    registry().lock().map(|seats| seats.records(pid_alive)).unwrap_or_default()
}

/// The apps with a window on the person's desktop: an app of the framework by its own report of
/// its windows, every other one unless it is drawing in Mind View (`in_mind_view`).
pub fn persons(in_mind_view: &HashSet<u32>) -> Vec<RunningApp> {
    registry().lock().map(|seats| seats.persons(in_mind_view, pid_alive, report_for)).unwrap_or_default()
}

/// Whether the shell has this app open.
pub fn is_running(app_id: &str) -> bool {
    running().iter().any(|a| a.app_id == app_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second copy of a single-instance app does not take the record from the window that is
    /// open, and does not take it away when it exits a moment later.
    #[cfg(unix)]
    #[test]
    fn a_second_copy_does_not_evict_the_window_that_is_open() {
        let app = "notes-test-second-copy";
        let open = std::process::id();
        assert!(mark_launched(app, open, "yantrik-notes"));
        let second = u32::MAX; // never a live pid
        assert!(!mark_launched(app, second, "yantrik-notes"), "the open window keeps the record");
        mark_exited(app, second);
        assert!(is_running(app), "the window is still open");
        mark_exited(app, open);
        assert!(!is_running(app));
    }

    /// A record left behind by a process that has gone is replaced, not honoured.
    #[cfg(unix)]
    #[test]
    fn a_dead_record_does_not_block_a_relaunch() {
        let app = "notes-test-dead-record";
        assert!(mark_launched(app, u32::MAX, "yantrik-notes"));
        assert!(mark_launched(app, std::process::id(), "yantrik-notes"));
        assert!(is_running(app));
        mark_exited(app, std::process::id());
    }
}
