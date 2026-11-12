//! #1201: a backup no marker names, and a live file edited since, are both kept. Real apply and
//! revert in a temp home; fake keys only.

use super::*;
use std::os::unix::fs::PermissionsExt;

const FAKE_KEY: &str = "nvapi-FAKE-0000";

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("handoff-replaced-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".config/yantrik")).unwrap();
    dir
}

fn entry() -> ProviderStoreEntry {
    ProviderStoreEntry {
        id: "p".into(),
        name: "NIM".into(),
        provider_type: "nvidia-nim".into(),
        base_url: "https://integrate.api.nvidia.com/v1".into(),
        api_key: Some(FAKE_KEY.into()),
        api_key_ref: None,
        unmigrated_key: None,
        auth_type: "bearer".into(),
        is_primary: true,
        is_fallback: false,
        model: "m/x".into(),
    }
}

fn apply_it(home: &Path) -> Marker {
    let plan = adapter_for("deepseek").unwrap().plan(home, &entry()).unwrap();
    apply(home, &plan).unwrap()
}

fn replaced_copies(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains(".yantrik-replaced-"))
        .collect();
    v.sort();
    v
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[test]
fn a_stray_backup_and_an_edited_live_file_are_both_kept_through_apply_and_revert() {
    let h = home("both");
    let live = h.join(".config/yantrik/deepseek.json");
    let stray = backup_path(&live);
    std::fs::write(&stray, r#"{"max_steps":1,"note":"old backup"}"#).unwrap();
    std::fs::write(&live, r#"{"max_steps":9,"note":"newer edit","api_key":"fake-live-key"}"#).unwrap();

    let marker = apply_it(&h);
    let copies = replaced_copies(live.parent().unwrap());
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert_eq!(read(&copies[0]), r#"{"max_steps":9,"note":"newer edit","api_key":"fake-live-key"}"#);
    let said = marker.replaced_note().unwrap();
    assert!(said.contains(&copies[0].display().to_string()) && said.contains("before this change"), "{said}");
    assert!(!said.contains("fake-live-key") && !said.contains(FAKE_KEY));

    let reverted = revert(&h, "deepseek").unwrap().expect("the result names the copy");
    assert!(reverted.contains(&copies[0].display().to_string()), "{reverted}");
    assert_eq!(read(&live), r#"{"max_steps":1,"note":"old backup"}"#, "the stray backup's content is restored");
    assert!(stray.exists(), "a backup this apply did not make is not deleted");
    assert_eq!(read(&copies[0]), r#"{"max_steps":9,"note":"newer edit","api_key":"fake-live-key"}"#, "the edit is still on disk");
}

#[test]
fn the_ordinary_path_writes_no_extra_file_and_says_nothing_extra() {
    let h = home("ordinary");
    let live = h.join(".config/yantrik/deepseek.json");
    std::fs::write(&live, r#"{"max_steps":3}"#).unwrap();
    let marker = apply_it(&h);
    assert!(replaced_copies(live.parent().unwrap()).is_empty());
    assert!(marker.replaced_note().is_none());
    assert_eq!(revert(&h, "deepseek").unwrap(), None);
    assert_eq!(read(&live), r#"{"max_steps":3}"#);
    assert!(!backup_path(&live).exists(), "a backup this apply made is removed by its revert");
    assert!(replaced_copies(live.parent().unwrap()).is_empty());
}

#[test]
fn a_backup_equal_to_the_live_file_needs_no_copy() {
    let h = home("equal");
    let live = h.join(".config/yantrik/deepseek.json");
    std::fs::write(&live, r#"{"a":1}"#).unwrap();
    std::fs::write(backup_path(&live), r#"{"a":1}"#).unwrap();
    assert!(apply_it(&h).replaced_note().is_none());
    assert!(replaced_copies(live.parent().unwrap()).is_empty());
}

#[test]
fn the_copy_has_the_originals_mode() {
    for mode in [0o600, 0o640] {
        let h = home(&format!("mode-{mode:o}"));
        let live = h.join(".config/yantrik/deepseek.json");
        std::fs::write(backup_path(&live), "{}").unwrap();
        std::fs::write(&live, r#"{"api_key":"fake"}"#).unwrap();
        std::fs::set_permissions(&live, std::fs::Permissions::from_mode(mode)).unwrap();
        apply_it(&h);
        let copy = &replaced_copies(live.parent().unwrap())[0];
        assert_eq!(std::fs::metadata(copy).unwrap().permissions().mode() & 0o7777, mode);
    }
}

#[test]
fn a_second_apply_never_overwrites_an_earlier_replaced_copy() {
    let h = home("twice");
    let live = h.join(".config/yantrik/deepseek.json");
    let stray = backup_path(&live);
    std::fs::write(&stray, "{\"v\":0}").unwrap();
    std::fs::write(&live, "{\"v\":1}").unwrap();
    apply_it(&h);
    revert(&h, "deepseek").unwrap();
    // The person edits again; the same stray backup is still there.
    std::fs::write(&live, "{\"v\":2}").unwrap();
    apply_it(&h);
    let copies = replaced_copies(live.parent().unwrap());
    let contents: Vec<String> = copies.iter().map(|p| read(p)).collect();
    assert_eq!(copies.len(), 2, "{copies:?}");
    assert!(contents.contains(&"{\"v\":1}".to_string()) && contents.contains(&"{\"v\":2}".to_string()), "{contents:?}");
}

#[test]
fn a_crash_after_the_backup_or_after_the_live_write_is_read_correctly_next_time() {
    // After the backup, before the live file was written: the backup equals the live file.
    let h = home("crash-1");
    let live = h.join(".config/yantrik/deepseek.json");
    std::fs::write(&live, r#"{"v":1}"#).unwrap();
    std::fs::write(backup_path(&live), r#"{"v":1}"#).unwrap();
    apply_it(&h);
    assert!(replaced_copies(live.parent().unwrap()).is_empty());
    // After the live write, before the marker: the live file is what this code wrote.
    let h = home("crash-2");
    let live = h.join(".config/yantrik/deepseek.json");
    std::fs::write(&live, r#"{"v":1}"#).unwrap();
    apply_it(&h);
    std::fs::remove_file(marker_path(&h, "deepseek")).unwrap();
    apply_it(&h);
    assert!(replaced_copies(live.parent().unwrap()).is_empty(), "its own write is not kept as the person's");
    revert(&h, "deepseek").unwrap();
    assert_eq!(read(&live), r#"{"v":1}"#);
}
