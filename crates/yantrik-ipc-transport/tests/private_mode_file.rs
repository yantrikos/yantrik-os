//! Where Private mode is published (#1415): a directory of its own, the old place read as a
//! fallback, and the shell moving the old file across at start. Its own test binary, so it can
//! own `HOME`.

use yantrik_ipc_transport::privacy;

/// Each test owns `HOME` while it runs.
static HOME: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn new_place_first_old_place_as_fallback_and_migration() {
    let _home = HOME.lock().unwrap_or_else(|e| e.into_inner());
    let home = std::env::temp_dir().join(format!("yantrik-privacy-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let config = home.join(".config/yantrik");
    std::fs::create_dir_all(&config).unwrap();
    std::env::set_var("HOME", &home);

    assert!(!privacy::is_private(), "nothing anywhere is off");

    // An old file alone still counts.
    std::fs::write(config.join("privacy.json"), r#"{"private": true}"#).unwrap();
    assert!(privacy::is_private());

    // Moved across: the new file says the same, the old one is gone, the directory exists.
    assert!(privacy::migrate_legacy(7).unwrap());
    assert!(!config.join("privacy.json").exists());
    assert_eq!(privacy::privacy_path().unwrap(), config.join("private-mode/privacy.json"));
    assert!(privacy::is_private());
    assert!(!privacy::migrate_legacy(8).unwrap(), "nothing left to move");

    // The new file wins over a stale old one, and flips by rename.
    std::fs::write(config.join("privacy.json"), r#"{"private": true}"#).unwrap();
    privacy::publish(false, 9).unwrap();
    assert!(!privacy::is_private());
    privacy::publish(true, 10).unwrap();
    assert!(privacy::is_private());

    // Unreadable is an error naming the path, and counts as private. The directory without the
    // file reads as private too, whatever the old place says.
    std::fs::remove_file(privacy::privacy_path().unwrap()).unwrap();
    assert!(privacy::is_private(), "a directory without the file is private");
    std::fs::write(config.join("privacy.json"), r#"{"private": false}"#).unwrap();
    assert!(privacy::is_private(), "and the old file never speaks for it");
    std::fs::remove_file(config.join("privacy.json")).unwrap();
    std::fs::create_dir(privacy::privacy_path().unwrap()).unwrap();
    let err = privacy::read_state().unwrap_err();
    assert!(err.to_string().contains("private-mode/privacy.json"), "{err}");
    assert!(privacy::is_private());
}

/// `rm -rf private-mode && ln -s /nonexistent private-mode`, and a false in the old place: the
/// readers must say private, and the shell's `publish` must put the switch back (#1415 review).
#[cfg(unix)]
#[test]
fn a_dangling_private_mode_link_is_private_and_is_repaired() {
    let _home = HOME.lock().unwrap_or_else(|e| e.into_inner());
    let home = std::env::temp_dir().join(format!("yantrik-privacy-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let config = home.join(".config/yantrik");
    std::fs::create_dir_all(&config).unwrap();
    std::env::set_var("HOME", &home);
    let dir = config.join("private-mode");

    std::os::unix::fs::symlink("/nonexistent-yantrik", &dir).unwrap();
    std::fs::write(config.join("privacy.json"), r#"{"private": false}"#).unwrap();
    assert!(privacy::is_private(), "a dangling link is not 'no file': private");
    assert!(privacy::read_state().is_err());

    privacy::publish(true, 1).unwrap();
    assert!(privacy::is_private());
    assert!(std::fs::symlink_metadata(&dir).unwrap().file_type().is_dir(), "a plain directory again");
    let aside: Vec<_> = std::fs::read_dir(&config)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("private-mode.bad-"))
        .collect();
    assert_eq!(aside.len(), 1, "the link was moved aside, not deleted");
    privacy::publish(false, 2).unwrap();
    assert!(!privacy::is_private());

    // A link to a directory someone else holds: never written through, and moved aside at start,
    // where what the switch said is lost and so it is published as on.
    let elsewhere = home.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
    assert!(privacy::is_private());
    assert!(!privacy::migrate_legacy(3).unwrap());
    assert!(privacy::is_private(), "migration after tampering publishes on");
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none(), "nothing written through the link");
}
