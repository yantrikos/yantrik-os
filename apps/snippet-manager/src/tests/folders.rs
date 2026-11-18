//! The library's folder and the drafts' are the person's alone: 0700, as every file in them is
//! 0600 (#948, item 5). They were made 0775 by the atomic writer's `create_dir_all`.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use serde_json::json;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App as _, Paths};

use super::{open, start_at};
use crate::SnippetManager;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).permissions().mode() & 0o777
}

#[test]
fn the_library_and_drafts_folders_are_made_private() {
    let f = open();
    let drafts = f.paths.state_dir().join(yantrik_app::drafts::DIR);
    for folder in [f.paths.data_dir(), f.paths.state_dir(), drafts.as_path()] {
        assert_eq!(mode(folder), 0o700, "{}", folder.display());
    }
    f.act("new", json!({ "title": "Key", "language": "Shell", "code": "echo" })).unwrap();
    let store = crate::store::file::state_path(f.paths.data_dir());
    assert_eq!(mode(&store), 0o600, "the store itself");
}

#[test]
fn folders_made_open_by_an_older_build_are_tightened() {
    let paths = Paths::temporary(SnippetManager::ID);
    let drafts = paths.state_dir().join(yantrik_app::drafts::DIR);
    for folder in [paths.data_dir(), drafts.as_path()] {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o775)).unwrap();
    }
    let _f = start_at(paths.clone(), FakeMind::new(), Vec::new());
    for folder in [paths.data_dir(), drafts.as_path()] {
        assert_eq!(mode(folder), 0o700, "{}", folder.display());
    }
}
