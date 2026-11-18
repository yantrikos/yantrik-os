use std::os::unix::fs::{symlink, PermissionsExt};

use super::*;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("studio-safe-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_new_file_is_never_written_through_a_link_or_over_a_file() {
    let dir = folder("new");
    let target = dir.join("target.txt");
    std::fs::write(&target, "THE PERSON'S").unwrap();
    let planted = dir.join("planted.png");
    symlink(&target, &planted).unwrap();
    assert!(write_new(&planted, b"x", 0o644).is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "THE PERSON'S", "a link was written through");

    let there = dir.join("there.png");
    std::fs::write(&there, "mine").unwrap();
    let e = write_new(&there, b"x", 0o644).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&there).unwrap(), "mine");

    write_new(&dir.join("fresh.png"), b"ok", 0o644).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("fresh.png")).unwrap(), "ok");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_replacement_is_private_whole_and_leaves_nothing_beside_it() {
    let dir = folder("replace");
    let path = dir.join("studio.json");
    std::fs::write(&path, "old").unwrap();
    replace(&path, b"new", 0o600).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!dir.join("studio.json.new").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_link_at_the_temporary_name_is_removed_and_not_followed() {
    let dir = folder("link");
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, "THE PERSON'S").unwrap();
    symlink(&victim, dir.join("studio.json.new")).unwrap();
    let path = dir.join("studio.json");
    replace(&path, b"new", 0o600).unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "THE PERSON'S", "the link was written through");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    // And a leftover from a crash is replaced, not an obstacle.
    std::fs::write(dir.join("studio.json.new"), "half").unwrap();
    replace(&path, b"newer", 0o600).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "newer");
    let _ = std::fs::remove_dir_all(&dir);
}
