//! A `.desktop` file on the desktop, resolved against the apps the dock knows.
//!
//! The file is read for its name and icon, but what it opens is decided by the app registry the
//! dock uses (`wire::dock`): the shortcut's file name (and its name) are looked up there, and a
//! shortcut that finds nothing is drawn as what it is, a shortcut to an app this machine does
//! not have, and says so when it is opened. A file dropped on the desktop therefore cannot start
//! a program the dock itself would not start; its `Exec` line is never run.

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use yantrik_bench::items::Item;
use yantrik_bench::shortcut::{self, Shortcut};

use crate::apps::DesktopEntry;

/// What a shortcut points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The id the shell's launcher takes (`launch-app`), and the id a pin is stored under.
    pub launch_id: String,
    /// The id the app's tile is drawn from.
    pub icon_id: String,
    /// The theme icon name for an app with no tile of ours: the installed entry's `Icon=`,
    /// falling back to the shortcut file's own — a shortcut to one of the shell's own screens
    /// has no entry to ask, and its file already carries the mark.
    pub theme_icon: String,
}

/// Read shortcuts once per (name, mtime): the listing is rebuilt on every change in the folder,
/// and a folder of shortcuts should not be reopened for each.
#[derive(Default)]
pub struct Shortcuts {
    read: HashMap<String, (Option<SystemTime>, Option<Shortcut>)>,
}

impl Shortcuts {
    pub fn get(&mut self, dir: &Path, item: &Item) -> Option<Shortcut> {
        if let Some((mtime, parsed)) = self.read.get(&item.name) {
            if *mtime == item.mtime {
                return parsed.clone();
            }
        }
        let parsed = shortcut::read(&dir.join(&item.name));
        self.read.insert(item.name.clone(), (item.mtime, parsed.clone()));
        parsed
    }

    pub fn retain(&mut self, items: &[Item]) {
        self.read.retain(|n, _| items.iter().any(|i| &i.name == n));
    }
}

/// The names a shortcut may be known by, in the order they are tried.
pub fn candidates(sc: &Shortcut) -> Vec<String> {
    let mut out = vec![sc.stem.clone()];
    if let Some(bare) = sc.stem.strip_prefix("yantrik-") {
        out.push(bare.to_string());
    }
    // The program the Exec line names, without arguments or field codes: `notes %U` is `notes`.
    if let Some(bin) = sc.exec.split_whitespace().next() {
        let bin = bin.rsplit('/').next().unwrap_or(bin);
        out.push(bin.strip_prefix("yantrik-").unwrap_or(bin).to_string());
    }
    out.push(sc.name.to_lowercase());
    out.dedup();
    out
}

/// What `sc` opens, if this machine has it.
pub fn resolve(sc: &Shortcut, installed: &[DesktopEntry]) -> Option<Target> {
    let id = candidates(sc).into_iter().find(|c| !c.is_empty() && crate::wire::dock::is_launchable(c, installed))?;
    let entry = installed
        .iter()
        .find(|e| e.app_id == id || crate::wire::pins::pin_id(&e.app_id) == id);
    // The entry's name first — it is the app's current one. The shortcut file's own `Icon=`
    // is next: a shortcut to one of the shell's own resolves without an entry, and drawing
    // that as a letter tile is what made our shipped Notes and Terminal look wrong.
    let entry_icon = entry.map(|e| e.icon.clone()).unwrap_or_default();
    Some(Target {
        icon_id: yantrik_ui_kit::app_names::icon_id_for(entry.map(|e| e.app_id.as_str()).unwrap_or(&id)),
        theme_icon: if entry_icon.is_empty() { sc.icon.clone() } else { entry_icon },
        launch_id: id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(stem: &str, name: &str, exec: &str) -> Shortcut {
        Shortcut { stem: stem.into(), name: name.into(), exec: exec.into(), icon: String::new() }
    }

    #[test]
    fn a_shortcut_is_known_by_its_file_its_program_and_its_name() {
        let c = candidates(&sc("yantrik-notes", "Notes", "/opt/yantrik/bin/yantrik-notes %U"));
        assert_eq!(c, ["yantrik-notes", "notes"]);
        assert!(c.contains(&"notes".to_string()));
        assert_eq!(candidates(&sc("blender", "Blender", "blender %f")), ["blender"]);
    }

    #[test]
    fn a_shortcut_to_nothing_this_machine_has_resolves_to_nothing() {
        let installed = crate::surfaces::shipped_catalogue();
        assert_eq!(resolve(&sc("no-such-app-anywhere", "No Such App", "no-such-app-anywhere"), &installed), None);
    }

    #[test]
    fn the_apps_this_os_ships_resolve_by_their_desktop_file_names() {
        let installed = crate::surfaces::shipped_catalogue();
        // Routes of the shell's own always open; an app with a program of its own opens only where
        // that program is installed, which a test machine does not promise.
        let t = resolve(&sc("files", "Files", "files"), &installed).expect("files resolves");
        assert_eq!(t.launch_id, "files");
        let t = resolve(&sc("yantrik-settings", "Settings", "yantrik-settings"), &installed).expect("settings resolves by its bare name");
        assert_eq!(t.launch_id, "settings");
    }

    #[test]
    fn a_shortcut_with_no_entry_to_ask_keeps_its_own_icon() {
        let installed = crate::surfaces::shipped_catalogue();
        // A shortcut to Files: one of the shell's own screens, so it resolves wherever the
        // tests run, and the catalogue has no entry for it — the icon named in the shortcut
        // file itself is the only one there is.
        let mut s = sc("files", "Files", "files");
        s.icon = "yantrik".into();
        let t = resolve(&s, &installed).expect("files resolves on any machine");
        assert_eq!(t.theme_icon, "yantrik", "the shortcut file's Icon= must survive to the tile");
    }

    #[test]
    fn a_shortcut_is_read_once_per_change() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.desktop"), "[Desktop Entry]\nType=Application\nName=A\nExec=a\n").unwrap();
        let item = Item { name: "a.desktop".into(), kind: yantrik_bench::items::Kind::App, size: 1, mtime: Some(SystemTime::UNIX_EPOCH) };
        let mut cache = Shortcuts::default();
        assert_eq!(cache.get(tmp.path(), &item).unwrap().name, "A");
        std::fs::write(tmp.path().join("a.desktop"), "[Desktop Entry]\nType=Application\nName=B\nExec=a\n").unwrap();
        assert_eq!(cache.get(tmp.path(), &item).unwrap().name, "A", "same mtime: not read again");
        let newer = Item { mtime: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1)), ..item };
        assert_eq!(cache.get(tmp.path(), &newer).unwrap().name, "B");
    }
}
