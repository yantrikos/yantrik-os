//! #1247, review R11 (H2, M4): the companion's background tasks, its scripts and its MCP proxy do not hand
//! it a protected file. Each test runs the real tool against a scratch home that holds fake secrets.
//! This file is its own process, so setting `HOME` here races no other test.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use yantrik_companion::task_manager::TaskManager;
use yantrik_companion::tools::{background_tasks, mcp};
use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::{Tool, ToolContext, ToolRegistry};

/// Fake contents only; a refusal must never print them.
const FAKE: &str = "FAKE-SECRET-1247";

/// The scratch home, with `HOME` pointing at it, made once; and a lock so the tests take turns.
fn home() -> (PathBuf, std::sync::MutexGuard<'static, ()>) {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    static TURN: Mutex<()> = Mutex::new(());
    let guard = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let home = HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("yantrik-r11-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for (rel, text) in [("backup/tokens.json", FAKE), (".kube/config", FAKE), (".hermes.yantrik-moved/config.yaml", FAKE), ("docs/notes.md", "ordinary notes")] {
            let at = home.join(rel);
            std::fs::create_dir_all(at.parent().expect("a parent")).expect("make the folder");
            std::fs::write(at, text).expect("write the fake file");
        }
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_RUNTIME_DIR", home.join("run"));
        std::fs::create_dir_all(home.join("run")).expect("the runtime folder");
        home
    });
    (home.clone(), guard)
}

fn has_bwrap() -> bool {
    std::process::Command::new("/usr/bin/bwrap").arg("--version").output().is_ok_and(|o| o.status.success())
}

/// `code_execute` through the registry, as the companion calls it.
fn run(reg: &ToolRegistry, args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: true, agent_spawner: None };
    reg.execute(&ctx, "code_execute", &args)
}

/// `run_background` and `check_background_task` on a fake `tokens.json`, in every spelling a shell allows.
#[test]
fn run_background_runs_only_what_run_command_may_and_never_reads_a_secret() {
    let (home, _turn) = home();
    let tasks = Mutex::new(TaskManager::new());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    TaskManager::ensure_table(&db.conn());
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: Some(&tasks), incognito: true, agent_spawner: None };
    let start = |command: &str| background_tasks::RunBackgroundTool.execute(&ctx, &serde_json::json!({"command": command, "label": "x"}));
    for command in [
        format!("head -c 400 {}/backup/tokens.json", home.display()),
        "head -c 400 ~/backup/tokens.json".to_string(),
        "tail ~/.hermes.yantrik-moved/config.yaml".to_string(),
        "cat ~/.kube/config".to_string(),
        "sh -c 'cat ~/backup/tokens.json'".to_string(),
        "cat ~/backup/tokens.json | head".to_string(),
        "python3 -c 'print(open(\"/etc/hostname\").read())'".to_string(),
    ] {
        let said = start(&command);
        assert!(said.starts_with("Command blocked"), "{command}: {said}");
        assert!(!said.contains("FAKE"), "{command}: {said}");
    }
    assert!(tasks.lock().expect("the task manager").list(&db.conn(), None).is_empty(), "no task was recorded for a refused line");

    // A line `run_command` runs is started, directly, and its output is shown.
    let said = start("echo hello-from-a-task");
    assert!(said.starts_with("Task started"), "{said}");
    let id = said.split_whitespace().nth(2).expect("an id").to_string();
    let mut shown = String::new();
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        shown = background_tasks::CheckBackgroundTaskTool.execute(&ctx, &serde_json::json!({"task_id": id}));
        if shown.contains("hello-from-a-task") {
            break;
        }
    }
    assert!(shown.contains("hello-from-a-task"), "{shown}");
}

