//! #1247 review N5: `dir_size` is a listing like `du` under `run_command`, and filtered like it.

use super::*;

const FAKE: &str = "FAKE-SECRET-DO-NOT-PRINT";

fn call(tool: &dyn Tool, args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = ToolContext {
        db: &db,
        max_permission: PermissionLevel::Dangerous,
        registry_metadata: None,
        task_manager: None,
        incognito: true,
        agent_spawner: None,
    };
    tool.execute(&ctx, &args)
}

#[test]
fn dir_size_counts_a_protected_folder_and_never_names_it() {
    let Ok(scratch) = yantrik_ml::private_dir::scratch_dir() else { return };
    let dir = scratch.join(format!("yantrik-1247-dirsize-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for rel in ["backup/private-keys-v1.d/ABC.key", "backup/notes-dir/a.md", "backup/b.md"] {
        let at = dir.join(rel);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, FAKE).unwrap();
    }
    let out = call(&DirSizeTool, serde_json::json!({ "path": dir.join("backup").to_string_lossy(), "depth": 3 }));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.contains("notes-dir") && out.contains("Total:"), "{out}");
    assert!(!out.contains("private-keys") && !out.contains("ABC.key") && !out.contains(FAKE), "{out}");
    assert!(out.contains("1 entries are not shown"), "{out}");
}

/// R11-M3: a walk that runs out of its time stops and says its figures are a lower bound; a walk with
/// time left says nothing of the kind.
#[test]
fn a_walk_that_runs_out_of_time_says_its_total_is_partial() {
    let Ok(scratch) = yantrik_ml::private_dir::scratch_dir() else { return };
    let dir = scratch.join(format!("yantrik-r11-partial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for n in 0..50 {
        std::fs::write(dir.join(format!("f{n}.txt")), b"x").unwrap();
    }
    let mut late = crate::disk_scan::Scan::with_limit(&dir, std::time::Duration::ZERO);
    std::thread::sleep(std::time::Duration::from_millis(2));
    let cut = crate::disk_scan::folder_sizes(&mut late, &dir, 1);
    let note = late.partial_note().unwrap_or_default();
    assert!(late.partial && note.contains("partial") && note.contains("lower bound"), "{note}");

    let mut whole = crate::disk_scan::Scan::new(&dir);
    let full = crate::disk_scan::folder_sizes(&mut whole, &dir, 1);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(whole.partial_note().is_none());
    assert!(cut.last().map(|(bytes, _)| *bytes) < full.last().map(|(bytes, _)| *bytes), "the cut walk counted less: {cut:?} / {full:?}");
}

/// The folder is read in this process (`disk_scan`), so a `$(…)` or a backtick in a name is only a name:
/// nothing is started that could run it.
#[test]
fn a_command_substitution_in_a_folder_name_stays_a_name() {
    let dir = std::env::temp_dir().join(format!("disk-inj-{}-$(touch INJECTED)`touch INJECTED`'q", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/a.txt"), vec![0u8; 5000]).unwrap();
    let largest = crate::disk_scan::largest_items(&mut crate::disk_scan::Scan::new(&dir), &dir);
    assert!(largest.iter().any(|(_, name)| name.contains("sub")), "{largest:?}");
    assert!(!dir.join("INJECTED").exists() && !std::path::Path::new("INJECTED").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
