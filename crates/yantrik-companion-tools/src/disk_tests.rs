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
