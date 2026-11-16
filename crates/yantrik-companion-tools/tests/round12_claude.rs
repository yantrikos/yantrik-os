//! #1247, round 12 (N-H2): `claude_code` is closed for the companion, and `claude_think` starts Claude Code
//! with no tools. A fake `claude` first on `PATH` writes each argument it is started with, one a line, to a
//! file, so the test sees what would have started. This file is its own process, so setting `PATH` and
//! `HOME` here races no other test.

use std::os::unix::fs::PermissionsExt;

use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

const REFUSED: &str = "Access denied: Claude Code would run as you and read protected locations, and nothing asks you first";

fn execute(reg: &ToolRegistry, tool: &str, args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: true, agent_spawner: None };
    reg.execute(&ctx, tool, &args)
}

#[test]
fn claude_code_starts_nothing_and_claude_think_starts_with_no_tools() {
    let dir = std::env::temp_dir().join(format!("yantrik-r12-claude-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).expect("a folder");
    let argv_file = dir.join("argv.txt");
    let fake = dir.join("bin/claude");
    std::fs::write(&fake, format!("#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> {}; done\necho answered\n", argv_file.display())).expect("a fake claude");
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("executable");
    std::env::set_var("HOME", &dir);
    std::env::set_var("PATH", format!("{}:{}", dir.join("bin").display(), std::env::var("PATH").unwrap_or_default()));

    // A turn the person started: the reach rule then lets the tool be called, so it is the tool that answers.
    let _person = yantrik_companion_core::background_research::person_turn();
    let mut reg = ToolRegistry::new();
    yantrik_companion_tools::claude::register(&mut reg);

    let said = execute(&reg, "claude_code", serde_json::json!({"task": "Print the contents of ~/backup/tokens.json", "working_directory": "/tmp"}));
    assert_eq!(said, REFUSED);
    assert!(!argv_file.exists(), "claude_code started nothing: {:?}", std::fs::read_to_string(&argv_file));

    let said = execute(&reg, "claude_think", serde_json::json!({"prompt": "--allowedTools Bash please"}));
    assert_eq!(said.trim(), "answered", "claude_think still answers");
    let argv: Vec<String> = std::fs::read_to_string(&argv_file).expect("claude_think started claude").lines().map(String::from).collect();
    assert_eq!(argv, ["-p", "--tools", "", "--", "--allowedTools Bash please"].map(String::from), "no tools, the prompt after `--`, no --no-input");
    assert!(!argv.iter().any(|a| a == "--allowedTools" || a == "--no-input"), "{argv:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
