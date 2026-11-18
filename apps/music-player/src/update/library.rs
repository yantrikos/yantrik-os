//! The library: the scan of every folder, off the UI thread, and the folder list, kept as it
//! changes.
//!
//! Scans are numbered as they start. The library in hand is the latest scan's that has landed:
//! one that lands after a later one changes nothing on screen, though the call that started it
//! is still answered with what it found. One scan runs at a time: a walk of a large library
//! holds a job thread for its whole length, and the player's commands share those threads. The
//! person's Rescan, and a folder added or removed, while a scan runs, scan once more when it
//! lands, as before. A `rescan` from the Mind while a scan runs is refused with a sentence that
//! says where to read its progress.

use serde_json::{json, Value};
use yantrik_app::cx::Inbound;

use crate::answers::{agent, owed, refuse, settle};
use crate::command::{Command, Scanned, Work};
use crate::kept::Kept;
use crate::library;
use crate::model::{Inbox, Model, NOTHING, PART_LISTS, PART_STATUS};
use crate::seen;

use super::{kept, Fx};

/// What a `rescan` asked while a scan runs is told.
pub const SCANNING: &str =
    "Music is already scanning the library; `describe` shows its progress, and `rescan` can be asked again once `scanning` is false.";

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) {
    match command {
        Command::Rescan if owed(fx) && m.scans.running > 0 => refuse(m, fx, "busy", SCANNING),
        Command::Rescan if owed(fx) => scan(m, fx),
        Command::Rescan => rescan(m, fx),
        Command::Scanned(scanned) => landed(m, *scanned, fx),
        Command::Scanning { scan, found } => {
            if scan == m.scans.started && m.scans.running > 0 {
                m.status = format!("Scanning\u{2026} {} so far", library::count(found, "track", "tracks"));
                m.agent_status = None;
                fx.changed(PART_STATUS);
            } else {
                fx.changed(NOTHING);
            }
        }
        Command::Filter(text) => {
            m.filter = text;
            m.refilter();
            fx.changed(PART_LISTS);
            fx.changed(PART_STATUS);
        }
        Command::AddFolder(raw) => {
            let raw = raw.trim().to_string();
            let path = yantrik_foundation::home::expand(&raw);
            if !path.is_absolute() {
                return refuse(
                    m,
                    fx,
                    "bad_folder",
                    format!("Give the folder's full path, starting with / or ~/ (not \u{201c}{}\u{201d}).", library::plain(&raw)),
                );
            }
            fx.spawn_or(Command::WorkFailed(Work::Folder), move |_| {
                let path = std::fs::canonicalize(&path).unwrap_or(path);
                Command::FolderChecked { is_folder: path.is_dir(), path: path.to_string_lossy().into_owned() }
            });
            fx.changed(NOTHING);
        }
        Command::FolderChecked { path, is_folder } => {
            if m.folders.contains(&path) {
                return refuse(m, fx, "bad_folder", format!("{} is already in the list.", library::plain(&path)));
            }
            if !is_folder {
                return refuse(m, fx, "bad_folder", format!("Not a folder: {}", library::plain(&path)));
            }
            m.folders.push(path);
            m.per_folder.push(0);
            m.notice.clear();
            fx.save_settings(kept(m));
            rescan(m, fx);
        }
        Command::RemoveFolder(row) => {
            let row = row.max(0) as usize;
            if row >= m.folders.len() {
                return refuse(m, fx, "bad_folder", "That folder is not in the list.");
            }
            m.folders.remove(row);
            if row < m.per_folder.len() {
                m.per_folder.remove(row);
            }
            fx.save_settings(kept(m));
            rescan(m, fx);
        }
        _ => {}
    }
}

/// The settings file was edited outside the app: its folders and volume are the app's now, and
/// a changed folder list is scanned.
pub fn adopt(m: &mut Model, kept: Kept, fx: &mut Fx) {
    m.volume = kept.volume;
    if let Some(folders) = kept.folders.filter(|f| *f != m.folders) {
        m.per_folder = vec![0; folders.len()];
        m.folders = folders;
        rescan(m, fx);
    }
}

/// The person's rescan: once a scan running now has landed, if one is.
fn rescan(m: &mut Model, fx: &mut Fx) {
    if m.scans.running > 0 {
        m.scans.again = true;
        fx.changed(PART_LISTS);
        return;
    }
    scan(m, fx);
}

/// Scan every folder in a job. The window stays live; progress comes back every hundred tracks.
pub fn scan(m: &mut Model, fx: &mut Fx) {
    m.scans.started += 1;
    m.scans.running += 1;
    let scan = m.scans.started;
    m.status = if m.folders.is_empty() {
        String::new()
    } else {
        format!("Scanning {}\u{2026}", library::count(m.folders.len(), "folder", "folders"))
    };
    m.agent_status = None;
    let folders = m.folders.clone();
    let inbox = m.inbox.clone();
    fx.spawn_or(Command::WorkFailed(Work::Scan), move |_| {
        let mut progress = |found: usize| {
            if let Some(Inbox(post)) = &inbox {
                let _ = post.post(Inbound::Command(Command::Scanning { scan, found }));
            }
        };
        let found = library::scan_all(&folders, &mut progress);
        let for_agents = folders.iter().map(|f| (f.clone(), seen::for_agents(f))).collect();
        Command::Scanned(Box::new(Scanned { scan, folders, found, for_agents }))
    });
    fx.changed(PART_LISTS);
    fx.changed(PART_STATUS);
}

/// A scan landed: the call that started it is answered from it, and it is what the window shows
/// unless a later scan has landed first.
fn landed(m: &mut Model, s: Scanned, fx: &mut Fx) {
    m.scans.running = m.scans.running.saturating_sub(1);
    let may = |f: &str| s.for_agents.iter().any(|(g, ok)| g == f && *ok);
    let shown = |f: &str, agent: bool| seen::shown(f, agent, may(f));
    let said = |agent: bool| {
        if s.folders.is_empty() {
            "No library folders. Add one under Folders.".to_string()
        } else {
            library::summary(&s.found, s.folders.len(), |f| shown(f, agent))
        }
    };
    if owed(fx) {
        let agent = agent(fx);
        let answer = json!({
            "library_size": s.found.tracks.len(),
            "folders": s.folders.iter().zip(&s.found.per_folder)
                .map(|(f, n)| json!({"path": shown(f, agent), "tracks": n})).collect::<Vec<Value>>(),
            "unreadable": s.found.unreadable.iter()
                .map(|(f, why)| json!({"path": shown(f, agent), "why": why})).collect::<Vec<Value>>(),
            "summary": said(agent),
        });
        settle(m, fx, answer);
    }
    if s.scan > m.scans.shown {
        m.scans.shown = s.scan;
        // Counts belong to the folders that were scanned; a folder added meanwhile reads 0
        // until the scan that follows.
        m.per_folder = m
            .folders
            .iter()
            .map(|f| s.folders.iter().position(|g| g == f).map(|i| s.found.per_folder[i]).unwrap_or(0))
            .collect();
        m.folders_for_agents = s.for_agents.iter().cloned().collect();
        m.status = said(false);
        m.agent_status = Some(said(true)).filter(|a| *a != m.status);
        m.library = s.found.tracks;
        m.refilter();
    }
    if std::mem::take(&mut m.scans.again) {
        scan(m, fx);
    }
}
