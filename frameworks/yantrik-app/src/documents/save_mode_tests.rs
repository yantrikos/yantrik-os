//! A Save As over a file the person said to replace, through the `Disk` seam: the new file wears
//! the old one's mode, and a failed directory sync is still reported (#989).

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use super::disk::{FaultyDisk, RealDisk, Step};
use super::files::save::{save, SaveJob};
use super::sentences_tests::Defaults;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yantrik-save-mode-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A Save As of "new" onto `target`, with the caller having said to replace what is there.
fn save_as(target: &Path) -> SaveJob {
    SaveJob {
        text: "new".into(),
        target: target.to_path_buf(),
        own: None,
        on_disk: None,
        unsure: None,
        inode: None,
        overwrite: true,
        agent: false,
        others: Vec::new(),
        elsewhere: PathBuf::new(),
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn a_save_as_over_a_file_keeps_its_mode() {
    let dir = scratch("keeps-mode");
    for (name, wanted) in [("shared.md", 0o644), ("script.sh", 0o755), ("private.md", 0o600)] {
        let path = dir.join(name);
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(wanted)).unwrap();
        save(&Defaults, &save_as(&path), &RealDisk).unwrap_or_else(|e| panic!("{name}: {}", e.error.sentence));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new", "{name}");
        assert_eq!(mode(&path), wanted, "{name} came out {:o}, not {wanted:o}", mode(&path));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_save_as_goes_through_the_disk_it_is_given() {
    let dir = scratch("through-disk");
    let path = dir.join("note.md");
    std::fs::write(&path, "old").unwrap();
    // A disk that fails the replace sees the save: the bytes are not written behind its back.
    let disk = FaultyDisk::new();
    disk.fail(Step::Rename, "note.md", Some(1));
    assert!(save(&Defaults, &save_as(&path), &disk).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
    assert_eq!(disk.hits.lock().unwrap().len(), 1, "the replace went through the disk");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_save_as_whose_directory_sync_failed_is_not_called_saved() {
    let dir = scratch("unsure");
    let path = dir.join("note.md");
    std::fs::write(&path, "old").unwrap();
    let disk = FaultyDisk::new();
    disk.fail(Step::SyncDir, "note.md", Some(1));
    let failed = save(&Defaults, &save_as(&path), &disk).err().expect("an unsure replace is not a saved one");
    // The new bytes are in place all the same; the report says their durability is not known.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(failed.error.durability, super::Durability::Unknown);
    assert!(failed.error.landed.is_some(), "{}", failed.error.sentence);
    let _ = std::fs::remove_dir_all(&dir);
}