/// N-H1: what `check_background_task` and the completion notice show of a listing is what `run_command`
/// shows: no protected name, in any spelling, and a count in its place. The notice reads the same
/// `TaskManager::output_of`.
#[test]
fn a_tasks_listing_names_no_protected_file_and_keeps_the_rest() {
    let (home, _turn) = home();
    for rel in ["backup/tokens (copy 1).json", "backup/notes.md"] {
        std::fs::write(home.join(rel), "ordinary").expect("a file");
    }
    let tasks = Mutex::new(TaskManager::new());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    TaskManager::ensure_table(&db.conn());
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: Some(&tasks), incognito: true, agent_spawner: None };
    for command in ["find ~/backup", "du -a ~/backup", "ls -la ~/backup"] {
        let said = background_tasks::RunBackgroundTool.execute(&ctx, &serde_json::json!({"command": command, "label": "x"}));
        if !said.starts_with("Task started") {
            // The gate itself may refuse a line that names a protected file: then nothing ran, nothing shows.
            assert!(said.starts_with("Command blocked") && !said.contains("FAKE"), "{command}: {said}");
            continue;
        }
        let id = said.split_whitespace().nth(2).expect("an id").to_string();
        let mut checked = String::new();
        for _ in 0..80 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            checked = background_tasks::CheckBackgroundTaskTool.execute(&ctx, &serde_json::json!({"task_id": id}));
            if checked.contains("Status: completed") || checked.contains("Status: failed") {
                break;
            }
        }
        let output = TaskManager::output_of(&db.conn(), &id, 50);
        let shown_by_check = checked.lines().filter(|l| !l.starts_with("Command:")).collect::<Vec<_>>().join("\n");
        for text in [&output, &shown_by_check] {
            assert!(!text.contains("tokens") && !text.contains(FAKE), "{command}: {text}");
        }
        assert!(output.contains("notes.md") && output.contains("entries are not shown"), "{command}: {output}");
    }
}

/// R12-H2: the tail is taken after the filter. `ls -R ~/Pictures` with 60 files in `Pictures/Screenshots`
/// is longer than the 20 lines the completion notice shows, so a tail taken first starts inside the
/// protected folder's section with its header gone and shows its names; `run_command` shows none.
#[test]
fn a_recursive_listing_loses_no_header_to_the_tail() {
    let (home, _turn) = home();
    std::fs::create_dir_all(home.join("Pictures/Screenshots")).expect("a folder");
    std::fs::create_dir_all(home.join("Pictures/holiday")).expect("a folder");
    for n in 0..60 {
        std::fs::write(home.join(format!("Pictures/Screenshots/payslip-acme-{n:02}.png")), "x").expect("a file");
    }
    std::fs::write(home.join("Pictures/holiday/beach.jpg"), "x").expect("a file");
    let tasks = Mutex::new(TaskManager::new());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    TaskManager::ensure_table(&db.conn());
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: Some(&tasks), incognito: true, agent_spawner: None };
    for command in ["ls -R ~/Pictures", "ls -laR ~/Pictures", "ls ~/backup/tokens.json"] {
        let said = background_tasks::RunBackgroundTool.execute(&ctx, &serde_json::json!({"command": command, "label": "x"}));
        if !said.starts_with("Task started") {
            assert!(said.starts_with("Command blocked") && !said.contains("FAKE"), "{command}: {said}");
            continue;
        }
        let id = said.split_whitespace().nth(2).expect("an id").to_string();
        let mut checked = String::new();
        for _ in 0..80 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            checked = background_tasks::CheckBackgroundTaskTool.execute(&ctx, &serde_json::json!({"task_id": id}));
            if checked.contains("Status: completed") || checked.contains("Status: failed") {
                break;
            }
        }
        let twenty = TaskManager::output_of(&db.conn(), &id, 20);
        let fifty = TaskManager::output_of(&db.conn(), &id, 50);
        for text in [&twenty, &fifty, &checked] {
            assert!(!text.contains("payslip") && !text.contains("tokens"), "{command}: {text}");
        }
        if command.contains("Pictures") {
            assert!(fifty.contains("entries are not shown"), "{command}: the count line");
        }
    }
}

/// A task row from before the gate (or one no vetted start made) shows no output, whatever its file holds.
#[test]
fn a_task_that_was_not_vetted_shows_no_output() {
    let (_home, _turn) = home();
    let tasks = Mutex::new(TaskManager::new());
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    TaskManager::ensure_table(&db.conn());
    // The old table had no `vetted` column: a row written by it has the default.
    db.conn().execute("INSERT INTO background_tasks (task_id, command, label, status, output_file, started_at) VALUES ('t9999', 'cat x', 'old', 'completed', '', 0)", []).expect("an old row");
    {
        use std::io::Write;
        let mut file = yantrik_ml::private_dir::create_scratch("yantrik-task-t9999.out").expect("a scratch file");
        writeln!(file, "{FAKE}").expect("write the output");
    }
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: Some(&tasks), incognito: true, agent_spawner: None };
    let said = background_tasks::CheckBackgroundTaskTool.execute(&ctx, &serde_json::json!({"task_id": "t9999"}));
    assert!(!said.contains("FAKE") && said.contains("No output yet"), "{said}");
    assert_eq!(TaskManager::output_of(&db.conn(), "t9999", 20), "");
}

