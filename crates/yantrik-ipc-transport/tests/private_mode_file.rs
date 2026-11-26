//! Where Private mode is published (#1415): a directory of its own, the old place read as a
//! fallback, and the shell moving the old file across at start. Its own test binary, so it can
//! own `HOME`.

use yantrik_ipc_transport::privacy;

#[test]
fn new_place_first_old_place_as_fallback_and_migration() {
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

    // Unreadable is an error naming the path, and counts as private.
    std::fs::remove_file(privacy::privacy_path().unwrap()).unwrap();
    std::fs::remove_file(config.join("privacy.json")).unwrap();
    std::fs::create_dir(privacy::privacy_path().unwrap()).unwrap();
    let err = privacy::read_state().unwrap_err();
    assert!(err.to_string().contains("private-mode/privacy.json"), "{err}");
    assert!(privacy::is_private());
}
