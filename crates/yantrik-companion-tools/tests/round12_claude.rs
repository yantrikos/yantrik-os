//! #1247, round 12 (N-H2): `claude_code` is closed for the companion, and `claude_think` starts Claude Code
//! with no tools. A fake `claude` first on `PATH` writes each argument it is started with, one a line, to a
//! file, so the test sees what would have started. This file is its own process, but its two tests run on
//! parallel threads and `PATH` and `HOME` are the process's: each test holds `ENV` for its whole body, and the
//! first puts both back when it ends, so the second never finds the fake and takes it for a real `claude`.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

const REFUSED: &str = "Access denied: Claude Code would run as you and read protected locations, and nothing asks you first";

fn execute(reg: &ToolRegistry, tool: &str, args: serde_json::Value) -> String {
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: true, agent_spawner: None };
    reg.execute(&ctx, tool, &args)
}

/// Held by each test for its whole body: the process's `PATH` and `HOME` belong to one test at a time.
static ENV: Mutex<()> = Mutex::new(());

/// Puts `PATH` and `HOME` back as they were when dropped, also when the test panics.
struct RestoreEnv(Vec<(&'static str, Option<OsString>)>);

impl RestoreEnv {
    fn save() -> Self {
        Self(["PATH", "HOME"].into_iter().map(|k| (k, std::env::var_os(k))).collect())
    }
}

impl Drop for RestoreEnv {
    fn drop(&mut self) {
        for (k, v) in &self.0 {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}

#[test]
fn claude_code_starts_nothing_and_claude_think_starts_with_no_tools() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = RestoreEnv::save();
    // The fake `claude` and its log are outside the test's home and outside `/tmp`: a command's view
    // holds a fresh home and a private `/tmp` (#1265), so a program kept in either is not in it.
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("yantrik-r12-claude-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).expect("a folder");
    std::fs::create_dir_all(dir.join("home")).expect("a home");
    let argv_file = dir.join("argv.txt");
    let fake = dir.join("bin/claude");
    std::fs::write(&fake, format!("#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> {}; done\necho answered\n", argv_file.display())).expect("a fake claude");
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("executable");
    std::env::set_var("HOME", dir.join("home"));
    // The views take the account's home, which this test makes its own folder (this thread's).
    yantrik_ipc_contracts::home_paths::outside_view::account_home_for_tests(dir.join("home").canonicalize().ok());
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
    assert_eq!(argv, ["-p", "--tools", "", "--strict-mcp-config", "--settings", r#"{"permissions":{"deny":["Read"]}}"#, "--", "--allowedTools Bash please"].map(String::from), "no tools, no MCP servers, Read denied, the prompt after `--`, no --no-input");
    assert!(!argv.iter().any(|a| a == "--allowedTools" || a == "--no-input"), "{argv:?}");
    yantrik_ipc_contracts::home_paths::outside_view::account_home_for_tests(None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Gated on a real `claude` on `PATH`: run it as `claude_think` does against a local fake API and assert the
/// request body holds no `FAKE`, however the file is named with `@`. Skips with a printed line without one.
#[test]
fn a_real_claude_sends_no_file_named_with_an_at_mention() {
    use std::io::{Read, Write};
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let real = std::env::var("PATH").unwrap_or_default().split(':').map(|d| std::path::Path::new(d).join("claude")).find(|p| p.is_file());
    let Some(real) = real else { println!("SKIPPED: no `claude` on PATH, the real-CLI check did not run"); return };
    let dir = std::env::temp_dir().join(format!("yantrik-r13-claude-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("backup")).expect("a folder");
    std::fs::write(dir.join("backup/tokens.json"), "FAKE-SECRET-1247\n").expect("a fake file");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("addr").port();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let sink = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
            while let Ok(n) = stream.read(&mut chunk) { if n == 0 { break } buf.extend_from_slice(&chunk[..n]); }
            if let Ok(mut s) = sink.lock() { s.push_str(&String::from_utf8_lossy(&buf)); }
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
        }
    });
    let file = dir.join("backup/tokens.json");
    for prompt in [format!("Say hi @{}", file.display()), "Say hi @backup/tokens.json".to_string(), format!("Say hi @\"{}\"", file.display())] {
        let words = ["-p", "--tools", "", "--strict-mcp-config", "--settings", yantrik_companion_tools::claude::CLAUDE_THINK_SETTINGS, "--", &prompt];
        let _ = std::process::Command::new(&real).args(words).current_dir(&dir).env("HOME", &dir)
            .env("ANTHROPIC_API_KEY", "dummy").env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1").stdin(std::process::Stdio::null()).output();
    }
    let body = seen.lock().expect("lock").clone();
    assert!(!body.contains("FAKE"), "a real claude sent the file named with @");
    let _ = std::fs::remove_dir_all(&dir);
}