/// `sandbox: "none"` and `"unsafe"` are refused for both script tools; the strict sandbox either runs
/// inside `bwrap` (where the home is not bound) or, with no `bwrap`, runs nothing.
#[test]
fn a_script_runs_in_the_sandbox_or_not_at_all() {
    let (home, _turn) = home();
    // A turn the person started: the reach rule then lets a script run at all, so it is the sandbox rule
    // that answers.
    let _person = yantrik_companion_core::background_research::person_turn();
    let reads_the_secret = format!("print(open('{}/backup/tokens.json').read())", home.display());
    let mut reg = ToolRegistry::new();
    yantrik_companion_tools::coder::register(&mut reg);
    let run = |args: serde_json::Value| run(&reg, args);
    for mode in ["none", "unsafe"] {
        let said = run(serde_json::json!({"language": "python", "code": reads_the_secret, "sandbox": mode}));
        assert!(said.starts_with("Access denied") && !said.contains("FAKE"), "{mode}: {said}");
    }
    let said = run(serde_json::json!({"language": "python", "code": reads_the_secret}));
    assert!(!said.contains("FAKE"), "the strict sandbox, or nothing: {said}");
    if !has_bwrap() {
        assert!(said.contains("Refused:"), "no bwrap, so no script runs: {said}");
    }
}

/// A fake MCP file server: `read_file` answers with the file's text; every call it hears is written to `calls`.
const SERVER: &str = r#"
import json, os, sys
calls = os.environ["CALLS"]
for line in sys.stdin:
    msg = json.loads(line)
    if "id" not in msg:
        continue
    method, id = msg["method"], msg["id"]
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "fake"}}
    elif method == "tools/list":
        schema = {"type": "object", "properties": {"path": {"type": "string"}}}
        result = {"tools": [{"name": "read_file", "description": "read", "inputSchema": schema}]}
    else:
        args = msg["params"]["arguments"]
        with open(calls, "a") as f:
            f.write(json.dumps(msg["params"]) + "\n")
        text = open(os.path.expanduser(args["path"])).read()
        result = {"content": [{"type": "text", "text": text}]}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": id, "result": result}) + "\n")
    sys.stdout.flush()
"#;

#[test]
fn the_mcp_proxy_refuses_a_protected_path_and_passes_a_file_through_whole() {
    let (home, _turn) = home();
    let _person = yantrik_companion_core::background_research::person_turn();
    if std::process::Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let dir = home.join("mcp");
    std::fs::create_dir_all(&dir).expect("the server folder");
    let (script, calls) = (dir.join("server.py"), dir.join("calls.log"));
    std::fs::write(&script, SERVER).expect("the server");
    let mut reg = ToolRegistry::new();
    let entry: yantrik_companion_core::config::McpServerEntry = serde_json::from_value(serde_json::json!({
        "id": "fake", "command": "python3", "args": [script.to_string_lossy()], "env": {"CALLS": calls.to_string_lossy()},
    }))
    .expect("a server entry");
    mcp::register_mcp_servers(&mut reg, &[entry]);
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: true, agent_spawner: None };
    let heard = || std::fs::read_to_string(&calls).unwrap_or_default();

    for path in ["~/backup/tokens.json", "~/.kube/config", "~/.hermes.yantrik-moved/config.yaml", &format!("{}/backup/tokens.json", home.display()), &format!("file://{}/.kube/config", home.display())] {
        let said = reg.execute(&ctx, "mcp__fake_read_file", &serde_json::json!({"path": path}));
        assert_eq!(said, "Access denied: path resolves to a protected location", "{path}");
    }
    assert_eq!(heard(), "", "the server never heard a refused call");

    let said = reg.execute(&ctx, "mcp__fake_read_file", &serde_json::json!({"path": "~/docs/notes.md"}));
    assert_eq!(said, "ordinary notes", "an ordinary file still reads");

    // R12-M1: what a server answers is passed through as it is (named in SECURITY.md, #1266). A file whose
    // lines mention secret names comes back whole, so the person loses nothing of an ordinary file.
    let doc = home.join("docs/setup.md");
    std::fs::write(&doc, "Copy .env.example to .env before running.\nAdd your key to ~/.ssh/id_ed25519.pub and run ssh-add.\nThen run make.\n").expect("a decoy");
    let said = reg.execute(&ctx, "mcp__fake_read_file", &serde_json::json!({"path": "~/docs/setup.md"}));
    assert_eq!(said, "Copy .env.example to .env before running.\nAdd your key to ~/.ssh/id_ed25519.pub and run ssh-add.\nThen run make.\n", "a file read through a server comes back whole: {said}");
}

