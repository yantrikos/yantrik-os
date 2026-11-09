use super::*;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::PathBuf;

fn home() -> (tempfile::TempDir, PathBuf) {
    workshops::roots_under_home_for_tests();
    let h = tempfile::tempdir().unwrap();
    let home = h.path().canonicalize().unwrap();
    (h, home)
}

fn make_root(home: &Path) {
    std::fs::create_dir_all(workshops::root(home)).unwrap();
    std::fs::set_permissions(workshops::root(home), std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn old_root(home: &Path) -> PathBuf {
    home.join(".local/share/yantrik/workshops")
}

fn forged_record(old: &Path, agent: &str, folder: &Path) {
    let rec = format!(r#"{{"grants":[{{"agent":"{agent}","folder":"{}","granted_unix":42}}]}}"#, folder.display());
    std::fs::write(old.join(".grants.json"), rec).unwrap();
}

#[test]
fn old_workshops_are_copied_once_with_no_grant_and_modes_cut_back() {
    let (_h, home) = home();
    make_root(&home);
    let old = old_root(&home);
    std::fs::create_dir_all(old.join("hermes/shop/sub")).unwrap();
    std::fs::write(old.join("hermes/shop/sub/a.txt"), "alpha").unwrap();
    std::fs::set_permissions(old.join("hermes/shop/sub/a.txt"), std::fs::Permissions::from_mode(0o4755)).unwrap();
    std::fs::set_permissions(old.join("hermes/shop/sub"), std::fs::Permissions::from_mode(0o777)).unwrap();
    std::fs::create_dir_all(old.join("pi/draft")).unwrap();
    // A link where a Mind's folder should be, and one where a workshop should be: left alone.
    let outside = home.join("Documents");
    std::fs::create_dir(&outside).unwrap();
    symlink(&outside, old.join("hermes/linked")).unwrap();
    symlink(&outside, old.join("evil")).unwrap();
    forged_record(&old, "hermes", &old.join("hermes/shop"));

    let moved = migrate(&home).unwrap().expect("there was an old root");
    assert_eq!(moved.workshops, 2, "{moved:?}");
    let new = workshops::root(&home);
    assert_eq!(std::fs::read_to_string(new.join("hermes/shop/sub/a.txt")).unwrap(), "alpha");
    assert_eq!(std::fs::metadata(new.join("hermes/shop/sub/a.txt")).unwrap().permissions().mode() & 0o7777, 0o700, "no setuid, no group or other bits");
    assert_eq!(std::fs::metadata(new.join("hermes/shop/sub")).unwrap().permissions().mode() & 0o777, 0o700);
    assert!(new.join("pi/draft").is_dir() && !new.join("hermes/linked").exists() && !new.join("evil").exists());
    // Gone from the old place, except the links that were not followed. The old record is gone too.
    assert!(!old.join(".grants.json").exists() && !old.join("hermes/shop").exists() && !old.join("pi").exists());
    assert!(old.join("evil").symlink_metadata().unwrap().is_symlink());
    // No grant came with it.
    let record = crate::workshop::record_in(&home);
    assert!(crate::workshop::granted_for(&record, &home, "hermes", new.join("hermes/shop").to_str().unwrap()).is_err());
    assert!(crate::workshop::load(&record).unwrap().is_empty());
}

#[test]
fn the_move_is_one_time_and_the_old_path_is_never_read_again() {
    let (_h, home) = home();
    make_root(&home);
    std::fs::create_dir_all(old_root(&home)).unwrap();
    assert_eq!(migrate(&home).unwrap().unwrap().workshops, 0, "the first run is the move");
    assert!(workshops::root(&home).join(".moved").is_file());
    // A Mind plants a workshop, a stray file (which used to keep the move alive) and a record.
    let old = old_root(&home);
    std::fs::create_dir_all(old.join("B/notes")).unwrap();
    std::fs::write(old.join("B/notes/Makefile"), "all:\n\tcurl x").unwrap();
    std::fs::write(old.join("zz"), "x").unwrap();
    forged_record(&old, "B", &old.join("B/notes"));
    assert_eq!(migrate(&home).unwrap(), None);
    assert_eq!(migrate(&home).unwrap(), None);
    assert!(!workshops::root(&home).join("B").exists(), "nothing came into the root");
    assert!(old.join("B/notes/Makefile").is_file() && old.join("zz").is_file());
}

#[test]
fn a_forged_record_changes_no_grant() {
    let (_h, home) = home();
    make_root(&home);
    let record = crate::workshop::record_in(&home);
    let real = crate::workshop::folder_for(&home, "B", "real", true).unwrap();
    crate::workshop::grant(&record, "B", &real, 7).unwrap();
    let before = std::fs::read_to_string(&record).unwrap();
    let old = old_root(&home);
    std::fs::create_dir_all(old.join("B/notes")).unwrap();
    forged_record(&old, "B", &old.join("B/notes"));
    migrate(&home).unwrap().unwrap();
    assert_eq!(std::fs::read_to_string(&record).unwrap(), before, "B's workshop is still the one it was granted");
    let notes = workshops::root(&home).join("B/notes");
    assert!(notes.is_dir() && crate::workshop::granted_for(&record, &home, "B", notes.to_str().unwrap()).is_err());
}

/// A rename would carry the inode into the root, and with it any process that stands in it.
#[test]
fn a_workshop_is_copied_never_renamed() {
    let (_h, home) = home();
    make_root(&home);
    let old = old_root(&home);
    std::fs::create_dir_all(old.join("A/x")).unwrap();
    let held = std::fs::File::open(old.join("A/x")).unwrap();
    migrate(&home).unwrap().unwrap();
    let copy = std::fs::metadata(workshops::root(&home).join("A/x")).unwrap();
    assert_ne!((copy.dev(), copy.ino()), (held.metadata().unwrap().dev(), held.metadata().unwrap().ino()));
}

#[test]
fn one_odd_entry_keeps_its_workshop_where_it_was() {
    let (_h, home) = home();
    make_root(&home);
    let old = old_root(&home);
    let odd = |name: &str| std::fs::create_dir_all(old.join(format!("m/{name}/d"))).unwrap();
    for name in ["link", "fifo", "second", "bell", "clean"] {
        odd(name);
        std::fs::write(old.join(format!("m/{name}/d/f")), "x").unwrap();
    }
    symlink("/etc", old.join("m/link/d/l")).unwrap();
    let fifo = CString::new(old.join("m/fifo/d/p").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    std::fs::write(home.join("keep"), "mine").unwrap();
    std::fs::hard_link(home.join("keep"), old.join("m/second/d/h")).unwrap();
    std::fs::write(old.join("m/bell/d/\u{7}x"), "x").unwrap();
    // A Mind's folder named with a control character is not made in the root at all.
    std::fs::create_dir_all(old.join("evil\u{202e}name/shop")).unwrap();

    let moved = migrate(&home).unwrap().unwrap();
    assert_eq!(moved.workshops, 1, "{moved:?}");
    let new = workshops::root(&home);
    assert!(new.join("m/clean/d/f").is_file());
    for name in ["link", "fifo", "second", "bell"] {
        assert!(!new.join("m").join(name).exists(), "{name} was not carried");
        assert!(old.join("m").join(name).is_dir(), "{name} stays");
    }
    assert!(std::fs::read_dir(&new).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains('\u{202e}')));
    assert_eq!(std::fs::read_to_string(home.join("keep")).unwrap(), "mine");
}

#[test]
fn nothing_moves_without_a_root_that_is_the_persons_and_nothing_makes_one() {
    let (_h, home) = home();
    let old = old_root(&home);
    std::fs::create_dir_all(old.join("hermes/shop")).unwrap();
    std::fs::write(old.join("hermes/shop/keep"), "x").unwrap();
    assert!(migrate(&home).unwrap_err().contains("never does"));
    assert!(old.join("hermes/shop/keep").is_file(), "the old content stays");
    assert!(!workshops::root(&home).exists() && !workshops::base(&home).exists(), "the shell made no root");
    // No old root at all, and no root: nothing to do.
    let (_h2, home2) = self::home();
    assert_eq!(migrate(&home2).unwrap(), None);
}
