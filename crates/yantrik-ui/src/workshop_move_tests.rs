use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};
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

#[test]
fn old_workshops_move_once_into_the_new_root_and_no_link_is_followed() {
    let (_h, home) = home();
    make_root(&home);
    let old = old_root(&home);
    std::fs::create_dir_all(old.join("hermes/shop/sub")).unwrap();
    std::fs::write(old.join("hermes/shop/sub/a.txt"), "alpha").unwrap();
    // A link inside a workshop is carried as a link, and the thing it names is not read.
    let outside = home.join("Documents");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("private"), "mine").unwrap();
    symlink(&outside, old.join("hermes/shop/to-docs")).unwrap();
    // A link where a workshop should be, and a link where a Mind's folder should be: left alone.
    symlink(&outside, old.join("hermes/linked")).unwrap();
    symlink(&outside, old.join("evil")).unwrap();
    std::fs::create_dir_all(old.join("pi/draft")).unwrap();
    // The old grant record for hermes/shop.
    let shop = old.join("hermes/shop");
    let rec = format!(r#"{{"grants":[{{"agent":"hermes","folder":"{}","granted_unix":42}}]}}"#, shop.display());
    std::fs::write(old.join(".grants.json"), rec).unwrap();

    let moved = migrate(&home).unwrap().expect("there was an old root");
    assert_eq!(moved.workshops, 2, "{moved:?}");
    assert_eq!(moved.grants, 1);
    let new = workshops::root(&home);
    assert_eq!(std::fs::read_to_string(new.join("hermes/shop/sub/a.txt")).unwrap(), "alpha");
    assert!(new.join("hermes/shop/to-docs").symlink_metadata().unwrap().is_symlink());
    assert_eq!(std::fs::read_to_string(outside.join("private")).unwrap(), "mine");
    assert!(new.join("pi/draft").is_dir() && !new.join("hermes/linked").exists() && !new.join("evil").exists());
    assert_eq!(std::fs::metadata(new.join("hermes")).unwrap().permissions().mode() & 0o777, 0o700);
    // The grant is the new folder, and it is a grant: the sealed rules accept it.
    let record = crate::workshop::record_in(&home);
    let granted = crate::workshop::granted_for(&record, &home, "hermes", new.join("hermes/shop").to_str().unwrap()).unwrap();
    assert_eq!(granted, new.join("hermes/shop"));
    // Gone from the old place, except the links that were not followed.
    assert!(!old.join(".grants.json").exists() && !old.join("hermes/shop").exists() && !old.join("pi").exists());
    assert!(old.join("evil").symlink_metadata().unwrap().is_symlink());
    // Again: nothing more to do, nothing moved twice.
    let again = migrate(&home).unwrap().unwrap();
    assert_eq!(again.workshops, 0);
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
    // No old root at all: nothing to do.
    let (_h2, home2) = self::home();
    assert_eq!(migrate(&home2).unwrap(), None);
}

/// Across two filesystems `renameat` is refused; the copy is by descriptor and keeps modes.
#[test]
fn a_copy_by_descriptor_keeps_modes_and_links_and_never_follows() {
    let (_h, home) = home();
    let src = home.join("src");
    std::fs::create_dir_all(src.join("shop/d")).unwrap();
    std::fs::write(src.join("shop/d/x"), "xx").unwrap();
    std::fs::set_permissions(src.join("shop/d/x"), std::fs::Permissions::from_mode(0o4755)).unwrap();
    std::fs::set_permissions(src.join("shop/d"), std::fs::Permissions::from_mode(0o750)).unwrap();
    symlink("/etc", src.join("shop/etc")).unwrap();
    let dst = home.join("dst");
    std::fs::create_dir(&dst).unwrap();
    let (s, d) = (open_dir_path(&src), open_dir_path(&dst));
    let name = c("shop");
    copy_tree(s.as_raw_fd(), &name, d.as_raw_fd(), &name, 0).unwrap();
    assert_eq!(std::fs::read_to_string(dst.join("shop/d/x")).unwrap(), "xx");
    assert_eq!(std::fs::metadata(dst.join("shop/d/x")).unwrap().permissions().mode() & 0o7777, 0o755, "no setuid");
    assert_eq!(std::fs::metadata(dst.join("shop/d")).unwrap().permissions().mode() & 0o777, 0o750);
    assert_eq!(std::fs::read_link(dst.join("shop/etc")).unwrap(), PathBuf::from("/etc"));
}

fn open_dir_path(p: &Path) -> OwnedFd {
    let c = CString::new(p.as_os_str().as_bytes()).unwrap();
    own(unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY) }).unwrap()
}
