//! What the shell's own `files_delete` names its target as on an approval card: the entry's name
//! and path, and its size, read from the disk (`approval_target`).
//!
//! The namer is the shell's answer to its own `app.name_target`, so it reads the folder from the
//! window the action itself acts in, and resolves the name with [`entry`] — the one function the
//! handler resolves it with too, untrimmed, so the two cannot name different things (security
//! review of #652, L4). The answer's identity is the entry's real path with its device and inode
//! ([`identity_of`]): the handler holds its own resolution to it when the grant is spent, so a
//! `files_go` between the Allow and the delete trashes nothing the card did not name (H1).
//!
//! The namer runs on the UI thread, inside `request_approval`. It reads only the folder on screen
//! there; the disk is read on a worker the card waits for at most [`PATIENCE`], and a folder's
//! entries are counted only up to [`COUNTED`] (M2). A slow disk names nothing, which leaves a
//! destructive card Decline only. And a mind is held to what it may see before anything is read
//! (L3): a name in a folder it may not open is not named, whether or not it is there.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use yantrik_app_runtime::control::Target;

use crate::control_files_mind as mind;
use crate::App;

/// How long the card waits for the disk to say what an entry is.
pub const PATIENCE: Duration = Duration::from_millis(500);

/// A folder's entries are counted up to this many, then said as "more than".
pub const COUNTED: usize = 10_000;

thread_local! {
    /// The window `files_delete` acts in, for its namer, which runs on the same (UI) thread.
    static FILES_UI: RefCell<slint::Weak<App>> = RefCell::new(slint::Weak::default());
}

/// Remember the window for [`named`]. Called once where the Files actions are published.
pub fn remember(ui: &App) {
    use slint::ComponentHandle;
    FILES_UI.with(|cell| *cell.borrow_mut() = ui.as_weak());
}

/// The entry `name` names in the folder Files shows as `label`: its path on disk, and as the
/// screen spells it (`~/…`). `None` for a name that is not one plain entry — empty, `.`, `..`, or
/// holding a `/` — and in Trash, which lists names from anywhere and deletes nothing. Exactly as
/// given: what the handler is handed is what it deletes, so nothing is trimmed here either.
pub fn entry(label: &str, name: &str) -> Option<(PathBuf, String)> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." || label == "Trash" {
        return None;
    }
    let shown = crate::filebrowser::child_path(label, name);
    Some((crate::filebrowser::expand_home(&shown), shown))
}

/// What `path` is, however it is spelled or moved to: its folder's real path with its own name,
/// and the device and inode the entry itself has (a link is the link, not what it points to).
/// `None` when nothing is there.
pub fn identity_of(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).ok()?;
    let real_folder = path.parent()?.canonicalize().ok()?;
    let real = real_folder.join(path.file_name()?);
    Some(format!("{} \u{00b7} dev {} inode {}", real.display(), meta.dev(), meta.ino()))
}

/// `files_delete`'s target: the entry called `args.name` in the folder on screen, or `None` when
/// Files is not up, the name is not one plain entry, a mind may not touch it, nothing is there,
/// or the disk did not answer within [`PATIENCE`].
pub fn named(args: &serde_json::Value) -> Option<Target> {
    let name = args["name"].as_str()?;
    let ui = FILES_UI.with(|cell| cell.borrow().upgrade())?;
    let label = ui.get_file_browser_path().to_string();
    // The same rules the delete itself meets, before the disk is looked at.
    mind::here(&label).ok()?;
    mind::may_touch(&label, &[name.to_string()]).ok()?;
    let (path, shown) = entry(&label, name)?;
    off_this_thread(move || target_for(&path, &shown), PATIENCE)
}

/// `work`'s answer from a worker, or `None` when it took longer than `patience`. A worker still
/// waiting on a hung disk is left to finish on its own; nobody reads its answer.
fn off_this_thread<T: Send + 'static>(work: impl FnOnce() -> Option<T> + Send + 'static, patience: Duration) -> Option<T> {
    let (send, answer) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("files-target".into())
        .spawn(move || {
            let _ = send.send(work());
        })
        .ok()?;
    answer.recv_timeout(patience).ok().flatten()
}

/// The rows for the entry at `path`, shown as `shown` (`~/…`): its name first and then the path
/// it is at — a long path is cut on the card, its name never — and its size, or for a folder how
/// many entries it holds, counted up to [`COUNTED`]. A link is named as a link; what it points
/// to is not deleted.
pub fn target_for(path: &Path, shown: &str) -> Option<Target> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let identity = identity_of(path)?;
    let size = if meta.file_type().is_symlink() {
        "a link; what it points to is not touched".to_string()
    } else if meta.is_dir() {
        folder(std::fs::read_dir(path).map(|d| d.take(COUNTED + 1).count()).unwrap_or(0))
    } else {
        crate::filebrowser::format_size(meta.len())
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Some(Target {
        rows: vec![("Item".into(), format!("{name} \u{00b7} {shown}")), ("Size".into(), size)],
        series: false,
        identity,
    })
}

/// "folder, 3 items inside", for `entries` counted up to [`COUNTED`] and one past it.
fn folder(entries: usize) -> String {
    match entries {
        n if n > COUNTED => format!("folder, more than {COUNTED} items inside"),
        1 => "folder, 1 item inside".to_string(),
        n => format!("folder, {n} items inside"),
    }
}

#[cfg(test)]
#[path = "control_files_target_tests.rs"]
mod tests;
