//! What the shell's own `files_delete` names its target as on an approval card: the path and the
//! size of the entry in the folder Files is showing, read from the disk now (`approval_target`).
//!
//! The namer is the shell's answer to its own `app.name_target`, so it reads the folder from the
//! window the action itself acts in — the same `file_browser_path` and the same `child_path` the
//! delete joins — and never from anything the request said beyond the name it asks about.

use std::cell::RefCell;
use std::path::Path;

use yantrik_app_runtime::control::Target;

use crate::App;

thread_local! {
    /// The window `files_delete` acts in, for its namer, which runs on the same (UI) thread.
    static FILES_UI: RefCell<slint::Weak<App>> = RefCell::new(slint::Weak::default());
}

/// Remember the window for [`named`]. Called once where the Files actions are published.
pub fn remember(ui: &App) {
    use slint::ComponentHandle;
    FILES_UI.with(|cell| *cell.borrow_mut() = ui.as_weak());
}

/// `files_delete`'s target: the entry called `args.name` in the folder on screen, or `None` when
/// Files is not up, the name is empty or not a plain entry name, or nothing is there.
pub fn named(args: &serde_json::Value) -> Option<Target> {
    let name = args["name"].as_str()?.trim();
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return None;
    }
    let ui = FILES_UI.with(|cell| cell.borrow().upgrade())?;
    let shown = crate::filebrowser::child_path(&ui.get_file_browser_path(), name);
    target_for(&crate::filebrowser::expand_home(&shown), &shown)
}

/// The rows for the entry at `path`, shown as `shown` (`~/…`): its path, and its size — or, for a
/// folder, how many entries it holds. A link is named as a link; what it points to is not deleted.
pub fn target_for(path: &Path, shown: &str) -> Option<Target> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let size = if meta.file_type().is_symlink() {
        "a link; what it points to is not touched".to_string()
    } else if meta.is_dir() {
        let entries = std::fs::read_dir(path).map(|d| d.count()).unwrap_or(0);
        format!("folder, {entries} item{} inside", if entries == 1 { "" } else { "s" })
    } else {
        crate::filebrowser::format_size(meta.len())
    };
    Some(Target {
        rows: vec![("Path".into(), shown.to_string()), ("Size".into(), size)],
        series: false,
        handles: vec!["name".into()],
    })
}

#[cfg(test)]
mod tests {
    use super::target_for;

    #[test]
    fn a_file_is_named_by_path_and_size_a_folder_by_its_entries_and_nothing_by_nothing() {
        let dir = std::env::temp_dir().join(format!("files-target-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        std::fs::write(dir.join("report.pdf"), vec![0u8; 2048]).unwrap();
        std::fs::write(dir.join("folder/a.txt"), b"a").unwrap();

        let file = target_for(&dir.join("report.pdf"), "~/report.pdf").unwrap();
        assert_eq!(file.rows[0], ("Path".to_string(), "~/report.pdf".to_string()));
        assert_eq!(file.rows[1], ("Size".to_string(), "2.0 KB".to_string()));
        assert_eq!(file.handles, ["name"]);
        assert_eq!(target_for(&dir.join("folder"), "~/folder").unwrap().rows[1].1, "folder, 1 item inside");
        assert!(target_for(&dir.join("missing"), "~/missing").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
