//! Whether a mind may paste what is on the Files clipboard into the folder on screen (#443).
//!
//! The clipboard is the person's: whatever they last copied or cut, from anywhere they could
//! see. A mind pasting it would copy ~/.ssh into ~/Documents, where every rule lets it read, or
//! move a folder named `applications` into ~/.local/share, where the session runs what is in it.
//! So every source must be something a mind may read, the top of what lands must be something
//! it may create, and the whole tree below, put where it lands, must pass
//! `home_paths::may_land`: nothing protected, nothing hidden, no file with a second name.

use std::path::{Path, PathBuf};

use yantrik_ipc_contracts::home_paths;

const RULE: &str = "a mind pastes only what is in the person's home, outside its protected places";

/// Refuse pasting `sources` into the folder `dest_label` (as the Files screen spells it), where
/// `cut` says whether the clipboard was made by cutting (#1083), with `home` as the person's home.
pub fn paste_verdict(sources: &[PathBuf], dest_label: &str, cut: bool, home: &Path) -> Result<(), String> {
    crate::control_files_mind::here_verdict(dest_label, home)?;
    let dest = home_paths::expand(dest_label, home).ok_or_else(|| format!("{dest_label} is not a folder"))?;
    let real_dest = dest.canonicalize().map_err(|e| format!("the folder on screen cannot be read: {e}"))?;
    for src in sources {
        let shown = src.to_string_lossy();
        let answer = home_paths::stat(&shown, home);
        if answer["exists"] != true {
            return Err(format!("{RULE}; {shown} is {}", answer["reason"].as_str().unwrap_or("not there")));
        }
        // Pasting a cut moves the source, so it is checked as a move: a mind that cut the
        // desktop would otherwise move the shell's folder somewhere else. A copy leaves the
        // original where it was, so nothing is moved and only what may be read decides.
        if cut {
            crate::control_files_mind::shift_verdict(src, home)?;
        }
        let Some(name) = src.file_name() else {
            return Err(format!("{shown} has no name to paste under"));
        };
        home_paths::may_create(&dest.join(name).to_string_lossy(), home)?;
        home_paths::may_land(src, &real_dest.join(name), home)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Home(PathBuf);
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn home(name: &str) -> (Home, PathBuf) {
        let dir = std::env::temp_dir().join(format!("yantrik-files-paste-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Documents")).unwrap();
        std::fs::create_dir_all(dir.join(".ssh")).unwrap();
        std::fs::write(dir.join(".ssh/id_ed25519"), "key").unwrap();
        std::fs::create_dir_all(dir.join("project/src")).unwrap();
        std::fs::write(dir.join("project/src/main.rs"), "fn main() {}").unwrap();
        let home = dir.canonicalize().unwrap();
        (Home(home.clone()), home)
    }

    fn refused_because(result: Result<(), String>, why: &str) {
        let err = result.expect_err(why);
        assert!(err.ends_with(why), "{err}");
    }

    #[test]
    fn a_folder_in_the_home_pastes_into_another() {
        let (_d, home) = home("plain");
        let label = home.join("Documents");
        assert!(paste_verdict(&[home.join("project")], label.to_str().unwrap(), true, &home).is_ok());
    }

    #[test]
    fn a_key_the_person_copied_is_not_pasted_by_a_mind() {
        let (_d, home) = home("key");
        let label = home.join("Documents");
        let label = label.to_str().unwrap();
        refused_because(paste_verdict(&[home.join(".ssh/id_ed25519")], label, true, &home), " is protected");
        refused_because(paste_verdict(&[home.join(".ssh")], label, true, &home), " is protected");
        refused_because(paste_verdict(&[PathBuf::from("/etc/hostname")], label, true, &home), " is outside");
    }

    #[test]
    fn a_tree_that_would_land_as_a_protected_or_hidden_place_is_refused() {
        // A folder named `applications`, pasted into ~/.local/share, is the menu's desktop entries.
        let (_d, home) = home("lands");
        std::fs::create_dir_all(home.join(".local/share")).unwrap();
        std::fs::create_dir_all(home.join("applications")).unwrap();
        std::fs::write(home.join("applications/term.desktop"), "[Desktop Entry]").unwrap();
        let share = home.join(".local/share");
        refused_because(paste_verdict(&[home.join("applications")], share.to_str().unwrap(), true, &home), " is protected");
        // And a hidden folder deep inside what is copied: programs read their startup there.
        std::fs::create_dir_all(home.join("backup/.config/autostart")).unwrap();
        std::fs::write(home.join("backup/.config/autostart/run.desktop"), "x").unwrap();
        let label = home.join("Documents");
        refused_because(paste_verdict(&[home.join("backup")], label.to_str().unwrap(), true, &home), " is hidden_place");
    }

    #[test]
    fn a_file_with_a_second_name_inside_the_tree_is_refused() {
        let (_d, home) = home("hard");
        std::fs::hard_link(home.join(".ssh/id_ed25519"), home.join("project/src/copy")).unwrap();
        let label = home.join("Documents");
        refused_because(paste_verdict(&[home.join("project")], label.to_str().unwrap(), true, &home), " is hard_link");
    }

    /// A copy leaves the shell's folder where it is; a cut is the move. Which one the clipboard
    /// holds is therefore what decides, and a mind that cut the desktop would otherwise move it
    /// out from under the Bench (#1083).
    #[test]
    fn a_cut_of_a_folder_the_shell_keeps_is_refused_where_a_copy_of_it_is_not() {
        let (_d, home) = home("cut");
        std::fs::create_dir_all(home.join("Schreibtisch/notes")).unwrap();
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::fs::write(home.join(".config/user-dirs.dirs"), "XDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n").unwrap();
        let dest = home.join("Documents");
        let dest = dest.to_str().unwrap();
        let desk = vec![home.join("Schreibtisch")];
        assert!(paste_verdict(&desk, dest, false, &home).is_ok(), "a copy moves nothing");
        refused_because(paste_verdict(&desk, dest, true, &home), "a mind does not move it");
    }

    #[test]
    fn a_link_in_the_tree_is_copied_as_a_link_and_not_followed() {
        let (_d, home) = home("link");
        symlink(home.join(".ssh"), home.join("project/keys")).unwrap();
        let label = home.join("Documents");
        assert!(paste_verdict(&[home.join("project")], label.to_str().unwrap(), true, &home).is_ok());
    }
}
