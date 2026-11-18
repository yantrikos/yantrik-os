//! The library's folder and the drafts' are the person's alone: 0700, and every file written in
//! them is 0600. The library is a deliverable a person copies out; the copy is theirs to share, the
//! original is not world readable.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use serde_json::json;
use yantrik_app::{App as _, Paths};

use super::{meadow_run, open, seeded, start_at};
use crate::Arcade;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).permissions().mode() & 0o777
}

#[test]
fn the_library_and_drafts_folders_are_made_private_and_what_is_written_in_them_is_0600() {
    let f = open();
    let drafts = f.paths.state_dir().join(yantrik_app::drafts::DIR);
    for folder in [f.paths.data_dir(), f.paths.state_dir(), drafts.as_path()] {
        assert_eq!(mode(folder), 0o700, "{}", folder.display());
    }
    seeded(&f);
    f.act("build", json!({ "game": "meadow-run" })).unwrap();
    let library = f.library();
    for file in [library.game_spec_path("meadow-run"), library.game_html("meadow-run"), library.root().join("characters/pip.json")] {
        assert_eq!(mode(&file), 0o600, "{}", file.display());
    }
    f.act("update_game", json!({ "spec": meadow_run("Meadow Run") })).unwrap();
    assert_eq!(mode(&library.game_spec_path("meadow-run")), 0o600);
}

#[test]
fn folders_made_open_by_an_older_build_are_tightened() {
    let paths = Paths::temporary(Arcade::ID);
    let drafts = paths.state_dir().join(yantrik_app::drafts::DIR);
    for folder in [paths.data_dir(), drafts.as_path()] {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o775)).unwrap();
    }
    let _f = start_at(paths.clone());
    for folder in [paths.data_dir(), drafts.as_path()] {
        assert_eq!(mode(folder), 0o700, "{}", folder.display());
    }
}

/// A library written by the build before this one (plain files, default modes) is read as it is:
/// nothing moves, nothing is migrated, and the next write of a file makes it 0600.
#[test]
fn a_library_from_the_old_build_is_read_in_place() {
    let paths = Paths::temporary(Arcade::ID);
    let old = crate::library::Library::at(paths.data_dir());
    old.save_character(super::PIP).unwrap();
    old.save_game(&meadow_run("Meadow Run")).unwrap();
    let f = start_at(paths);
    assert_eq!(f.state()["games"][0]["title"], json!("Meadow Run"));
    assert_eq!(f.state()["characters"][0]["name"], json!("Pip"));
}
