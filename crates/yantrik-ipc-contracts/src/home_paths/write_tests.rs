use std::os::unix::fs::symlink;
use std::path::PathBuf;

use super::*;

struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home(name: &str) -> (Home, PathBuf) {
    let dir = std::env::temp_dir().join(format!("yantrik-home-writes-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for folder in ["Documents", "notes", ".local/bin", "bin", ".config/nvim", ".cargo"] {
        std::fs::create_dir_all(dir.join(folder)).unwrap();
    }
    std::fs::write(dir.join(".gitconfig"), "[user]\n").unwrap();
    let home = dir.canonicalize().unwrap();
    (Home(home.clone()), home)
}

#[test]
fn an_agent_writes_nothing_where_programs_read_their_settings() {
    let (_d, home) = home("hidden");
    for asked in [
        "~/.gitconfig",
        "~/.tmux.conf",
        "~/.vimrc",
        "~/.local/bin/x",
        "~/bin/x",
        "~/.config/nvim/init.lua",
        "~/.cargo/config.toml",
        "~/notes/.git/config",
    ] {
        let err = may_write_file(asked, &home).unwrap_err();
        assert!(err.starts_with(HIDDEN_RULE) && err.ends_with(" is hidden_place"), "{asked}: {err}");
    }
    assert!(may_write_file("~/Documents/x.txt", &home).is_ok());
    assert!(may_write_file("~/notes/x.txt", &home).is_ok());
    assert!(may_write_file("~/binders.txt", &home).is_ok(), "only `bin` itself, not every name starting so");
}

#[test]
fn a_hidden_folder_is_not_made_nor_reached_through_a_link() {
    let (_d, home) = home("made");
    assert!(may_create("~/.new-settings", &home).unwrap_err().ends_with(" is hidden_place"));
    assert!(may_create("~/bin", &home).unwrap_err().ends_with(" is hidden_place"));
    assert!(may_create("~/notes/.git", &home).unwrap_err().ends_with(" is hidden_place"));
    assert!(may_create("~/notes/drafts", &home).is_ok());
    symlink(home.join(".config"), home.join("cfg")).unwrap();
    assert!(may_write_file("~/cfg/nvim/init.lua", &home).unwrap_err().ends_with(" is hidden_place"));
    assert!(may_create("~/cfg/new", &home).unwrap_err().ends_with(" is hidden_place"));
}

#[test]
fn reading_a_dotfile_that_is_not_protected_is_still_allowed() {
    let (_d, home) = home("read");
    assert!(may_read_file("~/.gitconfig", &home).is_ok());
}

#[test]
fn a_tree_that_would_land_as_a_protected_or_hidden_place_is_refused() {
    // `cfg` holding autostart/evil.desktop, renamed or moved to be ~/.config.
    let (_d, home) = home("tree");
    std::fs::create_dir_all(home.join("cfg/autostart")).unwrap();
    std::fs::write(home.join("cfg/autostart/evil.desktop"), "[Desktop Entry]").unwrap();
    let err = may_land(&home.join("cfg"), &home.join(".config"), &home).unwrap_err();
    assert!(err.ends_with(" is protected"), "{err}");
    // A project with its .git, pasted anywhere in the home, brings a config git runs.
    std::fs::create_dir_all(home.join("project/.git")).unwrap();
    std::fs::write(home.join("project/.git/config"), "[core]\n").unwrap();
    let err = may_land(&home.join("project"), &home.join("Documents/project"), &home).unwrap_err();
    assert!(err.ends_with(" is hidden_place"), "{err}");
    // A plain tree lands.
    std::fs::create_dir_all(home.join("photos/2026")).unwrap();
    std::fs::write(home.join("photos/2026/a.png"), "x").unwrap();
    assert!(may_land(&home.join("photos"), &home.join("Documents/photos"), &home).is_ok());
    // A second name inside is refused.
    std::fs::hard_link(home.join(".gitconfig"), home.join("photos/2026/b.png")).unwrap();
    let err = may_land(&home.join("photos"), &home.join("Documents/photos"), &home).unwrap_err();
    assert!(err.ends_with(" is hard_link"), "{err}");
}
