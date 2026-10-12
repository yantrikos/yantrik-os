//! What `files_delete`'s card names, how the delete is held to it, and how long naming may take.

use super::*;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("files-target-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_file_is_named_by_name_path_and_size_a_folder_by_its_entries_and_nothing_by_nothing() {
    let dir = scratch("rows");
    std::fs::create_dir_all(dir.join("folder")).unwrap();
    std::fs::write(dir.join("report.pdf"), vec![0u8; 2048]).unwrap();
    std::fs::write(dir.join("folder/a.txt"), b"a").unwrap();

    let file = target_for(&dir.join("report.pdf"), "~/report.pdf").unwrap();
    assert_eq!(file.rows[0], ("Item".to_string(), "report.pdf \u{00b7} ~/report.pdf".to_string()), "its name first");
    assert_eq!(file.rows[1], ("Size".to_string(), "2.0 KB".to_string()));
    assert_eq!(Some(file.identity), identity_of(&dir.join("report.pdf")));
    assert_eq!(target_for(&dir.join("folder"), "~/folder").unwrap().rows[1].1, "folder, 1 item inside");
    assert!(target_for(&dir.join("missing"), "~/missing").is_none());
    assert_eq!(folder(COUNTED + 1), format!("folder, more than {COUNTED} items inside"), "counted up to a bound");
    assert_eq!(folder(COUNTED), format!("folder, {COUNTED} items inside"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// L4: the namer and the handler resolve a name with one function, exactly as given: a name
/// with a trailing space is that name, never the one without it.
#[test]
fn a_name_resolves_one_way_for_the_card_and_the_delete() {
    let dir = scratch("entry");
    let label = dir.to_string_lossy().into_owned();
    assert_eq!(entry(&label, "thesis ").unwrap().0, dir.join("thesis "), "not trimmed");
    for name in ["", ".", "..", "a/b", "../x"] {
        assert!(entry(&label, name).is_none(), "{name:?}");
    }
    assert!(entry("Trash", "thesis").is_none(), "Trash deletes nothing");

    std::fs::write(dir.join("thesis"), b"t").unwrap();
    std::fs::write(dir.join("thesis "), b"u").unwrap();
    let plain = identity_of(&entry(&label, "thesis").unwrap().0);
    let spaced = identity_of(&entry(&label, "thesis ").unwrap().0);
    assert!(plain.is_some() && spaced.is_some() && plain != spaced, "two entries, two identities");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The identity is the thing, not its spelling: reached through a link to its folder, it is the
/// same; a link in its place is not.
#[test]
fn an_entry_is_identified_by_its_real_folder_and_inode() {
    let dir = scratch("identity");
    std::fs::create_dir_all(dir.join("tmp")).unwrap();
    std::fs::write(dir.join("tmp/thesis"), b"t").unwrap();
    std::os::unix::fs::symlink(dir.join("tmp"), dir.join("via")).unwrap();
    let real = identity_of(&dir.join("tmp/thesis")).unwrap();
    assert_eq!(identity_of(&dir.join("via/thesis")).as_deref(), Some(real.as_str()));
    assert!(real.contains("inode"), "{real}");
    std::fs::rename(dir.join("tmp/thesis"), dir.join("tmp/old")).unwrap();
    std::os::unix::fs::symlink(dir.join("tmp/old"), dir.join("tmp/thesis")).unwrap();
    assert_ne!(identity_of(&dir.join("tmp/thesis")).as_deref(), Some(real.as_str()), "a link put in its place");
    let _ = std::fs::remove_dir_all(&dir);
}

/// H1 for Files: the card named ~/tmp/thesis. Before the grant is spent, `files_go` (no card)
/// moves Files to the home folder, which holds a `thesis` of its own. The delete resolves the
/// same name against the folder on screen NOW, the shared check refuses, and both are still there.
#[test]
fn a_folder_change_after_the_allow_deletes_nothing() {
    let home = scratch("h1");
    std::fs::create_dir_all(home.join("tmp/thesis")).unwrap();
    std::fs::create_dir_all(home.join("thesis")).unwrap();
    let on_screen_then = home.join("tmp").to_string_lossy().into_owned();
    let on_screen_now = home.to_string_lossy().into_owned();

    let (path, shown) = entry(&on_screen_then, "thesis").unwrap();
    let card = target_for(&path, &shown).expect("named");
    let _grant = yantrik_app_runtime::control::GrantedTargetScope::enter(Some(card.identity.clone()));

    let now = entry(&on_screen_now, "thesis").and_then(|(path, _)| identity_of(&path));
    let refused = yantrik_app_runtime::control::held_to_grant(now.as_deref()).unwrap_err();
    assert!(refused.starts_with("the target changed after you allowed it"), "{refused}");
    assert!(home.join("thesis").exists() && home.join("tmp/thesis").exists(), "nothing was deleted");

    let same = entry(&on_screen_then, "thesis").and_then(|(path, _)| identity_of(&path));
    assert!(yantrik_app_runtime::control::held_to_grant(same.as_deref()).is_ok(), "the one it named still goes");
    let _ = std::fs::remove_dir_all(&home);
}

/// M2: a disk that does not answer in time names nothing, and the card does not wait for it.
#[test]
fn a_slow_disk_names_nothing_within_the_patience() {
    let started = std::time::Instant::now();
    let answer = off_this_thread(
        || {
            std::thread::sleep(Duration::from_secs(3));
            Some(1)
        },
        Duration::from_millis(50),
    );
    assert_eq!(answer, None);
    assert!(started.elapsed() < Duration::from_secs(1), "the card did not wait for the disk");
    assert_eq!(off_this_thread(|| Some(2), PATIENCE), Some(2));
}
