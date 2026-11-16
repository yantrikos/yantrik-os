//! #1247, the VM 520 gate (gate 12): what the real build showed that the unit tests had not, held
//! here with fake files. `list_files` printed every secret name; `analyze_disk` printed parts of
//! two; `run_command cat` read a committed blob from `.git/objects`; and the folder an update
//! renames the person's old Hermes folder to was an ordinary folder.

use crate::desktop::ListFilesTool;
use crate::disk::AnalyzeDiskTool;

const FAKE: &str = "FAKE-SECRET-DO-NOT-PRINT";

fn call(tool: &dyn crate::Tool, args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = crate::ToolContext {
        db: &db,
        max_permission: crate::PermissionLevel::Dangerous,
        registry_metadata: None,
        task_manager: None,
        incognito: true,
        agent_spawner: None,
    };
    tool.execute(&ctx, &args)
}

/// A folder in the tools' scratch directory, where `validate_path` lets a tool look.
struct Folder(std::path::PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn folder(tag: &str, files: &[&str]) -> Option<Folder> {
    let dir = yantrik_ml::private_dir::scratch_dir().ok()?.join(format!("yantrik-1247-g12-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for rel in files {
        let at = dir.join(rel);
        std::fs::create_dir_all(at.parent()?).ok()?;
        std::fs::write(at, FAKE).ok()?;
    }
    Some(Folder(dir))
}

const SECRETS: [&str; 5] = ["tokens.json", "server.pem", "id_ed25519", "tokens (copy 1).json", "tokens.json:Zone.Identifier"];
const DECOYS: [&str; 3] = ["notes.md", "keynote.txt", "notes-about-tokens.md"];

#[test]
fn list_files_counts_the_secrets_and_lists_the_decoys() {
    let all: Vec<&str> = SECRETS.iter().chain(DECOYS.iter()).copied().collect();
    let Some(dir) = folder("list", &all) else { return };
    let out = call(&ListFilesTool, serde_json::json!({ "path": dir.0.to_string_lossy() }));
    for decoy in DECOYS {
        assert!(out.contains(decoy), "{decoy}: {out}");
    }
    for name in ["tokens.json", "server.pem", "id_ed25519", "copy 1", "Zone.Identifier"] {
        assert!(!out.contains(name), "{name}: {out}");
    }
    assert!(out.contains("5 entries are not shown") && !out.contains(FAKE), "{out}");
    // A pattern asks for a subset, and only what it matches is counted.
    let pem = call(&ListFilesTool, serde_json::json!({ "path": dir.0.to_string_lossy(), "pattern": "*.pem" }));
    assert!(pem.contains("No files matching") && pem.contains("1 entries are not shown") && !pem.contains("server"), "{pem}");
}

#[test]
fn analyze_disk_shows_no_part_of_a_secrets_name() {
    let names = ["tokens.json:Zone.Identifier", "tokens.json.sync-conflict-20261115-123456-ABCDEFG", "pad/a.csv", "pad/b.csv", "notes.md"];
    let Some(dir) = folder("analyze", &names) else { return };
    let out = call(&AnalyzeDiskTool, serde_json::json!({ "path": dir.0.to_string_lossy() }));
    assert!(out.contains("csv") && out.contains("md"), "{out}");
    for part in ["Identifier", "sync-conflict", "ABCDEFG", "tokens"] {
        assert!(!out.contains(part), "{part}: {out}");
    }
}

#[test]
fn run_command_cannot_cat_a_git_object() {
    let Some(dir) = folder("git", &[".git/objects/0b/0123456789abcdef", ".git/HEAD", "notes.md"]) else { return };
    let at = |rel: &str| dir.0.join(rel).to_string_lossy().into_owned();
    assert!(crate::safe_command::protected(&at(".git/objects/0b/0123456789abcdef")));
    assert!(crate::safe_command::protected(&at(".git/objects")));
    assert!(!crate::safe_command::protected(&at(".git/HEAD")), "the rest of .git stays readable");
    assert!(!crate::safe_command::protected(&at("objects/notes.md")));
}

#[test]
fn the_moved_hermes_folder_is_as_protected_as_the_hermes_folder() {
    for p in ["/home/ann/.hermes.yantrik-moved/config.yaml", "/home/ann/.hermes.yantrik-moved/hermes-agent/x/y.py"] {
        assert!(yantrik_ipc_contracts::home_paths::is_protected(std::path::Path::new(p)), "{p}");
        assert!(crate::safe_command::protected(p), "{p}");
    }
    assert!(!yantrik_ipc_contracts::home_paths::is_protected(std::path::Path::new("/home/ann/.hermes-notes/x")));
}
