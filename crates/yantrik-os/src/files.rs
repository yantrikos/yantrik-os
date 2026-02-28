//! File system watcher — monitors directories via notify (inotify on Linux).
//!
//! Watches ~/Downloads, ~/Documents, ~/Desktop by default.
//! Emits FileChanged events for create/modify/delete/rename.

use crossbeam_channel::Sender;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::path::PathBuf;

use crate::events::{FileChangeKind, SystemEvent};

/// Main loop for the file watcher thread.
/// Blocks forever, emitting FileChanged events.
pub fn run_file_watcher(tx: Sender<SystemEvent>, watch_dirs: &[String]) {
    let tx_clone = tx.clone();

    let mut watcher = match notify::recommended_watcher(move |result: Result<Event, _>| {
        if let Ok(event) = result {
            handle_event(&tx_clone, event);
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create file watcher");
            return;
        }
    };

    // Expand ~ and watch each directory
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    for dir in watch_dirs {
        let expanded = dir.replace('~', &home);
        let path = PathBuf::from(&expanded);
        if path.exists() {
            match watcher.watch(&path, RecursiveMode::NonRecursive) {
                Ok(()) => tracing::info!(dir = %expanded, "Watching directory"),
                Err(e) => tracing::warn!(dir = %expanded, error = %e, "Failed to watch"),
            }
        } else {
            tracing::debug!(dir = %expanded, "Watch dir does not exist, skipping");
        }
    }

    // Block forever — the watcher's callback runs on notify's internal thread
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

fn handle_event(tx: &Sender<SystemEvent>, event: Event) {
    let paths = &event.paths;
    if paths.is_empty() {
        return;
    }

    let kind = match event.kind {
        EventKind::Create(_) => FileChangeKind::Created,
        EventKind::Modify(_) => FileChangeKind::Modified,
        EventKind::Remove(_) => FileChangeKind::Deleted,
        _ => return,
    };

    for path in paths {
        let path_str = path.to_string_lossy().to_string();
        let _ = tx.send(SystemEvent::FileChanged {
            path: path_str,
            kind: kind.clone(),
        });
    }
}
