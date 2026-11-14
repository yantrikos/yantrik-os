//! #1201: the copy of the person's live file has the original's mode. (The rest of the rule is the
//! shared table, rule_table_tests.rs; interrupted applies are in retry_tests.rs.)

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
