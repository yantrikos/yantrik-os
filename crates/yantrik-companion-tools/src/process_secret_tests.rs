//! #1247, gate 12 audit: `list_processes` prints no file a secret's name is in. The real tool, on a real
//! process whose command line names a fake credential, since a process says what it has open.

use super::*;

fn call(args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = crate::ToolContext {
        db: &db,
        max_permission: crate::PermissionLevel::Dangerous,
        registry_metadata: None,
        task_manager: None,
        incognito: true,
        agent_spawner: None,
    };
    ListProcessesTool.execute(&ctx, &args)
}

#[cfg(unix)]
#[test]
fn a_process_with_a_secret_in_its_command_line_is_listed_without_it() {
    // `sh -c 'sleep 20; :' NAME`: the name is the shell's $0, so it is in the command line `ps` prints (the
    // `; :` keeps the shell from replacing itself with `sleep`, which would drop the name).
    let Ok(mut child) = std::process::Command::new("sh").args(["-c", "sleep 20; :", "/home/ann/backup/tokens.json"]).spawn() else { return };
    let listed = call(serde_json::json!({ "limit": 5000 }));
    let by_secret = call(serde_json::json!({ "limit": 5000, "filter": "tokens.json" }));
    let _ = child.kill();
    let _ = child.wait();
    if listed.starts_with("Error") || listed.starts_with("ps failed") {
        return;
    }
    let row = listed.lines().find(|row| row.split_whitespace().nth(1) == Some(&child.id().to_string())).unwrap_or_default();
    assert!(row.contains("command hidden"), "the shell's own row, by its pid: {listed}");
    assert!(!listed.contains("tokens.json"), "{listed}");
    assert!(by_secret.starts_with("No processes matching"), "a filter cannot ask whether a process has the file: {by_secret}");
}

#[test]
fn an_ordinary_row_is_listed_whole() {
    let row = "ann 1234 0.5 1.2 1000 2000 pts/0 S 10:00 0:01 nvim /home/ann/Documents/notes.md";
    assert_eq!(shown_row(row), row);
    let hidden = shown_row("ann 1234 0.5 1.2 1000 2000 pts/0 S 10:00 0:01 nvim /home/ann/backup/tokens.json");
    assert!(hidden.starts_with("ann 1234 0.5 1.2 1000 2000 pts/0 S 10:00 0:01 (command hidden") && !hidden.contains("tokens"), "{hidden}");
}
